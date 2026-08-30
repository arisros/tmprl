//! Application state and the reducer.
//!
//! The one rule: [`App::handle`] is synchronous and never awaits. When it needs data it
//! spawns a task, which reports back as another [`Msg`]. Nothing on the keystroke path can
//! block on the network — see `docs/ARCHITECTURE.md`.

use std::sync::Arc;

use tmprl_client::{Conn, NamespaceInfo};
use tmprl_core::{
    Action, Chord, Keymap, Loadable, Mode, Pending, PendingEntry, Registry, Resolution,
    default_keymap,
};
use tokio::sync::mpsc::UnboundedSender;

/// Everything that can change the application state.
#[derive(Debug)]
pub enum Msg {
    Key(Chord),
    Tick,
    Redraw,
    Quit,
    Namespaces(Result<Vec<NamespaceInfo>, String>),
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

    pub namespaces: Loadable<Vec<NamespaceInfo>>,
    pub cursor: usize,
    /// Where a visual selection started, if one is active.
    pub anchor: Option<usize>,
    /// Rows the list pane can show — set by the renderer, used by half-page motions.
    pub page: usize,

    pub which_key: Vec<PendingEntry>,
    pub show_help: bool,
    /// `Some` while the `:` command line is open.
    pub cmdline: Option<String>,
    pub insert_buf: String,

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

    /// An app with no connection, for tests. Every command except `app.refresh` behaves
    /// identically, which is what makes the interface testable without a Temporal server.
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
            namespaces: Loadable::NotAsked,
            cursor: 0,
            anchor: None,
            page: 10,
            which_key: Vec::new(),
            show_help: false,
            cmdline: None,
            insert_buf: String::new(),
            note: None,
            should_quit: false,
            dirty: true,
            profile,
            namespace,
            conn,
            tx,
        }
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn rows(&self) -> &[NamespaceInfo] {
        self.namespaces.value().map(Vec::as_slice).unwrap_or(&[])
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
                    for c in flushed {
                        if let Some(ch) = c.as_insertable() {
                            self.insert_buf.push(ch);
                        }
                    }
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
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::OpenCommandLine => {
                self.cmdline = Some(String::new());
                self.mode = Mode::Command;
            }
            Action::Cancel => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.anchor = None;
                    self.mode = Mode::Normal;
                    self.pending.clear();
                    self.which_key.clear();
                }
            }
            Action::Refresh => self.load_namespaces(),

            Action::MoveDown => self.move_cursor(n as isize),
            Action::MoveUp => self.move_cursor(-(n as isize)),
            Action::MoveTop => self.cursor = 0,
            Action::MoveBottom => self.cursor = self.rows().len().saturating_sub(1),
            Action::HalfPageDown => self.move_cursor((self.page / 2).max(1) as isize),
            Action::HalfPageUp => self.move_cursor(-((self.page / 2).max(1) as isize)),

            Action::EnterInsert => {
                self.mode = Mode::Insert;
                self.insert_buf.clear();
            }
            Action::LeaveInsert => {
                self.mode = Mode::Normal;
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
        }
        self.clamp_cursor();
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.rows().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, len as isize - 1);
        self.cursor = next as usize;
    }

    fn clamp_cursor(&mut self) {
        let len = self.rows().len();
        self.cursor = self.cursor.min(len.saturating_sub(1));
        if len == 0 {
            self.cursor = 0;
        }
    }

    // ── yanking ──────────────────────────────────────────────────────────────

    fn field_under_cursor(&self) -> String {
        self.rows()
            .get(self.cursor)
            .map(|n| n.name.clone())
            .unwrap_or_default()
    }

    /// The selected rows as JSON, or just the row under the cursor when nothing is selected.
    fn records_selected(&self) -> String {
        let rows = self.rows();
        let (lo, hi) = self.selection().unwrap_or((self.cursor, self.cursor));
        let picked: Vec<String> = rows
            .iter()
            .skip(lo)
            .take(hi.saturating_sub(lo) + 1)
            .map(|n| {
                format!(
                    r#"{{"name":{},"state":{},"retentionDays":{}}}"#,
                    json_string(&n.name),
                    json_string(&n.state),
                    n.retention_days
                )
            })
            .collect();
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
}

/// Minimal JSON string escaping — enough for names and states, which are the only strings
/// yanked today. A real serializer arrives with payload rendering in M2.
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

    #[test]
    fn json_strings_escape_control_characters() {
        assert_eq!(json_string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(json_string("a\nb"), r#""a\nb""#);
        assert_eq!(json_string("a\u{1}b"), r#""a\u0001b""#);
    }
}
