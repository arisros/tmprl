//! The history screen: folds, follow mode, the payload pane's scroll, `]f` / `[f`.

use super::*;

impl App {
    /// The group the cursor is on, whether it sits on the group's own line or on one of its
    /// events. Folding from inside an expanded group is what a reader expects.
    pub(super) fn group_under_cursor(&self) -> Option<usize> {
        match self.view.history.value()?.row_at(self.view.cursor)? {
            Row::Group { group, .. } | Row::Event { group, .. } => Some(group),
        }
    }

    pub(super) fn toggle_fold(&mut self) {
        let Some(group) = self.group_under_cursor() else {
            return;
        };
        // Folding shut from inside a group would otherwise strand the cursor past the end;
        // `toggle` hands back where the group's own line is now, so it moves there.
        if let Some(outline) = self.view.history.value_mut()
            && let Some(row) = outline.toggle(group)
        {
            self.view.cursor = row;
        }
        self.clamp_cursor();
    }

    /// Apply a shape change, then put the cursor back on the group it was on. Expanding or
    /// collapsing everything moves every row, so an unadjusted cursor lands somewhere
    /// arbitrary.
    pub(super) fn with_outline(&mut self, f: impl FnOnce(&mut Outline)) {
        let was = self.group_under_cursor();
        let Some(outline) = self.view.history.value_mut() else {
            return;
        };
        f(outline);
        self.view.cursor = was
            .and_then(|g| outline.row_of_group(g))
            .unwrap_or(self.view.cursor);
        self.clamp_cursor();
    }

    pub(super) fn scroll_detail(&mut self, delta: isize) {
        let next = (self.view.detail_scroll as isize + delta)
            .clamp(0, self.view.detail_max_scroll as isize);
        self.view.detail_scroll = next as usize;
    }

    pub(super) fn toggle_follow(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("follow applies to a workflow history".into(), Note::Warn));
            return;
        }
        if self.view.following {
            self.stop_following();
            self.note = Some(("follow stopped".into(), Note::Info));
            return;
        }
        // Following a workflow that has already finished would poll forever for events that
        // can never arrive, so say so instead.
        if self.view.history_token.is_empty() && self.workflow_is_closed() {
            self.note = Some((
                "this workflow has closed, nothing to follow".into(),
                Note::Warn,
            ));
            return;
        }
        self.start_following();
    }

    /// Whether the run itself has ended, as opposed to merely being caught up.
    pub(super) fn workflow_is_closed(&self) -> bool {
        self.view
            .history
            .value()
            .map(|o| tmprl_core::outline::summarize(o.groups()))
            .is_some_and(|s| s.outcome != tmprl_core::history::Outcome::Pending)
    }

    /// Spawn the long-poll loop.
    ///
    /// The loop lives entirely in the task: the reducer never awaits it, and every batch of
    /// events comes back as an ordinary `Msg`. That is the whole reason a sixty-second long
    /// poll cannot freeze a keystroke.
    pub(super) fn start_following(&mut self) {
        let Some(row) = self.view.viewing.clone() else {
            return;
        };
        self.view.following = true;
        self.note = Some(("following, F to stop".into(), Note::Info));

        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.start_pending_poll();
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        let mut token = self.view.history_resume.clone();

        self.view.follow_task = Some(tokio::spawn(async move {
            loop {
                let result = conn
                    .follow_history(&row.namespace, &row.workflow_id, &row.run_id, token.clone())
                    .await;
                match result {
                    Ok(page) => {
                        let done = page.next_page_token.is_empty();
                        token = page.next_page_token.clone();
                        if tx
                            .send(Msg::History {
                                generation,
                                result: Ok((page.events, page.next_page_token)),
                            })
                            .is_err()
                        {
                            return; // the application is gone
                        }
                        if done {
                            return; // the workflow closed
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::History {
                            generation,
                            result: Err(e.to_string()),
                        });
                        return;
                    }
                }
            }
        }));
    }

    /// Stop tailing. Aborting matters: the task is parked inside a long poll and would
    /// otherwise keep a request open and keep pushing events into a screen that has moved on.
    pub(super) fn stop_following(&mut self) {
        self.view.stop_following();
    }

    pub(super) fn jump_failure(&mut self, forward: bool) {
        let Some(outline) = self.view.history.value() else {
            return;
        };
        let found = if forward {
            outline.next_failure(self.view.cursor)
        } else {
            outline.prev_failure(self.view.cursor)
        };
        match found {
            Some(row) => self.view.cursor = row,
            None => {
                self.note = Some((
                    if forward {
                        "no failure below".into()
                    } else {
                        "no failure above".into()
                    },
                    Note::Warn,
                ));
            }
        }
    }
}
