//! Moving around: opening what is under the cursor, going back up, the cursor itself,

use super::*;

impl App {
    pub(super) fn open_focused(&mut self) {
        match self.view.screen {
            Screen::Profiles => self.open_focused_profile(),
            Screen::Namespaces => {
                let scope = self.selected_namespaces();
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
            Screen::History => self.toggle_fold(),
            Screen::Schedules => {
                self.note = Some((
                    "a schedule has no detail view; gw for its workflows".into(),
                    Note::Warn,
                ));
            }
            Screen::Dashboard => self.drill(),
        }
    }

    pub(super) fn selected_namespaces(&self) -> Vec<String> {
        let (lo, hi) = self
            .selection()
            .unwrap_or((self.view.cursor, self.view.cursor));
        self.namespace_rows()
            .iter()
            .skip(lo)
            .take(hi.saturating_sub(lo) + 1)
            .map(|n| n.name.clone())
            .collect()
    }

    pub(super) fn go_up(&mut self) {
        // Marked per arm: Jumplist::push drops the forward entries, so a refused move must not mark.
        match self.view.screen {
            Screen::History => {
                self.mark_jump();
                self.view.screen = Screen::Workflows;
                self.view.cursor = self.view.workflow_cursor;
                self.view.viewing = None;
                self.reset_history();
                self.restore_cursor();
                if self.view.workflows.value().is_none() {
                    self.load_workflows(false);
                }
            }
            Screen::Workflows => {
                self.mark_jump();
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.view.anchor = None;
                self.clamp_cursor();
            }
            Screen::Schedules | Screen::Dashboard => {
                self.mark_jump();
                self.view.stop_dashboard();
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.view.anchor = None;
                self.clamp_cursor();
            }
            Screen::Namespaces => self.open_profiles(),
            Screen::Profiles => {
                self.note = Some(("already at the top level".into(), Note::Warn));
            }
        }
    }

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

    pub(super) fn open_workflow(&mut self, namespace: &str, run_id: &str) {
        let matches = |w: &&WorkflowRow| w.namespace == namespace && w.run_id == run_id;
        let Some(row) = self
            .workflow_rows()
            .iter()
            .find(matches)
            .or_else(|| self.picker_found.iter().find(matches))
            .cloned()
        else {
            self.note = Some(("that workflow is no longer in the list".into(), Note::Warn));
            return;
        };
        self.open_row(row);
    }

    pub(super) fn open_row(&mut self, row: WorkflowRow) {
        self.mark_jump();
        self.view.workflow_cursor = self.view.cursor;
        self.view.anchor = None;
        self.mode = Mode::Normal;
        self.view.screen = Screen::History;
        self.view.viewing = Some(row);
        self.view.cursor = 0;
        self.reset_history();
        self.load_history();
    }

    pub(super) fn reset_history(&mut self) {
        self.stop_following();
        self.scan = None;
        self.view.history = Loadable::NotAsked;
        self.view.history_events.clear();
        self.view.history_token.clear();
        self.view.history_resume.clear();
        self.view.handlers = None;
        self.view.queried = None;
    }

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

    pub(super) fn mark_jump(&mut self) {
        let here = self.here();
        self.jumps.push(here);
    }

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

    pub(super) fn go_to_jump(&mut self, to: Jump) {
        self.stop_following();
        self.mode = Mode::Normal;
        self.view.anchor = None;
        self.view.scope = to.scope;
        self.view.query = to.query;
        self.view.screen = to.screen;
        self.view.viewing = to.viewing;
        self.view.cursor = to.cursor;
        self.view.cursor_key = to.cursor_key;

        match to.screen {
            Screen::Profiles | Screen::Namespaces => self.clamp_cursor(),
            Screen::Workflows => self.load_workflows(false),
            Screen::Schedules => self.load_schedules(),
            Screen::Dashboard => self.load_dashboard(),
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
        self.decode_failed.clear();
        self.forget_search_attributes();
        match self.view.screen {
            Screen::Profiles => self.load_profiles(),
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => {
                self.view.history_events.clear();
                self.view.history_token.clear();
                self.view.history_resume.clear();
                self.load_history();
            }
            Screen::Schedules => self.load_schedules(),
            Screen::Dashboard => {
                if let Some(board) = self.view.dashboard.as_mut() {
                    board.forget();
                }
                self.load_dashboard();
            }
        }
    }

    pub(super) fn scroll_overlay(&mut self, delta: isize) {
        let at = self.overlay_scroll.min(self.overlay_max_scroll);
        let next = (at as isize + delta).clamp(0, self.overlay_max_scroll as isize);
        self.overlay_scroll = next as usize;
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
            self.view.detail_scroll = 0;
            self.view.piped = None;
            self.view.queried = None;
        }
        self.view.cursor = at;
        self.maybe_decode();
        self.remember_cursor();
        self.maybe_load_more();
    }

    pub(super) fn remember_cursor(&mut self) {
        if self.view.screen == Screen::Workflows {
            self.view.cursor_key = self
                .workflow_rows()
                .get(self.view.cursor)
                .map(|r| (r.namespace.clone(), r.run_id.clone()));
        }
    }

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
            Screen::Profiles | Screen::Namespaces | Screen::Schedules | Screen::Dashboard => {}
        }
    }

    pub(super) fn load_for_screen(&mut self) {
        match self.view.screen {
            Screen::Profiles => self.load_profiles(),
            Screen::Namespaces => self.load_namespaces(),
            Screen::Workflows => self.load_workflows(false),
            Screen::History => self.load_history(),
            Screen::Schedules => self.load_schedules(),
            Screen::Dashboard => self.load_dashboard(),
        }
    }

    pub(super) fn go_to(&mut self, screen: Screen) {
        match self.view.screen {
            Screen::Profiles | Screen::Namespaces => {
                self.note = Some(("open a namespace first".into(), Note::Warn));
                return;
            }
            Screen::History => {
                self.note = Some(("go up with `-` first".into(), Note::Warn));
                return;
            }
            Screen::Workflows | Screen::Schedules | Screen::Dashboard => {
                if self.view.screen == screen {
                    return;
                }
            }
        }
        self.view.stop_following();
        self.view.screen = screen;
        self.view.cursor = 0;
        self.view.anchor = None;
        self.load_for_screen();
    }
}

impl App {
    pub(super) fn open_child(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("a child is opened from a history".into(), Note::Warn));
            return;
        }
        let Some(outline) = self.view.history.value() else {
            return;
        };
        let group = match outline.row_at(self.view.cursor) {
            Some(Row::Group { group, .. } | Row::Event { group, .. }) => outline.group(group),
            None => None,
        };
        let Some(g) = group.filter(|g| g.category == tmprl_core::history::Category::ChildWorkflow)
        else {
            self.note = Some(("not a child workflow".into(), Note::Warn));
            return;
        };
        let events = outline.events();
        let Some(workflow_id) = g.field(events, "workflowId").map(str::to_string) else {
            self.note = Some(("this child has no workflow id yet".into(), Note::Warn));
            return;
        };
        let run_id = g.field(events, "runId").map(str::to_string);
        let namespace = g.field(events, "namespace").map(str::to_string);
        self.find_related(namespace, workflow_id, run_id);
    }

    pub(super) fn open_parent(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("a parent is opened from a history".into(), Note::Warn));
            return;
        }
        let field = |name: &str| {
            self.view.history_events.iter().find_map(|e| {
                e.fields
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v.clone())
            })
        };
        let Some(workflow_id) = field("parentWorkflowId") else {
            self.note = Some(("this workflow has no parent".into(), Note::Warn));
            return;
        };
        let (run_id, namespace) = (field("parentRunId"), field("parentNamespace"));
        self.find_related(namespace, workflow_id, run_id);
    }

    fn find_related(
        &mut self,
        namespace: Option<String>,
        workflow_id: String,
        run_id: Option<String>,
    ) {
        let Some(viewing) = self.view.viewing.clone() else {
            return;
        };
        let Some(query) = tmprl_core::query::by_execution(&workflow_id, run_id.as_deref()) else {
            self.note = Some((format!("cannot look up {workflow_id}"), Note::Warn));
            return;
        };
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let namespace = namespace.unwrap_or_else(|| viewing.namespace.clone());
        self.note = Some((format!("opening {workflow_id}…"), Note::Info));
        let from = (viewing.namespace, viewing.run_id);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = conn
                .list_workflows_across(&[namespace], &query, PICKER_SEARCH_LIMIT)
                .await
                .map(|(rows, _)| rows);
            let _ = tx.send(Msg::Related {
                from,
                id: workflow_id,
                result,
            });
        });
    }

    pub(super) fn related_found(
        &mut self,
        from: (String, String),
        id: String,
        result: Result<Vec<WorkflowRow>, Fault>,
    ) {
        let still_here = self.view.screen == Screen::History
            && self.view.viewing.as_ref().is_some_and(|w| {
                (w.namespace.as_str(), w.run_id.as_str()) == (from.0.as_str(), from.1.as_str())
            });
        if !still_here {
            return;
        }
        match result {
            Ok(rows) => match rows.into_iter().max_by_key(|row| row.start_time) {
                Some(row) => self.open_row(row),
                // Visibility lags the history, so a child that just started may not be listed yet.
                None => {
                    self.note = Some((format!("{id} is not listed yet, try again"), Note::Warn))
                }
            },
            Err(e) => self.fail_as(format!("{id}: {e}"), e, Note::Error),
        }
    }
}
