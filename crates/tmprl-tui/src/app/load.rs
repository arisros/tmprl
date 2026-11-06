//! Every request to the server. Each spawns a task and returns at once; the reply

use super::*;

impl App {
    pub fn load_schedules(&mut self) {
        self.view.generation = self.view.generation.wrapping_add(1);
        self.view.schedules.begin_refresh();
        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let namespaces = self.namespaces_in_scope();
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn.list_all_schedules(&namespaces, SCAN_PAGE_SIZE).await;
            let _ = tx.send(Msg::Schedules { generation, result });
        });
    }

    pub(super) fn namespaces_in_scope(&self) -> Vec<String> {
        if self.view.scope.is_empty() {
            vec![self.namespace.clone()]
        } else {
            self.view.scope.clone()
        }
    }

    pub fn load_namespaces(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.view.namespaces.begin_refresh();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = conn.list_namespaces().await;
            let _ = tx.send(Msg::Namespaces(res));
        });
    }

    pub fn load_search_attributes(&mut self) {
        let namespace = self.namespace().to_string();
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
            let result = conn.list_search_attributes(&namespace).await;
            let _ = tx.send(Msg::SearchAttributes { namespace, result });
        });
    }

    pub(super) fn forget_search_attributes(&mut self) {
        self.attributes_for = None;
    }

    pub fn load_workflows(&mut self, append: bool) {
        if !append {
            self.view.generation = self.view.generation.wrapping_add(1);
            self.view.workflows.begin_refresh();
            self.view.counts.begin_refresh();
            self.load_counts();
            self.load_search_attributes();
        }

        self.view.loading_more = true;

        let Some(conn) = self.conn.clone() else {
            return;
        };
        // An exhausted namespace given an empty token starts again from the first page.
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
            };
            let _ = tx.send(Msg::Workflows {
                generation,
                append,
                result,
            });
        });
    }

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
                .map(|p| (p.events, p.next_page_token));
            let _ = tx.send(Msg::History { generation, result });
        });
    }

    fn load_pending(&mut self) {
        let (Some(row), Some(conn)) = (self.view.viewing.clone(), self.conn.clone()) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn
                .pending_activities(&row.namespace, &row.workflow_id, &row.run_id)
                .await;
            let _ = tx.send(Msg::Pending { generation, result });
        });
    }

    pub(super) fn start_pending_poll(&mut self) {
        let (Some(row), Some(conn)) = (self.view.viewing.clone(), self.conn.clone()) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.view.generation);
        self.view.pending_task = Some(tokio::spawn(async move {
            let mut every = tokio::time::interval(PENDING_POLL);
            let mut failing = false;
            loop {
                every.tick().await;
                let result = conn
                    .pending_activities(&row.namespace, &row.workflow_id, &row.run_id)
                    .await;
                let (quiet, last) = match &result {
                    Ok(_) => (false, false),
                    Err(e) if e.is_transient() => (failing, false),
                    Err(_) => (false, true),
                };
                failing = result.is_err();
                if !quiet && tx.send(Msg::Pending { generation, result }).is_err() {
                    return;
                }
                if last {
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
            let result = conn.count_workflows_across(&scope, &query).await;
            let _ = tx.send(Msg::Counts { generation, result });
        });
    }
}
