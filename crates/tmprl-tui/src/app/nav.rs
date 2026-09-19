//! Moving around: opening what is under the cursor, going back up, the cursor itself,
//! paging in more rows, and the jumplist.

use super::*;

impl App {
    pub(super) fn open_focused(&mut self) {
        match self.view.screen {
            Screen::Namespaces => {
                // A visual selection opens every namespace in it as one merged list. That
                // is the whole multi-namespace fan-out: `V j j <CR>`, using the selection
                // machinery that already exists rather than a separate picker.
                let (lo, hi) = self
                    .selection()
                    .unwrap_or((self.view.cursor, self.view.cursor));
                let scope: Vec<String> = self
                    .namespace_rows()
                    .iter()
                    .skip(lo)
                    .take(hi.saturating_sub(lo) + 1)
                    .map(|n| n.name.clone())
                    .collect();
                if scope.is_empty() {
                    self.note = Some(("nothing to open".into(), Note::Warn));
                    return;
                }

                self.mark_jump();
                self.view.namespace_cursor = self.view.cursor;
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.view.screen = Screen::Workflows;
                self.view.scope = scope;
                self.view.cursor = 0;
                self.view.cursor_key = None;
                self.load_workflows(false);
            }
            Screen::Workflows => {
                let Some(row) = self.workflow_rows().get(self.view.cursor).cloned() else {
                    self.note = Some(("nothing to open".into(), Note::Warn));
                    return;
                };
                self.mark_jump();
                self.view.workflow_cursor = self.view.cursor;
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.view.screen = Screen::History;
                self.view.viewing = Some(row);
                self.view.cursor = 0;
                self.load_history();
            }
            // On the history screen, "open the focused item" is folding a group open.
            Screen::History => self.toggle_fold(),
            // A schedule's runs are ordinary workflows, so there is nothing of its own to
            // open. Saying so beats a key that appears broken.
            Screen::Schedules => {
                self.note = Some((
                    "a schedule has no detail view; gw for its workflows".into(),
                    Note::Warn,
                ));
            }
        }
    }

    pub(super) fn go_up(&mut self) {
        // Recorded per arm, not once at the top: `Jumplist::push` truncates the forward
        // entries, so marking a jump that then turns out to be refused would silently
        // destroy the `<C-i>` list for a keystroke that did nothing.
        match self.view.screen {
            Screen::History => {
                self.mark_jump();
                self.view.screen = Screen::Workflows;
                self.view.cursor = self.view.workflow_cursor;
                self.view.viewing = None;
                self.reset_history();
                self.restore_cursor();
            }
            Screen::Workflows => {
                self.mark_jump();
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.view.anchor = None;
                self.clamp_cursor();
            }
            Screen::Schedules => {
                self.mark_jump();
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.view.anchor = None;
                self.clamp_cursor();
            }
            Screen::Namespaces => {
                self.note = Some(("already at the top level".into(), Note::Warn));
            }
        }
    }

    /// Point this pane at one namespace and show its workflows.
    ///
    /// The query is kept. Switching namespace is usually "the same question, over there",
    /// and having to retype the filter every time would make the picker cost more than the
    /// navigation it replaces. Any fan-out collapses to the one namespace chosen.
    pub(super) fn switch_namespace(&mut self, name: &str) {
        self.mark_jump();
        self.stop_following();
        self.view.scope = vec![name.to_string()];
        self.view.screen = Screen::Workflows;
        self.view.viewing = None;
        self.view.history = Loadable::NotAsked;
        self.view.history_events.clear();
        self.view.history_token.clear();
        self.view.history_resume.clear();
        self.view.cursor = 0;
        self.view.cursor_key = None;
        self.view.anchor = None;
        self.mode = Mode::Normal;
        self.note = Some((format!("namespace: {name}"), Note::Info));
        self.load_workflows(false);
    }

    /// Jump straight to a workflow's history from the picker.
    ///
    /// The same landing as `Enter` on its row, reached without first putting the cursor
    /// there, which is the entire reason a picker beats scrolling.
    pub(super) fn open_workflow(&mut self, namespace: &str, run_id: &str) {
        let Some(row) = self
            .workflow_rows()
            .iter()
            .find(|w| w.namespace == namespace && w.run_id == run_id)
            .cloned()
        else {
            self.note = Some(("that workflow is no longer in the list".into(), Note::Warn));
            return;
        };
        self.mark_jump();
        self.view.workflow_cursor = self.view.cursor;
        self.view.anchor = None;
        self.mode = Mode::Normal;
        self.view.screen = Screen::History;
        self.view.viewing = Some(row);
        self.view.cursor = 0;
        // Everything the *previous* history left behind has to go, because unlike `Enter`
        // on the list this can be pressed while already inside a history. Left alone,
        // `load_history` sees non-empty events and neither bumps the generation nor starts
        // a refresh, sends the old run's continuation token to the new run, and merges
        // whatever comes back into the old run's events, so one outline shows two
        // workflows. A follow poll left running would keep feeding it too.
        self.reset_history();
        self.load_history();
    }

    /// Drop everything belonging to the history currently on screen.
    ///
    /// Every path that changes which workflow is being looked at needs this, and the ones
    /// that open-code it have drifted apart before, so it lives in one place.
    pub(super) fn reset_history(&mut self) {
        self.stop_following();
        self.view.history = Loadable::NotAsked;
        self.view.history_events.clear();
        self.view.history_token.clear();
        self.view.history_resume.clear();
    }

    /// Where this pane is now, for the jumplist.
    pub(super) fn here(&self) -> Jump {
        Jump {
            screen: self.view.screen,
            scope: self.view.scope.clone(),
            query: self.view.query.clone(),
            viewing: self.view.viewing.clone(),
            cursor: self.view.cursor,
            cursor_key: self.view.cursor_key.clone(),
        }
    }

    /// Record the current position before a navigation that counts as a jump.
    ///
    /// Called by the handful of moves vim would also call jumps: changing screen, `gg` and
    /// `G`, a search, taking something from a picker. Ordinary `j` and `k` are not jumps,
    /// which is the entire point, a jumplist that recorded every line would be a scroll
    /// history and `<C-o>` would be useless.
    pub(super) fn mark_jump(&mut self) {
        let here = self.here();
        self.jumps.push(here);
    }

    /// `<C-o>` and `<C-i>`.
    pub(super) fn jump(&mut self, back: bool) {
        let here = self.here();
        let target = if back {
            self.jumps.back(here).cloned()
        } else {
            self.jumps.forward().cloned()
        };
        let Some(target) = target else {
            self.note = Some((
                if back {
                    "no earlier position".into()
                } else {
                    "no later position".into()
                },
                Note::Warn,
            ));
            return;
        };
        self.go_to_jump(target);
    }

    /// Put the pane back where a jump says it was.
    ///
    /// A screen change re-fetches rather than restoring a cached list: what is there now is
    /// what should be shown, and a jump that reinstated a ten-minute-old workflow table
    /// would be showing history as if it were current.
    pub(super) fn go_to_jump(&mut self, to: Jump) {
        self.stop_following();
        self.mode = Mode::Normal;
        self.view.anchor = None;
        self.view.scope = to.scope;
        self.view.query = to.query;
        self.view.screen = to.screen;
        self.view.viewing = to.viewing;
        self.view.cursor = to.cursor;
        // Carried, not cleared: `restore_cursor` uses it to find the workflow again
        // wherever the refetched list has put it, and falls back to the index when the row
        // has genuinely gone.
        self.view.cursor_key = to.cursor_key;

        match to.screen {
            Screen::Namespaces => self.clamp_cursor(),
            Screen::Workflows => self.load_workflows(false),
            Screen::Schedules => self.load_schedules(),
            Screen::History => {
                self.view.history = Loadable::NotAsked;
                self.view.history_events.clear();
                self.view.history_token.clear();
                self.view.history_resume.clear();
                self.load_history();
            }
        }
    }

    pub(super) fn refresh(&mut self) {
        // `R` is the retry: a codec that was down, or a key that was wrong, is usually
        // fixed outside tmprl, and nothing else would clear the recorded failures.
        self.decode_failed.clear();
        match self.view.screen {
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => {
                self.view.history_events.clear();
                self.view.history_token.clear();
                self.view.history_resume.clear();
                self.load_history();
            }
            Screen::Schedules => self.load_schedules(),
        }
    }

    pub(super) fn scroll_help(&mut self, delta: isize) {
        let next = (self.help_scroll as isize + delta).clamp(0, self.help_max_scroll as isize);
        self.help_scroll = next as usize;
    }

    pub(super) fn move_cursor(&mut self, delta: isize) {
        let len = self.row_count();
        if len == 0 {
            self.view.cursor = 0;
            return;
        }
        let next = (self.view.cursor as isize + delta).clamp(0, len as isize - 1);
        self.set_cursor(next as usize);
    }

    pub(super) fn set_cursor(&mut self, at: usize) {
        if at != self.view.cursor {
            // The pane now shows a different row, so its offset and any filter result
            // from the previous one no longer apply.
            self.view.detail_scroll = 0;
            self.view.piped = None;
        }
        self.view.cursor = at;
        self.maybe_decode();
        self.remember_cursor();
        self.maybe_load_more();
    }

    /// Record which row the cursor is on, by identity. This is what a refresh restores.
    pub(super) fn remember_cursor(&mut self) {
        if self.view.screen == Screen::Workflows {
            self.view.cursor_key = self
                .workflow_rows()
                .get(self.view.cursor)
                .map(|r| (r.namespace.clone(), r.run_id.clone()));
        }
    }

    /// Put the cursor back on the row it was on, wherever that row has moved to.
    pub(super) fn restore_cursor(&mut self) {
        let Some((ns, run)) = self.view.cursor_key.clone() else {
            self.clamp_cursor();
            return;
        };
        if let Some(list) = self.view.workflows.value()
            && let Some(at) = list.position_of((&ns, &run))
        {
            self.view.cursor = at;
        }
        self.clamp_cursor();
    }

    pub(super) fn clamp_cursor(&mut self) {
        let len = self.row_count();
        self.view.cursor = self.view.cursor.min(len.saturating_sub(1));
        if len == 0 {
            self.view.cursor = 0;
        }
    }

    /// Infinite scroll: fetch the next page once the cursor is within a screen of the end.
    pub(super) fn maybe_load_more(&mut self) {
        if self.view.loading_more {
            return;
        }
        let len = self.row_count();
        let near_end = self.view.cursor + self.view.page.max(1) >= len;
        if !near_end {
            return;
        }
        match self.view.screen {
            Screen::Workflows => {
                if self
                    .view
                    .workflows
                    .value()
                    .is_some_and(WorkflowList::has_more)
                {
                    self.load_more();
                }
            }
            Screen::History => {
                if !self.view.history_token.is_empty() {
                    self.load_history();
                }
            }
            Screen::Namespaces | Screen::Schedules => {}
        }
    }

    /// Load whatever the focused pane's screen needs. A fresh pane has asked for nothing.
    pub(super) fn load_for_screen(&mut self) {
        match self.view.screen {
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => self.load_history(),
            Screen::Schedules => self.load_schedules(),
        }
    }

    /// Switch between the two lists a namespace holds.
    ///
    /// Only from a list, and only within the same scope. From a history the reader is inside
    /// one workflow, and jumping sideways from there would lose their place with no way back.
    pub(super) fn go_to(&mut self, screen: Screen) {
        match self.view.screen {
            Screen::Namespaces => {
                self.note = Some(("open a namespace first".into(), Note::Warn));
                return;
            }
            Screen::History => {
                self.note = Some(("go up with `-` first".into(), Note::Warn));
                return;
            }
            _ if self.view.screen == screen => return,
            _ => {}
        }
        self.view.stop_following();
        self.view.screen = screen;
        self.view.cursor = 0;
        self.view.anchor = None;
        self.load_for_screen();
    }
}
