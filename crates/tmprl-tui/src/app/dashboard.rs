//! The dashboard: opening it, the requests behind its panels, and what an item opens.

use super::*;

impl App {
    /// `gd`. From the namespace list it takes the namespaces under the cursor as its scope,
    /// the way `<CR>` does, so the dashboard is one key from the opening screen.
    pub(super) fn open_dashboard(&mut self) {
        match self.view.screen {
            Screen::Dashboard => return,
            Screen::History => {
                self.note = Some(("go up with `-` first".into(), Note::Warn));
                return;
            }
            Screen::Namespaces => {
                let scope = self.selected_namespaces();
                if scope.is_empty() {
                    self.note = Some(("nothing to open".into(), Note::Warn));
                    return;
                }
                self.mark_jump();
                self.view.namespace_cursor = self.view.cursor;
                self.view.scope = scope;
            }
            Screen::Workflows | Screen::Schedules => self.mark_jump(),
        }
        self.view.stop_following();
        self.view.anchor = None;
        self.mode = Mode::Normal;
        self.view.screen = Screen::Dashboard;
        self.view.cursor = 0;
        self.load_dashboard();
    }

    /// Ask for everything the panels read from. What is already on screen stays there
    /// until its replacement arrives.
    pub fn load_dashboard(&mut self) {
        self.view.generation = self.view.generation.wrapping_add(1);
        self.view.stop_dashboard();
        let mut board = match self.view.dashboard.take() {
            Some(board) if board.scope() == self.view.scope => board,
            _ => match &self.dashboard_layout {
                Some(layout) => Board::new(layout.clone(), &self.view.scope),
                None => Board::adaptive(&self.view.scope),
            },
        };
        board.begin_refresh();
        let sources = board.sources().to_vec();
        self.view.dashboard = Some(board);

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (view, generation, now) = (
            self.tabs.current().focused(),
            self.view.generation,
            now_ms(),
        );
        for (source, wanted) in sources.into_iter().enumerate() {
            let (conn, tx) = (conn.clone(), self.tx.clone());
            self.view.dashboard_tasks.push(tokio::spawn(async move {
                let result = fetch(&conn, &wanted, now).await;
                let _ = tx.send(Msg::Dashboard {
                    view,
                    generation,
                    source,
                    result,
                });
            }));
        }
    }

    pub(super) fn dashboard_reply(
        &mut self,
        id: ViewId,
        generation: u64,
        source: usize,
        result: Result<SourceData, Fault>,
    ) {
        let focused = id == self.tabs.current().focused();
        let Some(view) = self.pane_mut(id) else {
            return;
        };
        if view.generation != generation {
            return;
        }
        let Some(board) = view.dashboard.as_mut() else {
            return;
        };
        let fault = result.as_ref().err().cloned();
        // A cursor still at the top has not been put anywhere, and should not be carried
        // down the screen by panels filling in above the item it happened to be on.
        let anchor = (view.cursor > 0)
            .then(|| board.anchor(view.cursor))
            .flatten();
        board.apply(source, result);
        if view.screen == Screen::Dashboard
            && let Some(anchor) = anchor
        {
            view.cursor = board.reanchor(&anchor);
        }
        if focused {
            self.clamp_cursor();
            if let Some(fault) = fault {
                self.fail(fault, Note::Error);
            }
        }
    }

    pub(super) fn step_panel(&mut self, forward: bool) {
        let Some(board) = self
            .view
            .dashboard
            .as_ref()
            .filter(|_| self.view.screen == Screen::Dashboard)
        else {
            self.note = Some(("panels are on the dashboard, gd".into(), Note::Warn));
            return;
        };
        let at = board.panel_step(self.view.cursor, forward);
        self.set_cursor(at);
    }

    /// `<CR>` on a dashboard item: the workflows, the history or the schedule it stands for.
    pub(super) fn drill(&mut self) {
        let Some(board) = self.view.dashboard.as_ref() else {
            return;
        };
        if board.item(self.view.cursor).is_none() {
            self.note = Some(("nothing to open".into(), Note::Warn));
            return;
        }
        let Some(drill) = board.drill(self.view.cursor, now_ms()) else {
            self.note = Some((
                "that name has a quote in it, which a query cannot hold".into(),
                Note::Warn,
            ));
            return;
        };
        self.view.stop_dashboard();
        match drill {
            Drill::Workflow(row) => self.open_row(row),
            Drill::Query { namespaces, query } => {
                self.mark_jump();
                self.view.scope = namespaces;
                self.view.query = query;
                self.view.screen = Screen::Workflows;
                self.view.cursor = 0;
                self.view.cursor_key = None;
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.note = Some((format!("query: {}", self.view.query), Note::Info));
                self.load_workflows(false);
            }
            Drill::Schedule {
                namespace,
                schedule_id,
            } => {
                self.mark_jump();
                self.view.scope = vec![namespace];
                self.view.seek_schedule = Some(schedule_id);
                self.view.screen = Screen::Schedules;
                self.view.cursor = 0;
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.load_schedules();
            }
        }
    }

    pub(super) fn dashboard_item(&self) -> Option<&Item> {
        self.view.dashboard.as_ref()?.item(self.view.cursor)
    }
}

async fn fetch(conn: &Conn, source: &Source, now_ms: i64) -> Result<SourceData, Fault> {
    let query = source.query(now_ms);
    match source {
        Source::Counts { namespaces, .. } => conn
            .count_workflows_across(namespaces, &query)
            .await
            .map(SourceData::Counts),
        Source::Workflows { namespaces, .. } => conn
            .list_workflows_across(namespaces, &query, PAGE_SIZE)
            .await
            .map(|(rows, tokens)| SourceData::Workflows {
                rows,
                more: !tokens.is_empty(),
            }),
        // One namespace, for the reason `load_schedules` gives.
        Source::Schedules { namespaces } => {
            let namespace = namespaces.first().map_or("", String::as_str);
            conn.list_schedules(namespace, PAGE_SIZE, Vec::new())
                .await
                .map(|page| SourceData::Schedules(page.rows))
        }
    }
}
