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
    workflowservice::v1::{
        DeleteWorkflowExecutionRequest, RequestCancelWorkflowExecutionRequest,
        SignalWorkflowExecutionRequest, TerminateWorkflowExecutionRequest,
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
