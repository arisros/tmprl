//! The `:` command line and the `!` pipe prompt.

use super::*;

impl App {
    pub(super) fn prompt_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let kind = prompt.kind;
        match chord.key {
            Key::Esc => {
                self.awaiting_reason.clear();
                self.close_prompt();
            }
            Key::Enter => {
                let entered = prompt.buf.as_str().trim().to_string();
                self.close_prompt();
                if entered.is_empty() {
                    self.awaiting_reason.clear();
                    return;
                }
                self.histories
                    .entry(History::Prompt(kind))
                    .or_default()
                    .push(&entered);
                match kind {
                    PromptKind::Command => self.run_typed_command(&entered),
                    PromptKind::Pipe => self.run_pipe(entered),
                    PromptKind::Search => self.run_search(entered),
                    PromptKind::Signal | PromptKind::Update => self.confirm_named(kind, entered),
                    PromptKind::Backfill => self.confirm_backfill(entered),
                    PromptKind::Query => self.run_query(entered),
                    PromptKind::Reason => self.confirm_with_reason(entered),
                }
            }
            Key::Up | Key::Down => {
                let history = self.histories.entry(History::Prompt(kind)).or_default();
                let recalled = if chord.key == Key::Up {
                    history.older(prompt.buf.as_str())
                } else {
                    history.newer()
                };
                if let Some(text) = recalled {
                    prompt.buf.set(text);
                }
            }
            Key::Tab | Key::BackTab if kind == PromptKind::Command => {
                self.cycle_completion(chord.key == Key::Tab)
            }
            _ => match prompt.buf.key(chord) {
                Edit::Emptied => {
                    self.awaiting_reason.clear();
                    self.close_prompt();
                }
                Edit::Changed => self.cmdline_cycle = None,
                Edit::Moved | Edit::Unhandled => {}
            },
        }
    }

    pub(super) fn paste(&mut self, text: String) {
        if self.picker.is_some() {
            let line = text.lines().next().unwrap_or_default().trim().to_string();
            self.picker_paste(&line);
        } else if let Some(prompt) = self.prompt.as_mut() {
            prompt.buf.paste(&text);
            self.cmdline_cycle = None;
        } else if self.mode == Mode::Insert {
            self.insert_buf.paste(&text);
            self.recompute_completion();
        } else if self.mode == Mode::Normal {
            let id = text
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            if id.is_empty() {
                return;
            }
            self.run("find.workflow", None);
            if self.picker.is_some() {
                self.picker_paste(&id);
            }
        }
    }

    fn cycle_completion(&mut self, forward: bool) {
        let ids: Vec<&'static str> = self.cmdline_matches().iter().map(|c| c.id).collect();
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let (typed, next) = match self.cmdline_cycle.take() {
            None => (
                prompt.buf.as_str().to_string(),
                if forward { 0 } else { ids.len() - 1 },
            ),
            Some((typed, i)) if forward => (typed, (i + 1) % ids.len()),
            Some((typed, i)) => (typed, (i + ids.len() - 1) % ids.len()),
        };
        prompt.buf.set(ids[next]);
        self.cmdline_cycle = Some((typed, next));
    }

    pub fn cmdline_pick(&self) -> Option<usize> {
        self.cmdline_cycle.as_ref().map(|(_, i)| *i)
    }

    pub(super) fn close_prompt(&mut self) {
        if let Some(prompt) = &self.prompt {
            self.histories
                .entry(History::Prompt(prompt.kind))
                .or_default()
                .reset();
        }
        self.prompt = None;
        self.cmdline_cycle = None;
        self.mode = Mode::Normal;
    }

    pub(super) fn run_typed_command(&mut self, entered: &str) {
        if matches!(entered, "q" | "q!" | "qa" | "qa!" | "quit") {
            self.run("app.quit", None);
            return;
        }
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

    pub fn cmdline_matches(&self) -> Vec<&tmprl_core::Command> {
        match &self.prompt {
            Some(p) if p.kind == PromptKind::Command => {
                let typed = self
                    .cmdline_cycle
                    .as_ref()
                    .map_or(p.buf.as_str(), |(typed, _)| typed.as_str());
                self.registry.search(typed).into_iter().take(8).collect()
            }
            _ => Vec::new(),
        }
    }
}
