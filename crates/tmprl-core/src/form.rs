//! A small multi-field editor, for the things that need more than one value.
//!
//! Every other input in tmprl is one value, and the one-line prompt covers those. Creating a
//! schedule needs six, which as a positional line would be unreadable to type and silent to
//! mistype: a task queue and a cron string are both strings, so a swapped pair only surfaces
//! as a server error much later.
//!
//! Editing lives here rather than in the renderer so that field movement, validation and the
//! rendered command are all testable without a terminal.

/// One labelled line of a [`Form`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
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
            value: String::new(),
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
            f.value.push(c);
        }
    }

    /// Delete backwards. Reports whether anything was there, so the caller can decide what an
    /// empty backspace means.
    pub fn backspace(&mut self) -> bool {
        self.fields
            .get_mut(self.cursor)
            .and_then(|f| f.value.pop())
            .is_some()
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
    fn backspace_says_whether_it_removed_anything() {
        let mut f = Form::new_schedule();
        assert!(!f.backspace(), "nothing to delete on an empty field");
        f.push('x');
        assert!(f.backspace());
        assert_eq!(f.get("schedule id"), "");
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
