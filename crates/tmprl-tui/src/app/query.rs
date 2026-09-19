//! The workflow list's query bar: editing it, and the shortcuts that write into it.

use super::*;

impl App {
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

    /// `<leader>xx`: everything that went wrong.
    ///
    /// A query preset rather than a screen of its own, which is the whole reason the query
    /// bar is the interface. It lands in the bar, visible and editable, so narrowing it
    /// further is ordinary editing rather than a feature this would have had to grow.
    ///
    /// `IN` rather than three `OR`s: Temporal's visibility grammar takes it, and the result
    /// is short enough to read in the bar, which a three-clause disjunction is not.
    pub(super) fn show_problems(&mut self) {
        self.mark_jump();
        const PROBLEMS: &str =
            "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated') ORDER BY StartTime DESC";
        self.stop_following();
        self.view.query = PROBLEMS.to_string();
        if self.view.screen != Screen::Workflows {
            self.view.screen = Screen::Workflows;
            self.view.viewing = None;
            // Including the continuation tokens: `load_history` passes `history_token`
            // unconditionally, so a token left over from the history being abandoned would
            // be sent to whichever run is opened next.
            self.reset_history();
        }
        self.view.cursor = 0;
        self.view.cursor_key = None;
        self.note = Some(("problems: failed, timed out, terminated".into(), Note::Info));
        self.load_workflows(false);
    }

    /// Append a clause to the visibility query and apply it.
    ///
    /// `AND`ed onto whatever is already there rather than replacing it, so the picker
    /// composes: status, then type, then task queue, three visits and no typing. The text
    /// stays in the bar and stays editable, which is the rule the whole query bar is built
    /// on.
    pub(super) fn add_clause(&mut self, clause: &str) {
        self.mark_jump();
        // An `ORDER BY` is a trailing clause, not a predicate: `AND`ing it produces a query
        // the server rejects.
        let ordering = clause
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("order by");
        let current = self.view.query.trim().to_string();
        self.view.query = if current.is_empty() {
            clause.to_string()
        } else if ordering {
            format!("{current} {clause}")
        } else {
            format!("{current} AND {clause}")
        };
        if self.view.screen == Screen::Namespaces {
            self.view.screen = Screen::Workflows;
        }
        self.note = Some((format!("query: {}", self.view.query), Note::Info));
        self.load_workflows(false);
    }

    pub(super) fn select_view(&mut self, key: char) {
        let Some(view) = self.views.iter().find(|v| v.key == key) else {
            self.note = Some((format!("no saved view on `{key}`"), Note::Warn));
            return;
        };
        let (name, query) = (view.name.clone(), view.query.clone());
        self.mark_jump();
        // A view is a bookmark, not a mode: it fills the query bar, which stays editable.
        self.view.query = query;
        if self.view.screen == Screen::Namespaces {
            self.view.screen = Screen::Workflows;
        }
        self.note = Some((format!("view: {name}"), Note::Info));
        self.load_workflows(false);
    }

    /// Literal input, plus the two editing keys a text field cannot do without.
    pub(super) fn insert_keys(&mut self, flushed: Vec<Chord>) {
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
    pub(super) fn commit_insert(&mut self) {
        if self.insert_target != InsertTarget::Query {
            return;
        }
        self.view.query = self.insert_buf.clone();
        self.mode = Mode::Normal;
        self.insert_target = InsertTarget::Scratch;
        self.insert_buf.clear();
        self.load_workflows(false);
    }
}
