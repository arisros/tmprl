//! Every request to the server. Each spawns a task and returns at once; the reply
//! arrives as a [`Msg`] and is applied by [`App::handle`].

use super::*;

impl App {
    /// Schedules for the first namespace in scope.
    ///
    /// One namespace, not the fan-out: ListSchedules takes a single namespace and a schedule
    /// list merged across several has no ordering that means anything.
    pub fn load_schedules(&mut self) {
        self.view.generation = self.view.generation.wrapping_add(1);
        self.view.schedules.begin_refresh();
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let namespace = self
            .view
            .scope
            .first()
            .cloned()
            .unwrap_or_else(|| self.namespace.clone());
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn
                .list_schedules(&namespace, PAGE_SIZE, Vec::new())
                .await
                .map(|p| p.rows)
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Schedules { generation, result });
        });
    }

    /// Spawn a namespace fetch. Returns immediately; the result arrives as a `Msg`.
    pub fn load_namespaces(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.view.namespaces.begin_refresh();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = conn.list_namespaces().await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::Namespaces(res));
        });
    }

    /// Fetch the first page for the current query, and the header counts alongside it.
    ///
    /// Bumping the generation is what makes an in-flight reply for the previous query
    /// harmless: it arrives, does not match, and is dropped.
    pub fn load_workflows(&mut self, append: bool) {
        if !append {
            self.view.generation = self.view.generation.wrapping_add(1);
            self.view.workflows.begin_refresh();
            self.view.counts.begin_refresh();
            self.load_counts();
        }

        // Set before the connection guard: this records the decision to fetch, which is
        // what stops a second page being queued while the first is still in flight.
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        // On a continuation, ask only the namespaces that still have pages. Passing the
        // whole scope would hand an exhausted namespace an empty token, which the server
        // reads as "start again", so it would never finish.
        let tokens: Tokens = if append {
            self.view
                .workflows
                .value()
                .map(|l| l.tokens().to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let (tx, generation, scope, query) = (
            self.tx.clone(),
            self.view.generation,
            self.view.scope.clone(),
            self.view.query.clone(),
        );
        tokio::spawn(async move {
            let result = if append {
                conn.continue_workflows_across(&tokens, &query, PAGE_SIZE)
                    .await
            } else {
                conn.list_workflows_across(&scope, &query, PAGE_SIZE).await
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Workflows {
                generation,
                append,
                result,
            });
        });
    }

    /// Fetch a page of the focused workflow's history.
    ///
    /// Called for the first page and for each continuation; the accumulated events are
    /// re-grouped whenever one lands, because a page boundary routinely falls inside a
    /// group.
    pub fn load_history(&mut self) {
        let Some(row) = self.view.viewing.clone() else {
            return;
        };
        if self.view.history_events.is_empty() {
            self.view.generation = self.view.generation.wrapping_add(1);
            self.view.history.begin_refresh();
        }
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation, token) = (
            self.tx.clone(),
            self.view.generation,
            self.view.history_token.clone(),
        );
        tokio::spawn(async move {
            let result = conn
                .get_history(
                    &row.namespace,
                    &row.workflow_id,
                    &row.run_id,
                    HISTORY_PAGE_SIZE,
                    token,
                )
                .await
                .map(|p| (p.events, p.next_page_token))
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::History { generation, result });
        });
    }

    pub(super) fn load_more(&mut self) {
        let has_more = self
            .view
            .workflows
            .value()
            .is_some_and(WorkflowList::has_more);
        if has_more {
            self.load_workflows(true);
        }
    }

    pub(super) fn load_counts(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation, scope, query) = (
            self.tx.clone(),
            self.view.generation,
            self.view.scope.clone(),
            self.view.query.clone(),
        );
        tokio::spawn(async move {
            let result = conn
                .count_workflows_across(&scope, &query)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Counts { generation, result });
        });
    }
}
