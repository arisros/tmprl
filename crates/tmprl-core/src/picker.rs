//! The picker: a filtered list you type at.

use crate::fuzzy::{self, Match};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Workflow { namespace: String, run_id: String },
    Row(usize),
    Command(String),
    Pane(u64),
    Query(String),
    Namespace(String),
    Profile(String),
    Handler(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub fetched: bool,
    pub lookup: bool,
    pub note: String,
    pub preview: String,
    pub keywords: String,
    pub target: Target,
}

impl Item {
    pub fn new(label: impl Into<String>, target: Target) -> Self {
        Self {
            label: label.into(),
            fetched: false,
            lookup: false,
            note: String::new(),
            preview: String::new(),
            keywords: String::new(),
            target,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    pub fn with_preview(mut self, preview: impl Into<String>) -> Self {
        self.preview = preview.into();
        self
    }

    pub fn from_lookup(mut self) -> Self {
        self.fetched = true;
        self.lookup = true;
        self
    }

    pub fn searchable_as(mut self, keywords: impl Into<String>) -> Self {
        self.keywords = keywords.into();
        self
    }

    fn haystack(&self) -> String {
        if self.keywords.is_empty() {
            self.label.clone()
        } else {
            format!("{} {}", self.label, self.keywords)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Workflows,
    HistoryRows,
    Panes,
    Commands,
    Filters,
    Namespaces,
    Profiles,
    Queries,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Workflows => "workflows",
            Kind::HistoryRows => "events",
            Kind::Panes => "panes",
            Kind::Commands => "commands",
            Kind::Filters => "filters",
            Kind::Namespaces => "namespaces",
            Kind::Profiles => "profiles",
            Kind::Queries => "queries",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Picker {
    pub kind: Kind,
    pub prompt: String,
    items: Vec<Item>,
    hits: Vec<(usize, Match)>,
    pub cursor: usize,
}

impl Picker {
    pub fn new(kind: Kind, items: Vec<Item>) -> Self {
        let mut p = Self {
            kind,
            prompt: String::new(),
            items,
            hits: Vec::new(),
            cursor: 0,
        };
        p.refilter();
        p
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    pub fn answer(&mut self, items: impl IntoIterator<Item = Item>) -> usize {
        let cursor = self.cursor;
        let mut shown = 0;
        for item in items {
            shown += 1;
            match self.items.iter_mut().find(|i| i.target == item.target) {
                Some(held) => held.lookup = true,
                None => self.items.push(item),
            }
        }
        self.refilter();
        self.cursor = cursor.min(self.hits.len().saturating_sub(1));
        shown
    }

    pub fn shown(&self) -> usize {
        self.hits.len()
    }

    pub fn rows(&self) -> impl Iterator<Item = (&Item, &Match)> {
        self.hits.iter().map(|(i, m)| (&self.items[*i], m))
    }

    pub fn selected(&self) -> Option<&Item> {
        self.hits.get(self.cursor).map(|(i, _)| &self.items[*i])
    }

    pub fn accept(&self) -> Option<&Target> {
        self.selected().map(|i| &i.target)
    }

    pub fn push(&mut self, c: char) {
        self.prompt.push(c);
        self.forget_answers();
        self.refilter();
    }

    pub fn backspace(&mut self) -> bool {
        let had = self.prompt.pop().is_some();
        if had {
            self.forget_answers();
            self.refilter();
        }
        had
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.cursor = 0;
            return;
        }
        let last = self.hits.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    fn forget_answers(&mut self) {
        self.items.retain(|i| !i.fetched);
        for i in &mut self.items {
            i.lookup = false;
        }
    }

    fn refilter(&mut self) {
        let mut local: Vec<(usize, Match)> = Vec::new();
        let mut fetched: Vec<(usize, Match)> = Vec::new();
        for (at, item) in self.items.iter().enumerate() {
            if item.lookup {
                fetched.push((at, Match::default()));
            } else if let Some(m) = fuzzy::match_score(&self.prompt, &item.haystack()) {
                local.push((at, m));
            }
        }
        local.sort_by_key(|h| std::cmp::Reverse(h.1.score));
        local.extend(fetched);
        self.hits = local;
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[&str]) -> Vec<Item> {
        labels
            .iter()
            .enumerate()
            .map(|(i, l)| Item::new(*l, Target::Row(i)))
            .collect()
    }

    fn picker(labels: &[&str]) -> Picker {
        Picker::new(Kind::Workflows, items(labels))
    }

    fn labels(p: &Picker) -> Vec<String> {
        p.rows().map(|(i, _)| i.label.clone()).collect()
    }

    #[test]
    fn a_new_picker_shows_everything_in_the_order_given() {
        let p = picker(&["c", "a", "b"]);
        assert_eq!(labels(&p), vec!["c", "a", "b"]);
        assert_eq!(p.shown(), 3);
        assert_eq!(p.total(), 3);
    }

    #[test]
    fn typing_narrows_the_list() {
        let mut p = picker(&["order-checkout", "order-refund", "shipping"]);
        p.push('o');
        p.push('r');
        assert_eq!(p.shown(), 2, "shipping has no 'or'");
    }

    #[test]
    fn the_best_match_is_selected_as_you_type() {
        let mut p = picker(&["processor", "order-checkout"]);
        for c in "oc".chars() {
            p.push(c);
        }
        assert_eq!(
            p.selected().map(|i| i.label.as_str()),
            Some("order-checkout"),
            "a word-start match should outrank one buried mid-word"
        );
    }

    #[test]
    fn the_cursor_returns_to_the_top_on_every_keystroke() {
        let mut p = picker(&["alpha", "beta", "gamma"]);
        p.move_cursor(2);
        assert_eq!(p.cursor, 2);
        p.push('a');
        assert_eq!(p.cursor, 0);
    }

    #[test]
    fn the_cursor_clamps_rather_than_wrapping() {
        let mut p = picker(&["a", "b"]);
        p.move_cursor(10);
        assert_eq!(p.cursor, 1, "clamped to the last row");
        p.move_cursor(-10);
        assert_eq!(p.cursor, 0, "clamped to the first");
    }

    #[test]
    fn backspace_reports_when_there_is_nothing_left_to_delete() {
        let mut p = picker(&["a"]);
        p.push('a');
        assert!(p.backspace(), "deleted the 'a'");
        assert!(
            !p.backspace(),
            "empty, so the caller should close the picker"
        );
    }

    #[test]
    fn backspace_widens_the_list_again() {
        let mut p = picker(&["order", "shipping"]);
        p.push('o');
        p.push('r');
        assert_eq!(p.shown(), 1);
        p.backspace();
        p.backspace();
        assert_eq!(p.shown(), 2, "back to everything");
    }

    #[test]
    fn a_prompt_matching_nothing_leaves_no_selection() {
        let mut p = picker(&["order"]);
        for c in "zzz".chars() {
            p.push(c);
        }
        assert!(p.is_empty());
        assert_eq!(p.selected(), None);
        assert_eq!(p.accept(), None);
    }

    #[test]
    fn accept_returns_the_target_of_the_row_under_the_cursor() {
        let mut p = picker(&["alpha", "beta"]);
        p.move_cursor(1);
        assert_eq!(p.accept(), Some(&Target::Row(1)));
    }

    #[test]
    fn match_positions_come_back_for_highlighting() {
        let mut p = picker(&["order-checkout"]);
        p.push('o');
        let (item, m) = p.rows().next().unwrap();
        assert_eq!(m.positions.len(), 1);
        assert!(item.label.is_char_boundary(m.positions[0]));
    }

    #[test]
    fn an_answer_the_picker_already_holds_is_promoted_not_dropped() {
        let mut p = picker(&["order-1", "order-2"]);
        for c in "01a0c4bc".chars() {
            p.push(c);
        }
        assert!(p.is_empty(), "the run id matches no label");

        let shown = p.answer(vec![Item::new("order-2", Target::Row(1)).from_lookup()]);
        assert_eq!(shown, 1);
        assert_eq!(labels(&p), vec!["order-2"]);
        assert_eq!(p.total(), 2, "it was promoted, not added again");
        assert!(matches!(p.accept(), Some(Target::Row(1))));
    }

    #[test]
    fn answers_join_the_list_without_disturbing_the_prompt() {
        let mut p = picker(&["order-1", "order-2"]);
        p.push('o');
        p.push('r');
        p.answer(vec![Item::new("order-9", Target::Row(9)).from_lookup()]);

        assert_eq!(p.prompt, "or", "typing must survive the arrival");
        assert!(labels(&p).contains(&"order-9".to_string()));
        assert_eq!(p.items.iter().filter(|i| i.fetched).count(), 1);
    }

    #[test]
    fn a_fetched_candidate_survives_a_prompt_its_label_does_not_match() {
        let mut p = picker(&["order-1", "order-2"]);
        for c in "01a0c4bc-b01b".chars() {
            p.push(c);
        }
        assert!(p.is_empty(), "nothing loaded matches a run id");

        p.answer(vec![
            Item::new("big-history-1", Target::Row(7)).from_lookup(),
        ]);
        assert_eq!(labels(&p), vec!["big-history-1"], "the answer must show");
        assert!(matches!(p.accept(), Some(Target::Row(7))));
    }

    #[test]
    fn typing_again_forgets_what_was_fetched_for_the_old_prompt() {
        let mut p = picker(&["order-1"]);
        for c in "01a0".chars() {
            p.push(c);
        }
        p.answer(vec![
            Item::new("big-history-1", Target::Row(7)).from_lookup(),
        ]);
        assert_eq!(labels(&p), vec!["big-history-1"]);

        p.push('x');
        assert!(
            p.is_empty(),
            "a row fetched for another prompt must not linger"
        );
    }

    #[test]
    fn notes_are_not_matched_against() {
        let items = vec![Item::new("order-1", Target::Row(0)).with_note("Running")];
        let mut p = Picker::new(Kind::Workflows, items);
        for c in "running".chars() {
            p.push(c);
        }
        assert!(p.is_empty(), "the note is shown, not searched");
    }
}
