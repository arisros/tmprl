//! The picker: a filtered list you type at.
//!
//! One implementation behind every `<leader>f` binding. The things being picked are very
//! different, workflows, history rows, open panes, commands, query fragments, but the
//! interaction is identical every time: a prompt, a list that narrows as you type, a cursor
//! you move with `<C-n>` and `<C-p>`, `Enter` to take one. Building that five times is how
//! five slightly different pickers happen.
//!
//! Pure, and deliberately so. A picker holds strings and an opaque [`Target`], never a
//! `WorkflowRow` or a pane handle, so this module compiles without knowing what a workflow
//! is and can be driven in a unit test without a screen. Deciding what to *do* with an
//! accepted target belongs to `tmprl-tui`, and the match on [`Target`] there is exhaustive,
//! so a new kind of picker cannot be silently unhandled.

use crate::fuzzy::{self, Match};

/// What accepting an entry means.
///
/// Deliberately a small closed set of *outcomes* rather than one variant per picker: two
/// pickers that both end in "put the cursor on a row" should not need two code paths to do
/// it. `<leader>fl` and a future `<leader>fs` are both [`Target::Row`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Open this workflow's history. Namespace and run id, because a run id is only unique
    /// within a namespace and a picker can be fanned out over several.
    Workflow { namespace: String, run_id: String },
    /// Put the cursor on this row of the screen the picker was opened from.
    Row(usize),
    /// Run this command id.
    Command(String),
    /// Focus this pane.
    Pane(u64),
    /// Write this text into the query bar, leaving it editable.
    Query(String),
    /// Point this pane at a namespace and show its workflows.
    Namespace(String),
}

/// One candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// What is matched against and rendered in the list.
    pub label: String,
    /// The second, dimmer column: a workflow's type, a command's group. Not matched
    /// against, so that typing a status does not pull in every row that merely ends in it.
    pub note: String,
    /// The body of the preview pane. Empty means the picker has no preview to show, which
    /// is the honest thing for a list of command ids.
    pub preview: String,
    pub target: Target,
}

impl Item {
    pub fn new(label: impl Into<String>, target: Target) -> Self {
        Self {
            label: label.into(),
            note: String::new(),
            preview: String::new(),
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
}

/// Which picker is open. Only used for the title, but a title that says what you are
/// looking at is the difference between five pickers and one confusing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Workflows,
    HistoryRows,
    Panes,
    Commands,
    Filters,
    Namespaces,
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
        }
    }
}

#[derive(Debug, Clone)]
pub struct Picker {
    pub kind: Kind,
    /// What has been typed, verbatim.
    pub prompt: String,
    /// Every candidate, in the order it was handed over. That order is the tiebreak when
    /// scores are equal, so a workflow picker opens newest-first like the list behind it.
    items: Vec<Item>,
    /// Indices into `items`, best match first. Rebuilt on every keystroke.
    hits: Vec<(usize, Match)>,
    /// Position within `hits`, not within `items`.
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

    pub fn shown(&self) -> usize {
        self.hits.len()
    }

    /// The visible entries, best first, each with the match positions that produced it so
    /// the renderer can underline the characters that were actually typed.
    pub fn rows(&self) -> impl Iterator<Item = (&Item, &Match)> {
        self.hits.iter().map(|(i, m)| (&self.items[*i], m))
    }

    pub fn selected(&self) -> Option<&Item> {
        self.hits.get(self.cursor).map(|(i, _)| &self.items[*i])
    }

    /// The target of the entry under the cursor, if there is one.
    pub fn accept(&self) -> Option<&Target> {
        self.selected().map(|i| &i.target)
    }

    pub fn push(&mut self, c: char) {
        self.prompt.push(c);
        self.refilter();
    }

    /// Delete a character. `false` when there was nothing to delete, which the caller turns
    /// into closing the picker, the way backspace on an empty prompt does everywhere else.
    pub fn backspace(&mut self) -> bool {
        let had = self.prompt.pop().is_some();
        if had {
            self.refilter();
        }
        had
    }

    /// Move the cursor, clamping rather than wrapping.
    ///
    /// Clamping, unlike `/`, because a picker's list is right there: running off the bottom
    /// and reappearing at the top in a list you can see in full is disorienting, where in a
    /// thousand-row table a wrap is the only way to keep going.
    pub fn move_cursor(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.cursor = 0;
            return;
        }
        let last = self.hits.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    /// Re-rank against the current prompt.
    ///
    /// The cursor goes back to the top on every keystroke, because the best match is what
    /// typing another character is *for*. Holding position would leave the cursor on
    /// whatever happens to be at that index in a completely different list.
    fn refilter(&mut self) {
        self.hits = fuzzy::rank(&self.prompt, &self.items, |i| i.label.clone());
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
        // The list behind a workflow picker is already sorted newest-first; opening the
        // picker must not throw that away before anything has been typed.
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
        // Otherwise the cursor keeps an index into a list that no longer has the same
        // contents, and lands on something unrelated.
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
        // Accepting must be impossible rather than accidentally taking row 0 of a list that
        // is not showing anything.
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
    fn notes_are_not_matched_against() {
        // Typing a status should not drag in every row whose *type* happens to spell it.
        let items = vec![Item::new("order-1", Target::Row(0)).with_note("Running")];
        let mut p = Picker::new(Kind::Workflows, items);
        for c in "running".chars() {
            p.push(c);
        }
        assert!(p.is_empty(), "the note is shown, not searched");
    }
}
