//! The workflow list's query bar: editing it, and the shortcuts that write into it.

use super::*;

impl App {
    pub fn is_editing_query(&self) -> bool {
        self.mode == Mode::Insert && self.insert_target == InsertTarget::Query
    }

    pub fn query_display(&self) -> &str {
        if self.is_editing_query() {
            self.insert_buf.as_str()
        } else {
            &self.view.query
        }
    }

    pub(super) fn show_problems(&mut self) {
        self.mark_jump();
        const PROBLEMS: &str =
            "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated') ORDER BY StartTime DESC";
        self.stop_following();
        self.view.query = PROBLEMS.to_string();
        if self.view.screen != Screen::Workflows {
            self.view.screen = Screen::Workflows;
            self.view.viewing = None;
            self.reset_history();
        }
        self.view.cursor = 0;
        self.view.cursor_key = None;
        self.note = Some(("problems: failed, timed out, terminated".into(), Note::Info));
        self.load_workflows(false);
    }

    pub(super) fn add_clause(&mut self, clause: &str) {
        self.mark_jump();
        // ORDER BY cannot be ANDed; the server rejects it.
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
        if matches!(self.view.screen, Screen::Namespaces | Screen::Dashboard) {
            self.view.stop_dashboard();
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
        self.view.query = query;
        if matches!(self.view.screen, Screen::Namespaces | Screen::Dashboard) {
            self.view.stop_dashboard();
            self.view.screen = Screen::Workflows;
        }
        self.note = Some((format!("view: {name}"), Note::Info));
        self.load_workflows(false);
    }

    pub(super) fn insert_keys(&mut self, flushed: Vec<Chord>) {
        use tmprl_core::{Key, Mods};
        for c in flushed {
            match c.key {
                Key::Enter if c.mods.is_none() => self.commit_insert(),
                Key::Tab if c.mods.is_none() => self.accept_completion(),
                Key::Char('n') if c.mods == Mods::CTRL => self.move_completion(1),
                Key::Char('p') if c.mods == Mods::CTRL => self.move_completion(-1),
                Key::Up | Key::Down if self.insert_target == InsertTarget::Query => {
                    let history = self.histories.entry(History::Query).or_default();
                    let recalled = if c.key == Key::Up {
                        history.older(self.insert_buf.as_str())
                    } else {
                        history.newer()
                    };
                    if let Some(text) = recalled {
                        self.insert_buf.set(text);
                        self.completion = None;
                    }
                }
                _ => {
                    if self.insert_buf.key(c) == Edit::Changed {
                        self.recompute_completion();
                    }
                }
            }
        }
    }

    pub(super) fn recompute_completion(&mut self) {
        if !self.is_editing_query() {
            self.completion = None;
            return;
        }
        let clauses = self.filter_clauses();
        self.completion = Completion::new(self.insert_buf.as_str(), clauses);
    }

    fn move_completion(&mut self, delta: isize) {
        if let Some(c) = self.completion.as_mut() {
            c.move_cursor(delta);
        }
    }

    fn accept_completion(&mut self) {
        let Some(clause) = self
            .completion
            .as_ref()
            .and_then(|c| c.selected())
            .map(|c| c.text.clone())
        else {
            return;
        };
        self.insert_buf
            .set(complete::apply(self.insert_buf.as_str(), &clause));
        self.completion = None;
    }

    pub(super) fn commit_insert(&mut self) {
        if self.insert_target != InsertTarget::Query {
            return;
        }
        self.view.query = self.insert_buf.as_str().to_string();
        self.histories
            .entry(History::Query)
            .or_default()
            .push(&self.view.query);
        self.mode = Mode::Normal;
        self.insert_target = InsertTarget::Scratch;
        self.insert_buf.clear();
        self.completion = None;
        self.load_workflows(false);
    }
}
