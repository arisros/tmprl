//! The `:` command line and the `!` pipe prompt.

use super::*;

impl App {
    pub(super) fn prompt_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        match chord.key {
            Key::Esc => self.close_prompt(),
            Key::Enter => {
                let entered = prompt.buf.trim().to_string();
                let kind = prompt.kind;
                self.close_prompt();
                if entered.is_empty() {
                    return;
                }
                match kind {
                    PromptKind::Command => self.run_typed_command(&entered),
                    PromptKind::Pipe => self.run_pipe(entered),
                    PromptKind::Search => self.run_search(entered),
                    PromptKind::Signal | PromptKind::Update => self.confirm_named(kind, entered),
                    PromptKind::Backfill => self.confirm_backfill(entered),
                }
            }
            // Backspace on an empty line closes the prompt, as it does in vim.
            Key::Backspace if prompt.buf.pop().is_none() => self.close_prompt(),
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => prompt.buf.push(c),
            _ => {}
        }
    }

    pub(super) fn close_prompt(&mut self) {
        self.prompt = None;
        self.mode = Mode::Normal;
    }

    /// Resolve what was typed at `:` and run it. Accepts a unique prefix, the way vim
    /// accepts `:q` for `:quit`.
    pub(super) fn run_typed_command(&mut self, entered: &str) {
        let hits = self.registry.search(entered);
        match hits.iter().find(|c| c.id == entered).or(hits.first()) {
            Some(c) if hits.len() == 1 || c.id == entered => {
                let id = c.id;
                self.run(id, None);
            }
            Some(_) => {
                self.note = Some((
                    format!("ambiguous: {} commands match `{entered}`", hits.len()),
                    Note::Warn,
                ));
            }
            None => {
                self.note = Some((format!("no such command: {entered}"), Note::Error));
            }
        }
    }

    /// Completions for whatever is typed at `:`. A `!` prompt takes a shell command, which
    /// tmprl has no business completing.
    pub fn cmdline_matches(&self) -> Vec<&tmprl_core::Command> {
        match &self.prompt {
            Some(p) if p.kind == PromptKind::Command => {
                self.registry.search(&p.buf).into_iter().take(8).collect()
            }
            _ => Vec::new(),
        }
    }
}
