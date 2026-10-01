//! The note line's history, `:messages`.
//!
//! A note is gone by the next keystroke, which is right for "yanked 3 rows" and wrong for
//! a failure someone needs to read twice or paste into a ticket. Everything the note line
//! has said is kept here, and a failure keeps its code and the call it came from.

use super::*;

/// How many notes are kept. A session that has said more than this has scrolled past the
/// start long ago, and the log is for the last few minutes.
const KEPT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub at_ms: i64,
    pub level: Note,
    pub text: String,
    /// The failure behind the note, when there was one.
    pub fault: Option<Fault>,
}

impl App {
    /// Put a failure on the note line, and remember what it was.
    pub(super) fn fail(&mut self, fault: Fault, level: Note) {
        self.fail_as(fault.to_string(), fault, level);
    }

    /// The same, with wording of the caller's choosing on the note line.
    pub(super) fn fail_as(&mut self, text: String, fault: Fault, level: Note) {
        self.note = Some((text, level));
        self.note_fault = Some(fault);
    }

    /// Record the note this message set. Called once per message, by [`App::handle`].
    pub(super) fn log_note(&mut self) {
        let fault = self.note_fault.take();
        let Some((text, level)) = self.note.clone() else {
            return;
        };
        if self.messages.len() == KEPT {
            self.messages.pop_front();
        }
        self.messages.push_back(Logged {
            at_ms: now_ms(),
            level,
            text,
            fault,
        });
    }

    pub(super) fn overlay_open(&self) -> bool {
        self.show_help || self.show_messages
    }
}
