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

    /// Fetch what this cluster lets you filter on, unless it is already in hand.
    ///
    /// Once per namespace, alongside the workflow list rather than when the filter picker
    /// opens: the picker snapshots its items, so an attribute that arrived after it opened
    /// would not appear until it was opened again. `R` is the retry, which is what clears
    /// the namespace recorded here.
    pub fn load_search_attributes(&mut self) {
        let namespace = self.namespace().to_string();
        // A failure is remembered, not retried on every query change: on a cluster whose
        // operator service is closed off this would otherwise fire on every keystroke-
        // committed filter.
        if self.attributes_for.as_deref() == Some(namespace.as_str()) {
            return;
        }
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.attributes_for = Some(namespace.clone());
        self.search_attributes.begin_refresh();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = conn
                .list_search_attributes(&namespace)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::SearchAttributes { namespace, result });
        });
    }

    /// `R`: ask the cluster again next time the list loads.
    pub(super) fn forget_search_attributes(&mut self) {
        self.attributes_for = None;
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
            self.load_search_attributes();
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
        self.load_history_page(HISTORY_PAGE_SIZE);
    }

    pub(super) fn load_history_page(&mut self, page_size: i32) {
        let Some(row) = self.view.viewing.clone() else {
            return;
        };
        let first_page = self.view.history_events.is_empty();
        if first_page {
            self.view.generation = self.view.generation.wrapping_add(1);
            self.view.history.begin_refresh();
            // Activity ids repeat across runs ("1", "2", …), so another run's list would
            // attach itself to this one's rows.
            self.view.pending.clear();
        }
        self.view.loading_more = true;
        if first_page && row.status.is_running() {
            self.load_pending();
        }

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
                    page_size,
                    token,
                )
                .await
                .map(|p| (p.events, p.next_page_token))
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::History { generation, result });
        });
    }

    /// Describe the run on screen once, for its pending activities.
    ///
    /// A retry in progress writes no history event, so the attempt it is on, the failure
    /// that caused it and the time of the next try are only in this call's reply. See
    /// `tmprl_core::pending`.
    fn load_pending(&mut self) {
        let (Some(row), Some(conn)) = (self.view.viewing.clone(), self.conn.clone()) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn
                .pending_activities(&row.namespace, &row.workflow_id, &row.run_id)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Pending { generation, result });
        });
    }

    /// Re-describe on a timer for as long as a follow runs, because the history long poll
    /// has nothing to wake on while an activity retries.
    ///
    /// Stops at the first failure, so a describe the server refuses warns once rather than
    /// every few seconds.
    pub(super) fn start_pending_poll(&mut self) {
        let (Some(row), Some(conn)) = (self.view.viewing.clone(), self.conn.clone()) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        self.view.pending_task = Some(tokio::spawn(async move {
            let mut every = tokio::time::interval(PENDING_POLL);
            loop {
                every.tick().await;
                let result = conn
                    .pending_activities(&row.namespace, &row.workflow_id, &row.run_id)
                    .await
                    .map_err(|e| e.to_string());
                let failed = result.is_err();
                if tx.send(Msg::Pending { generation, result }).is_err() || failed {
                    return;
                }
            }
        }));
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
