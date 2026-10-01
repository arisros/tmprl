//! Pasted text: which input takes it, and what happens when none is open.

use super::*;

impl App {
    /// A bracketed paste, which the terminal delivers whole rather than as keys.
    ///
    /// It goes to whichever input owns the keyboard, asked in the order `on_key` asks, and
    /// lands at that input's cursor as one edit. Each input is a single line, so the line
    /// editor flattens the newlines: a paste can fill an input and can never submit one.
    pub(super) fn on_paste(&mut self, pasted: &str) {
        // Refused outright, even where the word typed would be wrong. What stands between
        // a keypress and a deleted history is that the word is typed by hand, and a
        // clipboard holding `delete` would make that one keystroke again.
        if self.confirm.is_some() {
            self.note = Some((
                "paste ignored, a confirmation has to be typed".into(),
                Note::Warn,
            ));
            return;
        }
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.buf.paste(pasted);
            return;
        }
        if let Some(picker) = self.picker.as_mut() {
            if picker.paste(pasted) {
                self.schedule_picker_search();
            }
            return;
        }
        if let Some(form) = self.form.as_mut() {
            form.paste(pasted);
            return;
        }
        if self.mode == Mode::Insert {
            // A `j` held in case a `k` follows was typed before the paste, so it goes in
            // before it.
            let held = std::mem::take(&mut self.pending.chords);
            self.which_key.clear();
            self.insert_keys(held);
            if self.insert_buf.paste(pasted) {
                self.recompute_completion();
            }
            return;
        }
        // Normal or Visual. Without bracketed paste every character here would have run as
        // a command, which is the reason the mode is asked for at all.
        self.note = Some(("paste ignored, no input is open".into(), Note::Warn));
    }
}
