//! Application state and the reducer.
//!
//! The one rule: [`App::handle`] is synchronous and never awaits. When it needs data it
//! spawns a task, which reports back as another [`Msg`]. Nothing on the keystroke path can
//! block on the network — see `docs/ARCHITECTURE.md`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tmprl_client::{Codec, Conn, NamespaceInfo};
use tmprl_core::history::{NormalizedEvent, group_events, merge_events};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::Payload;
use tmprl_core::{
    Action, Chord, Keymap, Loadable, Mode, Pending, PendingEntry, Registry, Resolution, SavedView,
    StatusCounts, WorkflowList, WorkflowRow, default_keymap,
};
use tokio::sync::mpsc::UnboundedSender;

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
    /// Output of an external command a `!` filter ran.
    Piped(Result<String, String>),
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
    pub mode: Mode,
    pub pending: Pending,
    pub registry: Registry,
    pub keymap: Keymap,

    pub screen: Screen,
    pub namespaces: Loadable<Vec<NamespaceInfo>>,
    pub workflows: Loadable<WorkflowList>,
    pub counts: Loadable<StatusCounts>,
    pub history: Loadable<Outline>,
    /// The workflow whose history is on screen.
    pub viewing: Option<WorkflowRow>,
    /// Whether the history is being tailed. Shown in the statusline, because a view that
    /// silently changes under you is worse than one that does not update.
    pub following: bool,
    /// Whether the payload pane is open under the history list.
    pub show_detail: bool,
    /// Output of the last `!` filter, shown in the pane in place of the payloads. `Err` is
    /// the command's own stderr, which is the only useful thing to show when jq rejects a
    /// filter.
    pub piped: Option<Result<String, String>>,
    /// Payloads a codec server has already decoded, keyed by the hash of the encrypted
    /// bytes. Decoding is a network hop per payload, and scrolling back over a row that has
    /// already been decoded should cost nothing.
    decoded: HashMap<u64, Payload>,
    /// Requests in flight, so a cursor resting on a row does not ask repeatedly.
    decoding: HashSet<u64>,
    codec: Option<Arc<Codec>>,
    /// First visible line of the payload pane, and how far it can usefully go. A payload can
    /// be far taller than the pane — clipping a stack trace silently hides its end, which is
    /// the part worth reading.
    pub detail_scroll: usize,
    pub detail_max_scroll: usize,

    /// The visibility query, verbatim. This string is the interface: everything that
    /// filters the list compiles into it, and it is always on screen and always editable.
    pub query: String,
    /// Namespaces the workflow list is fanned out over.
    pub scope: Vec<String>,
    pub views: Vec<SavedView>,

    pub cursor: usize,
    /// Where a visual selection started, if one is active.
    pub anchor: Option<usize>,
    /// Rows the list pane can show — set by the renderer, used by half-page motions.
    pub page: usize,

    pub which_key: Vec<PendingEntry>,
    pub show_help: bool,
    /// First visible line of the help overlay, and the largest useful value for it. The
    /// overlay is taller than most terminals now, so it scrolls with the ordinary motions
    /// rather than silently clipping the last groups.
    pub help_scroll: usize,
    pub help_max_scroll: usize,
    /// `Some` while a `:` or `!` prompt is open.
    pub prompt: Option<Prompt>,
    pub insert_buf: String,
    pub insert_target: InsertTarget,

    pub note: Option<(String, Note)>,
    pub should_quit: bool,
    pub dirty: bool,

    /// The row the cursor is on, by identity rather than by index. Rows arrive above the
    /// cursor on a live list, so an index silently drifts onto a different workflow.
    cursor_key: Option<(String, String)>,
    /// Cursor position on the namespace screen, restored by `-`.
    namespace_cursor: usize,
    /// Cursor position on the workflow list, restored by `-` from a history.
    workflow_cursor: usize,
    /// Bumped whenever the query or scope changes. Replies carrying an older generation
    /// belong to a query the user has already moved on from.
    generation: u64,
    /// Every history event loaded so far, for re-grouping when a page arrives.
    history_events: Vec<NormalizedEvent>,
    /// Continuation token for the history being read. Empty means fully loaded.
    history_token: Vec<u8>,
    /// The last *non-empty* token seen. Follow resumes from here: an empty token restarts
    /// from event 1, and paging leaves the token empty once it has caught up.
    history_resume: Vec<u8>,
    /// The follow task, so toggling off — or leaving the screen — actually stops the poll.
    follow_task: Option<tokio::task::JoinHandle<()>>,
    /// A page request is in flight; scrolling must not queue a second one.
    loading_more: bool,

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
            mode: Mode::Normal,
            pending: Pending::default(),
            registry: Registry::builtin(),
            keymap: default_keymap(),
            screen: Screen::Namespaces,
            namespaces: Loadable::NotAsked,
            workflows: Loadable::NotAsked,
            counts: Loadable::NotAsked,
            history: Loadable::NotAsked,
            viewing: None,
            following: false,
            show_detail: false,
            piped: None,
            decoded: HashMap::new(),
            decoding: HashSet::new(),
            codec: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            query: String::new(),
            scope: vec![namespace.clone()],
            views: Vec::new(),
            cursor: 0,
            anchor: None,
            page: 10,
            which_key: Vec::new(),
            show_help: false,
            help_scroll: 0,
            help_max_scroll: 0,
            prompt: None,
            insert_buf: String::new(),
            insert_target: InsertTarget::Scratch,
            note: None,
            should_quit: false,
            dirty: true,
            cursor_key: None,
            namespace_cursor: 0,
            workflow_cursor: 0,
            generation: 0,
            history_events: Vec::new(),
            history_token: Vec::new(),
            history_resume: Vec::new(),
            follow_task: None,
            loading_more: false,
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
        self.namespaces.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn workflow_rows(&self) -> &[WorkflowRow] {
        self.workflows
            .value()
            .map(WorkflowList::rows)
            .unwrap_or(&[])
    }

    /// How many rows the focused screen has.
    pub fn row_count(&self) -> usize {
        match self.screen {
            Screen::Namespaces => self.namespace_rows().len(),
            Screen::Workflows => self.workflow_rows().len(),
            Screen::History => self.history.value().map(Outline::len).unwrap_or(0),
        }
    }

    /// Whether the workflow list is fanned out over more than one namespace, which is when
    /// rows need to say which namespace they came from.
    pub fn is_fanned_out(&self) -> bool {
        self.scope.len() > 1
    }

    /// The query text to display: the live edit while Insert mode owns it, otherwise the
    /// applied query.
    pub fn query_display(&self) -> &str {
        if self.mode == Mode::Insert && self.insert_target == InsertTarget::Query {
            &self.insert_buf
        } else {
            &self.query
        }
    }

    pub fn is_editing_query(&self) -> bool {
        self.mode == Mode::Insert && self.insert_target == InsertTarget::Query
    }

    /// The inclusive row range currently selected, if in a visual mode.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.selection().is_some_and(|(lo, hi)| i >= lo && i <= hi)
    }

    // ── the reducer ──────────────────────────────────────────────────────────

    pub fn handle(&mut self, msg: Msg) {
        self.dirty = true;
        match msg {
            Msg::Key(chord) => self.on_key(chord),
            Msg::Quit => self.should_quit = true,
            Msg::Tick | Msg::Redraw => {}
            Msg::Piped(result) => {
                self.piped = Some(result);
                self.detail_scroll = 0;
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
                self.namespaces = Loadable::loaded(list);
                self.clamp_cursor();
            }
            Msg::Namespaces(Err(e)) => {
                self.note = Some((e.clone(), Note::Error));
                self.namespaces = Loadable::Failed(e);
            }
            Msg::Workflows {
                generation,
                append,
                result,
            } => {
                if generation != self.generation {
                    return; // a reply for a query the user has already replaced
                }
                self.loading_more = false;
                match result {
                    Ok((rows, tokens)) => {
                        match (append, self.workflows.value_mut()) {
                            (true, Some(list)) => list.append(rows, tokens),
                            _ => {
                                let mut list = WorkflowList::default();
                                list.reset(rows, tokens);
                                self.workflows = Loadable::loaded(list);
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
                            self.workflows = Loadable::Failed(e);
                        }
                    }
                }
            }
            Msg::History { generation, result } => {
                if generation != self.generation {
                    return;
                }
                self.loading_more = false;
                match result {
                    Ok((events, token)) => {
                        if !token.is_empty() {
                            self.history_resume = token.clone();
                        } else if self.following {
                            // Follow only ever sees an empty token when the workflow has
                            // closed. There is nothing further to tail, so stop rather than
                            // spin on a call that now returns instantly.
                            self.stop_following();
                            self.note =
                                Some(("workflow closed — follow stopped".into(), Note::Info));
                        }
                        self.history_token = token;
                        // Merged, not appended: a resumed follow replays the page its token
                        // sat in, and listing those events twice would inflate every group.
                        merge_events(&mut self.history_events, events);
                        // Re-group the whole accumulated history rather than patching: a
                        // page boundary can land in the middle of a group, so the last
                        // group of a page is routinely completed by the next one.
                        let groups = group_events(&self.history_events);
                        let events = self.history_events.clone();
                        match self.history.value_mut() {
                            Some(outline) => outline.replace(events, groups),
                            None => self.history = Loadable::loaded(Outline::new(events, groups)),
                        }
                        self.clamp_cursor();
                    }
                    Err(e) => {
                        self.note = Some((e.clone(), Note::Error));
                        if self.history_events.is_empty() {
                            self.history = Loadable::Failed(e);
                        }
                    }
                }
            }
            Msg::Counts { generation, result } => {
                if generation != self.generation {
                    return;
                }
                self.counts = match result {
                    Ok(c) => Loadable::loaded(c),
                    Err(e) => Loadable::Failed(e),
                };
            }
        }
    }

    fn on_key(&mut self, chord: Chord) {
        // The command line owns every key while it is open, so that `:` can accept a name
        // containing characters that are bound elsewhere.
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
                    self.anchor = None;
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
                self.scroll_help((self.page / 2).max(1) as isize)
            }
            Action::HalfPageUp if self.show_help => {
                self.scroll_help(-((self.page / 2).max(1) as isize))
            }

            Action::MoveDown => self.move_cursor(n as isize),
            Action::MoveUp => self.move_cursor(-(n as isize)),
            Action::MoveTop => self.set_cursor(0),
            Action::MoveBottom => self.set_cursor(self.row_count().saturating_sub(1)),
            Action::HalfPageDown => self.move_cursor((self.page / 2).max(1) as isize),
            Action::HalfPageUp => self.move_cursor(-((self.page / 2).max(1) as isize)),

            Action::OpenItem => self.open_focused(),
            Action::GoUp => self.go_up(),

            Action::EnterInsert => {
                self.mode = Mode::Insert;
                // On the workflow list the only text field is the query bar, so that is
                // what Insert mode edits. It is seeded with the applied query so `i` is an
                // edit, not a retype.
                if self.screen == Screen::Workflows {
                    self.insert_target = InsertTarget::Query;
                    self.insert_buf = self.query.clone();
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
                self.anchor = Some(self.cursor);
            }
            Action::EnterVisualLine => {
                self.mode = Mode::VisualLine;
                self.anchor = Some(self.cursor);
            }

            Action::YankField => self.yank(self.field_under_cursor()),
            Action::YankRecord => self.yank(self.records_selected()),

            Action::LoadMore => self.load_more(),
            Action::SelectView(key) => self.select_view(key),

            Action::ToggleFold => self.toggle_fold(),
            Action::ExpandAll => self.with_outline(|o| o.expand_all()),
            Action::CollapseAll => self.with_outline(|o| o.collapse_all()),
            Action::TogglePlumbing => {
                let showing = self.history.value().is_some_and(Outline::show_plumbing);
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
            Action::ToggleDetail => {
                if self.screen == Screen::History {
                    self.show_detail = !self.show_detail;
                    self.detail_scroll = 0;
                    self.piped = None;
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
        match self.screen {
            Screen::Namespaces => {
                // A visual selection opens every namespace in it as one merged list. That
                // is the whole multi-namespace fan-out: `V j j <CR>`, using the selection
                // machinery that already exists rather than a separate picker.
                let (lo, hi) = self.selection().unwrap_or((self.cursor, self.cursor));
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

                self.namespace_cursor = self.cursor;
                self.anchor = None;
                self.mode = Mode::Normal;
                self.screen = Screen::Workflows;
                self.scope = scope;
                self.cursor = 0;
                self.cursor_key = None;
                self.load_workflows(false);
            }
            Screen::Workflows => {
                let Some(row) = self.workflow_rows().get(self.cursor).cloned() else {
                    self.note = Some(("nothing to open".into(), Note::Warn));
                    return;
                };
                self.workflow_cursor = self.cursor;
                self.anchor = None;
                self.mode = Mode::Normal;
                self.screen = Screen::History;
                self.viewing = Some(row);
                self.cursor = 0;
                self.load_history();
            }
            // On the history screen, "open the focused item" is folding a group open.
            Screen::History => self.toggle_fold(),
        }
    }

    fn go_up(&mut self) {
        match self.screen {
            Screen::History => {
                self.stop_following();
                self.screen = Screen::Workflows;
                self.cursor = self.workflow_cursor;
                self.viewing = None;
                self.history = Loadable::NotAsked;
                self.history_events.clear();
                self.history_token.clear();
                self.history_resume.clear();
                self.restore_cursor();
            }
            Screen::Workflows => {
                self.screen = Screen::Namespaces;
                self.cursor = self.namespace_cursor;
                self.anchor = None;
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
        match self.history.value()?.row_at(self.cursor)? {
            Row::Group { group, .. } | Row::Event { group, .. } => Some(group),
        }
    }

    fn toggle_fold(&mut self) {
        let Some(group) = self.group_under_cursor() else {
            return;
        };
        // Folding shut from inside a group would otherwise strand the cursor past the end;
        // `toggle` hands back where the group's own line is now, so it moves there.
        if let Some(outline) = self.history.value_mut()
            && let Some(row) = outline.toggle(group)
        {
            self.cursor = row;
        }
        self.clamp_cursor();
    }

    /// Apply a shape change, then put the cursor back on the group it was on. Expanding or
    /// collapsing everything moves every row, so an unadjusted cursor lands somewhere
    /// arbitrary.
    fn with_outline(&mut self, f: impl FnOnce(&mut Outline)) {
        let was = self.group_under_cursor();
        let Some(outline) = self.history.value_mut() else {
            return;
        };
        f(outline);
        self.cursor = was
            .and_then(|g| outline.row_of_group(g))
            .unwrap_or(self.cursor);
        self.clamp_cursor();
    }

    // ── follow mode ──────────────────────────────────────────────────────────

    fn scroll_detail(&mut self, delta: isize) {
        let next = (self.detail_scroll as isize + delta).clamp(0, self.detail_max_scroll as isize);
        self.detail_scroll = next as usize;
    }

    fn toggle_follow(&mut self) {
        if self.screen != Screen::History {
            self.note = Some(("follow applies to a workflow history".into(), Note::Warn));
            return;
        }
        if self.following {
            self.stop_following();
            self.note = Some(("follow stopped".into(), Note::Info));
            return;
        }
        // Following a workflow that has already finished would poll forever for events that
        // can never arrive, so say so instead.
        if self.history_token.is_empty() && self.workflow_is_closed() {
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
        self.history
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
        let Some(row) = self.viewing.clone() else {
            return;
        };
        self.following = true;
        self.note = Some(("following — F to stop".into(), Note::Info));

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.generation);
        let mut token = self.history_resume.clone();

        self.follow_task = Some(tokio::spawn(async move {
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
        self.following = false;
        if let Some(task) = self.follow_task.take() {
            task.abort();
        }
    }

    fn jump_failure(&mut self, forward: bool) {
        let Some(outline) = self.history.value() else {
            return;
        };
        let found = if forward {
            outline.next_failure(self.cursor)
        } else {
            outline.prev_failure(self.cursor)
        };
        match found {
            Some(row) => self.cursor = row,
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
        self.query = query;
        if self.screen == Screen::Namespaces {
            self.screen = Screen::Workflows;
        }
        self.note = Some((format!("view: {name}"), Note::Info));
        self.load_workflows(false);
    }

    fn refresh(&mut self) {
        match self.screen {
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => {
                self.history_events.clear();
                self.history_token.clear();
                self.history_resume.clear();
                self.load_history();
            }
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
            self.cursor = 0;
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, len as isize - 1);
        self.set_cursor(next as usize);
    }

    fn set_cursor(&mut self, at: usize) {
        if at != self.cursor {
            // The pane now shows a different value; keeping the old offset would open it
            // part-way down something the reader has not seen the start of. A filter result
            // belonged to the row it was run on, so it goes too rather than sitting under a
            // heading that no longer describes it.
            self.detail_scroll = 0;
            self.piped = None;
        }
        self.cursor = at;
        self.maybe_decode();
        self.remember_cursor();
        self.maybe_load_more();
    }

    /// Record which row the cursor is on, by identity. This is what a refresh restores.
    fn remember_cursor(&mut self) {
        if self.screen == Screen::Workflows {
            self.cursor_key = self
                .workflow_rows()
                .get(self.cursor)
                .map(|r| (r.namespace.clone(), r.run_id.clone()));
        }
    }

    /// Put the cursor back on the row it was on, wherever that row has moved to.
    fn restore_cursor(&mut self) {
        let Some((ns, run)) = self.cursor_key.clone() else {
            self.clamp_cursor();
            return;
        };
        if let Some(list) = self.workflows.value()
            && let Some(at) = list.position_of((&ns, &run))
        {
            self.cursor = at;
        }
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.row_count();
        self.cursor = self.cursor.min(len.saturating_sub(1));
        if len == 0 {
            self.cursor = 0;
        }
    }

    /// Infinite scroll: fetch the next page once the cursor is within a screen of the end.
    fn maybe_load_more(&mut self) {
        if self.loading_more {
            return;
        }
        let len = self.row_count();
        let near_end = self.cursor + self.page.max(1) >= len;
        if !near_end {
            return;
        }
        match self.screen {
            Screen::Workflows => {
                if self.workflows.value().is_some_and(WorkflowList::has_more) {
                    self.load_more();
                }
            }
            Screen::History => {
                if !self.history_token.is_empty() {
                    self.load_history();
                }
            }
            Screen::Namespaces => {}
        }
    }

    // ── yanking ──────────────────────────────────────────────────────────────

    fn field_under_cursor(&self) -> String {
        match self.screen {
            Screen::Namespaces => self
                .namespace_rows()
                .get(self.cursor)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            // The workflow id is the field you actually want to paste into a CLI command.
            Screen::Workflows => self
                .workflow_rows()
                .get(self.cursor)
                .map(|w| w.workflow_id.clone())
                .unwrap_or_default(),
            Screen::History => self.history_field_under_cursor(),
        }
    }

    /// On a group line, the thing it is about; on an event line, the event's own name. Both
    /// are what you would paste into a search or a CLI command.
    fn history_field_under_cursor(&self) -> String {
        let Some(outline) = self.history.value() else {
            return String::new();
        };
        match outline.row_at(self.cursor) {
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
        let (lo, hi) = self.selection().unwrap_or((self.cursor, self.cursor));
        let take = hi.saturating_sub(lo) + 1;
        let picked: Vec<String> = match self.screen {
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
        let Some(outline) = self.history.value() else {
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
                self.anchor = None;
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
        self.query = self.insert_buf.clone();
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
                }
            }
            // Backspace on an empty line closes the prompt, as it does in vim.
            Key::Backspace if prompt.buf.pop().is_none() => self.close_prompt(),
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => prompt.buf.push(c),
            _ => {}
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
        if !self.show_detail || self.screen != Screen::History {
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
        for event in &mut self.history_events {
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
        let groups = group_events(&self.history_events);
        let events = self.history_events.clone();
        match self.history.value_mut() {
            Some(outline) => outline.replace(events, groups),
            None => self.history = Loadable::loaded(Outline::new(events, groups)),
        }
    }

    // ── piping payloads through an external command ──────────────────────────

    /// The payloads the cursor is on, as one JSON object.
    ///
    /// For a group that is its input *and* its result, which live on two different events —
    /// the same pair the payload pane shows.
    fn payloads_under_cursor(&self) -> Vec<(String, tmprl_core::payload::Payload)> {
        let Some(outline) = self.history.value() else {
            return Vec::new();
        };
        match outline.row_at(self.cursor) {
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
        if self.screen != Screen::History {
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
        self.show_detail = true;
        self.detail_scroll = 0;
        self.piped = Some(Ok(format!("running `{command}`…")));

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
        self.namespaces.begin_refresh();
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
            self.generation = self.generation.wrapping_add(1);
            self.workflows.begin_refresh();
            self.counts.begin_refresh();
            self.load_counts();
        }

        // Set before the connection guard: this records the decision to fetch, which is
        // what stops a second page being queued while the first is still in flight.
        self.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        // On a continuation, ask only the namespaces that still have pages. Passing the
        // whole scope would hand an exhausted namespace an empty token, which the server
        // reads as "start again" — so it would never finish.
        let tokens: Tokens = if append {
            self.workflows
                .value()
                .map(|l| l.tokens().to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let (tx, generation, scope, query) = (
            self.tx.clone(),
            self.generation,
            self.scope.clone(),
            self.query.clone(),
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
        let Some(row) = self.viewing.clone() else {
            return;
        };
        if self.history_events.is_empty() {
            self.generation = self.generation.wrapping_add(1);
            self.history.begin_refresh();
        }
        self.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation, token) =
            (self.tx.clone(), self.generation, self.history_token.clone());
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
        let has_more = self.workflows.value().is_some_and(WorkflowList::has_more);
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
            self.generation,
            self.scope.clone(),
            self.query.clone(),
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
        app.screen = Screen::Workflows;
        app.handle(Msg::Workflows {
            generation: app.generation,
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
        assert_eq!(app.workflow_rows()[app.cursor].run_id, "r1");

        app.handle(Msg::Workflows {
            generation: app.generation,
            append: false,
            result: Ok((
                vec![wf("default", "r9", 900), wf("default", "r1", 100)],
                vec![],
            )),
        });
        assert_eq!(app.cursor, 1, "cursor should have followed r1 down a row");
        assert_eq!(app.workflow_rows()[app.cursor].run_id, "r1");
    }

    #[test]
    fn a_reply_for_a_superseded_query_is_dropped() {
        // Type a new query while the old one is still in flight: the stale reply must not
        // repaint the table with rows the user is no longer looking at.
        let mut app = app();
        loaded(&mut app, vec![wf("default", "old", 100)], vec![]);
        let stale = app.generation;

        app.query = "WorkflowType = 'New'".into();
        app.load_workflows(false);
        assert_ne!(app.generation, stale);

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
            generation: app.generation,
            append: true,
            result: Err("connection reset".into()),
        });
        assert_eq!(app.workflow_rows().len(), 1, "rows must survive");
        assert!(matches!(app.note, Some((_, Note::Error))));
    }

    #[test]
    fn a_failed_first_page_shows_the_error_state() {
        let mut app = app();
        app.screen = Screen::Workflows;
        app.handle(Msg::Workflows {
            generation: app.generation,
            append: false,
            result: Err("permission denied".into()),
        });
        assert_eq!(app.workflows.error(), Some("permission denied"));
    }

    #[test]
    fn enter_opens_a_namespace_and_dash_goes_back() {
        let mut app = app();
        app.namespaces = Loadable::loaded(vec![
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

        assert_eq!(app.screen, Screen::Workflows);
        assert_eq!(
            app.scope,
            ["beta"],
            "the focused namespace becomes the scope"
        );

        app.run("nav.up", None);
        assert_eq!(app.screen, Screen::Namespaces);
        assert_eq!(app.cursor, 1, "the namespace cursor is restored");
    }

    #[test]
    fn a_visual_selection_of_namespaces_opens_a_fan_out() {
        let mut app = app();
        app.namespaces = Loadable::loaded(
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

        assert_eq!(app.scope, ["alpha", "beta"]);
        assert!(
            app.is_fanned_out(),
            "rows must be tagged with their namespace"
        );
        assert_eq!(app.mode, Mode::Normal, "opening ends the selection");
        assert!(app.anchor.is_none());
    }

    #[test]
    fn opening_without_a_selection_scopes_to_one_namespace() {
        let mut app = app();
        app.namespaces = Loadable::loaded(vec![NamespaceInfo {
            name: "alpha".into(),
            state: "Registered".into(),
            retention_days: 1,
            description: String::new(),
        }]);
        app.run("nav.open", None);
        assert_eq!(app.scope, ["alpha"]);
        assert!(!app.is_fanned_out());
    }

    #[test]
    fn insert_mode_edits_the_query_on_the_workflow_screen() {
        let mut app = app();
        loaded(&mut app, vec![], vec![]);
        app.query = "A = 1".into();

        app.run("mode.insert", None);
        assert!(app.is_editing_query());
        assert_eq!(app.insert_buf, "A = 1", "the edit starts from the query");

        app.handle(Msg::Key(Chord::plain(Key::Backspace)));
        type_chars(&mut app, "2");
        assert_eq!(app.query_display(), "A = 2");

        app.handle(Msg::Key(Chord::plain(Key::Enter)));
        assert_eq!(app.query, "A = 2", "Enter applies the query");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn escape_abandons_a_query_edit() {
        let mut app = app();
        loaded(&mut app, vec![], vec![]);
        app.query = "A = 1".into();

        app.run("mode.insert", None);
        type_chars(&mut app, "999");
        app.handle(Msg::Key(Chord::plain(Key::Esc)));

        assert_eq!(app.query, "A = 1", "Esc must not apply the edit");
        assert_eq!(app.query_display(), "A = 1");
    }

    #[test]
    fn insert_mode_on_the_namespace_screen_is_not_the_query_bar() {
        let mut app = app();
        app.run("mode.insert", None);
        assert!(!app.is_editing_query());
        type_chars(&mut app, "xy");
        assert_eq!(app.insert_buf, "xy");
        assert_eq!(app.query, "", "the namespace screen has no query bar");
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
        assert_eq!(app.query, "ExecutionStatus = 'Failed'");
        assert_eq!(app.screen, Screen::Workflows);

        // Still text, still editable — a view is a bookmark, not a mode.
        app.run("mode.insert", None);
        assert_eq!(app.insert_buf, "ExecutionStatus = 'Failed'");
    }

    #[test]
    fn scrolling_near_the_end_asks_for_the_next_page_once() {
        let mut app = app();
        app.page = 2;
        let rows: Vec<WorkflowRow> = (0..10)
            .map(|i| wf("default", &format!("r{i}"), 1000 - i))
            .collect();
        loaded(&mut app, rows, vec![("default".into(), vec![7])]);
        assert!(
            !app.loading_more,
            "a completed load clears the in-flight flag"
        );

        app.run("motion.bottom", None);
        assert!(
            app.loading_more,
            "reaching the end should request the next page"
        );

        // A second motion while that request is in flight must not queue another.
        app.run("motion.up", None);
        app.run("motion.bottom", None);
        assert!(app.loading_more);
    }

    #[test]
    fn scrolling_does_not_page_when_the_list_is_complete() {
        let mut app = app();
        app.page = 2;
        let rows: Vec<WorkflowRow> = (0..5)
            .map(|i| wf("default", &format!("r{i}"), 1000 - i))
            .collect();
        loaded(&mut app, rows, vec![]);
        app.run("motion.bottom", None);
        assert!(!app.loading_more, "no token means nothing left to fetch");
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
        assert_eq!(app.screen, Screen::History);
        app.handle(Msg::History {
            generation: app.generation,
            result: Ok((history_events(), Vec::new())),
        });
        app
    }

    #[test]
    fn opening_a_workflow_reads_its_history() {
        let app = viewing_history();
        assert_eq!(app.viewing.as_ref().unwrap().run_id, "r1");
        // Three groups: the workflow and two activities. The workflow task is plumbing.
        assert_eq!(app.row_count(), 3);
    }

    #[test]
    fn dash_returns_to_the_workflow_it_came_from() {
        let mut app = viewing_history();
        app.run("nav.up", None);
        assert_eq!(app.screen, Screen::Workflows);
        assert!(app.viewing.is_none());
        assert!(
            app.history.value().is_none(),
            "leaving must drop the history rather than show a stale one on re-entry"
        );
    }

    #[test]
    fn folding_a_group_shows_its_events_and_keeps_the_cursor_on_it() {
        let mut app = viewing_history();
        app.run("motion.down", None); // onto the "Charge" activity
        let before = app.row_count();
        let at = app.cursor;

        app.run("history.fold", None);
        assert_eq!(app.row_count(), before + 3, "its three events appeared");
        assert_eq!(app.cursor, at, "the cursor stays on the group's own line");

        // Folding shut from *inside* the group must not strand the cursor past the end.
        app.run("motion.down", None);
        app.run("motion.down", None);
        app.run("history.fold", None);
        assert_eq!(app.row_count(), before);
        assert_eq!(app.cursor, at);
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
        let outline = app.history.value().unwrap();
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
            generation: app.generation,
            result: Ok((history_events()[..5].to_vec(), vec![7])),
        });
        let charge = app.history.value().unwrap().group(2).unwrap().clone();
        assert!(charge.is_open(), "the group is incomplete on page one");

        // The rest arrives and completes it — which is why pages are re-grouped whole.
        app.handle(Msg::History {
            generation: app.generation,
            result: Ok((history_events()[5..].to_vec(), Vec::new())),
        });
        let charge = app.history.value().unwrap().group(2).unwrap();
        assert!(!charge.is_open(), "the second page closed the group");
        assert_eq!(app.row_count(), 3);
    }

    #[test]
    fn a_stale_history_reply_is_dropped() {
        let mut app = viewing_history();
        let stale = app.generation;
        app.generation = app.generation.wrapping_add(1);

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
            generation: app.generation,
            // A non-empty token is what a running workflow returns.
            result: Ok((running_history(), vec![9])),
        });
        app
    }

    #[test]
    fn follow_starts_and_stops_on_the_same_key() {
        let mut app = viewing_running();
        assert!(!app.following);

        app.run("history.follow", None);
        assert!(app.following, "F should start following");
        assert!(matches!(app.note, Some((_, Note::Info))));

        app.run("history.follow", None);
        assert!(!app.following, "F again should stop");
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
            generation: app.generation,
            result: Ok((events, Vec::new())),
        });

        assert!(app.history_token.is_empty());
        app.run("history.follow", None);

        assert!(!app.following, "there is nothing to follow");
        let (msg, kind) = app.note.clone().unwrap();
        assert_eq!(kind, Note::Warn);
        assert!(msg.contains("closed"), "got {msg}");
    }

    #[test]
    fn follow_is_not_offered_away_from_a_history() {
        let mut app = app();
        loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
        app.run("history.follow", None);
        assert!(!app.following);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn an_empty_token_while_following_means_the_workflow_closed() {
        let mut app = viewing_running();
        app.run("history.follow", None);
        assert!(app.following);

        // The long poll returns the terminal event with no continuation token.
        use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
        app.handle(Msg::History {
            generation: app.generation,
            result: Ok((
                vec![hev(9, G::Workflow, R::Closes, C::Workflow).with_outcome(O::Completed)],
                Vec::new(),
            )),
        });

        assert!(!app.following, "follow must stop when the workflow closes");
        let (msg, _) = app.note.clone().unwrap();
        assert!(msg.contains("closed"), "got {msg}");
    }

    #[test]
    fn replayed_events_do_not_duplicate_when_follow_resumes() {
        // Follow resumes from the last non-empty token, which replays that page.
        let mut app = viewing_running();
        let before = app.history_events.len();

        app.handle(Msg::History {
            generation: app.generation,
            result: Ok((running_history(), vec![9])),
        });
        assert_eq!(
            app.history_events.len(),
            before,
            "a replayed page must not be appended twice"
        );
    }

    #[test]
    fn the_resume_token_is_the_last_non_empty_one() {
        // Paging leaves history_token empty once caught up; following from that would
        // restart the read at event 1.
        let mut app = viewing_running();
        assert_eq!(app.history_resume, vec![9]);

        app.handle(Msg::History {
            generation: app.generation,
            result: Ok((Vec::new(), Vec::new())),
        });
        assert!(app.history_token.is_empty(), "caught up");
        assert_eq!(
            app.history_resume,
            vec![9],
            "the resume point is remembered"
        );
    }

    #[test]
    fn leaving_the_history_stops_following() {
        let mut app = viewing_running();
        app.run("history.follow", None);
        assert!(app.following);

        app.run("nav.up", None);
        assert!(
            !app.following,
            "a poll must not outlive the screen it feeds"
        );
        assert!(app.history_resume.is_empty());
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
            generation: app.generation,
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
        app.piped = Some(Ok("{}".into()));
        app.run("motion.down", None);
        assert!(app.piped.is_none());
    }

    #[test]
    fn a_pipe_result_message_opens_the_pane_and_lands() {
        let mut app = viewing_payloads();
        app.handle(Msg::Piped(Ok("42\n".into())));
        assert_eq!(app.piped, Some(Ok("42\n".into())));
        assert_eq!(app.detail_scroll, 0);
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
        assert_eq!(app.query, "ExecutionStatus = 'Running'");
    }

    #[test]
    fn opening_nothing_says_so_instead_of_changing_screen() {
        let mut app = app();
        app.run("nav.open", None);
        assert_eq!(app.screen, Screen::Namespaces);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }

    #[test]
    fn going_up_from_the_top_level_says_so() {
        let mut app = app();
        app.run("nav.up", None);
        assert_eq!(app.screen, Screen::Namespaces);
        assert!(matches!(app.note, Some((_, Note::Warn))));
    }
}
