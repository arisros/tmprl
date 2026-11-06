//! The note line's history, `:messages`.

use super::*;

const KEPT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub at_ms: i64,
    pub level: Note,
    pub text: String,
    pub fault: Option<Fault>,
}

impl App {
    pub(super) fn fail(&mut self, fault: Fault, level: Note) {
        self.fail_as(fault.to_string(), fault, level);
    }

    pub(super) fn fail_as(&mut self, text: String, fault: Fault, level: Note) {
        self.note = Some((text, level));
        self.note_fault = Some(fault);
    }

    pub(super) fn log_note(&mut self) {
        let fault = self.note_fault.take();
        let Some((text, level)) = self.note.clone() else {
            return;
        };
        match (level, &fault) {
            (Note::Error, Some(f)) => tracing::error!(fault = %f, "{text}"),
            (Note::Error, None) => tracing::error!("{text}"),
            (Note::Warn, Some(f)) => tracing::warn!(fault = %f, "{text}"),
            (Note::Warn, None) => tracing::warn!("{text}"),
            (Note::Info, _) => tracing::info!("{text}"),
        }
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
