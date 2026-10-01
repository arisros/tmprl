//! A small multi-field editor, for the things that need more than one value.
//!
//! Every other input in tmprl is one value, and the one-line prompt covers those. Creating a
//! schedule needs six, which as a positional line would be unreadable to type and silent to
//! mistype: a task queue and a cron string are both strings, so a swapped pair only surfaces
//! as a server error much later.
//!
//! Editing lives here rather than in the renderer so that field movement, validation and the
//! rendered command are all testable without a terminal.

use crate::key::Chord;
use crate::line::{Edit, LineEdit};

/// One labelled line of a [`Form`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub value: LineEdit,
    /// Shown in place of an empty value, to say what belongs there.
    pub hint: &'static str,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub title: &'static str,
    pub fields: Vec<Field>,
    /// Which field the caret is on.
    pub cursor: usize,
}

impl Form {
    /// The fields `temporal schedule create` needs.
    ///
    /// `workflow id` is the id given to the workflows the schedule starts, not the schedule's
    /// own id; the server appends the scheduled time to it, so the two are different things
    /// and naming both avoids the guess.
    pub fn new_schedule() -> Self {
        let f = |label, hint, required| Field {
            label,
            value: LineEdit::default(),
            hint,
            required,
        };
        Form {
            title: "new schedule",
            fields: vec![
                f("schedule id", "nightly-recon", true),
                f("workflow id", "recon", true),
                f("workflow type", "OrderWorkflow", true),
                f("task queue", "demo-tq", true),
                f("spec", "0 2 * * *  or  @every 1h", true),
                f("input", "optional JSON", false),
            ],
            cursor: 0,
        }
    }

    /// Move down a field, wrapping. Tab in a form goes forward and stops nowhere.
    pub fn next(&mut self) {
        if !self.fields.is_empty() {
            self.cursor = (self.cursor + 1) % self.fields.len();
        }
    }

    pub fn previous(&mut self) {
        if !self.fields.is_empty() {
            self.cursor = (self.cursor + self.fields.len() - 1) % self.fields.len();
        }
    }

    pub fn push(&mut self, c: char) {
        if let Some(f) = self.fields.get_mut(self.cursor) {
            f.value.insert(c);
        }
    }

    /// An editing key, in the focused field.
    pub fn key(&mut self, chord: Chord) -> Edit {
        self.fields
            .get_mut(self.cursor)
            .map_or(Edit::Ignored, |f| f.value.key(chord))
    }

    /// Pasted text, into the focused field at its cursor. A paste never moves to the next
    /// field: where one value ends and the next begins is not something a clipboard says.
    pub fn paste(&mut self, pasted: &str) -> bool {
        self.fields
            .get_mut(self.cursor)
            .is_some_and(|f| f.value.paste(pasted))
    }

    /// Whether the focused field has nothing in it, so the caller can decide what a
    /// backspace there means.
    pub fn on_empty_field(&self) -> bool {
        self.fields
            .get(self.cursor)
            .is_none_or(|f| f.value.is_empty())
    }

    /// The value of a field by label, trimmed. Empty when absent.
    pub fn get(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.trim())
            .unwrap_or("")
    }

    /// The first required field still empty, if any.
    ///
    /// Checked before the confirmation rather than after, so the reader is sent back to the
    /// field that is missing instead of reading a command with a hole in it.
    pub fn missing(&self) -> Option<&'static str> {
        self.fields
            .iter()
            .find(|f| f.required && f.value.trim().is_empty())
            .map(|f| f.label)
    }

    /// Put the caret on a named field, for jumping to the one that failed validation.
    pub fn focus(&mut self, label: &str) {
        if let Some(i) = self.fields.iter().position(|f| f.label == label) {
            self.cursor = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Key;

    #[test]
    fn a_new_schedule_asks_for_everything_the_server_requires() {
        // schedule-id, workflow-id, task-queue and type are all required by
        // `temporal schedule create`, and a spec with none of cron/interval/calendar
        // creates a schedule that never fires.
        let f = Form::new_schedule();
        let required: Vec<&str> = f
            .fields
            .iter()
            .filter(|f| f.required)
            .map(|f| f.label)
            .collect();
        assert_eq!(
            required,
            [
                "schedule id",
                "workflow id",
                "workflow type",
                "task queue",
                "spec"
            ]
        );
    }

    #[test]
    fn typing_lands_on_the_focused_field_only() {
        let mut f = Form::new_schedule();
        for c in "nightly".chars() {
            f.push(c);
        }
        f.next();
        for c in "recon".chars() {
            f.push(c);
        }
        assert_eq!(f.get("schedule id"), "nightly");
        assert_eq!(f.get("workflow id"), "recon");
    }

    #[test]
    fn the_caret_wraps_in_both_directions() {
        let mut f = Form::new_schedule();
        let last = f.fields.len() - 1;
        f.previous();
        assert_eq!(f.cursor, last, "back from the first goes to the last");
        f.next();
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn an_empty_field_says_so_before_and_after_a_backspace() {
        let mut f = Form::new_schedule();
        assert!(f.on_empty_field());
        f.push('x');
        assert!(!f.on_empty_field());
        assert_eq!(f.key(Chord::plain(Key::Backspace)), Edit::Changed);
        assert!(f.on_empty_field());
        assert_eq!(f.get("schedule id"), "");
    }

    #[test]
    fn editing_keys_act_on_the_focused_field_and_leave_the_rest() {
        let mut f = Form::new_schedule();
        for c in "nightly".chars() {
            f.push(c);
        }
        f.next();
        for c in "recon".chars() {
            f.push(c);
        }
        f.key(Chord::plain(Key::Home));
        f.push('x');
        f.key(Chord::ctrl('k'));
        assert_eq!(f.get("workflow id"), "x");
        assert_eq!(f.get("schedule id"), "nightly");
    }

    #[test]
    fn each_field_keeps_its_own_cursor() {
        let mut f = Form::new_schedule();
        f.push('a');
        f.push('c');
        f.key(Chord::plain(Key::Left));
        f.next();
        f.previous();
        f.push('b');
        assert_eq!(f.get("schedule id"), "abc");
    }

    #[test]
    fn a_paste_fills_one_field_and_stays_in_it() {
        let mut f = Form::new_schedule();
        assert!(f.paste("nightly\nrecon\n"));
        assert_eq!(f.cursor, 0, "a newline in a paste is not a Tab");
        assert_eq!(f.get("schedule id"), "nightly recon");
        assert_eq!(f.get("workflow id"), "");
    }

    #[test]
    fn a_missing_required_field_is_named_rather_than_merely_counted() {
        // The reader is sent back to the field, so the answer has to be which one.
        let mut f = Form::new_schedule();
        assert_eq!(f.missing(), Some("schedule id"));
        for (label, v) in [
            ("schedule id", "nightly"),
            ("workflow id", "recon"),
            ("workflow type", "OrderWorkflow"),
            ("task queue", "demo-tq"),
        ] {
            f.focus(label);
            for c in v.chars() {
                f.push(c);
            }
        }
        assert_eq!(f.missing(), Some("spec"));
        f.focus("spec");
        for c in "0 2 * * *".chars() {
            f.push(c);
        }
        assert_eq!(f.missing(), None, "input is optional");
    }

    #[test]
    fn whitespace_alone_does_not_satisfy_a_required_field() {
        let mut f = Form::new_schedule();
        f.push(' ');
        assert_eq!(f.missing(), Some("schedule id"));
    }
}
