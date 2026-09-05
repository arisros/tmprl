//! The operations that change a cluster.
//!
//! Everything else in this crate reads. These four write, and two of them cannot be undone,
//! so nothing here decides *whether* to act — the confirmation in `tmprl-core` does that, and
//! the reducer only reaches this module once the reader has said yes.
//!
//! Every request carries an `identity`. Temporal records it on the resulting history event,
//! so a workflow terminated from tmprl says so in its own history rather than appearing to
//! have stopped on its own.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::{Payload as ProtoPayload, Payloads, WorkflowExecution},
    enums::v1::UpdateWorkflowExecutionLifecycleStage,
    update::v1::{Input as UpdateInput, Meta as UpdateMeta, Request as UpdateRequest, WaitPolicy},
    workflowservice::v1::{
        DeleteWorkflowExecutionRequest, RequestCancelWorkflowExecutionRequest,
        ResetWorkflowExecutionRequest, SignalWorkflowExecutionRequest,
        TerminateWorkflowExecutionRequest, UpdateWorkflowExecutionRequest,
    },
};
use tmprl_core::mutation::Mutation;

use super::OpError;
use crate::Conn;

/// What tmprl calls itself on the events it causes.
fn identity() -> String {
    format!(
        "tmprl@{}",
        std::env::var("USER").unwrap_or_else(|_| "unknown".into())
    )
}

/// A fresh request id.
///
/// `ResetWorkflowExecution` rejects a request without one ("RequestId is not set on request").
/// The others accept one, where it makes a retried call idempotent rather than doubling the
/// effect.
fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn execution(workflow_id: &str, run_id: &str) -> Option<WorkflowExecution> {
    Some(WorkflowExecution {
        workflow_id: workflow_id.to_string(),
        run_id: run_id.to_string(),
    })
}

impl Conn {
    /// Carry out a confirmed mutation.
    ///
    /// One entry point rather than four, so the reducer has exactly one place that writes and
    /// the audit log has exactly one thing to wrap.
    pub async fn mutate(&self, m: &Mutation) -> Result<(), OpError> {
        match m {
            Mutation::Cancel {
                namespace,
                workflow_id,
                run_id,
            } => {
                self.wf()
                    .request_cancel_workflow_execution(Request::new(
                        RequestCancelWorkflowExecutionRequest {
                            namespace: namespace.clone(),
                            workflow_execution: execution(workflow_id, run_id),
                            identity: identity(),
                            request_id: request_id(),
                            ..Default::default()
                        },
                    ))
                    .await
                    .map_err(|s| OpError::rpc("RequestCancelWorkflowExecution", s))?;
            }

            Mutation::Terminate {
                namespace,
                workflow_id,
                run_id,
                reason,
            } => {
                self.wf()
                    .terminate_workflow_execution(Request::new(TerminateWorkflowExecutionRequest {
                        namespace: namespace.clone(),
                        workflow_execution: execution(workflow_id, run_id),
                        reason: reason.clone(),
                        identity: identity(),
                        ..Default::default()
                    }))
                    .await
                    .map_err(|s| OpError::rpc("TerminateWorkflowExecution", s))?;
            }

            Mutation::Signal {
                namespace,
                workflow_id,
                run_id,
                name,
                input,
            } => {
                self.wf()
                    .signal_workflow_execution(Request::new(SignalWorkflowExecutionRequest {
                        namespace: namespace.clone(),
                        workflow_execution: execution(workflow_id, run_id),
                        signal_name: name.clone(),
                        input: input.as_deref().map(json_payload),
                        identity: identity(),
                        request_id: request_id(),
                        ..Default::default()
                    }))
                    .await
                    .map_err(|s| OpError::rpc("SignalWorkflowExecution", s))?;
            }

            Mutation::Delete {
                namespace,
                workflow_id,
                run_id,
            } => {
                self.wf()
                    .delete_workflow_execution(Request::new(DeleteWorkflowExecutionRequest {
                        namespace: namespace.clone(),
                        workflow_execution: execution(workflow_id, run_id),
                    }))
                    .await
                    .map_err(|s| OpError::rpc("DeleteWorkflowExecution", s))?;
            }

            Mutation::Reset {
                namespace,
                workflow_id,
                run_id,
                event_id,
                reason,
            } => {
                self.wf()
                    .reset_workflow_execution(Request::new(ResetWorkflowExecutionRequest {
                        namespace: namespace.clone(),
                        workflow_execution: execution(workflow_id, run_id),
                        reason: reason.clone(),
                        // Already resolved to a completed workflow task; the server rejects
                        // anything else.
                        workflow_task_finish_event_id: *event_id,
                        // Nothing is excluded from reapplication, which is the server's
                        // default and almost always what is wanted: a reset should not
                        // silently swallow signals someone sent in after the reset point.
                        // The older `reset_reapply_type` field is deprecated in favour of
                        // this exclude list.
                        reset_reapply_exclude_types: Vec::new(),
                        identity: identity(),
                        request_id: request_id(),
                        ..Default::default()
                    }))
                    .await
                    .map_err(|s| OpError::rpc("ResetWorkflowExecution", s))?;
            }

            Mutation::Update {
                namespace,
                workflow_id,
                run_id,
                name,
                input,
            } => {
                let resp = self
                    .wf()
                    .update_workflow_execution(Request::new(UpdateWorkflowExecutionRequest {
                        namespace: namespace.clone(),
                        workflow_execution: execution(workflow_id, run_id),
                        // Wait for the outcome rather than for acceptance, so the reported
                        // result is the workflow's answer and not just "it was allowed in".
                        wait_policy: Some(WaitPolicy {
                            lifecycle_stage: UpdateWorkflowExecutionLifecycleStage::Completed
                                as i32,
                        }),
                        request: Some(UpdateRequest {
                            request_id: request_id(),
                            meta: Some(UpdateMeta {
                                update_id: request_id(),
                                identity: identity(),
                            }),
                            input: Some(UpdateInput {
                                name: name.clone(),
                                args: input.as_deref().map(json_payload),
                                ..Default::default()
                            }),
                            completion_callbacks: Vec::new(),
                            links: Vec::new(),
                        }),
                        ..Default::default()
                    }))
                    .await
                    .map_err(|s| OpError::rpc("UpdateWorkflowExecution", s))?
                    .into_inner();

                // An update can be *accepted* and then rejected by the workflow itself. That
                // is a failure of the thing the user asked for, so it is reported as one
                // rather than as a success that quietly did nothing.
                if let Some(outcome) = resp.outcome
                    && let Some(
                        temporalio_common::protos::temporal::api::update::v1::outcome::Value::Failure(f),
                    ) = outcome.value
                {
                    return Err(OpError::Rpc {
                        operation: "UpdateWorkflowExecution",
                        code: "Rejected".into(),
                        message: f.message,
                    });
                }
            }
        }
        Ok(())
    }
}

/// A signal argument, encoded the way an SDK would send it.
///
/// `json/plain` with the text as typed. The confirmation shows the same string next to
/// `--input`, so what the CLI would send and what tmprl sends are the same bytes.
fn json_payload(input: &str) -> Payloads {
    Payloads {
        payloads: vec![ProtoPayload {
            metadata: [("encoding".to_string(), b"json/plain".to_vec())]
                .into_iter()
                .collect(),
            data: input.as_bytes().to_vec(),
            external_payloads: Vec::new(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_argument_is_sent_as_json_plain() {
        let p = json_payload(r#"{"a":1}"#);
        assert_eq!(p.payloads.len(), 1);
        assert_eq!(p.payloads[0].data, br#"{"a":1}"#);
        assert_eq!(
            p.payloads[0].metadata.get("encoding").map(|v| v.as_slice()),
            Some(&b"json/plain"[..])
        );
    }

    #[test]
    fn tmprl_names_itself_on_what_it_causes() {
        // A workflow terminated from here should say so in its own history rather than
        // appearing to have stopped on its own.
        assert!(identity().starts_with("tmprl@"));
    }

    #[test]
    fn an_execution_carries_both_ids() {
        let e = execution("w", "r").unwrap();
        assert_eq!(e.workflow_id, "w");
        assert_eq!(e.run_id, "r");
    }
}
