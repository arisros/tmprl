//! A small multi-field editor, for the things that need more than one value.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub title: &'static str,
    pub fields: Vec<Field>,
    pub cursor: usize,
}

impl Form {
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

    pub fn backspace(&mut self) -> bool {
        self.fields
            .get_mut(self.cursor)
            .and_then(|f| f.value.pop())
            .is_some()
    }

    pub fn get(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.trim())
            .unwrap_or("")
    }

    pub fn missing(&self) -> Option<&'static str> {
        self.fields
            .iter()
            .find(|f| f.required && f.value.trim().is_empty())
            .map(|f| f.label)
    }

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
