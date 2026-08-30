//! Editing modes.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
    /// The `:` command line.
    Command,
}

impl Mode {
    /// Shown in the statusline. Uppercase, like vim's.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
            Mode::Command => "COMMAND",
        }
    }

    /// Whether a leading digit starts a count rather than being literal input.
    pub fn takes_counts(self) -> bool {
        matches!(self, Mode::Normal | Mode::Visual | Mode::VisualLine)
    }

    /// Whether unmatched keys should be inserted as text.
    pub fn is_text_entry(self) -> bool {
        matches!(self, Mode::Insert | Mode::Command)
    }

    pub fn is_visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_normal_and_visual_take_counts() {
        assert!(Mode::Normal.takes_counts());
        assert!(Mode::Visual.takes_counts());
        assert!(Mode::VisualLine.takes_counts());
        assert!(!Mode::Insert.takes_counts());
        assert!(!Mode::Command.takes_counts());
    }

    #[test]
    fn text_entry_modes_are_not_count_modes() {
        for m in [
            Mode::Normal,
            Mode::Insert,
            Mode::Visual,
            Mode::VisualLine,
            Mode::Command,
        ] {
            assert!(
                !(m.takes_counts() && m.is_text_entry()),
                "{m} cannot be both"
            );
        }
    }
}
