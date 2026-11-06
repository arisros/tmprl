//! The operations that change a cluster.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::{Payload as ProtoPayload, Payloads, WorkflowExecution, WorkflowType},
    enums::v1::UpdateWorkflowExecutionLifecycleStage,
    schedule::v1::{
        BackfillRequest, Schedule, ScheduleAction, SchedulePatch, ScheduleSpec,
        TriggerImmediatelyRequest, schedule_action::Action as ScheduleActionKind,
    },
    taskqueue::v1::TaskQueue,
    update::v1::{Input as UpdateInput, Meta as UpdateMeta, Request as UpdateRequest, WaitPolicy},
    workflow::v1::NewWorkflowExecutionInfo,
    workflowservice::v1::{
        CreateScheduleRequest, DeleteScheduleRequest, DeleteWorkflowExecutionRequest,
        PatchScheduleRequest, RequestCancelWorkflowExecutionRequest, ResetWorkflowExecutionRequest,
        SignalWorkflowExecutionRequest, TerminateWorkflowExecutionRequest,
        UpdateWorkflowExecutionRequest,
    },
};
use tmprl_core::mutation::Mutation;

use super::OpError;
use crate::Conn;

fn identity() -> String {
    format!(
        "tmprl@{}",
        std::env::var("USER").unwrap_or_else(|_| "unknown".into())
    )
}

// ResetWorkflowExecution rejects a request without a request id.
fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn timestamp(ms: i64) -> prost_wkt_types::Timestamp {
    prost_wkt_types::Timestamp {
        seconds: ms.div_euclid(1_000),
        nanos: (ms.rem_euclid(1_000) * 1_000_000) as i32,
    }
}

fn execution(workflow_id: &str, run_id: &str) -> Option<WorkflowExecution> {
    Some(WorkflowExecution {
        workflow_id: workflow_id.to_string(),
        run_id: run_id.to_string(),
    })
}

impl Conn {
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
                    .map_err(|s| super::rpc("RequestCancelWorkflowExecution", s))?;
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
                    .map_err(|s| super::rpc("TerminateWorkflowExecution", s))?;
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
                    .map_err(|s| super::rpc("SignalWorkflowExecution", s))?;
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
                    .map_err(|s| super::rpc("DeleteWorkflowExecution", s))?;
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
                        workflow_task_finish_event_id: *event_id,
                        reset_reapply_exclude_types: Vec::new(),
                        identity: identity(),
                        request_id: request_id(),
                        ..Default::default()
                    }))
                    .await
                    .map_err(|s| super::rpc("ResetWorkflowExecution", s))?;
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
                    .map_err(|s| super::rpc("UpdateWorkflowExecution", s))?
                    .into_inner();

                if let Some(outcome) = resp.outcome
                    && let Some(
                        temporalio_common::protos::temporal::api::update::v1::outcome::Value::Failure(f),
                    ) = outcome.value
                {
                    return Err(OpError::rpc(
                        "UpdateWorkflowExecution",
                        tmprl_core::fault::Code::Rejected,
                        f.message,
                    ));
                }
            }

            Mutation::PauseSchedule {
                namespace,
                schedule_id,
                paused,
            } => {
                let note = format!("{} from tmprl", if *paused { "paused" } else { "resumed" });
                self.wf()
                    .patch_schedule(Request::new(PatchScheduleRequest {
                        namespace: namespace.clone(),
                        schedule_id: schedule_id.clone(),
                        patch: Some(SchedulePatch {
                            pause: if *paused { note.clone() } else { String::new() },
                            unpause: if *paused { String::new() } else { note },
                            ..Default::default()
                        }),
                        identity: identity(),
                        request_id: request_id(),
                    }))
                    .await
                    .map_err(|s| super::rpc("PatchSchedule", s))?;
            }

            Mutation::TriggerSchedule {
                namespace,
                schedule_id,
            } => {
                self.wf()
                    .patch_schedule(Request::new(PatchScheduleRequest {
                        namespace: namespace.clone(),
                        schedule_id: schedule_id.clone(),
                        patch: Some(SchedulePatch {
                            trigger_immediately: Some(TriggerImmediatelyRequest {
                                overlap_policy: 0,
                                scheduled_time: None,
                            }),
                            ..Default::default()
                        }),
                        identity: identity(),
                        request_id: request_id(),
                    }))
                    .await
                    .map_err(|s| super::rpc("PatchSchedule", s))?;
            }

            Mutation::CreateSchedule {
                namespace,
                schedule_id,
                workflow_id,
                workflow_type,
                task_queue,
                spec,
                input,
            } => {
                self.wf()
                    .create_schedule(Request::new(CreateScheduleRequest {
                        namespace: namespace.clone(),
                        schedule_id: schedule_id.clone(),
                        schedule: Some(Schedule {
                            spec: Some(ScheduleSpec {
                                cron_string: vec![spec.clone()],
                                ..Default::default()
                            }),
                            action: Some(ScheduleAction {
                                action: Some(ScheduleActionKind::StartWorkflow(
                                    NewWorkflowExecutionInfo {
                                        workflow_id: workflow_id.clone(),
                                        workflow_type: Some(WorkflowType {
                                            name: workflow_type.clone(),
                                        }),
                                        task_queue: Some(TaskQueue {
                                            name: task_queue.clone(),
                                            ..Default::default()
                                        }),
                                        input: input.as_deref().map(json_payload),
                                        ..Default::default()
                                    },
                                )),
                            }),
                            ..Default::default()
                        }),
                        identity: identity(),
                        request_id: request_id(),
                        ..Default::default()
                    }))
                    .await
                    .map_err(|s| super::rpc("CreateSchedule", s))?;
            }

            Mutation::BackfillSchedule {
                namespace,
                schedule_id,
                range,
                overlap,
            } => {
                self.wf()
                    .patch_schedule(Request::new(PatchScheduleRequest {
                        namespace: namespace.clone(),
                        schedule_id: schedule_id.clone(),
                        patch: Some(SchedulePatch {
                            backfill_request: vec![BackfillRequest {
                                start_time: Some(timestamp(range.start_ms)),
                                end_time: Some(timestamp(range.end_ms)),
                                overlap_policy: overlap.code(),
                            }],
                            ..Default::default()
                        }),
                        identity: identity(),
                        request_id: request_id(),
                    }))
                    .await
                    .map_err(|s| super::rpc("PatchSchedule", s))?;
            }

            Mutation::DeleteSchedule {
                namespace,
                schedule_id,
            } => {
                self.wf()
                    .delete_schedule(Request::new(DeleteScheduleRequest {
                        namespace: namespace.clone(),
                        schedule_id: schedule_id.clone(),
                        identity: identity(),
                    }))
                    .await
                    .map_err(|s| super::rpc("DeleteSchedule", s))?;
            }
        }
        Ok(())
    }
}

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
        assert!(identity().starts_with("tmprl@"));
    }

    #[test]
    fn an_execution_carries_both_ids() {
        let e = execution("w", "r").unwrap();
        assert_eq!(e.workflow_id, "w");
        assert_eq!(e.run_id, "r");
    }
}
