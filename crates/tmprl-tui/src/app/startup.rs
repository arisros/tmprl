//! Where the command line asked to open: a namespace, a query, a workflow, the dashboard,

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Startup {
    pub namespace: Option<String>,
    pub query: Option<String>,
    pub workflow: Option<String>,
    pub readonly: bool,
    pub dashboard: bool,
    pub log: Option<String>,
}

impl Startup {
    pub fn opens_workflows(&self) -> bool {
        self.namespace.is_some() || self.query.is_some() || self.workflow.is_some()
    }
}

impl App {
    pub fn start(&mut self, startup: Startup) {
        self.readonly_flag = startup.readonly;
        self.load_namespaces();
        if !startup.opens_workflows() && !startup.dashboard {
            return;
        }
        if let Some(namespace) = startup.namespace {
            self.view.scope = vec![namespace.clone()];
            self.namespace = namespace;
        }
        if startup.dashboard {
            self.view.screen = Screen::Dashboard;
            self.load_dashboard();
            return;
        }
        self.view.screen = Screen::Workflows;
        if let Some(query) = startup.query {
            self.view.query = query;
        }
        match startup.workflow {
            Some(id) => self.find_startup_workflow(id),
            None => self.load_workflows(false),
        }
    }

    fn find_startup_workflow(&mut self, id: String) {
        let Some(query) = tmprl_core::query::by_id(&id) else {
            self.note = Some((
                format!("a quote cannot go in a workflow lookup: {id}"),
                Note::Warn,
            ));
            self.load_workflows(false);
            return;
        };
        self.view.generation = self.view.generation.wrapping_add(1);
        self.view.workflows.begin_refresh();

        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, generation, scope) = (
            self.tx.clone(),
            self.view.generation,
            self.view.scope.clone(),
        );
        tokio::spawn(async move {
            let result = conn
                .list_workflows_across(&scope, &query, PICKER_SEARCH_LIMIT)
                .await
                .map(|(rows, _)| rows);
            let _ = tx.send(Msg::StartupWorkflow {
                generation,
                id,
                result,
            });
        });
    }

    pub(super) fn startup_workflow_found(
        &mut self,
        generation: u64,
        id: String,
        result: Result<Vec<WorkflowRow>, Fault>,
    ) {
        if generation != self.view.generation || self.view.screen != Screen::Workflows {
            return;
        }
        let rows = match result {
            Ok(rows) => rows,
            Err(e) => {
                self.load_workflows(false);
                self.fail_as(format!("workflow {id}: {e}"), e, Note::Error);
                return;
            }
        };
        let runs = rows.len();
        let Some(newest) = rows.into_iter().max_by_key(|row| row.start_time) else {
            self.load_workflows(false);
            self.note = Some((
                format!(
                    "no workflow or run with id {id} in {}, showing the list",
                    self.view.scope.join(", ")
                ),
                Note::Warn,
            ));
            return;
        };
        self.open_row(newest);
        if runs > 1 && self.note.is_none() {
            self.note = Some((
                format!("{runs} runs of {id}, opened the newest"),
                Note::Info,
            ));
        }
    }
}
