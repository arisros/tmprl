//! Application state and the reducer.
//!
//! [`App::handle`] never awaits: it spawns a task, which reports back as another [`Msg`].
//!
//! | File | |
//! |---|---|
//! | `nav` | opening, going back up, the cursor, the jumplist |
//! | `dashboard` | the dashboard: opening it, its requests, what an item opens |
//! | `query` | the workflow list's query bar |
//! | `find` | the pickers and `/` search |
//! | `history` | folds, follow, `]f` / `[f` |
//! | `payload` | decoding, the `!` pipe, `$EDITOR` |
//! | `yank` | copying fields, rows and payloads |
//! | `mutate` | cancel, terminate, signal, reset, schedules |
//! | `windows` | splits and tabs |
//! | `prompt` | the `:` and `!` prompts |
//! | `load` | every request to the server |
//! | `startup` | where the command line asked to open |
//! | `messages` | the note line's history, `:messages` |
//! | `mouse` | the wheel and the click, when `[input] mouse` asks for them |
//! | `profile` | listing the connection profiles and switching to one |
//! | `interaction` | workflow queries: the handler picker and the answer |

mod dashboard;
mod find;
mod history;
mod interaction;
mod load;
mod messages;
mod mouse;
mod mutate;
mod nav;
mod payload;
mod profile;
mod prompt;
mod query;
mod startup;
mod windows;
mod yank;

pub use mouse::Mouse;
pub use startup::Startup;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use tmprl_client::{Codec, Conn, NamespaceInfo, ProfileSummary};
use tmprl_core::ScheduleRow;
use tmprl_core::clock::{Clock, TimeFormat};
use tmprl_core::complete::{self, Completion};
use tmprl_core::dashboard::{Board, Drill, Heading, Item, Layout, Outcome, Source, SourceData};
use tmprl_core::fault::Fault;
use tmprl_core::filter::{self, SearchAttribute};
use tmprl_core::form::Form;
use tmprl_core::history::{NormalizedEvent, group_events, merge_events};
use tmprl_core::jumplist::Jumplist;
use tmprl_core::line::{Edit, LineEdit, LineHistory};
use tmprl_core::mutation::{Confirm, Mutation};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::Payload;
use tmprl_core::pending::PendingActivity;
use tmprl_core::picker::{self, Picker, Target};
use tmprl_core::search::{self, Search};
use tmprl_core::timerange::parse_backfill;
use tmprl_core::{
    Action, Chord, ColorDepth, Keymap, Loadable, Mode, PayloadPart, Pending, PendingEntry,
    Registry, Resolution, SavedView, StatusCounts, ThemeOverrides, WorkflowList, WorkflowRow,
    default_keymap,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::theme::Theme;
use crate::view::View;
pub use messages::Logged;
pub(crate) use payload::cut_for_pane;
use tmprl_ui::{Axis, Direction, Rect as UiRect, Tabs, ViewId};

const PAGE_SIZE: i32 = 50;

const SCAN_PAGE_SIZE: i32 = 1000;

const PICKER_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);
const PICKER_SEARCH_MIN: usize = 6;
const PICKER_SEARCH_LIMIT: i32 = 20;

const HISTORY_PAGE_SIZE: i32 = 500;

const PENDING_POLL: std::time::Duration = std::time::Duration::from_secs(5);

use tmprl_client::Continuation as Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Profiles,
    Namespaces,
    Workflows,
    History,
    Schedules,
    Dashboard,
}

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

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Jump {
    pub screen: Screen,
    pub scope: Vec<String>,
    pub query: String,
    pub viewing: Option<WorkflowRow>,
    pub cursor: usize,
    pub cursor_key: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    pub path: std::path::PathBuf,
    pub dir: std::path::PathBuf,
    pub what: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRequest {
    pub path: std::path::PathBuf,
    pub line: Option<u32>,
    pub root: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptKind {
    Command,
    Pipe,
    Search,
    Signal,
    Update,
    Backfill,
    Query,
    Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub buf: LineEdit,
}

impl Prompt {
    pub fn sigil(&self) -> &'static str {
        match self.kind {
            PromptKind::Command => ":",
            PromptKind::Pipe => "!",
            PromptKind::Search => "/",
            PromptKind::Signal => "signal:",
            PromptKind::Update => "update:",
            PromptKind::Backfill => "backfill:",
            PromptKind::Query => "query:",
            PromptKind::Reason => "reason:",
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ShownText {
    pub lines: Vec<String>,
    pub hidden: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeState {
    NoCodec,
    InFlight,
    Idle,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum History {
    Prompt(PromptKind),
    Query,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertTarget {
    Scratch,
    Query,
}

#[derive(Debug)]
pub enum Msg {
    Key(Chord),
    Paste(String),
    Mouse(Mouse),
    Tick,
    Redraw,
    Quit,
    Namespaces(Result<Vec<NamespaceInfo>, Fault>),
    Profiles(Result<Vec<ProfileSummary>, Fault>),
    Connected {
        seq: u64,
        result: Result<Box<Conn>, String>,
    },
    Reply {
        epoch: u64,
        msg: Box<Msg>,
    },
    SearchAttributes {
        namespace: String,
        result: Result<Vec<SearchAttribute>, Fault>,
    },
    Workflows {
        generation: u64,
        append: bool,
        result: Result<(Vec<WorkflowRow>, Tokens), Fault>,
    },
    Counts {
        generation: u64,
        result: Result<StatusCounts, Fault>,
    },
    Schedules {
        generation: u64,
        result: Result<Vec<ScheduleRow>, Fault>,
    },
    Piped(Result<String, String>),
    Source(Result<String, String>),
    Mutated {
        mutation: Box<Mutation>,
        sent_to: (String, String),
        result: Result<(), Fault>,
        batch: Option<(usize, usize)>,
    },
    Decoded(Result<Vec<(u64, Payload)>, Fault>),
    History {
        generation: u64,
        result: Result<(Vec<NormalizedEvent>, Vec<u8>), Fault>,
    },
    PickerDebounce {
        search: u64,
    },
    PickerFound {
        search: u64,
        result: Result<Vec<WorkflowRow>, Fault>,
    },
    Related {
        from: (String, String),
        id: String,
        result: Result<Vec<WorkflowRow>, Fault>,
    },
    StartupWorkflow {
        generation: u64,
        id: String,
        result: Result<Vec<WorkflowRow>, Fault>,
    },
    Dashboard {
        view: ViewId,
        generation: u64,
        source: usize,
        timed: bool,
        result: Result<SourceData, Fault>,
    },
    Pending {
        generation: u64,
        result: Result<Vec<PendingActivity>, Fault>,
    },
    FollowRetrying {
        generation: u64,
        error: Fault,
        wait: std::time::Duration,
    },
    QueryHandlers {
        generation: u64,
        result: Result<Vec<Payload>, Fault>,
    },
    Queried {
        generation: u64,
        name: String,
        result: Result<Vec<Payload>, Fault>,
    },
}

fn audit_outcome(result: &Result<(), Fault>) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("failed: {e}"),
    }
}

#[derive(Clone)]
pub struct Outbox {
    tx: UnboundedSender<Msg>,
    epoch: u64,
}

impl Outbox {
    pub fn send(&self, msg: Msg) -> Result<(), ()> {
        self.tx
            .send(Msg::Reply {
                epoch: self.epoch,
                msg: Box::new(msg),
            })
            .map_err(drop)
    }
}

#[derive(Debug, Clone, Copy)]
struct Scan {
    from: usize,
    forward: bool,
    inclusive: bool,
    generation: u64,
    started_with: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Info,
    Warn,
    Error,
}

pub(super) enum AuditSink {
    File,
    #[cfg(test)]
    Captured(Vec<String>),
}

pub struct App {
    pub view: View,
    parked: std::collections::HashMap<ViewId, View>,
    pub tabs: Tabs,
    next_view_id: u64,
    frame: UiRect,

    pub mode: Mode,
    pub pending: Pending,
    pub registry: Registry,
    pub keymap: Keymap,
    decoded: HashMap<u64, Payload>,
    decoding: HashSet<u64>,
    decode_failed: HashMap<u64, String>,
    shown: std::cell::RefCell<HashMap<u64, Arc<ShownText>>>,
    codec: Option<Arc<Codec>>,
    pub views: Vec<SavedView>,
    dashboard_layout: Option<Layout>,
    dashboard_refresh: tmprl_core::config::Refresh,
    pub search_attributes: Loadable<Vec<SearchAttribute>>,
    pub completion: Option<Completion>,
    attributes_for: Option<String>,

    pub which_key: Vec<PendingEntry>,
    pub show_help: bool,
    pub show_messages: bool,
    pub overlay_scroll: usize,
    pub overlay_max_scroll: usize,
    pub prompt: Option<Prompt>,
    pub confirm: Option<Confirm>,
    pub form: Option<Form>,
    pub insert_buf: LineEdit,
    histories: HashMap<History, LineHistory>,
    awaiting_reason: Vec<Mutation>,
    cmdline_cycle: Option<(String, usize)>,
    pub insert_target: InsertTarget,
    pub picker: Option<Picker>,
    picker_found: Vec<WorkflowRow>,
    picker_search: u64,
    pub editing: Option<EditRequest>,
    pub opening: Option<SourceRequest>,
    source_command: Option<String>,
    pub jumps: Jumplist<Jump>,
    scan: Option<Scan>,
    pub search: Search,

    pub times: TimeFormat,
    pub clock: Clock,
    pub theme: Theme,
    theme_overrides: ThemeOverrides,

    pub note: Option<(String, Note)>,
    pub messages: VecDeque<Logged>,
    note_fault: Option<Fault>,
    keep_note: bool,
    pub should_quit: bool,
    pub dirty: bool,

    profile: String,
    address: String,
    accent: Option<tmprl_core::config::Accent>,
    readonly: bool,
    readonly_flag: bool,
    audit_sink: AuditSink,
    payload_pane: tmprl_core::config::PayloadPane,
    payload_size: u16,
    dragging: bool,
    text_selection: Option<crate::view::TextSelection>,
    last_click: Option<(std::time::Instant, u16, u16)>,
    mouse: bool,
    yank_max: usize,
    namespace: String,
    conn: Option<Arc<Conn>>,
    config: Option<tmprl_core::config::Config>,
    temporal_config: Option<String>,
    switching: Option<profile::Switch>,
    switch_seq: u64,
    profile_picker_pending: bool,
    tx: Outbox,
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

    #[cfg(test)]
    pub fn detached(profile: &str, namespace: &str, tx: UnboundedSender<Msg>) -> Self {
        let mut app = Self::build(
            None,
            profile.to_string(),
            "http://detached".to_string(),
            namespace.to_string(),
            tx,
        );
        app.audit_sink = AuditSink::Captured(Vec::new());
        app
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
            shown: Default::default(),
            codec: None,
            views: Vec::new(),
            dashboard_layout: None,
            dashboard_refresh: Default::default(),
            search_attributes: Loadable::default(),
            completion: None,
            attributes_for: None,
            which_key: Vec::new(),
            show_help: false,
            show_messages: false,
            overlay_scroll: 0,
            overlay_max_scroll: 0,
            prompt: None,
            confirm: None,
            form: None,
            insert_buf: LineEdit::default(),
            histories: HashMap::new(),
            cmdline_cycle: None,
            awaiting_reason: Vec::new(),
            insert_target: InsertTarget::Scratch,
            picker: None,
            picker_found: Vec::new(),
            picker_search: 0,
            editing: None,
            opening: None,
            source_command: None,
            jumps: Jumplist::default(),
            scan: None,
            search: Search::default(),
            times: TimeFormat::default(),
            clock: Clock::system(),
            theme: Theme::default(),
            theme_overrides: ThemeOverrides::default(),
            note: None,
            messages: VecDeque::new(),
            note_fault: None,
            keep_note: true,
            should_quit: false,
            dirty: true,
            profile,
            address,
            accent: None,
            readonly: false,
            readonly_flag: false,
            audit_sink: AuditSink::File,
            payload_pane: Default::default(),
            payload_size: tmprl_core::config::PAYLOAD_SIZE,
            dragging: false,
            text_selection: None,
            last_click: None,
            mouse: false,
            yank_max: tmprl_core::config::DEFAULT_YANK_MAX,
            namespace,
            conn,
            config: None,
            temporal_config: None,
            switching: None,
            switch_seq: 0,
            profile_picker_pending: false,
            tx: Outbox { tx, epoch: 0 },
        }
    }

    pub fn apply_config(&mut self, keys: Option<&str>, views: Option<&str>, config: Option<&str>) {
        if let Some(src) = config {
            match tmprl_core::config::parse_config(src) {
                Ok(cfg) => {
                    self.payload_pane = cfg.payload_pane;
                    self.payload_size = cfg.payload_size;
                    self.mouse = cfg.mouse;
                    self.dashboard_refresh = cfg.dashboard_refresh;
                    self.yank_max = cfg.yank_max;
                    self.source_command = cfg.source.clone();
                    if let Some(depth) = cfg.colors {
                        self.theme = Theme::new(depth, &self.theme_overrides);
                    }
                    if let Ok(clock) = Clock::from_config(cfg.timezone.as_deref()) {
                        self.clock = clock;
                    }
                    self.config = Some(cfg);
                    self.apply_profile_config();
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
        if let Some(src) = keys
            && let Err(e) = tmprl_core::config::apply_keys(src, &self.registry, &mut self.keymap)
        {
            self.note = Some((e.to_string(), Note::Error));
        }
    }

    pub fn apply_theme(&mut self, depth: ColorDepth, theme: Option<&str>) {
        let overrides = match theme.map(tmprl_core::theme::parse_theme) {
            None => ThemeOverrides::default(),
            Some(Ok(overrides)) => overrides,
            Some(Err(e)) => {
                self.note = Some((e.to_string(), Note::Error));
                ThemeOverrides::default()
            }
        };
        self.theme = Theme::new(depth, &overrides);
        self.theme_overrides = overrides;
    }

    pub fn apply_dashboard(&mut self, dashboard: Option<&str>) {
        match dashboard.map(tmprl_core::dashboard::parse_dashboard) {
            None => {}
            Some(Ok(layout)) => self.dashboard_layout = Some(layout).filter(|l| !l.is_empty()),
            Some(Err(e)) => self.note = Some((e.to_string(), Note::Error)),
        }
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn address(&self) -> &str {
        &self.address
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

    pub fn selection(&self) -> Option<(usize, usize)> {
        self.view.selection()
    }

    pub fn handle(&mut self, msg: Msg) {
        self.dirty = true;
        let before = self.note.take();
        self.keep_note = true;
        self.apply(msg);
        if self.note.is_some() {
            self.log_note();
        } else if self.keep_note {
            self.note = before;
        }
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Key(chord) => self.on_key(chord),
            Msg::Mouse(mouse) => self.on_mouse(mouse),
            Msg::Quit => self.should_quit = true,
            Msg::Tick => self.tick_dashboards(now_ms()),
            Msg::Redraw => {}
            Msg::Reply { epoch, msg } if epoch == self.epoch() => self.apply(*msg),
            Msg::Reply { msg, .. } => {
                if let Msg::Mutated {
                    mutation,
                    sent_to,
                    result,
                    ..
                } = *msg
                {
                    self.audit_to(&mutation, &audit_outcome(&result), &sent_to);
                }
            }
            Msg::Profiles(result) => self.on_profiles(result),
            Msg::Connected { seq, result } => self.on_connected(seq, result),
            Msg::Mutated {
                mutation,
                sent_to,
                result,
                batch,
            } => {
                self.audit_to(&mutation, &audit_outcome(&result), &sent_to);
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
                        // ListSchedules is eventually consistent, so the refresh can still return the old state.
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
                        self.refresh();
                    }
                    Err(e) => self.fail(e, Note::Error),
                }
            }
            Msg::Paste(text) => self.paste(text),
            Msg::Piped(result) => {
                self.view.piped = Some(result);
                self.view.detail_scroll = 0;
            }
            Msg::Source(result) => self.source_reply(result),
            Msg::Decoded(Ok(pairs)) => {
                for (key, payload) in pairs {
                    self.decoding.remove(&key);
                    self.decoded.insert(key, payload);
                }
                self.apply_decoded();
            }
            Msg::Decoded(Err(e)) => {
                for key in std::mem::take(&mut self.decoding) {
                    self.decode_failed.insert(key, e.to_string());
                }
                self.fail(e, Note::Error);
            }
            Msg::SearchAttributes { namespace, result } => {
                if namespace != self.namespace() {
                    return;
                }
                match result {
                    Ok(attrs) => self.search_attributes = Loadable::loaded(attrs),
                    Err(e) => self.search_attributes = Loadable::Failed(e),
                }
            }
            Msg::Namespaces(Ok(list)) => {
                self.view.namespaces = Loadable::loaded(list);
                self.clamp_cursor();
            }
            Msg::Namespaces(Err(e)) => {
                // A Temporal Cloud key scoped to one namespace is refused ListNamespaces.
                if e.is_refusal() {
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
                    self.fail(e.clone(), Note::Error);
                    self.view.namespaces = Loadable::Failed(e);
                }
            }
            Msg::Workflows {
                generation,
                append,
                result,
            } => {
                if generation != self.view.generation {
                    return;
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
                        self.apply_decoded();
                        self.restore_cursor();
                    }
                    Err(e) => {
                        self.fail(e.clone(), Note::Error);
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
                    Ok((mut events, token)) => {
                        self.decode_from_cache(&mut events);
                        if !token.is_empty() {
                            self.view.history_resume = token.clone();
                        } else if self.view.following {
                            self.stop_following();
                            self.view.pending.clear();
                            self.note =
                                Some(("workflow closed, follow stopped".into(), Note::Info));
                        }
                        self.view.history_token = token;
                        merge_events(&mut self.view.history_events, events);
                        let groups = group_events(&self.view.history_events);
                        let events = self.view.history_events.clone();
                        match self.view.history.value_mut() {
                            Some(outline) => outline.replace(events, groups),
                            None => {
                                self.view.history = Loadable::loaded(Outline::new(events, groups))
                            }
                        }
                        self.clamp_cursor();
                        self.maybe_decode();
                        self.continue_scan(generation);
                    }
                    Err(e) => {
                        self.scan = None;
                        self.fail(e.clone(), Note::Error);
                        if self.view.history_events.is_empty() {
                            self.view.history = Loadable::Failed(e);
                        }
                    }
                }
            }
            Msg::PickerDebounce { search } => self.picker_search_due(search),
            Msg::PickerFound { search, result } => {
                if search != self.picker_search {
                    return;
                }
                match result {
                    Ok(rows) => self.picker_takes(rows),
                    Err(e) => self.fail_as(format!("workflow search: {e}"), e, Note::Warn),
                }
            }
            Msg::Related { from, id, result } => self.related_found(from, id, result),
            Msg::StartupWorkflow {
                generation,
                id,
                result,
            } => self.startup_workflow_found(generation, id, result),
            Msg::QueryHandlers { generation, result } => {
                self.query_handlers_reply(generation, result)
            }
            Msg::Queried {
                generation,
                name,
                result,
            } => self.queried_reply(generation, name, result),
            Msg::FollowRetrying {
                generation,
                error,
                wait,
            } => {
                if generation != self.view.generation || !self.view.following {
                    return;
                }
                self.fail_as(
                    format!("follow: {error}, retrying in {}s", wait.as_secs()),
                    error,
                    Note::Warn,
                );
            }
            Msg::Pending { generation, result } => {
                if generation != self.view.generation {
                    return;
                }
                match result {
                    Ok(pending) => self.view.pending = pending,
                    Err(e) => self.fail_as(format!("pending activities: {e}"), e, Note::Warn),
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
                        self.fail(e.clone(), Note::Error);
                        Loadable::Failed(e)
                    }
                };
                if let Some(id) = self.view.seek_schedule.take()
                    && let Some(at) = self
                        .view
                        .schedule_rows()
                        .iter()
                        .position(|s| s.schedule_id == id)
                {
                    self.view.cursor = at;
                }
                self.clamp_cursor();
            }
            Msg::Dashboard {
                view,
                generation,
                source,
                timed,
                result,
            } => self.dashboard_reply(view, generation, source, timed, result),
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

        self.keep_note = false;
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
                    self.insert_keys(flushed);
                }
            }
        }
    }

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
                self.show_messages = false;
                self.overlay_scroll = 0;
            }
            Action::ToggleMessages => {
                self.show_messages = !self.show_messages;
                self.show_help = false;
                self.overlay_scroll = usize::MAX;
            }
            Action::OpenCommandLine => {
                self.prompt = Some(Prompt {
                    kind: PromptKind::Command,
                    buf: LineEdit::default(),
                });
                self.mode = Mode::Command;
            }
            Action::Cancel => {
                if self.scan.is_some() {
                    self.stop_scan();
                } else if self.overlay_open() {
                    self.show_help = false;
                    self.show_messages = false;
                    self.overlay_scroll = 0;
                } else {
                    self.view.anchor = None;
                    self.view.detail_focus = false;
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

            Action::MoveDown if self.overlay_open() => self.scroll_overlay(n as isize),
            Action::MoveUp if self.overlay_open() => self.scroll_overlay(-(n as isize)),
            Action::MoveTop if self.overlay_open() => self.overlay_scroll = 0,
            Action::MoveBottom if self.overlay_open() => {
                self.overlay_scroll = self.overlay_max_scroll
            }
            Action::HalfPageDown if self.overlay_open() => {
                self.scroll_overlay((self.view.page / 2).max(1) as isize)
            }
            Action::HalfPageUp if self.overlay_open() => {
                self.scroll_overlay(-((self.view.page / 2).max(1) as isize))
            }
            Action::PageDown if self.overlay_open() => {
                self.scroll_overlay(self.view.page.max(1) as isize)
            }
            Action::PageUp if self.overlay_open() => {
                self.scroll_overlay(-(self.view.page.max(1) as isize))
            }

            Action::MoveDown if self.detail_focused() => self.scroll_detail(n as isize),
            Action::MoveUp if self.detail_focused() => self.scroll_detail(-(n as isize)),
            Action::MoveTop if self.detail_focused() => self.view.detail_scroll = 0,
            Action::MoveBottom if self.detail_focused() => {
                self.view.detail_scroll = self.view.detail_max_scroll
            }
            Action::HalfPageDown if self.detail_focused() => {
                self.scroll_detail((self.view.detail_page / 2).max(1) as isize)
            }
            Action::HalfPageUp if self.detail_focused() => {
                self.scroll_detail(-((self.view.detail_page / 2).max(1) as isize))
            }
            Action::PageDown if self.detail_focused() => {
                self.scroll_detail(self.view.detail_page.max(1) as isize)
            }
            Action::PageUp if self.detail_focused() => {
                self.scroll_detail(-(self.view.detail_page.max(1) as isize))
            }

            Action::MoveDown if self.on_dashboard() => self.step_dashboard(Heading::Down, n),
            Action::MoveUp if self.on_dashboard() => self.step_dashboard(Heading::Up, n),
            Action::MoveLeft if self.on_dashboard() && !self.overlay_open() => {
                self.step_dashboard(Heading::Left, n)
            }
            Action::MoveRight if self.on_dashboard() && !self.overlay_open() => {
                self.step_dashboard(Heading::Right, n)
            }
            Action::MoveLeft | Action::MoveRight => {}
            Action::MoveDown => self.move_cursor(n as isize),
            Action::MoveUp => self.move_cursor(-(n as isize)),
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
            Action::PageDown => self.move_cursor(self.view.page.max(1) as isize),
            Action::PageUp => self.move_cursor(-(self.view.page.max(1) as isize)),

            Action::OpenItem => self.open_focused(),
            Action::GoUp => self.go_up(),
            Action::GoSchedules => self.go_to(Screen::Schedules),
            Action::GoWorkflows => self.go_to(Screen::Workflows),
            Action::GoDashboard => self.open_dashboard(),
            Action::NextPanel => self.step_panel(true),
            Action::PrevPanel => self.step_panel(false),
            Action::JumpBack => self.jump(true),
            Action::JumpForward => self.jump(false),

            Action::PauseSchedule => self.confirm_mutation(MutationKind::PauseSchedule),
            Action::TriggerSchedule => self.confirm_mutation(MutationKind::TriggerSchedule),
            Action::DeleteSchedule => self.confirm_mutation(MutationKind::DeleteSchedule),
            Action::BackfillSchedule => self.confirm_mutation(MutationKind::BackfillSchedule),
            Action::CreateSchedule => self.open_new_schedule_form(),

            Action::EnterInsert => {
                self.mode = Mode::Insert;
                if self.view.screen == Screen::Workflows {
                    self.insert_target = InsertTarget::Query;
                    self.insert_buf.set(self.view.query.clone());
                } else {
                    self.insert_target = InsertTarget::Scratch;
                    self.insert_buf.clear();
                }
            }
            Action::LeaveInsert => {
                if self.completion.take().is_some() {
                    return;
                }
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
            Action::YankWorkflowId => self.yank_execution(|w| &w.workflow_id),
            Action::YankRunId => self.yank_execution(|w| &w.run_id),

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
            Action::FindProfile => self.find_profile(),
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
            Action::OpenSource => self.open_source(),
            Action::OpenChild => self.open_child(),
            Action::OpenParent => self.open_parent(),

            Action::CancelWorkflow => self.confirm_mutation(MutationKind::Cancel),
            Action::TerminateWorkflow => self.confirm_mutation(MutationKind::Terminate),
            Action::SignalWorkflow => self.confirm_mutation(MutationKind::Signal),
            Action::DeleteWorkflow => self.confirm_mutation(MutationKind::Delete),
            Action::ResetWorkflow => self.confirm_mutation(MutationKind::Reset),
            Action::UpdateWorkflow => self.confirm_mutation(MutationKind::Update),
            Action::QueryWorkflow => self.open_query(),
            Action::StackTrace => self.stack_trace(),

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
                    self.view.detail_focus = false;
                    self.view.piped = None;
                    self.view.queried = None;
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
        self.readonly || self.readonly_flag
    }

    pub fn dashboard_refresh(&self) -> tmprl_core::config::Refresh {
        self.dashboard_refresh
    }

    pub fn payload_pane(&self) -> tmprl_core::config::PayloadPane {
        self.payload_pane
    }

    pub fn text_selection(&self) -> Option<crate::view::TextSelection> {
        self.text_selection
    }

    pub fn payload_size(&self) -> u16 {
        self.payload_size
    }

    pub fn mouse(&self) -> bool {
        self.mouse
    }
}

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

async fn run_resolver(program: &str, args: &[String], input: Vec<u8>) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run `{program}`: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&input).await;
        let _ = stdin.shutdown().await;
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("`{program}` failed: {e}"))?;

    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(match stderr.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(last) => last.trim().to_string(),
        None => format!("`{program}` exited with {}", out.status),
    })
}

#[cfg(test)]
mod tests;
