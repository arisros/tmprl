//! Where the command line asked to open: a namespace, a query, a workflow, read-only.

use super::*;

/// What the command line asked for beyond which cluster to connect to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Startup {
    /// `-n`: open this namespace's workflows instead of the namespace list.
    pub namespace: Option<String>,
    /// `-q`: the visibility query to open the workflow list with.
    pub query: Option<String>,
    /// `-w`: a workflow id or run id whose history to open.
    pub workflow: Option<String>,
    /// `--readonly`: refuse every mutation, whatever `config.toml` says.
    pub readonly: bool,
}

impl Startup {
    /// Whether any of this skips the namespace list.
    pub fn opens_workflows(&self) -> bool {
        self.namespace.is_some() || self.query.is_some() || self.workflow.is_some()
    }
}

impl App {
    /// The first fetches, and the screen the command line asked for.
    ///
    /// Called once, after `apply_config` and before the first frame. The namespace list is
    /// fetched whichever screen this lands on: it is what `-` goes up to and what
    /// `<leader>fn` picks from.
    pub fn start(&mut self, startup: Startup) {
        self.readonly_flag = startup.readonly;
        self.load_namespaces();
        if !startup.opens_workflows() {
            return;
        }
        if let Some(namespace) = startup.namespace {
            // The connection was already bound to it; this is the same fact for the panes.
            self.view.scope = vec![namespace.clone()];
            self.namespace = namespace;
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

    /// Ask the server for the workflow `-w` named.
    ///
    /// The picker's exact lookup, by workflow id or run id, without its prefix fallback: a
    /// picker shows what a prefix matched and lets the reader choose, while this opens
    /// what it finds, and opening a different workflow than the one named is worse than
    /// saying there is none.
    ///
    /// The list is not fetched alongside it. Opening the history would bump the generation
    /// and drop the list's reply, so the list is fetched when it is needed: at once if the
    /// lookup finds nothing, or by `-` if it does.
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

    /// The lookup for `-w` came back: open the newest run, or fall back to the list.
    pub(super) fn startup_workflow_found(
        &mut self,
        generation: u64,
        id: String,
        result: Result<Vec<WorkflowRow>, Fault>,
    ) {
        // The reader got there first: another fetch, or `-` up to the namespaces. Opening a
        // history over wherever they went would be the stale reply painted, not dropped.
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
        // A workflow id names every run it ever had, and the one being asked about is
        // almost always the latest.
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
        // A config error from startup may be on the line, and it matters more than this.
        if runs > 1 && self.note.is_none() {
            self.note = Some((
                format!("{runs} runs of {id}, opened the newest"),
                Note::Info,
            ));
        }
    }
}
