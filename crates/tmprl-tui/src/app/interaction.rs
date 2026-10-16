//! Workflow queries: asking for the handlers, the picker over them, and the answer.

use tmprl_core::fault::Code;
use tmprl_core::interaction::{self, Handlers};

use super::*;

impl App {
    /// `<leader>cq`: ask the workflow which queries it has, then offer them.
    pub(super) fn open_query(&mut self) {
        let Some(row) = self.query_target() else {
            return;
        };
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.note = Some((
            format!("asking {} for its queries…", row.workflow_id),
            Note::Info,
        ));
        let (tx, codec, generation) = (self.tx.clone(), self.codec.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn
                .query_workflow(
                    &row.namespace,
                    &row.workflow_id,
                    &row.run_id,
                    interaction::METADATA,
                )
                .await;
            let result = match result {
                Ok(payloads) => Ok(decode(codec.as_deref(), &row.namespace, payloads).await),
                Err(e) => Err(e),
            };
            let _ = tx.send(Msg::QueryHandlers { generation, result });
        });
    }

    /// `<leader>cs`: the stack trace, which every SDK answers without being asked to.
    pub(super) fn stack_trace(&mut self) {
        if self.query_target().is_some() {
            self.run_query(interaction::STACK_TRACE.into());
        }
    }

    pub(super) fn query_handlers_reply(
        &mut self,
        generation: u64,
        result: Result<Vec<Payload>, Fault>,
    ) {
        if generation != self.view.generation {
            return;
        }
        self.note = None;
        let handlers = match result {
            Ok(payloads) => payloads.first().and_then(|p| {
                matches!(p.encoding.as_str(), "json/plain" | "json/protobuf")
                    .then(|| std::str::from_utf8(&p.data).ok())
                    .flatten()
                    .and_then(Handlers::from_metadata_json)
            }),
            Err(e) => match interaction::known_from_refusal(&e.message) {
                Some(names) => Some(Handlers::from_names(names)),
                // Refused without a list: the workflow is there and answers, it just cannot
                // say what it has, so the name is the reader's to give.
                None if e.code == Code::InvalidArgument => None,
                None => {
                    self.fail_as(format!("query: {e}"), e, Note::Error);
                    return;
                }
            },
        };
        match handlers {
            Some(handlers) => {
                let details = handlers.current_details.clone();
                self.view.handlers = Some(handlers);
                self.open_picker(picker::Kind::Queries);
                if let Some(details) = details {
                    self.note = Some((details, Note::Info));
                }
            }
            None => {
                self.view.handlers = None;
                self.mode = Mode::Command;
                self.prompt = Some(Prompt {
                    kind: PromptKind::Query,
                    buf: String::new(),
                });
            }
        }
    }

    pub(super) fn query_items(&self) -> Vec<picker::Item> {
        self.view
            .handlers
            .as_ref()
            .map(|h| {
                h.offered()
                    .into_iter()
                    .map(|h| {
                        picker::Item::new(h.name.clone(), Target::Handler(h.name))
                            .with_note(h.description)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Ask one handler. The answer replaces the payloads in the pane until the cursor moves.
    pub(super) fn run_query(&mut self, name: String) {
        let (Some(row), Some(conn)) = (self.query_target(), self.conn.clone()) else {
            return;
        };
        self.view.show_detail = true;
        self.view.detail_scroll = 0;
        self.view.queried = None;
        self.view.piped = Some(Ok(format!("asking `{name}`…")));
        let (tx, codec, generation) = (self.tx.clone(), self.codec.clone(), self.view.generation);
        tokio::spawn(async move {
            let result = conn
                .query_workflow(&row.namespace, &row.workflow_id, &row.run_id, &name)
                .await;
            let result = match result {
                Ok(payloads) => Ok(decode(codec.as_deref(), &row.namespace, payloads).await),
                Err(e) => Err(e),
            };
            let _ = tx.send(Msg::Queried {
                generation,
                name,
                result,
            });
        });
    }

    pub(super) fn queried_reply(
        &mut self,
        generation: u64,
        name: String,
        result: Result<Vec<Payload>, Fault>,
    ) {
        if generation != self.view.generation {
            return;
        }
        self.view.piped = None;
        match result {
            Ok(payloads) => {
                self.view.detail_scroll = 0;
                self.view.queried = Some(interaction::labelled(&name, payloads));
            }
            Err(e) => self.fail_as(format!("query `{name}`: {e}"), e, Note::Error),
        }
    }

    /// The workflow a query goes to: the one whose history is open, since the answer is
    /// shown in that history's payload pane.
    fn query_target(&mut self) -> Option<WorkflowRow> {
        if self.view.screen != Screen::History {
            self.note = Some((
                "queries are asked from a workflow history, open one first".into(),
                Note::Warn,
            ));
            return None;
        }
        self.view.viewing.clone()
    }
}

/// Decode what the codec server can, and keep the rest as it came.
///
/// Done in the task rather than through the pane's lazy decode, because the handler list has
/// to be read before anything is drawn.
async fn decode(codec: Option<&Codec>, namespace: &str, payloads: Vec<Payload>) -> Vec<Payload> {
    let Some(codec) = codec else {
        return payloads;
    };
    if !payloads.iter().any(Payload::needs_codec) {
        return payloads;
    }
    codec.decode(namespace, &payloads).await.unwrap_or(payloads)
}
