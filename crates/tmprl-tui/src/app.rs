//! Application state and the reducer.
//!
//! The one rule: [`App::handle`] is synchronous and never awaits. When it needs data it
//! spawns a task, which reports back as another [`Msg`]. Nothing on the keystroke path can
//! block on the network — see `docs/ARCHITECTURE.md`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tmprl_client::{Codec, Conn, NamespaceInfo};
use tmprl_core::ScheduleRow;
use tmprl_core::history::{NormalizedEvent, group_events, merge_events};
use tmprl_core::mutation::{Confirm, Mutation};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::Payload;
use tmprl_core::{
    Action, Chord, Keymap, Loadable, Mode, Pending, PendingEntry, Registry, Resolution, SavedView,
    StatusCounts, WorkflowList, WorkflowRow, default_keymap,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::view::View;
use tmprl_ui::{Axis, Direction, Rect as UiRect, Tabs, ViewId};

/// Rows fetched per namespace per page. Large enough that scrolling rarely waits, small
/// enough that the first screen arrives promptly on a slow link.
const PAGE_SIZE: i32 = 50;

/// History events per page. Larger than the workflow page because events are small and a
/// history is read top to bottom, so the first screen wants plenty behind it.
const HISTORY_PAGE_SIZE: i32 = 500;

/// Continuation tokens, one per namespace that still has pages. The client owns the shape;
/// this is an alias so the reducer reads the same way.
use tmprl_client::Continuation as Tokens;

/// Which list is on screen. Temporal's objects form a hierarchy and `-` walks up it, so
/// this is a level rather than a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Namespaces,
    Workflows,
    History,
    Schedules,
}

/// Which mutation a key asked for, before it is turned into a `Mutation` with a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
    PauseSchedule,
    TriggerSchedule,
    DeleteSchedule,
    Cancel,
    Terminate,
    Signal,
    Delete,
    Reset,
    Update,
}

/// What a prompt at the bottom of the screen is collecting.
///
/// Both prompts edit identically — the same keys, the same backspace-on-empty-closes rule —
/// and differ only in what Enter does with the text. Sharing the editing is what keeps them
/// from drifting apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// `:` — a command id, with completions.
    Command,
    /// `!` — a shell command to filter the focused payloads through.
    Pipe,
    /// The name of a signal to send.
    Signal,
    /// The name of an update to send.
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub buf: String,
}

impl Prompt {
    /// What is drawn to the left of the text.
    pub fn sigil(&self) -> &'static str {
        match self.kind {
            PromptKind::Command => ":",
            PromptKind::Pipe => "!",
            PromptKind::Signal => "signal:",
            PromptKind::Update => "update:",
        }
    }
}

/// Where an encrypted payload has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeState {
    /// No codec server is configured, so it cannot be read at all.
    NoCodec,
    /// A decode is out.
    InFlight,
    /// A codec is configured but nothing has been asked yet.
    Idle,
}

/// What Insert mode is editing.
///
/// On the workflow list, Insert mode edits the visibility query — that is the only text
/// field on the screen, so making `i` mean anything else would be a wasted key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertTarget {
    Scratch,
    Query,
}

/// Everything that can change the application state.
#[derive(Debug)]
pub enum Msg {
    Key(Chord),
    Tick,
    Redraw,
    Quit,
    Namespaces(Result<Vec<NamespaceInfo>, String>),
    /// A page of the workflow list. `generation` is the query this was issued for; a reply
    /// for a superseded query is dropped rather than pasted over the current one.
    Workflows {
        generation: u64,
        append: bool,
        result: Result<(Vec<WorkflowRow>, Tokens), String>,
    },
    Counts {
        generation: u64,
        result: Result<StatusCounts, String>,
    },
    Schedules {
        generation: u64,
        result: Result<Vec<ScheduleRow>, String>,
    },
    /// Output of an external command a `!` filter ran.
    Piped(Result<String, String>),
    /// A mutation finished, one way or the other.
    Mutated {
        mutation: Box<Mutation>,
        result: Result<(), String>,
    },
    /// Payloads a codec server decoded, paired with the hash of what was sent.
    Decoded(Result<Vec<(u64, Payload)>, String>),
    /// A page of a workflow's history, already normalised by the client.
    History {
        generation: u64,
        result: Result<(Vec<NormalizedEvent>, Vec<u8>), String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Info,
    Warn,
    Error,
}

pub struct App {
    /// The focused pane's state, held directly rather than looked up.
    ///
    /// Every command in the reducer acts on the focused window, so keeping it here means
    /// the whole reducer reads `self.view` without a lookup that could fail. The other
    /// panes wait in `parked`, and focus changes swap between the two.
    pub view: View,
    /// The panes that are not focused, by id.
    parked: std::collections::HashMap<ViewId, View>,
    /// The window tree: which panes exist, where they sit, which is focused.
    pub tabs: Tabs,
    /// Ids are handed out and never reused, so a stale reference cannot silently resolve
    /// to a different pane.
    next_view_id: u64,
    /// The body area the panes were last laid out in, so focus movement is geometric
    /// against what is actually on screen.
    frame: UiRect,

    pub mode: Mode,
    pub pending: Pending,
    pub registry: Registry,
    pub keymap: Keymap,
    /// Payloads a codec server has already decoded, keyed by the hash of the encrypted
    /// bytes. Decoding is a network hop per payload, and scrolling back over a row that has
    /// already been decoded should cost nothing.
    decoded: HashMap<u64, Payload>,
    /// Requests in flight, so a cursor resting on a row does not ask repeatedly.
    decoding: HashSet<u64>,
    codec: Option<Arc<Codec>>,
    pub views: Vec<SavedView>,

    pub which_key: Vec<PendingEntry>,
    pub show_help: bool,
    /// First visible line of the help overlay, and the largest useful value for it. The
    /// overlay is taller than most terminals now, so it scrolls with the ordinary motions
    /// rather than silently clipping the last groups.
    pub help_scroll: usize,
    pub help_max_scroll: usize,
    /// `Some` while a `:` or `!` prompt is open.
    pub prompt: Option<Prompt>,
    /// `Some` while a destructive action is waiting to be confirmed. Nothing has happened to
    /// the cluster while this is set.
    pub confirm: Option<Confirm>,
    pub insert_buf: String,
    pub insert_target: InsertTarget,

    pub note: Option<(String, Note)>,
    pub should_quit: bool,
    pub dirty: bool,

    profile: String,
    namespace: String,
    conn: Option<Arc<Conn>>,
    tx: UnboundedSender<Msg>,
}

impl App {
    pub fn new(conn: Conn, tx: UnboundedSender<Msg>) -> Self {
        let (profile, namespace) = (conn.profile().to_string(), conn.namespace().to_string());
        Self::build(Some(Arc::new(conn)), profile, namespace, tx)
    }

    /// An app with no connection, for tests. Every command except the ones that fetch
    /// behaves identically, which is what makes the interface testable without a Temporal
    /// server.
    #[cfg(test)]
    pub fn detached(profile: &str, namespace: &str, tx: UnboundedSender<Msg>) -> Self {
        Self::build(None, profile.to_string(), namespace.to_string(), tx)
    }

    fn build(
        conn: Option<Arc<Conn>>,
        profile: String,
        namespace: String,
        tx: UnboundedSender<Msg>,
    ) -> Self {
        Self {
            view: View::new(&namespace),
            parked: std::collections::HashMap::new(),
            tabs: Tabs::new(ViewId(0)),
            next_view_id: 1,
            frame: UiRect::new(0, 0, 80, 24),
            mode: Mode::Normal,
            pending: Pending::default(),
            registry: Registry::builtin(),
            keymap: default_keymap(),
            decoded: HashMap::new(),
            decoding: HashSet::new(),
            codec: None,
            views: Vec::new(),
            which_key: Vec::new(),
            show_help: false,
            help_scroll: 0,
            help_max_scroll: 0,
            prompt: None,
            confirm: None,
            insert_buf: String::new(),
            insert_target: InsertTarget::Scratch,
            note: None,
            should_quit: false,
            dirty: true,
            profile,
            namespace,
            conn,
            tx,
        }
    }

    /// Install the user's `keys.toml` and `views.toml`. Called once at startup, before the
    /// first frame, so the help overlay and which-key describe the keymap actually in use.
    pub fn apply_config(&mut self, keys: Option<&str>, views: Option<&str>, config: Option<&str>) {
        if let Some(src) = config {
            match tmprl_core::config::parse_config(src) {
                Ok(cfg) => {
                    self.codec = cfg.codec.map(|c| Arc::new(Codec::new(c.endpoint, c.auth)));
                }
                Err(e) => self.note = Some((e.to_string(), Note::Error)),
            }
        }
        if let Some(src) = views {
            match tmprl_core::config::parse_views(src) {
                Ok(v) => {
                    self.registry.add_views(&v);
                    if let Err(e) = tmprl_core::config::bind_views(&v, &mut self.keymap) {
                        self.note = Some((e.to_string(), Note::Error));
                    }
                    self.views = v;
                }
                Err(e) => self.note = Some((e.to_string(), Note::Error)),
            }
        }
        // Keys are applied after views so that a user binding can override a view's
        // default `<leader>N` slot.
        if let Some(src) = keys
            && let Err(e) = tmprl_core::config::apply_keys(src, &self.registry, &mut self.keymap)
        {
            self.note = Some((e.to_string(), Note::Error));
        }
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn namespace_rows(&self) -> &[NamespaceInfo] {
        self.view
            .namespaces
            .value()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn workflow_rows(&self) -> &[WorkflowRow] {
        self.view
            .workflows
            .value()
            .map(WorkflowList::rows)
            .unwrap_or(&[])
    }

    pub fn row_count(&self) -> usize {
        self.view.row_count()
    }

    pub fn is_editing_query(&self) -> bool {
        self.mode == Mode::Insert && self.insert_target == InsertTarget::Query
    }

    /// The query text to show: the live edit while Insert mode owns it, otherwise what is
    /// applied. Used by the tests and by anything that needs the focused pane's query.
    pub fn query_display(&self) -> &str {
        if self.is_editing_query() {
            &self.insert_buf
        } else {
            &self.view.query
        }
    }

    /// The inclusive row range selected in the focused pane, if a visual mode is active.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.view.selection()
    }

    // ── the reducer ──────────────────────────────────────────────────────────

    pub fn handle(&mut self, msg: Msg) {
        self.dirty = true;
        match msg {
            Msg::Key(chord) => self.on_key(chord),
            Msg::Quit => self.should_quit = true,
            Msg::Tick | Msg::Redraw => {}
            Msg::Mutated { mutation, result } => {
                let outcome = match &result {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("failed: {e}"),
                };
                self.audit(&mutation, &outcome);
                match result {
                    Ok(()) => {
                        self.note = Some((
                            format!("{} {}", mutation.past_tense(), mutation.workflow_id()),
                            Note::Info,
                        ));
                        // ListSchedules is eventually consistent, so the refresh below can
                        // still return the old state. The server has accepted the change, so
                        // reflect it locally rather than showing a screen that contradicts
                        // the message next to it.
                        if let Mutation::PauseSchedule {
                            schedule_id,
                            paused,
                            ..
                        } = mutation.as_ref()
                            && let Some(rows) = self.view.schedules.value_mut()
                            && let Some(row) =
                                rows.iter_mut().find(|r| &r.schedule_id == schedule_id)
                        {
                            row.paused = *paused;
                        }
                        // The list is now out of date about the thing just changed.
                        self.refresh();
                    }
                    Err(e) => self.note = Some((e, Note::Error)),
                }
            }
            Msg::Piped(result) => {
                self.view.piped = Some(result);
                self.view.detail_scroll = 0;
            }
            Msg::Decoded(Ok(pairs)) => {
                for (key, payload) in pairs {
                    self.decoding.remove(&key);
                    self.decoded.insert(key, payload);
                }
                self.apply_decoded();
            }
            Msg::Decoded(Err(e)) => {
                // Clearing the in-flight set is what lets a retry happen at all; leaving it
                // populated would make one failure permanent for the session.
                self.decoding.clear();
                self.note = Some((e, Note::Error));
            }
            Msg::Namespaces(Ok(list)) => {
                self.view.namespaces = Loadable::loaded(list);
                self.clamp_cursor();
            }
            Msg::Namespaces(Err(e)) => {
                self.note = Some((e.clone(), Note::Error));
                self.view.namespaces = Loadable::Failed(e);
            }
            Msg::Workflows {
                generation,
                append,
                result,
            } => {
                if generation != self.view.generation {
                    return; // a reply for a query the user has already replaced
                }
                self.view.loading_more = false;
                match result {
                    Ok((rows, tokens)) => {
                        match (append, self.view.workflows.value_mut()) {
                            (true, Some(list)) => list.append(rows, tokens),
                            _ => {
                                let mut list = WorkflowList::default();
                                list.reset(rows, tokens);
                                self.view.workflows = Loadable::loaded(list);
                            }
                        }
                        // A later page can carry a payload already decoded from an
                        // earlier one; swap it in rather than asking the server again.
                        self.apply_decoded();
                        self.restore_cursor();
                    }
                    Err(e) => {
                        self.note = Some((e.clone(), Note::Error));
                        // Keep whatever is already on screen when a *further* page fails;
                        // only a failed first page leaves the list with nothing to show.
                        if !append {
                            self.view.workflows = Loadable::Failed(e);
                        }
                    }
                }
            }
            Msg::History { generation, result } => {
                if generation != self.view.generation {
                    return;
                }
                self.view.loading_more = false;
                match result {
                    Ok((events, token)) => {
                        if !token.is_empty() {
                            self.view.history_resume = token.clone();
                        } else if self.view.following {
                            // Follow only ever sees an empty token when the workflow has
                            // closed. There is nothing further to tail, so stop rather than
                            // spin on a call that now returns instantly.
                            self.stop_following();
                            self.note =
                                Some(("workflow closed — follow stopped".into(), Note::Info));
                        }
                        self.view.history_token = token;
                        // Merged, not appended: a resumed follow replays the page its token
                        // sat in, and listing those events twice would inflate every group.
                        merge_events(&mut self.view.history_events, events);
                        // Re-group the whole accumulated history rather than patching: a
                        // page boundary can land in the middle of a group, so the last
                        // group of a page is routinely completed by the next one.
                        let groups = group_events(&self.view.history_events);
                        let events = self.view.history_events.clone();
                        match self.view.history.value_mut() {
                            Some(outline) => outline.replace(events, groups),
                            None => {
                                self.view.history = Loadable::loaded(Outline::new(events, groups))
                            }
                        }
                        self.clamp_cursor();
                    }
                    Err(e) => {
                        self.note = Some((e.clone(), Note::Error));
                        if self.view.history_events.is_empty() {
                            self.view.history = Loadable::Failed(e);
                        }
                    }
                }
            }
            Msg::Schedules { generation, result } => {
                if generation != self.view.generation {
                    return;
                }
                self.view.loading_more = false;
                self.view.schedules = match result {
                    Ok(rows) => Loadable::loaded(rows),
                    Err(e) => {
                        self.note = Some((e.clone(), Note::Error));
                        Loadable::Failed(e)
                    }
                };
                self.clamp_cursor();
            }
            Msg::Counts { generation, result } => {
                if generation != self.view.generation {
                    return;
                }
                self.view.counts = match result {
                    Ok(c) => Loadable::loaded(c),
                    Err(e) => Loadable::Failed(e),
                };
            }
        }
    }

    fn on_key(&mut self, chord: Chord) {
        // The command line owns every key while it is open, so that `:` can accept a name
        // containing characters that are bound elsewhere.
        // A pending destructive action owns every key: nothing bound elsewhere should be
        // able to fire while one is waiting.
        if self.confirm.is_some() {
            self.confirm_key(chord);
            return;
        }
        if self.prompt.is_some() {
            self.prompt_key(chord);
            return;
        }

        self.note = None;
        match self.keymap.resolve(self.mode, &mut self.pending, chord) {
            Resolution::Count(_) => {
                self.which_key.clear();
            }
            Resolution::Pending { candidates } => {
                self.which_key = candidates;
            }
            Resolution::Run { id, count } => {
                self.which_key.clear();
                self.run(id, count);
            }
            Resolution::Unbound { flushed } => {
                self.which_key.clear();
                if self.mode == Mode::Insert {
                    // Keys held for an incomplete sequence are literal input after all.
                    self.insert_keys(flushed);
                }
            }
        }
    }

    /// Run a command by id. This is the single dispatch point: keys, the command line, and
    /// (later) macros and `--exec` all arrive here.
    pub fn run(&mut self, id: &str, count: Option<u32>) {
        let Some(cmd) = self.registry.get(id) else {
            self.note = Some((format!("no such command: {id}"), Note::Error));
            return;
        };
        let n = count.unwrap_or(1) as usize;

        match cmd.action {
            Action::Quit => self.should_quit = true,
            Action::ToggleHelp => {
                self.show_help = !self.show_help;
                self.help_scroll = 0;
            }
            Action::OpenCommandLine => {
                self.prompt = Some(Prompt {
                    kind: PromptKind::Command,
                    buf: String::new(),
                });
                self.mode = Mode::Command;
            }
            Action::Cancel => {
                if self.show_help {
                    self.show_help = false;
                    self.help_scroll = 0;
                } else {
                    self.view.anchor = None;
                    self.mode = Mode::Normal;
                    self.pending.clear();
                    self.which_key.clear();
                }
            }
            Action::Refresh => self.refresh(),

            // While the help overlay is open the motions scroll it. It is the frontmost
            // thing on screen, so moving a cursor hidden behind it would be surprising.
            Action::MoveDown if self.show_help => self.scroll_help(n as isize),
            Action::MoveUp if self.show_help => self.scroll_help(-(n as isize)),
            Action::MoveTop if self.show_help => self.help_scroll = 0,
            Action::MoveBottom if self.show_help => self.help_scroll = self.help_max_scroll,
            Action::HalfPageDown if self.show_help => {
                self.scroll_help((self.view.page / 2).max(1) as isize)
            }
            Action::HalfPageUp if self.show_help => {
                self.scroll_help(-((self.view.page / 2).max(1) as isize))
            }

            Action::MoveDown => self.move_cursor(n as isize),
            Action::MoveUp => self.move_cursor(-(n as isize)),
            Action::MoveTop => self.set_cursor(0),
            Action::MoveBottom => self.set_cursor(self.row_count().saturating_sub(1)),
            Action::HalfPageDown => self.move_cursor((self.view.page / 2).max(1) as isize),
            Action::HalfPageUp => self.move_cursor(-((self.view.page / 2).max(1) as isize)),

            Action::OpenItem => self.open_focused(),
            Action::GoUp => self.go_up(),
            Action::GoSchedules => self.go_to(Screen::Schedules),
            Action::GoWorkflows => self.go_to(Screen::Workflows),

            Action::PauseSchedule => self.confirm_mutation(MutationKind::PauseSchedule),
            Action::TriggerSchedule => self.confirm_mutation(MutationKind::TriggerSchedule),
            Action::DeleteSchedule => self.confirm_mutation(MutationKind::DeleteSchedule),

            Action::EnterInsert => {
                self.mode = Mode::Insert;
                // On the workflow list the only text field is the query bar, so that is
                // what Insert mode edits. It is seeded with the applied query so `i` is an
                // edit, not a retype.
                if self.view.screen == Screen::Workflows {
                    self.insert_target = InsertTarget::Query;
                    self.insert_buf = self.view.query.clone();
                } else {
                    self.insert_target = InsertTarget::Scratch;
                    self.insert_buf.clear();
                }
            }
            Action::LeaveInsert => {
                // Esc abandons the edit; the applied query is unchanged. Enter applies —
                // see `insert_keys`.
                self.mode = Mode::Normal;
                self.insert_target = InsertTarget::Scratch;
                self.insert_buf.clear();
            }
            Action::EnterVisual => {
                self.mode = Mode::Visual;
                self.view.anchor = Some(self.view.cursor);
            }
            Action::EnterVisualLine => {
                self.mode = Mode::VisualLine;
                self.view.anchor = Some(self.view.cursor);
            }

            Action::YankField => self.yank(self.field_under_cursor()),
            Action::YankRecord => self.yank(self.records_selected()),

            Action::LoadMore => self.load_more(),
            Action::SelectView(key) => self.select_view(key),

            Action::ToggleFold => self.toggle_fold(),
            Action::ExpandAll => self.with_outline(|o| o.expand_all()),
            Action::CollapseAll => self.with_outline(|o| o.collapse_all()),
            Action::TogglePlumbing => {
                let showing = self
                    .view
                    .history
                    .value()
                    .is_some_and(Outline::show_plumbing);
                self.with_outline(|o| o.set_show_plumbing(!showing));
                self.note = Some((
                    if showing {
                        "workflow tasks hidden".into()
                    } else {
                        "workflow tasks shown".into()
                    },
                    Note::Info,
                ));
            }
            Action::NextFailure => self.jump_failure(true),
            Action::PrevFailure => self.jump_failure(false),
            Action::ToggleFollow => self.toggle_follow(),
            Action::DetailDown => self.scroll_detail(n as isize),
            Action::DetailUp => self.scroll_detail(-(n as isize)),
            Action::OpenPipe => self.open_pipe(),

            Action::CancelWorkflow => self.confirm_mutation(MutationKind::Cancel),
            Action::TerminateWorkflow => self.confirm_mutation(MutationKind::Terminate),
            Action::SignalWorkflow => self.confirm_mutation(MutationKind::Signal),
            Action::DeleteWorkflow => self.confirm_mutation(MutationKind::Delete),
            Action::ResetWorkflow => self.confirm_mutation(MutationKind::Reset),
            Action::UpdateWorkflow => self.confirm_mutation(MutationKind::Update),

            Action::SplitRight => self.split(Axis::Columns),
            Action::SplitDown => self.split(Axis::Rows),
            Action::CloseWindow => self.close_window(),
            Action::EqualizeWindows => self.tabs.current_mut().equalize(),
            Action::FocusLeft => self.focus_window(Direction::Left),
            Action::FocusRight => self.focus_window(Direction::Right),
            Action::FocusUp => self.focus_window(Direction::Up),
            Action::FocusDown => self.focus_window(Direction::Down),
            Action::GrowLeft => self.resize_window(Direction::Left),
            Action::GrowRight => self.resize_window(Direction::Right),
            Action::GrowUp => self.resize_window(Direction::Up),
            Action::GrowDown => self.resize_window(Direction::Down),
            Action::NewTab => self.new_tab(),
            Action::CloseTab => self.close_tab(),
            Action::NextTab => self.switch_tab(true),
            Action::PrevTab => self.switch_tab(false),
            Action::ToggleDetail => {
                if self.view.screen == Screen::History {
                    self.view.show_detail = !self.view.show_detail;
                    self.view.detail_scroll = 0;
                    self.view.piped = None;
                    self.maybe_decode();
                } else {
                    self.note = Some((
                        "payloads are shown on a workflow history".into(),
                        Note::Warn,
                    ));
                }
            }
        }
        self.clamp_cursor();
    }

    // ── navigation ───────────────────────────────────────────────────────────

    fn open_focused(&mut self) {
        match self.view.screen {
            Screen::Namespaces => {
                // A visual selection opens every namespace in it as one merged list. That
                // is the whole multi-namespace fan-out: `V j j <CR>`, using the selection
                // machinery that already exists rather than a separate picker.
                let (lo, hi) = self
                    .selection()
                    .unwrap_or((self.view.cursor, self.view.cursor));
                let scope: Vec<String> = self
                    .namespace_rows()
                    .iter()
                    .skip(lo)
                    .take(hi.saturating_sub(lo) + 1)
                    .map(|n| n.name.clone())
                    .collect();
                if scope.is_empty() {
                    self.note = Some(("nothing to open".into(), Note::Warn));
                    return;
                }

                self.view.namespace_cursor = self.view.cursor;
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.view.screen = Screen::Workflows;
                self.view.scope = scope;
                self.view.cursor = 0;
                self.view.cursor_key = None;
                self.load_workflows(false);
            }
            Screen::Workflows => {
                let Some(row) = self.workflow_rows().get(self.view.cursor).cloned() else {
                    self.note = Some(("nothing to open".into(), Note::Warn));
                    return;
                };
                self.view.workflow_cursor = self.view.cursor;
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.view.screen = Screen::History;
                self.view.viewing = Some(row);
                self.view.cursor = 0;
                self.load_history();
            }
            // On the history screen, "open the focused item" is folding a group open.
            Screen::History => self.toggle_fold(),
            // A schedule's runs are ordinary workflows, so there is nothing of its own to
            // open. Saying so beats a key that appears broken.
            Screen::Schedules => {
                self.note = Some((
                    "a schedule has no detail view; gw for its workflows".into(),
                    Note::Warn,
                ));
            }
        }
    }

    fn go_up(&mut self) {
        match self.view.screen {
            Screen::History => {
                self.stop_following();
                self.view.screen = Screen::Workflows;
                self.view.cursor = self.view.workflow_cursor;
                self.view.viewing = None;
                self.view.history = Loadable::NotAsked;
                self.view.history_events.clear();
                self.view.history_token.clear();
                self.view.history_resume.clear();
                self.restore_cursor();
            }
            Screen::Workflows => {
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.view.anchor = None;
                self.clamp_cursor();
            }
            Screen::Schedules => {
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.view.anchor = None;
                self.clamp_cursor();
            }
            Screen::Namespaces => {
                self.note = Some(("already at the top level".into(), Note::Warn));
            }
        }
    }

    // ── history ──────────────────────────────────────────────────────────────

    /// The group the cursor is on, whether it sits on the group's own line or on one of its
    /// events. Folding from inside an expanded group is what a reader expects.
    fn group_under_cursor(&self) -> Option<usize> {
        match self.view.history.value()?.row_at(self.view.cursor)? {
            Row::Group { group, .. } | Row::Event { group, .. } => Some(group),
        }
    }

    fn toggle_fold(&mut self) {
        let Some(group) = self.group_under_cursor() else {
            return;
        };
        // Folding shut from inside a group would otherwise strand the cursor past the end;
        // `toggle` hands back where the group's own line is now, so it moves there.
        if let Some(outline) = self.view.history.value_mut()
            && let Some(row) = outline.toggle(group)
        {
            self.view.cursor = row;
        }
        self.clamp_cursor();
    }

    /// Apply a shape change, then put the cursor back on the group it was on. Expanding or
    /// collapsing everything moves every row, so an unadjusted cursor lands somewhere
    /// arbitrary.
    fn with_outline(&mut self, f: impl FnOnce(&mut Outline)) {
        let was = self.group_under_cursor();
        let Some(outline) = self.view.history.value_mut() else {
            return;
        };
        f(outline);
        self.view.cursor = was
            .and_then(|g| outline.row_of_group(g))
            .unwrap_or(self.view.cursor);
        self.clamp_cursor();
    }

    // ── follow mode ──────────────────────────────────────────────────────────

    fn scroll_detail(&mut self, delta: isize) {
        let next = (self.view.detail_scroll as isize + delta)
            .clamp(0, self.view.detail_max_scroll as isize);
        self.view.detail_scroll = next as usize;
    }

    fn toggle_follow(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("follow applies to a workflow history".into(), Note::Warn));
            return;
        }
        if self.view.following {
            self.stop_following();
            self.note = Some(("follow stopped".into(), Note::Info));
            return;
        }
        // Following a workflow that has already finished would poll forever for events that
        // can never arrive, so say so instead.
        if self.view.history_token.is_empty() && self.workflow_is_closed() {
            self.note = Some((
                "this workflow has closed — nothing to follow".into(),
                Note::Warn,
            ));
            return;
        }
        self.start_following();
    }

    /// Whether the run itself has ended, as opposed to merely being caught up.
    fn workflow_is_closed(&self) -> bool {
        self.view
            .history
            .value()
            .map(|o| tmprl_core::outline::summarize(o.groups()))
            .is_some_and(|s| s.outcome != tmprl_core::history::Outcome::Pending)
    }

    /// Spawn the long-poll loop.
    ///
    /// The loop lives entirely in the task: the reducer never awaits it, and every batch of
    /// events comes back as an ordinary `Msg`. That is the whole reason a sixty-second long
    /// poll cannot freeze a keystroke.
    fn start_following(&mut self) {
        let Some(row) = self.view.viewing.clone() else {
            return;
        };
        self.view.following = true;
        self.note = Some(("following — F to stop".into(), Note::Info));

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        let mut token = self.view.history_resume.clone();

        self.view.follow_task = Some(tokio::spawn(async move {
            loop {
                let result = conn
                    .follow_history(&row.namespace, &row.workflow_id, &row.run_id, token.clone())
                    .await;
                match result {
                    Ok(page) => {
                        let done = page.next_page_token.is_empty();
                        token = page.next_page_token.clone();
                        if tx
                            .send(Msg::History {
                                generation,
                                result: Ok((page.events, page.next_page_token)),
                            })
                            .is_err()
                        {
                            return; // the application is gone
                        }
                        if done {
                            return; // the workflow closed
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::History {
                            generation,
                            result: Err(e.to_string()),
                        });
                        return;
                    }
                }
            }
        }));
    }

    /// Stop tailing. Aborting matters: the task is parked inside a long poll and would
    /// otherwise keep a request open and keep pushing events into a screen that has moved on.
    fn stop_following(&mut self) {
        self.view.stop_following();
    }

    fn jump_failure(&mut self, forward: bool) {
        let Some(outline) = self.view.history.value() else {
            return;
        };
        let found = if forward {
            outline.next_failure(self.view.cursor)
        } else {
            outline.prev_failure(self.view.cursor)
        };
        match found {
            Some(row) => self.view.cursor = row,
            None => {
                self.note = Some((
                    if forward {
                        "no failure below".into()
                    } else {
                        "no failure above".into()
                    },
                    Note::Warn,
                ));
            }
        }
    }

    fn select_view(&mut self, key: char) {
        let Some(view) = self.views.iter().find(|v| v.key == key) else {
            self.note = Some((format!("no saved view on `{key}`"), Note::Warn));
            return;
        };
        let (name, query) = (view.name.clone(), view.query.clone());
        // A view is a bookmark, not a mode: it fills the query bar, which stays editable.
        self.view.query = query;
        if self.view.screen == Screen::Namespaces {
            self.view.screen = Screen::Workflows;
        }
        self.note = Some((format!("view: {name}"), Note::Info));
        self.load_workflows(false);
    }

    fn refresh(&mut self) {
        match self.view.screen {
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => {
                self.view.history_events.clear();
                self.view.history_token.clear();
                self.view.history_resume.clear();
                self.load_history();
            }
            Screen::Schedules => self.load_schedules(),
        }
    }

    // ── cursor ───────────────────────────────────────────────────────────────

    fn scroll_help(&mut self, delta: isize) {
        let next = (self.help_scroll as isize + delta).clamp(0, self.help_max_scroll as isize);
        self.help_scroll = next as usize;
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.row_count();
        if len == 0 {
            self.view.cursor = 0;
            return;
        }
        let next = (self.view.cursor as isize + delta).clamp(0, len as isize - 1);
        self.set_cursor(next as usize);
    }

    fn set_cursor(&mut self, at: usize) {
        if at != self.view.cursor {
            // The pane now shows a different value; keeping the old offset would open it
            // part-way down something the reader has not seen the start of. A filter result
            // belonged to the row it was run on, so it goes too rather than sitting under a
            // heading that no longer describes it.
            self.view.detail_scroll = 0;
            self.view.piped = None;
        }
        self.view.cursor = at;
        self.maybe_decode();
        self.remember_cursor();
        self.maybe_load_more();
    }

    /// Record which row the cursor is on, by identity. This is what a refresh restores.
    fn remember_cursor(&mut self) {
        if self.view.screen == Screen::Workflows {
            self.view.cursor_key = self
                .workflow_rows()
                .get(self.view.cursor)
                .map(|r| (r.namespace.clone(), r.run_id.clone()));
        }
    }

    /// Put the cursor back on the row it was on, wherever that row has moved to.
    fn restore_cursor(&mut self) {
        let Some((ns, run)) = self.view.cursor_key.clone() else {
            self.clamp_cursor();
            return;
        };
        if let Some(list) = self.view.workflows.value()
            && let Some(at) = list.position_of((&ns, &run))
        {
            self.view.cursor = at;
        }
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.row_count();
        self.view.cursor = self.view.cursor.min(len.saturating_sub(1));
        if len == 0 {
            self.view.cursor = 0;
        }
    }

    /// Infinite scroll: fetch the next page once the cursor is within a screen of the end.
    fn maybe_load_more(&mut self) {
        if self.view.loading_more {
            return;
        }
        let len = self.row_count();
        let near_end = self.view.cursor + self.view.page.max(1) >= len;
        if !near_end {
            return;
        }
        match self.view.screen {
            Screen::Workflows => {
                if self
                    .view
                    .workflows
                    .value()
                    .is_some_and(WorkflowList::has_more)
                {
                    self.load_more();
                }
            }
            Screen::History => {
                if !self.view.history_token.is_empty() {
                    self.load_history();
                }
            }
            Screen::Namespaces | Screen::Schedules => {}
        }
    }

    // ── yanking ──────────────────────────────────────────────────────────────

    fn field_under_cursor(&self) -> String {
        match self.view.screen {
            Screen::Namespaces => self
                .namespace_rows()
                .get(self.view.cursor)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            // The workflow id is the field you actually want to paste into a CLI command.
            Screen::Workflows => self
                .workflow_rows()
                .get(self.view.cursor)
                .map(|w| w.workflow_id.clone())
                .unwrap_or_default(),
            Screen::History => self.history_field_under_cursor(),
            Screen::Schedules => self
                .view
                .schedule_rows()
                .get(self.view.cursor)
                .map(|s| s.schedule_id.clone())
                .unwrap_or_default(),
        }
    }

    /// On a group line, the thing it is about; on an event line, the event's own name. Both
    /// are what you would paste into a search or a CLI command.
    fn history_field_under_cursor(&self) -> String {
        let Some(outline) = self.view.history.value() else {
            return String::new();
        };
        match outline.row_at(self.view.cursor) {
            Some(Row::Group { group, .. }) => outline
                .group(group)
                .map(|g| {
                    if g.subject.is_empty() {
                        format!("{:?}", g.category)
                    } else {
                        g.subject.clone()
                    }
                })
                .unwrap_or_default(),
            Some(Row::Event { event, .. }) => outline
                .event(event)
                .map(|e| e.name.to_string())
                .unwrap_or_default(),
            None => String::new(),
        }
    }

    /// The selected rows as JSON, or just the row under the cursor when nothing is selected.
    fn records_selected(&self) -> String {
        let (lo, hi) = self
            .selection()
            .unwrap_or((self.view.cursor, self.view.cursor));
        let take = hi.saturating_sub(lo) + 1;
        let picked: Vec<String> = match self.view.screen {
            Screen::Namespaces => self
                .namespace_rows()
                .iter()
                .skip(lo)
                .take(take)
                .map(|n| {
                    format!(
                        r#"{{"name":{},"state":{},"retentionDays":{}}}"#,
                        json_string(&n.name),
                        json_string(&n.state),
                        n.retention_days
                    )
                })
                .collect(),
            Screen::Workflows => self
                .workflow_rows()
                .iter()
                .skip(lo)
                .take(take)
                .map(|w| {
                    format!(
                        r#"{{"namespace":{},"workflowId":{},"runId":{},"type":{},"taskQueue":{},"status":{},"historyLength":{}}}"#,
                        json_string(&w.namespace),
                        json_string(&w.workflow_id),
                        json_string(&w.run_id),
                        json_string(&w.workflow_type),
                        json_string(&w.task_queue),
                        json_string(w.status.query_name()),
                        w.history_length
                    )
                })
                .collect(),
            Screen::History => self.history_records(lo, take),
            Screen::Schedules => self
                .view
                .schedule_rows()
                .iter()
                .skip(lo)
                .take(take)
                .map(|s| {
                    format!(
                        r#"{{"namespace":{},"scheduleId":{},"workflowType":{},"paused":{},"spec":{}}}"#,
                        json_string(&s.namespace),
                        json_string(&s.schedule_id),
                        json_string(&s.workflow_type),
                        s.paused,
                        json_string(&s.spec)
                    )
                })
                .collect(),
        };
        match picked.len() {
            0 => String::new(),
            1 => picked.into_iter().next().unwrap(),
            _ => format!("[{}]", picked.join(",")),
        }
    }

    /// The selected history rows as JSON. A group serialises as the summary the compact
    /// view shows; an event as its own fields.
    fn history_records(&self, lo: usize, take: usize) -> Vec<String> {
        let Some(outline) = self.view.history.value() else {
            return Vec::new();
        };
        (lo..lo.saturating_add(take))
            .map_while(|r| outline.row_at(r))
            .filter_map(|row| match row {
                Row::Group { group, .. } => outline.group(group).map(|g| {
                    format!(
                        r#"{{"group":{},"category":{},"outcome":{},"attempts":{},"events":{}}}"#,
                        json_string(&g.subject),
                        json_string(&format!("{:?}", g.category)),
                        json_string(g.outcome.label()),
                        g.attempts,
                        g.events.len()
                    )
                }),
                Row::Event { event, .. } => outline.event(event).map(|e| {
                    format!(
                        r#"{{"eventId":{},"event":{},"subject":{}}}"#,
                        e.id,
                        json_string(e.name),
                        json_string(&e.subject)
                    )
                }),
            })
            .collect()
    }

    fn yank(&mut self, text: String) {
        if text.is_empty() {
            self.note = Some(("nothing to yank".into(), Note::Warn));
            return;
        }
        let n = text.len();
        match crate::clipboard::yank(&text) {
            Ok(()) => {
                self.note = Some((format!("yanked {n} bytes to clipboard"), Note::Info));
                self.view.anchor = None;
                self.mode = Mode::Normal;
            }
            Err(e) => self.note = Some((format!("yank failed: {e}"), Note::Error)),
        }
    }

    // ── insert mode ──────────────────────────────────────────────────────────

    /// Literal input, plus the two editing keys a text field cannot do without.
    fn insert_keys(&mut self, flushed: Vec<Chord>) {
        use tmprl_core::Key;
        for c in flushed {
            match c.key {
                Key::Backspace if c.mods.is_none() => {
                    self.insert_buf.pop();
                }
                Key::Enter if c.mods.is_none() => self.commit_insert(),
                _ => {
                    if let Some(ch) = c.as_insertable() {
                        self.insert_buf.push(ch);
                    }
                }
            }
        }
    }

    /// Enter in Insert mode. On the query bar this applies the query and reloads.
    fn commit_insert(&mut self) {
        if self.insert_target != InsertTarget::Query {
            return;
        }
        self.view.query = self.insert_buf.clone();
        self.mode = Mode::Normal;
        self.insert_target = InsertTarget::Scratch;
        self.insert_buf.clear();
        self.load_workflows(false);
    }

    // ── command line ─────────────────────────────────────────────────────────

    fn prompt_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        match chord.key {
            Key::Esc => self.close_prompt(),
            Key::Enter => {
                let entered = prompt.buf.trim().to_string();
                let kind = prompt.kind;
                self.close_prompt();
                if entered.is_empty() {
                    return;
                }
                match kind {
                    PromptKind::Command => self.run_typed_command(&entered),
                    PromptKind::Pipe => self.run_pipe(entered),
                    PromptKind::Signal | PromptKind::Update => self.confirm_named(kind, entered),
                }
            }
            // Backspace on an empty line closes the prompt, as it does in vim.
            Key::Backspace if prompt.buf.pop().is_none() => self.close_prompt(),
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => prompt.buf.push(c),
            _ => {}
        }
    }

    // ── windows ──────────────────────────────────────────────────────────────

    /// Park the focused pane and take up whichever one the tree now points at.
    ///
    /// The reducer always acts on `self.view`, so every operation that can move focus ends
    /// here. Swapping rather than looking up is what lets the rest of the reducer stay
    /// unaware that there is more than one pane.
    fn refocus(&mut self, previous: ViewId) {
        let now = self.tabs.current().focused();
        if now == previous {
            return;
        }
        let namespace = self.namespace.clone();
        let incoming = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
        let outgoing = std::mem::replace(&mut self.view, incoming);
        self.parked.insert(previous, outgoing);
    }

    fn fresh_view_id(&mut self) -> ViewId {
        let id = ViewId(self.next_view_id);
        self.next_view_id += 1;
        id
    }

    /// Split the focused window. The new pane starts where this one is, which is almost
    /// always what you wanted it for — comparing two histories means opening the same place
    /// twice and then navigating one of them away.
    fn split(&mut self, axis: Axis) {
        let previous = self.tabs.current().focused();
        let id = self.fresh_view_id();
        let forked = self.view.fork();
        self.parked.insert(id, forked);
        self.tabs.current_mut().split(axis, id);
        self.refocus(previous);
        // The new pane knows where it is but has fetched nothing yet.
        self.load_for_screen();
        self.note = Some((format!("{} windows", self.tabs.current().len()), Note::Info));
    }

    fn close_window(&mut self) {
        let previous = self.tabs.current().focused();
        if !self.tabs.current_mut().close() {
            self.note = Some(("last window — <Space>q to quit tmprl".into(), Note::Warn));
            return;
        }
        // Its state goes with it, and View's Drop stops any follow poll it had running.
        self.parked.remove(&previous);
        let now = self.tabs.current().focused();
        let namespace = self.namespace.clone();
        let incoming = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
        self.view = incoming;
    }

    fn focus_window(&mut self, dir: Direction) {
        let previous = self.tabs.current().focused();
        if self.tabs.current_mut().focus_direction(dir, self.frame) {
            self.refocus(previous);
        }
    }

    fn resize_window(&mut self, dir: Direction) {
        // Ten cells' worth, as `<leader>r{hjkl}` promises. Weights are relative, so this is
        // a nudge rather than an exact cell count.
        self.tabs.current_mut().resize(dir, 10);
    }

    fn new_tab(&mut self) {
        let previous = self.tabs.current().focused();
        let id = self.fresh_view_id();
        self.parked.insert(
            previous,
            std::mem::replace(&mut self.view, View::new(&self.namespace)),
        );
        self.tabs.open(id);
        // The new tab's view is the one we just made; nothing to take from `parked`.
        let _ = previous;
        self.load_for_screen();
    }

    fn close_tab(&mut self) {
        if self.tabs.len() == 1 {
            self.note = Some(("last tab — <Space>q to quit tmprl".into(), Note::Warn));
            return;
        }
        // Every pane in the tab goes, along with whatever each was polling.
        for id in self.tabs.current().views() {
            self.parked.remove(&id);
        }
        self.tabs.close();
        let now = self.tabs.current().focused();
        let namespace = self.namespace.clone();
        self.view = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
    }

    fn switch_tab(&mut self, forward: bool) {
        if self.tabs.len() == 1 {
            return;
        }
        let previous = self.tabs.current().focused();
        self.parked.insert(
            previous,
            std::mem::replace(&mut self.view, View::new(&self.namespace)),
        );
        if forward {
            self.tabs.next();
        } else {
            self.tabs.previous();
        }
        let now = self.tabs.current().focused();
        let namespace = self.namespace.clone();
        self.view = self
            .parked
            .remove(&now)
            .unwrap_or_else(|| View::new(&namespace));
    }

    /// Load whatever the focused pane's screen needs. A fresh pane has asked for nothing.
    fn load_for_screen(&mut self) {
        match self.view.screen {
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => self.load_history(),
            Screen::Schedules => self.load_schedules(),
        }
    }

    /// Record the area the panes were laid out in, so focus movement is geometric against
    /// what is actually on screen rather than against a guess.
    pub fn set_frame(&mut self, area: UiRect) {
        self.frame = area;
    }

    /// A non-focused pane's state, for rendering it.
    pub fn parked_view(&self, id: ViewId) -> Option<&View> {
        self.parked.get(&id)
    }

    // ── mutations ────────────────────────────────────────────────────────────

    /// Switch between the two lists a namespace holds.
    ///
    /// Only from a list, and only within the same scope. From a history the reader is inside
    /// one workflow, and jumping sideways from there would lose their place with no way back.
    fn go_to(&mut self, screen: Screen) {
        match self.view.screen {
            Screen::Namespaces => {
                self.note = Some(("open a namespace first".into(), Note::Warn));
                return;
            }
            Screen::History => {
                self.note = Some(("go up with `-` first".into(), Note::Warn));
                return;
            }
            _ if self.view.screen == screen => return,
            _ => {}
        }
        self.view.stop_following();
        self.view.screen = screen;
        self.view.cursor = 0;
        self.view.anchor = None;
        self.load_for_screen();
    }

    /// Schedules for the first namespace in scope.
    ///
    /// One namespace, not the fan-out: ListSchedules takes a single namespace and a schedule
    /// list merged across several has no ordering that means anything.
    pub fn load_schedules(&mut self) {
        self.view.generation = self.view.generation.wrapping_add(1);
        self.view.schedules.begin_refresh();
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let namespace = self
            .view
            .scope
            .first()
            .cloned()
            .unwrap_or_else(|| self.namespace.clone());
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn
                .list_schedules(&namespace, PAGE_SIZE, Vec::new())
                .await
                .map(|p| p.rows)
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Schedules { generation, result });
        });
    }

    /// The schedule a schedule mutation would act on.
    fn target_schedule(&self) -> Option<ScheduleRow> {
        if self.view.screen != Screen::Schedules {
            return None;
        }
        self.view.schedule_rows().get(self.view.cursor).cloned()
    }

    /// The workflow a mutation would act on: the row under the cursor on the workflow list,
    /// or the one whose history is open.
    fn target_workflow(&self) -> Option<WorkflowRow> {
        match self.view.screen {
            Screen::Workflows => self.view.workflow_rows().get(self.view.cursor).cloned(),
            Screen::History => self.view.viewing.clone(),
            Screen::Namespaces | Screen::Schedules => None,
        }
    }

    /// Open the confirmation for a mutation. Nothing happens to the cluster here.
    fn confirm_mutation(&mut self, kind: MutationKind) {
        // Schedule operations act on a schedule id, not an execution.
        if matches!(
            kind,
            MutationKind::PauseSchedule
                | MutationKind::TriggerSchedule
                | MutationKind::DeleteSchedule
        ) {
            let Some(row) = self.target_schedule() else {
                self.note = Some(("no schedule under the cursor".into(), Note::Warn));
                return;
            };
            let (namespace, schedule_id) = (row.namespace.clone(), row.schedule_id.clone());
            let mutation = match kind {
                MutationKind::PauseSchedule => Mutation::PauseSchedule {
                    namespace,
                    schedule_id,
                    // One key toggles, so the target state is the opposite of now.
                    paused: !row.paused,
                },
                MutationKind::TriggerSchedule => Mutation::TriggerSchedule {
                    namespace,
                    schedule_id,
                },
                _ => Mutation::DeleteSchedule {
                    namespace,
                    schedule_id,
                },
            };
            self.confirm = Some(Confirm::new(mutation));
            return;
        }

        let Some(row) = self.target_workflow() else {
            self.note = Some(("no workflow under the cursor".into(), Note::Warn));
            return;
        };
        let (namespace, workflow_id, run_id) = (
            row.namespace.clone(),
            row.workflow_id.clone(),
            row.run_id.clone(),
        );

        let mutation = match kind {
            MutationKind::Cancel => Mutation::Cancel {
                namespace,
                workflow_id,
                run_id,
            },
            MutationKind::Terminate => Mutation::Terminate {
                namespace,
                workflow_id,
                run_id,
                // A reason is required by the API and useful in the history. Editing it
                // before confirming is M4 work; a default beats an empty string.
                reason: "terminated from tmprl".into(),
            },
            MutationKind::Delete => Mutation::Delete {
                namespace,
                workflow_id,
                run_id,
            },
            MutationKind::Signal | MutationKind::Update => {
                // Both need a name, which has to be typed. Reuse the prompt rather than
                // inventing a second text field.
                self.prompt = Some(Prompt {
                    kind: if matches!(kind, MutationKind::Signal) {
                        PromptKind::Signal
                    } else {
                        PromptKind::Update
                    },
                    buf: String::new(),
                });
                self.mode = Mode::Command;
                return;
            }
            // Handled above, before a workflow target is looked for.
            MutationKind::PauseSchedule
            | MutationKind::TriggerSchedule
            | MutationKind::DeleteSchedule => return,
            MutationKind::Reset => {
                // A reset needs an event to go back to, which only the history view has.
                let Some(event_id) = self.reset_target() else {
                    self.note = Some((
                        "reset needs a workflow history with a completed workflow task above \
                         the cursor"
                            .into(),
                        Note::Warn,
                    ));
                    return;
                };
                Mutation::Reset {
                    namespace,
                    workflow_id,
                    run_id,
                    event_id,
                    reason: "reset from tmprl".into(),
                }
            }
        };
        self.confirm = Some(Confirm::new(mutation));
    }

    /// Turn a typed signal or update name into a confirmation.
    fn confirm_named(&mut self, kind: PromptKind, name: String) {
        let Some(row) = self.target_workflow() else {
            return;
        };
        let (namespace, workflow_id, run_id) = (row.namespace, row.workflow_id, row.run_id);
        let mutation = match kind {
            PromptKind::Update => Mutation::Update {
                namespace,
                workflow_id,
                run_id,
                name,
                input: None,
            },
            _ => Mutation::Signal {
                namespace,
                workflow_id,
                run_id,
                name,
                input: None,
            },
        };
        self.confirm = Some(Confirm::new(mutation));
    }

    /// The event a reset would go back to: the last completed workflow task at or before the
    /// cursor. Resolved rather than demanded, because the workflow tasks the server needs
    /// are exactly the rows the outline folds away.
    fn reset_target(&self) -> Option<i64> {
        if self.view.screen != Screen::History {
            return None;
        }
        let outline = self.view.history.value()?;
        let at = match outline.row_at(self.view.cursor)? {
            Row::Event { event, .. } => outline.event(event)?.id,
            Row::Group { group, .. } => *outline.group(group)?.events.last()?,
        };
        tmprl_core::history::reset_point(outline.events(), at)
    }

    /// Keys while a confirmation is up.
    ///
    /// This owns every key, so nothing bound elsewhere can act while a destructive action is
    /// pending. Enter is the only way forward and Esc is always a way out.
    fn confirm_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(confirm) = self.confirm.as_mut() else {
            return;
        };
        match chord.key {
            Key::Esc => {
                self.confirm = None;
                self.note = Some(("cancelled".into(), Note::Info));
            }
            Key::Enter if confirm.is_satisfied() => {
                let mutation = confirm.mutation.clone();
                self.confirm = None;
                self.run_mutation(mutation);
            }
            // Enter with the word unfinished is not a refusal, just not yet.
            Key::Enter => {}
            Key::Backspace => {
                confirm.entered.pop();
            }
            Key::Char(c) if chord.mods.is_none() => confirm.entered.push(c),
            _ => {}
        }
    }

    /// Send it. Spawned like every other RPC — a mutation must not freeze a keystroke either.
    fn run_mutation(&mut self, mutation: Mutation) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.note = Some((format!("{}…", mutation.verb().to_lowercase()), Note::Info));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = conn.mutate(&mutation).await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::Mutated {
                mutation: Box::new(mutation),
                result,
            });
        });
    }

    /// Record what was attempted, whether or not it worked.
    ///
    /// Appended, never rewritten, and failures go in too: the log is what was *attempted*,
    /// which is the question being asked when someone reads it.
    fn audit(&mut self, mutation: &Mutation, outcome: &str) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if let Err(e) = crate::config::append_audit(&mutation.audit_line(at, outcome)) {
            // A failed audit write must not be silent: the log is the record that an
            // irreversible thing happened.
            self.note = Some((format!("audit log: {e}"), Note::Error));
        }
    }

    // ── codec server ─────────────────────────────────────────────────────────

    /// Identity of an encrypted payload, for the decode cache.
    ///
    /// The ciphertext plus its encoding: the same bytes decode to the same value, so a row
    /// revisited costs nothing, and two different payloads cannot collide on content alone.
    fn payload_key(p: &Payload) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        p.encoding.hash(&mut h);
        p.data.hash(&mut h);
        h.finish()
    }

    /// What the pane should say about an encrypted payload.
    ///
    /// "Needs a codec server" is only true when none is configured; once one is, the honest
    /// answer is that a request is out.
    pub fn decode_state(&self, p: &Payload) -> DecodeState {
        if self.codec.is_none() {
            DecodeState::NoCodec
        } else if self.decoding.contains(&Self::payload_key(p)) {
            DecodeState::InFlight
        } else {
            DecodeState::Idle
        }
    }

    /// Ask the codec server about anything encrypted under the cursor.
    ///
    /// Lazy on purpose: only what the pane is actually showing. Decoding a whole history
    /// up front would be thousands of round trips for values nobody looked at.
    fn maybe_decode(&mut self) {
        if !self.view.show_detail || self.view.screen != Screen::History {
            return;
        }
        let Some(codec) = self.codec.clone() else {
            return;
        };

        let wanted: Vec<Payload> = self
            .payloads_under_cursor()
            .into_iter()
            .map(|(_, p)| p)
            .filter(|p| p.needs_codec())
            .filter(|p| {
                let key = Self::payload_key(p);
                !self.decoded.contains_key(&key) && !self.decoding.contains(&key)
            })
            .collect();
        if wanted.is_empty() {
            return;
        }

        for p in &wanted {
            self.decoding.insert(Self::payload_key(p));
        }
        let keys: Vec<u64> = wanted.iter().map(Self::payload_key).collect();
        let namespace = self
            .view
            .viewing
            .as_ref()
            .map(|w| w.namespace.clone())
            .unwrap_or_default();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let result = codec
                .decode(&namespace, &wanted)
                .await
                .map(|out| keys.into_iter().zip(out).collect::<Vec<_>>())
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Decoded(result));
        });
    }

    /// Swap every decoded payload into the history in place.
    ///
    /// Replacing the payload rather than keeping a cache the views consult means everything
    /// downstream — the pane, `!` piping, yanking — reads the plaintext without knowing a
    /// codec exists. It is also why this runs after each history page: a later page can
    /// carry the same encrypted value.
    fn apply_decoded(&mut self) {
        if self.decoded.is_empty() {
            return;
        }
        let mut changed = false;
        for event in &mut self.view.history_events {
            for (_, p) in &mut event.payloads {
                if !p.needs_codec() {
                    continue;
                }
                if let Some(plain) = self.decoded.get(&Self::payload_key(p)) {
                    *p = plain.clone();
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
        let groups = group_events(&self.view.history_events);
        let events = self.view.history_events.clone();
        match self.view.history.value_mut() {
            Some(outline) => outline.replace(events, groups),
            None => self.view.history = Loadable::loaded(Outline::new(events, groups)),
        }
    }

    // ── piping payloads through an external command ──────────────────────────

    /// The payloads the cursor is on, as one JSON object.
    ///
    /// For a group that is its input *and* its result, which live on two different events —
    /// the same pair the payload pane shows.
    fn payloads_under_cursor(&self) -> Vec<(String, tmprl_core::payload::Payload)> {
        let Some(outline) = self.view.history.value() else {
            return Vec::new();
        };
        match outline.row_at(self.view.cursor) {
            Some(Row::Event { event, .. }) => outline
                .event(event)
                .map(|e| e.payloads.clone())
                .unwrap_or_default(),
            Some(Row::Group { group, .. }) => {
                let Some(g) = outline.group(group) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                for id in [g.events.first(), g.events.last()].into_iter().flatten() {
                    if let Some(e) = outline.events().iter().find(|e| e.id == *id) {
                        out.extend(e.payloads.iter().cloned());
                    }
                }
                out
            }
            None => Vec::new(),
        }
    }

    /// Open the `!` prompt, if there is anything under the cursor worth piping.
    fn open_pipe(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("piping applies to a workflow history".into(), Note::Warn));
            return;
        }
        let payloads = self.payloads_under_cursor();
        if payloads.is_empty() {
            self.note = Some(("nothing here to pipe".into(), Note::Warn));
            return;
        }
        if tmprl_core::payload::payloads_as_json(&payloads).0.is_none() {
            // Encrypted or binary. Piping it produces a parse error that explains nothing,
            // so refuse with a reason instead.
            self.note = Some((
                "no readable payload here — encrypted or binary".into(),
                Note::Warn,
            ));
            return;
        }
        self.prompt = Some(Prompt {
            kind: PromptKind::Pipe,
            // Pre-filled: `jq` is what this is for, and an empty prompt makes you type the
            // same three characters every time.
            buf: "jq .".into(),
        });
        self.mode = Mode::Command;
    }

    /// Run the typed command with the focused payloads on stdin.
    ///
    /// Spawned, never awaited here — an external command can take as long as it likes and
    /// must not be able to freeze a keystroke. The output arrives as a `Msg`.
    fn run_pipe(&mut self, command: String) {
        let (json, skipped) = tmprl_core::payload::payloads_as_json(&self.payloads_under_cursor());
        let Some(json) = json else {
            self.note = Some(("nothing readable to pipe".into(), Note::Warn));
            return;
        };
        if !skipped.is_empty() {
            self.note = Some((
                format!("piping without {} (not readable)", skipped.join(", ")),
                Note::Warn,
            ));
        }

        // The output replaces the pane, so open it if it is shut — otherwise the result
        // would land somewhere the reader cannot see.
        self.view.show_detail = true;
        self.view.detail_scroll = 0;
        self.view.piped = Some(Ok(format!("running `{command}`…")));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = pipe_through(&command, json.into_bytes()).await;
            let _ = tx.send(Msg::Piped(result));
        });
    }

    fn close_prompt(&mut self) {
        self.prompt = None;
        self.mode = Mode::Normal;
    }

    /// Resolve what was typed at `:` and run it. Accepts a unique prefix, the way vim
    /// accepts `:q` for `:quit`.
    fn run_typed_command(&mut self, entered: &str) {
        let hits = self.registry.search(entered);
        match hits.iter().find(|c| c.id == entered).or(hits.first()) {
            Some(c) if hits.len() == 1 || c.id == entered => {
                let id = c.id;
                self.run(id, None);
            }
            Some(_) => {
                self.note = Some((
                    format!("ambiguous: {} commands match `{entered}`", hits.len()),
                    Note::Warn,
                ));
            }
            None => {
                self.note = Some((format!("no such command: {entered}"), Note::Error));
            }
        }
    }

    /// Completions for whatever is typed at `:`. A `!` prompt takes a shell command, which
    /// tmprl has no business completing.
    pub fn cmdline_matches(&self) -> Vec<&tmprl_core::Command> {
        match &self.prompt {
            Some(p) if p.kind == PromptKind::Command => {
                self.registry.search(&p.buf).into_iter().take(8).collect()
            }
            _ => Vec::new(),
        }
    }

    // ── IO ───────────────────────────────────────────────────────────────────

    /// Spawn a namespace fetch. Returns immediately; the result arrives as a `Msg`.
    pub fn load_namespaces(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.view.namespaces.begin_refresh();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = conn.list_namespaces().await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::Namespaces(res));
        });
    }

    /// Fetch the first page for the current query, and the header counts alongside it.
    ///
    /// Bumping the generation is what makes an in-flight reply for the previous query
    /// harmless: it arrives, does not match, and is dropped.
    pub fn load_workflows(&mut self, append: bool) {
        if !append {
            self.view.generation = self.view.generation.wrapping_add(1);
            self.view.workflows.begin_refresh();
            self.view.counts.begin_refresh();
            self.load_counts();
        }

        // Set before the connection guard: this records the decision to fetch, which is
        // what stops a second page being queued while the first is still in flight.
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        // On a continuation, ask only the namespaces that still have pages. Passing the
        // whole scope would hand an exhausted namespace an empty token, which the server
        // reads as "start again" — so it would never finish.
        let tokens: Tokens = if append {
            self.view
                .workflows
                .value()
                .map(|l| l.tokens().to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let (tx, generation, scope, query) = (
            self.tx.clone(),
            self.view.generation,
            self.view.scope.clone(),
            self.view.query.clone(),
        );
        tokio::spawn(async move {
            let result = if append {
                conn.continue_workflows_across(&tokens, &query, PAGE_SIZE)
                    .await
            } else {
                conn.list_workflows_across(&scope, &query, PAGE_SIZE).await
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Workflows {
                generation,
                append,
                result,
            });
        });
    }

    /// Fetch a page of the focused workflow's history.
    ///
    /// Called for the first page and for each continuation; the accumulated events are
    /// re-grouped whenever one lands, because a page boundary routinely falls inside a
    /// group.
    pub fn load_history(&mut self) {
        let Some(row) = self.view.viewing.clone() else {
            return;
        };
        if self.view.history_events.is_empty() {
            self.view.generation = self.view.generation.wrapping_add(1);
            self.view.history.begin_refresh();
        }
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation, token) = (
            self.tx.clone(),
            self.view.generation,
            self.view.history_token.clone(),
        );
        tokio::spawn(async move {
            let result = conn
                .get_history(
                    &row.namespace,
                    &row.workflow_id,
                    &row.run_id,
                    HISTORY_PAGE_SIZE,
                    token,
                )
                .await
                .map(|p| (p.events, p.next_page_token))
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::History { generation, result });
        });
    }

    fn load_more(&mut self) {
        let has_more = self
            .view
            .workflows
            .value()
            .is_some_and(WorkflowList::has_more);
        if has_more {
            self.load_workflows(true);
        }
    }

    fn load_counts(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation, scope, query) = (
            self.tx.clone(),
            self.view.generation,
            self.view.scope.clone(),
            self.view.query.clone(),
        );
        tokio::spawn(async move {
            let result = conn
                .count_workflows_across(&scope, &query)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Counts { generation, result });
        });
    }
}

/// Run `command` in a shell with `input` on stdin, and collect what it says.
///
/// A shell rather than a bare exec, so that `jq .result | head -20` works — `!` is a filter,
/// and filters are pipelines. On a non-zero exit the stderr is what is worth showing: when a
/// jq expression is wrong, jq's own message is the entire diagnosis.
async fn pipe_through(command: &str, input: Vec<u8>) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run `{command}`: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        // A filter that does not read its input — `!wc -l` after an early exit — closes the
        // pipe, and writing to a closed pipe is not an error worth reporting.
        let _ = stdin.write_all(&input).await;
        let _ = stdin.shutdown().await;
    }

    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("`{command}` failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.trim().is_empty() {
            format!("`{command}` exited with {}", out.status)
        } else {
            stderr
        })
    }
}

/// Minimal JSON string escaping — enough for the identifiers and enum names yanked today.
/// A real serializer arrives with payload rendering in M2.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmprl_core::{Key, WorkflowStatus};
    use tokio::sync::mpsc::unbounded_channel;

    fn app() -> App {
        let (tx, _rx) = unbounded_channel();
        App::detached("prod", "default", tx)
    }

    fn wf(ns: &str, run: &str, start: i64) -> WorkflowRow {
        WorkflowRow {
            namespace: ns.into(),
            workflow_id: format!("order-{run}"),
            run_id: run.into(),
            workflow_type: "Checkout".into(),
            task_queue: "tq".into(),
            status: WorkflowStatus::Running,
            start_time: Some(start),
            close_time: None,
            history_length: 4,
        }
    }

    fn loaded(app: &mut App, rows: Vec<WorkflowRow>, tokens: Tokens) {
        app.view.screen = Screen::Workflows;
        app.handle(Msg::Workflows {
            generation: app.view.generation,
            append: false,
            result: Ok((rows, tokens)),
        });
    }

    fn type_chars(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle(Msg::Key(Chord::ch(c)));
        }
    }

    #[test]
    fn json_strings_escape_control_characters() {
        assert_eq!(json_string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(json_string("a\nb"), r#""a\nb""#);
        assert_eq!(json_string("a\u{1}b"), r#""a\u0001b""#);
    }

    #[test]
    fn the_cursor_stays_on_its_workflow_when_a_newer_one_arrives() {
        // The whole reason the cursor is anchored to a run id. A live list grows at the
        // top; an index-based cursor would quietly select a different workflow.
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("motion.top", None);
        assert_eq!(app.workflow_rows()[app.view.cursor].run_id, "r1");

        app.handle(Msg::Workflows {
            generation: app.view.generation,
            append: false,
            result: Ok((
                vec![wf("default", "r9", 900), wf("default", "r1", 100)],
                vec![],
            )),
        });
        assert_eq!(
            app.view.cursor, 1,
            "cursor should have followed r1 down a row"
        );
        assert_eq!(app.workflow_rows()[app.view.cursor].run_id, "r1");
    }

    #[test]
    fn a_reply_for_a_superseded_query_is_dropped() {
        // Type a new query while the old one is still in flight: the stale reply must not
        // repaint the table with rows the user is no longer looking at.
        let mut app = app();
        loaded(&mut app, vec![wf("default", "old", 100)], vec![]);
        let stale = app.view.generation;

        app.view.query = "WorkflowType = 'New'".into();
        app.load_workflows(false);
        assert_ne!(app.view.generation, stale);

        app.handle(Msg::Workflows {
            generation: stale,
            append: false,
            result: Ok((vec![wf("default", "stale", 1)], vec![])),
        });
        assert_eq!(
            app.workflow_rows()[0].run_id,
            "old",
            "a stale reply must not land"
        );
    }

    #[test]
    fn a_failed_extra_page_keeps_the_rows_already_on_screen() {
        let mut app = app();
        loaded(
            &mut app,
            vec![wf("default", "r1", 100)],
            vec![("default".into(), vec![1])],
        );
        app.handle(Msg::Workflows {
            generation: app.view.generation,
            append: true,
            result: Err("connection reset".into()),
        });
        assert_eq!(app.workflow_rows().len(), 1, "rows must survive");
        assert!(matches!(app.note, Some((_, Note::Error))));
    }

    #[test]
    fn a_failed_first_page_shows_the_error_state() {
        let mut app = app();
        app.view.screen = Screen::Workflows;
        app.handle(Msg::Workflows {
            generation: app.view.generation,
            append: false,
            result: Err("permission denied".into()),
        });
        assert_eq!(app.view.workflows.error(), Some("permission denied"));
    }

    #[test]
    fn enter_opens_a_namespace_and_dash_goes_back() {
        let mut app = app();
        app.view.namespaces = Loadable::loaded(vec![
            NamespaceInfo {
                name: "alpha".into(),
                state: "Registered".into(),
                retention_days: 1,
                description: String::new(),
            },
            NamespaceInfo {
                name: "beta".into(),
                state: "Registered".into(),
                retention_days: 1,
                description: String::new(),
            },
        ]);
        app.run("motion.bottom", None);
        app.run("nav.open", None);

        assert_eq!(app.view.screen, Screen::Workflows);
        assert_eq!(
            app.view.scope,
            ["beta"],
            "the focused namespace becomes the scope"
        );

        app.run("nav.up", None);
        assert_eq!(app.view.screen, Screen::Namespaces);
        assert_eq!(app.view.cursor, 1, "the namespace cursor is restored");
    }

    #[test]
    fn a_visual_selection_of_namespaces_opens_a_fan_out() {
        let mut app = app();
        app.view.namespaces = Loadable::loaded(
            ["alpha", "beta", "gamma"]
                .iter()
                .map(|n| NamespaceInfo {
                    name: (*n).into(),
                    state: "Registered".into(),
                    retention_days: 1,
                    description: String::new(),
                })
                .collect(),
        );

        // Driven through the keymap, not by calling `run` directly: the binding is half of
        // the feature, and a test that skips it cannot tell you the key does nothing.
        for chord in [
            Chord::ch('g'),
            Chord::ch('g'),
            Chord::ch('V'),
            Chord::ch('j'),
            Chord::plain(Key::Enter),
        ] {
            app.handle(Msg::Key(chord));
        }

        assert_eq!(app.view.scope, ["alpha", "beta"]);
        assert!(
            app.view.is_fanned_out(),
            "rows must be tagged with their namespace"
        );
        assert_eq!(app.mode, Mode::Normal, "opening ends the selection");
        assert!(app.view.anchor.is_none());
    }

    #[test]
    fn opening_without_a_selection_scopes_to_one_namespace() {
        let mut app = app();
        app.view.namespaces = Loadable::loaded(vec![NamespaceInfo {
            name: "alpha".into(),
            state: "Registered".into(),
            retention_days: 1,
            description: String::new(),
        }]);
        app.run("nav.open", None);
        assert_eq!(app.view.scope, ["alpha"]);
        assert!(!app.view.is_fanned_out());
    }

    #[test]
    fn insert_mode_edits_the_query_on_the_workflow_screen() {
        let mut app = app();
        loaded(&mut app, vec![], vec![]);
        app.view.query = "A = 1".into();

        app.run("mode.insert", None);
        assert!(app.is_editing_query());
        assert_eq!(app.insert_buf, "A = 1", "the edit starts from the query");

        app.handle(Msg::Key(Chord::plain(Key::Backspace)));
        type_chars(&mut app, "2");
        assert_eq!(app.query_display(), "A = 2");

        app.handle(Msg::Key(Chord::plain(Key::Enter)));
        assert_eq!(app.view.query, "A = 2", "Enter applies the query");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn escape_abandons_a_query_edit() {
        let mut app = app();
        loaded(&mut app, vec![], vec![]);
        app.view.query = "A = 1".into();

        app.run("mode.insert", None);
        type_chars(&mut app, "999");
        app.handle(Msg::Key(Chord::plain(Key::Esc)));

        assert_eq!(app.view.query, "A = 1", "Esc must not apply the edit");
        assert_eq!(app.query_display(), "A = 1");
    }

    #[test]
    fn insert_mode_on_the_namespace_screen_is_not_the_query_bar() {
        let mut app = app();
        app.run("mode.insert", None);
        assert!(!app.is_editing_query());
        type_chars(&mut app, "xy");
        assert_eq!(app.insert_buf, "xy");
        assert_eq!(app.view.query, "", "the namespace screen has no query bar");
    }

    #[test]
    fn a_saved_view_fills_the_query_bar_and_leaves_it_editable() {
        let mut app = app();
        let views = vec![SavedView {
            key: '1',
            name: "Broken".into(),
            query: "ExecutionStatus = 'Failed'".into(),
        }];
        app.registry.add_views(&views);
        app.views = views;

        app.run("view.1", None);
        assert_eq!(app.view.query, "ExecutionStatus = 'Failed'");
        assert_eq!(app.view.screen, Screen::Workflows);

        // Still text, still editable — a view is a bookmark, not a mode.
        app.run("mode.insert", None);
        assert_eq!(app.insert_buf, "ExecutionStatus = 'Failed'");
    }

    #[test]
    fn scrolling_near_the_end_asks_for_the_next_page_once() {
        let mut app = app();
        app.view.page = 2;
        let rows: Vec<WorkflowRow> = (0..10)
            .map(|i| wf("default", &format!("r{i}"), 1000 - i))
            .collect();
        loaded(&mut app, rows, vec![("default".into(), vec![7])]);
        assert!(
            !app.view.loading_more,
            "a completed load clears the in-flight flag"
        );

        app.run("motion.bottom", None);
        assert!(
            app.view.loading_more,
            "reaching the end should request the next page"
        );

        // A second motion while that request is in flight must not queue another.
        app.run("motion.up", None);
        app.run("motion.bottom", None);
        assert!(app.view.loading_more);
    }

    #[test]
    fn scrolling_does_not_page_when_the_list_is_complete() {
        let mut app = app();
        app.view.page = 2;
        let rows: Vec<WorkflowRow> = (0..5)
            .map(|i| wf("default", &format!("r{i}"), 1000 - i))
            .collect();
        loaded(&mut app, rows, vec![]);
        app.run("motion.bottom", None);
        assert!(
            !app.view.loading_more,
            "no token means nothing left to fetch"
        );
    }

    #[test]
    fn yank_on_a_workflow_row_takes_the_workflow_id() {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        assert_eq!(app.field_under_cursor(), "order-r1");

        let record = app.records_selected();
        assert!(
            record.contains(r#""workflowId":"order-r1""#),
            "got {record}"
        );
        assert!(record.contains(r#""status":"Running""#), "got {record}");
        assert!(record.contains(r#""namespace":"default""#), "got {record}");
    }

    #[test]
    fn a_visual_selection_yanks_every_selected_workflow() {
        let mut app = app();
        loaded(
            &mut app,
            vec![wf("default", "r1", 300), wf("default", "r2", 200)],
            vec![],
        );
        app.run("motion.top", None);
        app.run("mode.visual", None);
        app.run("motion.down", None);

        let record = app.records_selected();
        assert!(record.starts_with('['), "a multi-row yank is an array");
        assert!(record.contains("order-r1") && record.contains("order-r2"));
    }

    // ── history ──────────────────────────────────────────────────────────────

    fn hev(
        id: i64,
        group: tmprl_core::history::GroupRef,
        role: tmprl_core::history::Role,
        cat: tmprl_core::history::Category,
    ) -> NormalizedEvent {
        NormalizedEvent::new(id, "E", cat, group, role).with_time(Some(id * 1000))
    }

    /// A workflow, a workflow task (plumbing) and two activities, the second of which
    /// failed.
    fn history_events() -> Vec<NormalizedEvent> {
        use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
        vec![
            hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
            hev(2, G::Opened(2), R::Opens, C::WorkflowTask),
            hev(3, G::Opened(2), R::Closes, C::WorkflowTask),
            hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("Charge"),
            hev(5, G::Opened(4), R::Continues, C::Activity),
            hev(6, G::Opened(4), R::Closes, C::Activity)
                .with_outcome(tmprl_core::history::Outcome::Completed),
            hev(7, G::Opened(7), R::Opens, C::Activity).with_subject("Ship"),
            hev(8, G::Opened(7), R::Closes, C::Activity)
                .with_outcome(tmprl_core::history::Outcome::Failed),
        ]
    }

    fn viewing_history() -> App {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("nav.open", None);
        assert_eq!(app.view.screen, Screen::History);
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((history_events(), Vec::new())),
        });
        app
    }

    #[test]
    fn opening_a_workflow_reads_its_history() {
        let app = viewing_history();
        assert_eq!(app.view.viewing.as_ref().unwrap().run_id, "r1");
        // Three groups: the workflow and two activities. The workflow task is plumbing.
        assert_eq!(app.row_count(), 3);
    }

    #[test]
    fn dash_returns_to_the_workflow_it_came_from() {
        let mut app = viewing_history();
        app.run("nav.up", None);
        assert_eq!(app.view.screen, Screen::Workflows);
        assert!(app.view.viewing.is_none());
        assert!(
            app.view.history.value().is_none(),
            "leaving must drop the history rather than show a stale one on re-entry"
        );
    }

    #[test]
    fn folding_a_group_shows_its_events_and_keeps_the_cursor_on_it() {
        let mut app = viewing_history();
        app.run("motion.down", None); // onto the "Charge" activity
        let before = app.row_count();
        let at = app.view.cursor;

        app.run("history.fold", None);
        assert_eq!(app.row_count(), before + 3, "its three events appeared");
        assert_eq!(
            app.view.cursor, at,
            "the cursor stays on the group's own line"
        );

        // Folding shut from *inside* the group must not strand the cursor past the end.
        app.run("motion.down", None);
        app.run("motion.down", None);
        app.run("history.fold", None);
        assert_eq!(app.row_count(), before);
        assert_eq!(app.view.cursor, at);
    }

    #[test]
    fn expanding_everything_keeps_the_cursor_on_the_same_group() {
        let mut app = viewing_history();
        app.run("motion.bottom", None); // the failed "Ship" activity
        let group = app.group_under_cursor();

        app.run("history.expand-all", None);
        assert_eq!(
            app.group_under_cursor(),
            group,
            "expanding moves every row; the cursor must follow its group"
        );

        app.run("history.collapse-all", None);
        assert_eq!(app.group_under_cursor(), group);
    }

    #[test]
    fn workflow_tasks_are_hidden_until_asked_for() {
        let mut app = viewing_history();
        assert_eq!(app.row_count(), 3);

        app.run("history.plumbing", None);
        assert_eq!(app.row_count(), 4, "the workflow-task group appeared");
        assert!(matches!(app.note, Some((_, Note::Info))));

        app.run("history.plumbing", None);
        assert_eq!(app.row_count(), 3);
    }

    #[test]
    fn failures_are_reachable_by_key() {
        let mut app = viewing_history();
        app.run("motion.top", None);
        app.run("history.next-failure", None);

        let group = app.group_under_cursor().expect("on a group");
        let outline = app.view.history.value().unwrap();
        assert_eq!(outline.group(group).unwrap().subject, "Ship");

        // Saying so beats moving the cursor nowhere and looking broken.
        app.run("history.next-failure", None);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn a_second_history_page_is_appended_and_regrouped() {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("nav.open", None);

        // First page stops mid-group: "Charge" is scheduled but has not finished.
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((history_events()[..5].to_vec(), vec![7])),
        });
        let charge = app.view.history.value().unwrap().group(2).unwrap().clone();
        assert!(charge.is_open(), "the group is incomplete on page one");

        // The rest arrives and completes it — which is why pages are re-grouped whole.
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((history_events()[5..].to_vec(), Vec::new())),
        });
        let charge = app.view.history.value().unwrap().group(2).unwrap();
        assert!(!charge.is_open(), "the second page closed the group");
        assert_eq!(app.row_count(), 3);
    }

    #[test]
    fn a_stale_history_reply_is_dropped() {
        let mut app = viewing_history();
        let stale = app.view.generation;
        app.view.generation = app.view.generation.wrapping_add(1);

        app.handle(Msg::History {
            generation: stale,
            result: Ok((Vec::new(), Vec::new())),
        });
        assert_eq!(
            app.row_count(),
            3,
            "a reply for an abandoned read must not land"
        );
    }

    #[test]
    fn yanking_a_history_row_takes_something_useful() {
        let mut app = viewing_history();
        app.run("motion.bottom", None);
        assert_eq!(app.field_under_cursor(), "Ship");

        let record = app.records_selected();
        assert!(record.contains(r#""group":"Ship""#), "got {record}");
        assert!(record.contains(r#""outcome":"Failed""#), "got {record}");
    }

    // ── follow mode ──────────────────────────────────────────────────────────

    /// A history whose workflow has *not* closed: no terminal event.
    fn running_history() -> Vec<NormalizedEvent> {
        use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
        vec![
            hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
            hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("Charge"),
        ]
    }

    fn viewing_running() -> App {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("nav.open", None);
        app.handle(Msg::History {
            generation: app.view.generation,
            // A non-empty token is what a running workflow returns.
            result: Ok((running_history(), vec![9])),
        });
        app
    }

    #[test]
    fn follow_starts_and_stops_on_the_same_key() {
        let mut app = viewing_running();
        assert!(!app.view.following);

        app.run("history.follow", None);
        assert!(app.view.following, "F should start following");
        assert!(matches!(app.note, Some((_, Note::Info))));

        app.run("history.follow", None);
        assert!(!app.view.following, "F again should stop");
    }

    #[test]
    fn follow_refuses_on_a_workflow_that_has_already_closed() {
        // Polling a closed workflow waits for events that can never arrive. "Closed" means
        // the *workflow* group has a terminal event — an activity finishing is not enough.
        use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("nav.open", None);

        let mut events = history_events();
        events.push(hev(9, G::Workflow, R::Closes, C::Workflow).with_outcome(O::Completed));
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((events, Vec::new())),
        });

        assert!(app.view.history_token.is_empty());
        app.run("history.follow", None);

        assert!(!app.view.following, "there is nothing to follow");
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Warn);
        assert!(msg.contains("closed"), "got {msg}");
    }

    #[test]
    fn follow_is_not_offered_away_from_a_history() {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("history.follow", None);
        assert!(!app.view.following);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn an_empty_token_while_following_means_the_workflow_closed() {
        let mut app = viewing_running();
        app.run("history.follow", None);
        assert!(app.view.following);

        // The long poll returns the terminal event with no continuation token.
        use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((
                vec![hev(9, G::Workflow, R::Closes, C::Workflow).with_outcome(O::Completed)],
                Vec::new(),
            )),
        });

        assert!(
            !app.view.following,
            "follow must stop when the workflow closes"
        );
        let (msg, _) = app.note.clone().unwrap();
        assert!(msg.contains("closed"), "got {msg}");
    }

    #[test]
    fn replayed_events_do_not_duplicate_when_follow_resumes() {
        // Follow resumes from the last non-empty token, which replays that page.
        let mut app = viewing_running();
        let before = app.view.history_events.len();

        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((running_history(), vec![9])),
        });
        assert_eq!(
            app.view.history_events.len(),
            before,
            "a replayed page must not be appended twice"
        );
    }

    #[test]
    fn the_resume_token_is_the_last_non_empty_one() {
        // Paging leaves history_token empty once caught up; following from that would
        // restart the read at event 1.
        let mut app = viewing_running();
        assert_eq!(app.view.history_resume, vec![9]);

        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((Vec::new(), Vec::new())),
        });
        assert!(app.view.history_token.is_empty(), "caught up");
        assert_eq!(
            app.view.history_resume,
            vec![9],
            "the resume point is remembered"
        );
    }

    #[test]
    fn leaving_the_history_stops_following() {
        let mut app = viewing_running();
        app.run("history.follow", None);
        assert!(app.view.following);

        app.run("nav.up", None);
        assert!(
            !app.view.following,
            "a poll must not outlive the screen it feeds"
        );
        assert!(app.view.history_resume.is_empty());
    }

    // ── piping ───────────────────────────────────────────────────────────────

    /// A history whose activity carries a JSON input and result.
    fn viewing_payloads() -> App {
        use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};

        let mut scheduled = hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("Charge");
        scheduled.payloads.push((
            "input".into(),
            Payload::new("json/plain", br#"{"amount":100}"#.to_vec()),
        ));
        let mut completed = hev(6, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed);
        completed.payloads.push((
            "result".into(),
            Payload::new("json/plain", b"\"charged\"".to_vec()),
        ));
        let mut secret = hev(7, G::Opened(7), R::Opens, C::Activity).with_subject("Secret");
        secret.payloads.push((
            "input".into(),
            Payload::new("binary/encrypted", vec![0u8; 16]),
        ));

        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("nav.open", None);
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((
                vec![
                    hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
                    scheduled,
                    completed,
                    secret,
                ],
                Vec::new(),
            )),
        });
        app
    }

    #[test]
    fn the_pipe_prompt_gathers_a_group_s_input_and_result() {
        let mut app = viewing_payloads();
        app.run("motion.down", None); // the Charge group
        let payloads = app.payloads_under_cursor();
        let labels: Vec<&str> = payloads.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            ["input", "result"],
            "a group's arguments and its result live on two different events"
        );
    }

    #[test]
    fn the_pipe_prompt_opens_prefilled_with_jq() {
        let mut app = viewing_payloads();
        app.run("motion.down", None);
        app.run("payload.pipe", None);

        let p = app.prompt.clone().expect("a prompt should open");
        assert_eq!(p.kind, PromptKind::Pipe);
        assert_eq!(
            p.buf, "jq .",
            "an empty prompt means retyping jq every time"
        );
        assert_eq!(p.sigil(), "!");
    }

    #[test]
    fn piping_is_refused_when_nothing_readable_is_under_the_cursor() {
        let mut app = viewing_payloads();
        app.run("motion.bottom", None); // the encrypted group
        app.run("payload.pipe", None);

        assert!(app.prompt.is_none(), "there is nothing worth piping");
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Warn);
        assert!(
            msg.contains("encrypted"),
            "the reason should be given: {msg}"
        );
    }

    #[test]
    fn piping_is_refused_away_from_a_history() {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("payload.pipe", None);
        assert!(app.prompt.is_none());
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn a_filter_result_is_dropped_when_the_cursor_moves() {
        // The output belonged to the row it was run on; leaving it up under a different
        // heading would be a lie.
        let mut app = viewing_payloads();
        app.run("motion.down", None);
        app.view.piped = Some(Ok("{}".into()));
        app.run("motion.down", None);
        assert!(app.view.piped.is_none());
    }

    #[test]
    fn a_pipe_result_message_opens_the_pane_and_lands() {
        let mut app = viewing_payloads();
        app.handle(Msg::Piped(Ok("42\n".into())));
        assert_eq!(app.view.piped, Some(Ok("42\n".into())));
        assert_eq!(app.view.detail_scroll, 0);
    }

    #[test]
    fn both_prompts_edit_the_same_way() {
        // `:` and `!` share their editing; only Enter differs. This pins that they do.
        use tmprl_core::Key;
        for open in ["app.command-line", "payload.pipe"] {
            let mut app = viewing_payloads();
            app.run("motion.down", None);
            app.run(open, None);
            let start = app.prompt.clone().unwrap().buf.len();

            app.handle(Msg::Key(Chord::ch('x')));
            assert_eq!(app.prompt.clone().unwrap().buf.len(), start + 1, "{open}");
            app.handle(Msg::Key(Chord::plain(Key::Backspace)));
            assert_eq!(app.prompt.clone().unwrap().buf.len(), start, "{open}");
            app.handle(Msg::Key(Chord::plain(Key::Esc)));
            assert!(app.prompt.is_none(), "{open}: Esc should close");
            assert_eq!(app.mode, Mode::Normal, "{open}");
        }
    }

    #[test]
    fn backspace_on_an_empty_prompt_closes_it() {
        use tmprl_core::Key;
        let mut app = viewing_payloads();
        app.run("app.command-line", None);
        app.handle(Msg::Key(Chord::plain(Key::Backspace)));
        assert!(app.prompt.is_none(), "as it does in vim");
    }

    /// The runner is IO, so it is exercised against real commands rather than mocked.
    #[tokio::test]
    async fn a_filter_receives_the_payloads_on_stdin() {
        let out = pipe_through("cat", br#"{"a":1}"#.to_vec()).await.unwrap();
        assert_eq!(out, r#"{"a":1}"#);
    }

    #[tokio::test]
    async fn a_failing_filter_reports_the_command_s_own_stderr() {
        // When a jq expression is wrong, jq's message is the entire diagnosis; paraphrasing
        // it would lose the line and column.
        let err = pipe_through("echo 'boom' >&2; exit 3", Vec::new())
            .await
            .unwrap_err();
        assert!(err.contains("boom"), "got {err:?}");
    }

    #[tokio::test]
    async fn a_filter_that_exits_silently_still_reports_failure() {
        let err = pipe_through("exit 1", Vec::new()).await.unwrap_err();
        assert!(err.contains("exited"), "got {err:?}");
    }

    #[tokio::test]
    async fn a_filter_that_ignores_its_input_does_not_error() {
        // `head -1` closes the pipe early; writing to a closed pipe is not a failure.
        let out = pipe_through("echo done", vec![b'x'; 1_000_000])
            .await
            .unwrap();
        assert_eq!(out.trim(), "done");
    }

    // ── codec server ─────────────────────────────────────────────────────────

    #[test]
    fn a_decoded_payload_replaces_the_encrypted_one_everywhere() {
        // Replacing in place is what lets the pane, `!` piping and yanking all read the
        // plaintext without knowing a codec exists.
        let mut app = viewing_payloads();
        app.run("motion.bottom", None); // the encrypted group

        let encrypted = app
            .payloads_under_cursor()
            .into_iter()
            .next()
            .map(|(_, p)| p)
            .expect("an encrypted payload");
        assert!(encrypted.needs_codec());
        let key = App::payload_key(&encrypted);

        app.handle(Msg::Decoded(Ok(vec![(
            key,
            Payload::new("json/plain", br#"{"secret":true}"#.to_vec()),
        )])));

        let (_, now) = app
            .payloads_under_cursor()
            .into_iter()
            .next()
            .expect("still a payload");
        assert!(!now.needs_codec(), "it should be plaintext now");
        assert_eq!(
            now.render(),
            tmprl_core::payload::Rendered::Text("{\n  \"secret\": true\n}".into())
        );
        // And it is pipeable, which it was not before.
        assert!(now.pipeable().is_some());
    }

    #[test]
    fn a_decode_failure_is_reported_and_can_be_retried() {
        // Leaving the in-flight set populated would make one failure permanent for the
        // session, with no way to ask again.
        let mut app = viewing_payloads();
        app.decoding.insert(42);
        app.handle(Msg::Decoded(Err("codec server returned 502".into())));

        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Error);
        assert!(msg.contains("502"), "the server's own words: {msg}");
        assert!(app.decoding.is_empty(), "a retry must be possible");
    }

    #[test]
    fn the_same_ciphertext_is_only_decoded_once() {
        let a = Payload::new("binary/encrypted", vec![1, 2, 3]);
        let b = Payload::new("binary/encrypted", vec![1, 2, 3]);
        let c = Payload::new("binary/encrypted", vec![9, 9, 9]);
        assert_eq!(App::payload_key(&a), App::payload_key(&b));
        assert_ne!(App::payload_key(&a), App::payload_key(&c));
    }

    #[test]
    fn a_payload_key_distinguishes_encodings_with_identical_bytes() {
        let a = Payload::new("binary/encrypted", vec![1, 2, 3]);
        let b = Payload::new("binary/plain", vec![1, 2, 3]);
        assert_ne!(App::payload_key(&a), App::payload_key(&b));
    }

    #[test]
    fn nothing_is_decoded_without_a_configured_codec() {
        // No endpoint means no request; the badge stays and says what is needed.
        let mut app = viewing_payloads();
        app.run("motion.bottom", None);
        app.run("history.detail", None);
        assert!(app.decoding.is_empty(), "there is nowhere to send it");
    }

    #[test]
    fn a_config_without_a_codec_section_is_not_an_error() {
        let mut app = app();
        app.apply_config(None, None, Some("# nothing here\n"));
        assert!(app.note.is_none());
        assert!(app.codec.is_none());
    }

    #[test]
    fn a_broken_config_is_surfaced() {
        let mut app = app();
        app.apply_config(None, None, Some("[codec]\nauth = \"x\"\n"));
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Error);
        assert!(msg.contains("codec.endpoint"), "got {msg}");
    }

    #[test]
    fn a_configured_codec_is_used() {
        let mut app = app();
        app.apply_config(
            None,
            None,
            Some("[codec]\nendpoint = \"http://localhost:8081\"\n"),
        );
        assert!(app.note.is_none());
        assert!(app.codec.is_some());
    }

    // ── windows ──────────────────────────────────────────────────────────────

    #[test]
    fn splitting_keeps_the_old_pane_and_focuses_a_fresh_one() {
        let mut app = app();
        app.view.namespaces = Loadable::loaded(vec![NamespaceInfo {
            name: "alpha".into(),
            state: "Registered".into(),
            retention_days: 1,
            description: String::new(),
        }]);

        app.run("window.split-right", None);
        assert_eq!(app.tabs.current().len(), 2);

        // The focused pane is the new one and starts empty.
        assert_eq!(app.view.namespace_rows().len(), 0);

        // The pane we came from must still hold what it had loaded.
        let others: Vec<_> = app
            .tabs
            .current()
            .views()
            .into_iter()
            .filter_map(|id| app.parked_view(id))
            .collect();
        assert_eq!(others.len(), 1, "exactly one pane is parked");
        assert_eq!(
            others[0].namespace_rows().len(),
            1,
            "the original pane kept its namespaces"
        );
    }

    #[test]
    fn a_new_pane_opens_where_you_split_from() {
        // Splitting is almost always "show me this again so I can take one of them
        // elsewhere". Landing back at the namespace list would make the diff case two
        // navigations instead of one keystroke.
        let mut app = app();
        app.view.screen = Screen::Workflows;
        app.view.query = "ExecutionStatus = 'Failed'".into();
        app.view.scope = vec!["payments".into()];

        app.run("window.split-right", None);
        assert_eq!(app.view.screen, Screen::Workflows);
        assert_eq!(app.view.query, "ExecutionStatus = 'Failed'");
        assert_eq!(app.view.scope, ["payments"]);
        // But none of the other pane's loaded data came with it.
        assert!(app.view.workflows.value().is_none());
    }

    #[test]
    fn focus_moves_between_panes_and_carries_their_state() {
        let mut app = app();
        app.view.query = "left".into();
        app.run("window.split-right", None);
        app.view.query = "right".into();

        app.run("window.focus-left", None);
        assert_eq!(app.view.query, "left", "each pane keeps its own query");
        app.run("window.focus-right", None);
        assert_eq!(app.view.query, "right");
    }

    #[test]
    fn closing_a_window_leaves_the_survivor_focused_with_its_own_state() {
        let mut app = app();
        app.view.query = "kept".into();
        app.run("window.split-right", None);
        app.view.query = "doomed".into();

        app.run("window.close", None);
        assert_eq!(app.tabs.current().len(), 1);
        assert_eq!(app.view.query, "kept");
    }

    #[test]
    fn the_last_window_refuses_to_close_and_says_how_to_quit() {
        let mut app = app();
        app.run("window.close", None);
        assert_eq!(app.tabs.current().len(), 1);
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Warn);
        assert!(msg.contains("quit"), "should point at the way out: {msg}");
    }

    #[test]
    fn tabs_keep_separate_windows_and_state() {
        let mut app = app();
        app.view.query = "first tab".into();
        app.run("window.split-right", None);
        assert_eq!(app.tabs.current().len(), 2);

        app.run("tab.new", None);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs.current().len(), 1, "a new tab has one window");
        assert_eq!(app.view.query, "", "and a fresh view");

        app.run("tab.previous", None);
        assert_eq!(app.tabs.current().len(), 2, "the split is still there");
        assert_eq!(app.view.query, "first tab");
    }

    #[test]
    fn the_last_tab_refuses_to_close() {
        let mut app = app();
        app.run("tab.close", None);
        assert_eq!(app.tabs.len(), 1);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn a_closed_window_stops_the_poll_it_was_running() {
        // View's Drop aborts the follow task. Without it a closed pane keeps a long poll
        // open and keeps pushing events at a pane that no longer exists.
        let mut app = app();
        app.run("window.split-right", None);
        app.view.following = true;
        app.run("window.close", None);
        assert!(!app.view.following, "the survivor was never following");
        assert_eq!(app.tabs.current().len(), 1);
    }

    // ── mutations ────────────────────────────────────────────────────────────

    fn on_a_workflow() -> App {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app
    }

    #[test]
    fn a_mutation_key_only_opens_a_confirmation() {
        // Nothing reaches the cluster until the reader says yes.
        let mut app = on_a_workflow();
        app.run("workflow.terminate", None);

        let c = app.confirm.clone().expect("a confirmation should open");
        assert_eq!(c.mutation.verb(), "Terminate");
        assert_eq!(c.mutation.workflow_id(), "order-r1");
        assert_eq!(c.mutation.namespace(), "default");
    }

    #[test]
    fn a_confirmation_owns_every_key_while_it_is_up() {
        // Nothing bound elsewhere may fire while a destructive action is pending.
        let mut app = on_a_workflow();
        let before = app.view.cursor;
        app.run("workflow.terminate", None);

        app.handle(Msg::Key(Chord::ch('j')));
        assert_eq!(app.view.cursor, before, "j must not move the cursor");
        assert!(
            app.confirm.is_some(),
            "and must not dismiss the confirmation"
        );

        app.handle(Msg::Key(Chord::ch(' ')));
        assert!(
            app.which_key.is_empty(),
            "the leader must not open which-key"
        );
    }

    #[test]
    fn escape_always_backs_out() {
        let mut app = on_a_workflow();
        app.run("workflow.terminate", None);
        app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Esc)));

        assert!(app.confirm.is_none());
        let (msg, _) = app.note.clone().unwrap();
        assert_eq!(msg, "cancelled");
    }

    #[test]
    fn deleting_costs_a_word_and_nearly_is_not_enough() {
        let mut app = on_a_workflow();
        app.run("workflow.delete", None);
        let c = app.confirm.clone().unwrap();
        assert_eq!(c.typed_word.as_deref(), Some("delete"));

        // Enter with the word unfinished is not a refusal, just not yet.
        for ch in "delet".chars() {
            app.handle(Msg::Key(Chord::ch(ch)));
        }
        app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));
        assert!(app.confirm.is_some(), "still waiting for the word");

        app.handle(Msg::Key(Chord::ch('e')));
        assert!(app.confirm.clone().unwrap().is_satisfied());
    }

    #[test]
    fn a_mutation_needs_something_under_the_cursor() {
        let mut app = app(); // namespace screen, nothing selected
        app.run("workflow.terminate", None);
        assert!(app.confirm.is_none());
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn a_history_screen_mutates_the_workflow_it_is_showing() {
        let mut app = on_a_workflow();
        app.run("nav.open", None);
        assert_eq!(app.view.screen, Screen::History);

        app.run("workflow.cancel", None);
        let c = app
            .confirm
            .clone()
            .expect("the open workflow is the target");
        assert_eq!(c.mutation.workflow_id(), "order-r1");
    }

    #[test]
    fn a_signal_asks_for_its_name_before_confirming() {
        let mut app = on_a_workflow();
        app.run("workflow.signal", None);
        assert!(app.confirm.is_none(), "a signal needs a name first");
        assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Signal);

        for ch in "retry".chars() {
            app.handle(Msg::Key(Chord::ch(ch)));
        }
        app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

        let c = app.confirm.clone().expect("now it can be confirmed");
        assert!(
            c.mutation.cli().contains("--name retry"),
            "{}",
            c.mutation.cli()
        );
        assert!(!c.mutation.is_destructive(), "a signal is not a loss");
    }

    #[test]
    fn a_reset_resolves_to_a_workflow_task_the_cursor_is_not_on() {
        // The rows the server needs are exactly the ones the outline folds away, so "reset
        // to here" walks back — and the confirmation shows which id it landed on.
        use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
        let mut app = on_a_workflow();
        app.run("nav.open", None);
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((
                vec![
                    hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("W"),
                    hev(2, G::Opened(2), R::Opens, C::WorkflowTask),
                    hev(3, G::Opened(2), R::Closes, C::WorkflowTask).with_outcome(O::Completed),
                    hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("A"),
                    hev(5, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed),
                ],
                Vec::new(),
            )),
        });
        app.run("motion.bottom", None); // the activity group, not a workflow task

        app.run("workflow.reset", None);
        let c = app.confirm.clone().expect("a confirmation");
        assert!(
            c.mutation.cli().contains("--event-id 3"),
            "should resolve back to the completed workflow task: {}",
            c.mutation.cli()
        );
        assert!(c.mutation.is_destructive(), "a reset abandons work");
    }

    #[test]
    fn a_reset_needs_a_history_and_says_so() {
        let mut app = on_a_workflow(); // still on the workflow list
        app.run("workflow.reset", None);
        assert!(app.confirm.is_none());
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Warn);
        assert!(msg.contains("history"), "got {msg}");
    }

    #[test]
    fn a_history_with_no_completed_task_cannot_be_reset() {
        use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
        let mut app = on_a_workflow();
        app.run("nav.open", None);
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((
                vec![hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("W")],
                Vec::new(),
            )),
        });
        app.run("workflow.reset", None);
        assert!(app.confirm.is_none(), "there is nowhere valid to reset to");
    }

    #[test]
    fn an_update_asks_for_its_name_and_is_not_destructive() {
        let mut app = on_a_workflow();
        app.run("workflow.update", None);
        assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Update);
        assert_eq!(app.prompt.clone().unwrap().sigil(), "update:");

        for ch in "setLimit".chars() {
            app.handle(Msg::Key(Chord::ch(ch)));
        }
        app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

        let c = app.confirm.clone().expect("a confirmation");
        assert!(
            c.mutation.cli().contains("update execute"),
            "{}",
            c.mutation.cli()
        );
        assert!(c.mutation.cli().contains("--name setLimit"));
        assert!(
            !c.mutation.is_destructive(),
            "an update adds, it does not end"
        );
    }

    #[test]
    fn a_signal_and_an_update_do_not_get_confused() {
        let mut app = on_a_workflow();
        app.run("workflow.signal", None);
        for ch in "ping".chars() {
            app.handle(Msg::Key(Chord::ch(ch)));
        }
        app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));
        let cli = app.confirm.clone().unwrap().mutation.cli();
        assert!(cli.contains("workflow signal"), "{cli}");
        assert!(!cli.contains("update"), "{cli}");
    }

    #[test]
    fn a_finished_mutation_reports_and_refreshes() {
        let mut app = on_a_workflow();
        let m = Mutation::Cancel {
            namespace: "default".into(),
            workflow_id: "order-r1".into(),
            run_id: "r1".into(),
        };
        app.handle(Msg::Mutated {
            mutation: Box::new(m),
            result: Ok(()),
        });
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Info);
        assert!(msg.contains("order-r1"), "got {msg}");
    }

    #[test]
    fn a_failed_mutation_shows_the_servers_reason() {
        let mut app = on_a_workflow();
        app.handle(Msg::Mutated {
            mutation: Box::new(Mutation::Cancel {
                namespace: "default".into(),
                workflow_id: "w".into(),
                run_id: "r".into(),
            }),
            result: Err("PermissionDenied: not allowed".into()),
        });
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Error);
        assert!(msg.contains("PermissionDenied"), "got {msg}");
    }

    fn on_schedules() -> App {
        let mut app = app();
        app.view.screen = Screen::Schedules;
        app.view.scope = vec!["default".into()];
        app.view.schedules = Loadable::loaded(vec![ScheduleRow {
            namespace: "default".into(),
            schedule_id: "nightly".into(),
            workflow_type: "Reconcile".into(),
            paused: false,
            notes: String::new(),
            spec: "0 2 * * *".into(),
            next_run: None,
            recent_runs: 0,
        }]);
        app
    }

    #[test]
    fn pausing_toggles_towards_the_opposite_of_now() {
        // One key does both, so the target state is whatever the schedule is not.
        let mut app = on_schedules();
        app.run("schedule.pause", None);
        let m = app.confirm.clone().unwrap().mutation;
        assert_eq!(m.verb(), "Pause");
        assert!(m.cli().ends_with("--pause"));

        app.confirm = None;
        app.view.schedules.value_mut().unwrap()[0].paused = true;
        app.run("schedule.pause", None);
        let m = app.confirm.clone().unwrap().mutation;
        assert_eq!(m.verb(), "Resume");
        assert!(m.cli().ends_with("--unpause"));
    }

    #[test]
    fn a_paused_schedule_shows_the_new_state_before_the_list_catches_up() {
        // ListSchedules is eventually consistent, so a refresh straight after the patch can
        // return the old state and contradict the message beside it.
        let mut app = on_schedules();
        app.handle(Msg::Mutated {
            mutation: Box::new(Mutation::PauseSchedule {
                namespace: "default".into(),
                schedule_id: "nightly".into(),
                paused: true,
            }),
            result: Ok(()),
        });
        assert!(app.view.schedule_rows()[0].paused);
    }

    #[test]
    fn deleting_a_schedule_costs_the_typed_word() {
        let mut app = on_schedules();
        app.run("schedule.delete", None);
        let c = app.confirm.clone().unwrap();
        assert_eq!(c.typed_word.as_deref(), Some("delete"));
        assert!(c.mutation.cli().starts_with("temporal schedule delete "));
    }

    #[test]
    fn schedule_keys_need_a_schedule_under_the_cursor() {
        let mut app = app(); // namespace screen
        app.run("schedule.trigger", None);
        assert!(app.confirm.is_none());
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn gs_and_gw_switch_lists_within_a_namespace() {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        assert_eq!(app.view.screen, Screen::Workflows);

        app.run("nav.schedules", None);
        assert_eq!(app.view.screen, Screen::Schedules);
        app.run("nav.workflows", None);
        assert_eq!(app.view.screen, Screen::Workflows);
    }

    #[test]
    fn switching_lists_is_refused_from_a_namespace_or_a_history() {
        // From a history the reader is inside one workflow; jumping sideways loses the place.
        let mut app = app();
        app.run("nav.schedules", None);
        assert_eq!(app.view.screen, Screen::Namespaces);
        assert!(matches!(app.note, Some((_, Note::Warn))));

        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("nav.open", None);
        assert_eq!(app.view.screen, Screen::History);
        app.run("nav.schedules", None);
        assert_eq!(app.view.screen, Screen::History, "still in the history");
    }

    #[test]
    fn dash_from_schedules_goes_back_to_namespaces() {
        let mut app = on_schedules();
        app.run("nav.up", None);
        assert_eq!(app.view.screen, Screen::Namespaces);
    }

    #[test]
    fn config_errors_are_surfaced_rather_than_swallowed() {
        let mut app = app();
        app.apply_config(Some("[normal]\n\"x\" = \"nope.nope\"\n"), None, None);
        let (msg, kind) = app.note.clone().expect("a bad binding must be reported");
        assert_eq!(kind, Note::Error);
        assert!(msg.contains("nope.nope"), "got {msg}");
    }

    #[test]
    fn views_from_config_become_commands_and_bindings() {
        let mut app = app();
        app.apply_config(
            None,
            Some("[[view]]\nkey = \"1\"\nname = \"Running\"\nquery = \"ExecutionStatus = 'Running'\"\n"),
            None,
        );
        assert!(app.note.is_none(), "a valid config must not warn");
        assert_eq!(app.registry.get("view.1").unwrap().title, "Running");

        app.handle(Msg::Key(Chord::ch(' ')));
        app.handle(Msg::Key(Chord::ch('1')));
        assert_eq!(app.view.query, "ExecutionStatus = 'Running'");
    }

    #[test]
    fn opening_nothing_says_so_instead_of_changing_screen() {
        let mut app = app();
        app.run("nav.open", None);
        assert_eq!(app.view.screen, Screen::Namespaces);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn going_up_from_the_top_level_says_so() {
        let mut app = app();
        app.run("nav.up", None);
        assert_eq!(app.view.screen, Screen::Namespaces);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }
}
