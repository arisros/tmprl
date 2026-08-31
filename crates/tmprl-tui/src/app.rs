//! Application state and the reducer.
//!
//! The one rule: [`App::handle`] is synchronous and never awaits. When it needs data it
//! spawns a task, which reports back as another [`Msg`]. Nothing on the keystroke path can
//! block on the network — see `docs/ARCHITECTURE.md`.

use std::sync::Arc;

use tmprl_client::{Conn, NamespaceInfo};
use tmprl_core::{
    Action, Chord, Keymap, Loadable, Mode, Pending, PendingEntry, Registry, Resolution, SavedView,
    StatusCounts, WorkflowList, WorkflowRow, default_keymap,
};
use tokio::sync::mpsc::UnboundedSender;

/// Rows fetched per namespace per page. Large enough that scrolling rarely waits, small
/// enough that the first screen arrives promptly on a slow link.
const PAGE_SIZE: i32 = 50;

/// Continuation tokens, one per namespace that still has pages. The client owns the shape;
/// this is an alias so the reducer reads the same way.
use tmprl_client::Continuation as Tokens;

/// Which list is on screen. Temporal's objects form a hierarchy and `-` walks up it, so
/// this is a level rather than a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Namespaces,
    Workflows,
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
    /// `Some` while the `:` command line is open.
    pub cmdline: Option<String>,
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
    /// Bumped whenever the query or scope changes. Replies carrying an older generation
    /// belong to a query the user has already moved on from.
    generation: u64,
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
            cmdline: None,
            insert_buf: String::new(),
            insert_target: InsertTarget::Scratch,
            note: None,
            should_quit: false,
            dirty: true,
            cursor_key: None,
            namespace_cursor: 0,
            generation: 0,
            loading_more: false,
            profile,
            namespace,
            conn,
            tx,
        }
    }

    /// Install the user's `keys.toml` and `views.toml`. Called once at startup, before the
    /// first frame, so the help overlay and which-key describe the keymap actually in use.
    pub fn apply_config(&mut self, keys: Option<&str>, views: Option<&str>) {
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
        if self.cmdline.is_some() {
            self.cmdline_key(chord);
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
                self.cmdline = Some(String::new());
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
            // Honest rather than silent: the binding exists because it works one level up.
            Screen::Workflows => {
                self.note = Some((
                    "workflow detail arrives in M2; the list is all there is today".into(),
                    Note::Warn,
                ));
            }
        }
    }

    fn go_up(&mut self) {
        match self.screen {
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
        self.cursor = at;
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
        if self.screen != Screen::Workflows || self.loading_more {
            return;
        }
        let len = self.row_count();
        let has_more = self.workflows.value().is_some_and(WorkflowList::has_more);
        if has_more && self.cursor + self.page.max(1) >= len {
            self.load_more();
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
        };
        match picked.len() {
            0 => String::new(),
            1 => picked.into_iter().next().unwrap(),
            _ => format!("[{}]", picked.join(",")),
        }
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

    fn cmdline_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(buf) = self.cmdline.as_mut() else {
            return;
        };
        match chord.key {
            Key::Esc => {
                self.cmdline = None;
                self.mode = Mode::Normal;
            }
            Key::Enter => {
                let entered = buf.trim().to_string();
                self.cmdline = None;
                self.mode = Mode::Normal;
                if entered.is_empty() {
                    return;
                }
                // Accept a unique prefix, the way vim accepts `:q` for `:quit`.
                let hits = self.registry.search(&entered);
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
            // Backspace on an empty line closes the command line, as it does in vim.
            Key::Backspace if buf.pop().is_none() => {
                self.cmdline = None;
                self.mode = Mode::Normal;
            }
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => buf.push(c),
            _ => {}
        }
    }

    /// Completions for whatever is typed in the command line.
    pub fn cmdline_matches(&self) -> Vec<&tmprl_core::Command> {
        match &self.cmdline {
            Some(q) => self.registry.search(q).into_iter().take(8).collect(),
            None => Vec::new(),
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

    #[test]
    fn config_errors_are_surfaced_rather_than_swallowed() {
        let mut app = app();
        app.apply_config(Some("[normal]\n\"x\" = \"nope.nope\"\n"), None);
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
