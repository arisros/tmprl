//! Application state and the reducer.
//!
//! The one rule: [`App::handle`] is synchronous and never awaits. When it needs data it
//! spawns a task, which reports back as another [`Msg`]. Nothing on the keystroke path can
//! block on the network. See `docs/ARCHITECTURE.md`.
//!
//! This file holds the state, [`App::handle`] (every message: keys, server replies, ticks)
//! and [`App::run`] (every command, dispatched). What each command does lives beside the others of its kind:
//!
//! | File | |
//! |---|---|
//! | `nav` | opening, going back up, the cursor, the jumplist |
//! | `query` | the workflow list's query bar |
//! | `find` | the pickers and `/` search |
//! | `history` | folds, follow, `]f` / `[f` |
//! | `payload` | decoding, the `!` pipe, `$EDITOR` |
//! | `yank` | copying fields, rows and payloads |
//! | `mutate` | cancel, terminate, signal, reset, schedules |
//! | `windows` | splits and tabs |
//! | `prompt` | the `:` and `!` prompts |
//! | `load` | every request to the server |

mod find;
mod history;
mod load;
mod mutate;
mod nav;
mod payload;
mod prompt;
mod query;
mod windows;
mod yank;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tmprl_client::{Codec, Conn, NamespaceInfo};
use tmprl_core::ScheduleRow;
use tmprl_core::clock::{Clock, TimeFormat};
use tmprl_core::form::Form;
use tmprl_core::history::{NormalizedEvent, group_events, merge_events};
use tmprl_core::jumplist::Jumplist;
use tmprl_core::mutation::{Confirm, Mutation};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::Payload;
use tmprl_core::picker::{self, Picker, Target};
use tmprl_core::search::{self, Search};
use tmprl_core::timerange::parse_backfill;
use tmprl_core::{
    Action, Chord, Keymap, Loadable, Mode, PayloadPart, Pending, PendingEntry, Registry,
    Resolution, SavedView, StatusCounts, WorkflowList, WorkflowRow, WorkflowStatus, default_keymap,
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
    BackfillSchedule,
    Cancel,
    Terminate,
    Signal,
    Delete,
    Reset,
    Update,
}

/// Wall-clock now, epoch millis.
///
/// A backfill window is resolved against it, and a clock before the epoch would make the
/// window nonsense rather than merely wrong, so it saturates at zero.
/// Wall-clock now, epoch millis: what every age, countdown and "running until now" is
/// measured against. One definition, so the list, the schedules and the timeline agree.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Somewhere the cursor has been, in enough detail to get back to it.
///
/// Held by value rather than as an index, because every index a pane has is into a list that
/// a refresh can replace. A jump back to "row 12 of the workflow list" would land on a
/// different workflow ten seconds later; a jump back to a run id lands on the workflow.
///
/// The history itself is not stored, only which workflow it was. Coming back re-fetches,
/// which is a round trip, but the alternative is keeping every history ever visited in
/// memory for the length of the session.
#[derive(Debug, Clone, PartialEq)]
pub struct Jump {
    pub screen: Screen,
    pub scope: Vec<String>,
    pub query: String,
    pub viewing: Option<WorkflowRow>,
    /// Which row, by index. Only a fallback: on the workflow list `cursor_key` is what
    /// actually gets you back, and this is what is used where there is no identity to use,
    /// the namespace list and a history outline.
    pub cursor: usize,
    /// Which row, by identity, on the workflow list. A live list grows at the top, so an
    /// index alone would return you to a different workflow than the one you left.
    pub cursor_key: Option<(String, String)>,
}

/// A payload written to disk, waiting for `$EDITOR` to be run over it.
///
/// `App` cannot open an editor itself: the editor wants the terminal, and the terminal
/// belongs to the event loop. So the reducer does the part it can do, choosing the payload
/// and writing the file, and leaves this behind for the loop to act on. That keeps `App`
/// free of terminal types and keeps the whole thing testable, the test asserts a file was
/// written and never spawns anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    pub path: std::path::PathBuf,
    /// The private directory holding `path`, removed once the editor exits. Decoded
    /// payloads should not outlive the moment they were being read.
    pub dir: std::path::PathBuf,
    /// What was written, for the message afterwards.
    pub what: String,
}

/// What a prompt at the bottom of the screen is collecting.
///
/// Both prompts edit identically, the same keys, the same backspace-on-empty-closes rule,
/// and differ only in what Enter does with the text. Sharing the editing is what keeps them
/// from drifting apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// `:`, a command id, with completions.
    Command,
    /// `!`, a shell command to filter the focused payloads through.
    Pipe,
    /// `/`, a pattern to find within the rows already on screen.
    Search,
    /// The name of a signal to send.
    Signal,
    /// The name of an update to send.
    Update,
    /// The window a schedule backfill covers, and optionally its overlap policy.
    Backfill,
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
            PromptKind::Search => "/",
            PromptKind::Signal => "signal:",
            PromptKind::Update => "update:",
            PromptKind::Backfill => "backfill:",
        }
    }
}

/// Where an encrypted payload has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeState {
    /// No codec server is configured, so it cannot be read at all.
    NoCodec,
    /// A decode is out.
    InFlight,
    /// A codec is configured but nothing has been asked yet.
    Idle,
    /// The codec server was asked and refused. Kept per payload rather than shown as a
    /// passing note: a note is gone by the next keystroke, and the badge left behind is
    /// identical to one that was never asked about, so the reader is told nothing.
    Failed(String),
}

/// What Insert mode is editing.
///
/// On the workflow list, Insert mode edits the visibility query: the only text
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
        /// `Some((done, total))` when this is one row of a batch, so the status line can
        /// count up rather than flashing each row's name in turn.
        batch: Option<(usize, usize)>,
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
    /// Why a decode failed, by payload. Cleared by `R`, which is the retry.
    decode_failed: HashMap<u64, String>,
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
    /// `Some` while a multi-field form is open, for the inputs a single line cannot carry.
    pub form: Option<Form>,
    pub insert_buf: String,
    pub insert_target: InsertTarget,
    /// `Some` while a `<leader>f` picker is open. It owns the keyboard while it is, the
    /// way a prompt does.
    pub picker: Option<Picker>,
    /// Set when `<leader>e` has written a payload out; the event loop takes it, drops
    /// the terminal, runs the editor and puts the terminal back.
    pub editing: Option<EditRequest>,
    /// `<C-o>` / `<C-i>`. Session-level, not per-pane: the jumps you want to retrace
    /// are the ones *you* made, and they cross panes as readily as they cross screens.
    pub jumps: Jumplist<Jump>,
    /// The last pattern searched for, vim's `/` register.
    ///
    /// Session-level rather than per-pane, and deliberately so: the pattern is something
    /// *you* are looking for, not a property of a window. Splitting a pane and pressing `n`
    /// should keep looking for the same thing, and typing a query you have already typed
    /// once into the other half is exactly the friction this is avoiding.
    pub search: Search,

    /// Whether time columns read as an age or as a wall clock, `<leader>T`. Session-level:
    /// it is how *you* are reading the screen, so it holds across panes and tabs.
    pub times: TimeFormat,
    /// The zone wall-clock times are rendered in, from `config.toml`.
    pub clock: Clock,

    pub note: Option<(String, Note)>,
    pub should_quit: bool,
    pub dirty: bool,

    profile: String,
    address: String,
    /// Colour for the profile name, from `config.toml`. `None` renders as it always did.
    accent: Option<tmprl_core::config::Accent>,
    /// Refuse every mutation on this profile.
    readonly: bool,
    /// Where `K` opens, from `config.toml`'s `[layout]`.
    payload_pane: tmprl_core::config::PayloadPane,
    namespace: String,
    conn: Option<Arc<Conn>>,
    tx: UnboundedSender<Msg>,
}

impl App {
    pub fn new(conn: Conn, tx: UnboundedSender<Msg>) -> Self {
        let (profile, address, namespace) = (
            conn.profile().to_string(),
            conn.address().to_string(),
            conn.namespace().to_string(),
        );
        Self::build(Some(Arc::new(conn)), profile, address, namespace, tx)
    }

    /// An app with no connection, for tests. Every command except the ones that fetch
    /// behaves identically, which is what makes the interface testable without a Temporal
    /// server.
    #[cfg(test)]
    pub fn detached(profile: &str, namespace: &str, tx: UnboundedSender<Msg>) -> Self {
        Self::build(
            None,
            profile.to_string(),
            "http://detached".to_string(),
            namespace.to_string(),
            tx,
        )
    }

    fn build(
        conn: Option<Arc<Conn>>,
        profile: String,
        address: String,
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
            decode_failed: HashMap::new(),
            codec: None,
            views: Vec::new(),
            which_key: Vec::new(),
            show_help: false,
            help_scroll: 0,
            help_max_scroll: 0,
            prompt: None,
            confirm: None,
            form: None,
            insert_buf: String::new(),
            insert_target: InsertTarget::Scratch,
            picker: None,
            editing: None,
            jumps: Jumplist::default(),
            search: Search::default(),
            times: TimeFormat::default(),
            clock: Clock::system(),
            note: None,
            should_quit: false,
            dirty: true,
            profile,
            address,
            accent: None,
            readonly: false,
            payload_pane: Default::default(),
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
                    let resolved = cfg.resolve(&self.profile);
                    self.codec = resolved
                        .codec
                        .map(|c| Arc::new(Codec::new(c.endpoint, c.auth)));
                    self.accent = resolved.accent;
                    self.readonly = resolved.readonly;
                    self.payload_pane = cfg.payload_pane;
                    // Already validated by `parse_config`, so this cannot be the zone
                    // failing; unwrapping to the system zone here would be unreachable.
                    if let Ok(clock) = Clock::from_config(cfg.timezone.as_deref()) {
                        self.clock = clock;
                    }
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

    /// The inclusive row range selected in the focused pane, if a visual mode is active.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.view.selection()
    }

    pub fn handle(&mut self, msg: Msg) {
        self.dirty = true;
        match msg {
            Msg::Key(chord) => self.on_key(chord),
            Msg::Quit => self.should_quit = true,
            Msg::Tick | Msg::Redraw => {}
            Msg::Mutated {
                mutation,
                result,
                batch,
            } => {
                let outcome = match &result {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("failed: {e}"),
                };
                self.audit(&mutation, &outcome);
                match result {
                    Ok(()) => {
                        self.note = Some((
                            match batch {
                                Some((done, total)) => {
                                    format!("{} {done}/{total}", mutation.past_tense())
                                }
                                None => {
                                    format!("{} {}", mutation.past_tense(), mutation.workflow_id())
                                }
                            },
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
                // Record the reason against the payloads that were out, so the badge can say
                // why instead of looking exactly like one nothing was ever asked about. The
                // in-flight set is still emptied, which is what lets a retry happen at all.
                for key in std::mem::take(&mut self.decoding) {
                    self.decode_failed.insert(key, e.clone());
                }
                self.note = Some((e, Note::Error));
            }
            Msg::Namespaces(Ok(list)) => {
                self.view.namespaces = Loadable::loaded(list);
                self.clamp_cursor();
            }
            Msg::Namespaces(Err(e)) => {
                // A Temporal Cloud API key is scoped to one namespace, and listing every
                // namespace on the account is an admin operation it is correctly refused.
                // Failing here would strand the reader on the opening screen with a key that
                // can do everything they actually came for, so fall back to the namespace
                // the profile already names.
                if is_permission_denied(&e) {
                    self.view.namespaces = Loadable::loaded(vec![NamespaceInfo {
                        name: self.namespace.clone(),
                        state: "Registered".into(),
                        retention_days: 0,
                        description: "from the profile; this key cannot list namespaces".into(),
                    }]);
                    self.clamp_cursor();
                    self.note = Some((
                        format!(
                            "this key cannot list namespaces, showing {} from the profile",
                            self.namespace
                        ),
                        Note::Info,
                    ));
                } else {
                    self.note = Some((e.clone(), Note::Error));
                    self.view.namespaces = Loadable::Failed(e);
                }
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
                                Some(("workflow closed, follow stopped".into(), Note::Info));
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
        // A pending destructive action owns every key, so nothing bound elsewhere can
        // fire while one is waiting.
        if self.confirm.is_some() {
            self.confirm_key(chord);
            return;
        }
        if self.prompt.is_some() {
            self.prompt_key(chord);
            return;
        }
        if self.picker.is_some() {
            self.picker_key(chord);
            return;
        }
        if self.form.is_some() {
            self.form_key(chord);
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
            Action::ToggleTimes => {
                self.times = self.times.toggled();
                self.note = Some((
                    format!("times: {} ({})", self.times.label(), self.clock.name()),
                    Note::Info,
                ));
            }

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
            // `gg` and `G` are jumps, as they are in vim: they are how you leave where
            // you were, which is exactly what `<C-o>` is for getting back from.
            Action::MoveTop => {
                self.mark_jump();
                self.set_cursor(0)
            }
            Action::MoveBottom => {
                self.mark_jump();
                self.set_cursor(self.row_count().saturating_sub(1))
            }
            Action::HalfPageDown => self.move_cursor((self.view.page / 2).max(1) as isize),
            Action::HalfPageUp => self.move_cursor(-((self.view.page / 2).max(1) as isize)),

            Action::OpenItem => self.open_focused(),
            Action::GoUp => self.go_up(),
            Action::GoSchedules => self.go_to(Screen::Schedules),
            Action::GoWorkflows => self.go_to(Screen::Workflows),
            Action::JumpBack => self.jump(true),
            Action::JumpForward => self.jump(false),

            Action::PauseSchedule => self.confirm_mutation(MutationKind::PauseSchedule),
            Action::TriggerSchedule => self.confirm_mutation(MutationKind::TriggerSchedule),
            Action::DeleteSchedule => self.confirm_mutation(MutationKind::DeleteSchedule),
            Action::BackfillSchedule => self.confirm_mutation(MutationKind::BackfillSchedule),
            Action::CreateSchedule => self.open_new_schedule_form(),

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
                // Esc abandons the edit; the applied query is unchanged. Enter applies,
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
            Action::YankPayloadAll => self.yank_payload(PayloadPart::All),
            Action::YankPayloadInput => self.yank_payload(PayloadPart::Input),
            Action::YankPayloadResult => self.yank_payload(PayloadPart::Result),

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
            Action::OpenSearch => self.open_search(),
            Action::FindWorkflow => self.open_picker(picker::Kind::Workflows),
            Action::FindEvent => self.open_picker(picker::Kind::HistoryRows),
            Action::FindPane => self.open_picker(picker::Kind::Panes),
            Action::FindCommand => self.open_picker(picker::Kind::Commands),
            Action::FindFilter => self.open_picker(picker::Kind::Filters),
            Action::FindNamespace => self.open_picker(picker::Kind::Namespaces),
            Action::ProblemList => self.show_problems(),
            Action::SearchNext => self.jump_match(true),
            Action::SearchPrev => self.jump_match(false),

            Action::NextFailure => self.jump_failure(true),
            Action::PrevFailure => self.jump_failure(false),
            Action::ToggleFollow => self.toggle_follow(),
            Action::ToggleTimeline => {
                if self.view.screen != Screen::History {
                    self.note = Some(("the timeline is of a workflow history".into(), Note::Warn));
                } else {
                    self.view.timeline = !self.view.timeline;
                    self.note = Some((
                        if self.view.timeline {
                            "timeline on (zg folds idle time, <leader>G to leave)".into()
                        } else {
                            "timeline off".into()
                        },
                        Note::Info,
                    ));
                }
            }
            Action::ToggleGaps => {
                if !self.view.timeline {
                    self.note = Some((
                        "idle time folds on the timeline, <leader>G".into(),
                        Note::Warn,
                    ));
                } else {
                    self.view.timeline_gaps_open = !self.view.timeline_gaps_open;
                    self.note = Some((
                        if self.view.timeline_gaps_open {
                            "idle time drawn to scale".into()
                        } else {
                            "idle time folded".into()
                        },
                        Note::Info,
                    ));
                }
            }
            Action::DetailDown => self.scroll_detail(n as isize),
            Action::DetailUp => self.scroll_detail(-(n as isize)),
            Action::OpenPipe => self.open_pipe(),
            Action::OpenEditor => self.open_editor(),

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

    pub fn accent(&self) -> Option<tmprl_core::config::Accent> {
        self.accent
    }

    pub fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn payload_pane(&self) -> tmprl_core::config::PayloadPane {
        self.payload_pane
    }
}

/// Run `command` in a shell with `input` on stdin, and collect what it says.
///
/// A shell rather than a bare exec, so that `jq .result | head -20` works, `!` is a filter,
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
        // A filter that does not read its input (`!wc -l` after an early exit) closes the
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

/// Whether a failure is the server refusing the operation rather than the call going wrong.
///
/// Matched on the message because the error has already been flattened to a `String` by the
/// time it crosses the task boundary. Both spellings appear: gRPC's status name, and the
/// sentence Temporal Cloud returns.
fn is_permission_denied(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    e.contains("permissiondenied")
        || e.contains("permission denied")
        || e.contains("does not have permission")
        || e.contains("request unauthorized")
}

#[cfg(test)]
mod tests;
