//! `DescribeWorkflowExecution`: the workflow's own state, and what the history cannot say yet.

use std::collections::HashMap;

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::{Payload as ProtoPayload, WorkflowExecution},
    enums::v1::{
        ParentClosePolicy as ProtoClosePolicy, PendingActivityState as ProtoState,
        PendingWorkflowTaskState as ProtoTaskState,
    },
    workflow::v1::{PendingActivityInfo, PendingChildExecutionInfo, PendingWorkflowTaskInfo},
    workflowservice::v1::{DescribeWorkflowExecutionRequest, DescribeWorkflowExecutionResponse},
};
use tmprl_core::info::{Execution, PendingChild, PendingWorkflowTask, WorkflowInfo};
use tmprl_core::payload::Payload;
use tmprl_core::pending::{PendingActivity, PendingState};

use super::OpError;
use super::history::{convert, normalize_failure};
use super::workflow::{epoch_millis, status_from_proto};
use crate::Conn;

#[derive(Debug, Clone, Default)]
pub struct Described {
    pub info: WorkflowInfo,
    pub pending: Vec<PendingActivity>,
}

impl Conn {
    pub async fn describe(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
    ) -> Result<Described, OpError> {
        let resp = self
            .wf()
            .describe_workflow_execution(Request::new(DescribeWorkflowExecutionRequest {
                namespace: namespace.to_string(),
                execution: Some(WorkflowExecution {
                    workflow_id: workflow_id.to_string(),
                    run_id: run_id.to_string(),
                }),
            }))
            .await
            .map_err(|s| super::rpc("DescribeWorkflowExecution", s))?
            .into_inner();
        Ok(described_from(resp))
    }

    pub async fn pending_activities(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
    ) -> Result<Vec<PendingActivity>, OpError> {
        Ok(self.describe(namespace, workflow_id, run_id).await?.pending)
    }
}

fn described_from(resp: DescribeWorkflowExecutionResponse) -> Described {
    let pending = resp
        .pending_activities
        .into_iter()
        .map(pending_from)
        .collect();
    let mut info = WorkflowInfo {
        pending_children: resp.pending_children.into_iter().map(child_from).collect(),
        workflow_task: resp.pending_workflow_task.map(task_from),
        ..Default::default()
    };
    if let Some(config) = resp.execution_config {
        info.execution_timeout_ms = config.workflow_execution_timeout.map(duration_millis);
        info.run_timeout_ms = config.workflow_run_timeout.map(duration_millis);
        info.task_timeout_ms = config.default_workflow_task_timeout.map(duration_millis);
    }
    if let Some(e) = resp.workflow_execution_info {
        info.status = status_from_proto(e.status());
        let execution = e.execution.unwrap_or_default();
        info.workflow_id = execution.workflow_id;
        info.run_id = execution.run_id;
        info.first_run_id = e.first_run_id;
        info.workflow_type = e.r#type.map(|t| t.name).unwrap_or_default();
        info.task_queue = e.task_queue;
        info.start_time = e.start_time.map(epoch_millis);
        info.execution_time = e.execution_time.map(epoch_millis);
        info.close_time = e.close_time.map(epoch_millis);
        info.history_length = e.history_length;
        info.history_size_bytes = e.history_size_bytes;
        info.parent = e.parent_execution.map(execution_from);
        info.root = e.root_execution.map(execution_from);
        info.memo = sorted(e.memo.map(|m| m.fields).unwrap_or_default());
        info.search_attributes = sorted(
            e.search_attributes
                .map(|s| s.indexed_fields)
                .unwrap_or_default(),
        );
    }
    Described { info, pending }
}

fn sorted(fields: HashMap<String, ProtoPayload>) -> Vec<(String, Payload)> {
    let mut out: Vec<(String, Payload)> =
        fields.into_iter().map(|(k, p)| (k, convert(p))).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn execution_from(e: WorkflowExecution) -> Execution {
    Execution {
        workflow_id: e.workflow_id,
        run_id: e.run_id,
    }
}

fn child_from(c: PendingChildExecutionInfo) -> PendingChild {
    PendingChild {
        parent_close_policy: close_policy_label(c.parent_close_policy()),
        workflow_id: c.workflow_id,
        run_id: c.run_id,
        workflow_type: c.workflow_type_name,
        initiated_id: c.initiated_id,
    }
}

fn task_from(t: PendingWorkflowTaskInfo) -> PendingWorkflowTask {
    PendingWorkflowTask {
        state: task_state_label(t.state()),
        scheduled_at: t.scheduled_time.map(epoch_millis),
        started_at: t.started_time.map(epoch_millis),
        attempt: t.attempt,
    }
}

fn duration_millis(d: prost_wkt_types::Duration) -> i64 {
    d.seconds * 1000 + i64::from(d.nanos) / 1_000_000
}

fn close_policy_label(p: ProtoClosePolicy) -> &'static str {
    match p {
        ProtoClosePolicy::Unspecified => "unspecified",
        ProtoClosePolicy::Terminate => "terminate",
        ProtoClosePolicy::Abandon => "abandon",
        ProtoClosePolicy::RequestCancel => "cancel",
    }
}

fn task_state_label(s: ProtoTaskState) -> &'static str {
    match s {
        ProtoTaskState::Unspecified => "unspecified",
        ProtoTaskState::Scheduled => "scheduled",
        ProtoTaskState::Started => "started",
    }
}

fn pending_from(p: PendingActivityInfo) -> PendingActivity {
    PendingActivity {
        state: state_from_proto(p.state()),
        activity_id: p.activity_id,
        activity_type: p.activity_type.map(|t| t.name).unwrap_or_default(),
        attempt: p.attempt,
        maximum_attempts: p.maximum_attempts,
        last_failure: p.last_failure.map(normalize_failure),
        next_attempt_at: p.next_attempt_schedule_time.map(epoch_millis),
        last_started_at: p.last_started_time.map(epoch_millis),
        last_heartbeat_at: p.last_heartbeat_time.map(epoch_millis),
        last_worker: Some(p.last_worker_identity).filter(|s| !s.is_empty()),
    }
}

fn state_from_proto(s: ProtoState) -> PendingState {
    match s {
        ProtoState::Unspecified => PendingState::Unspecified,
        ProtoState::Scheduled => PendingState::Scheduled,
        ProtoState::Started => PendingState::Started,
        ProtoState::CancelRequested => PendingState::CancelRequested,
        ProtoState::Paused => PendingState::Paused,
        ProtoState::PauseRequested => PendingState::PauseRequested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::ActivityType, failure::v1::Failure,
    };

    #[test]
    fn a_retrying_activity_keeps_its_attempt_failure_and_next_try() {
        let p = pending_from(PendingActivityInfo {
            activity_id: "1".into(),
            activity_type: Some(ActivityType {
                name: "ChargeCard".into(),
            }),
            state: ProtoState::Scheduled as i32,
            attempt: 4,
            maximum_attempts: 10,
            last_failure: Some(Failure {
                message: "card declined".into(),
                ..Default::default()
            }),
            next_attempt_schedule_time: Some(prost_wkt_types::Timestamp {
                seconds: 1_700_000_000,
                nanos: 0,
            }),
            ..Default::default()
        });

        assert_eq!(p.activity_id, "1");
        assert_eq!(p.activity_type, "ChargeCard");
        assert_eq!(p.state, PendingState::Scheduled);
        assert_eq!(p.attempts_label(), "4/10");
        assert_eq!(
            p.last_failure.map(|f| f.message),
            Some("card declined".into())
        );
        assert_eq!(p.next_attempt_at, Some(1_700_000_000_000));
        assert_eq!(p.last_worker, None, "an empty identity is no identity");
    }

    #[test]
    fn a_describe_reply_becomes_the_info_pane_in_order() {
        use temporalio_common::protos::temporal::api::{
            common::v1::{Memo, SearchAttributes, WorkflowType},
            enums::v1::WorkflowExecutionStatus,
            workflow::v1::{WorkflowExecutionConfig, WorkflowExecutionInfo},
        };
        let payload = |s: &str| ProtoPayload {
            metadata: [("encoding".to_string(), b"json/plain".to_vec())].into(),
            data: s.as_bytes().to_vec(),
            ..Default::default()
        };
        let d = described_from(DescribeWorkflowExecutionResponse {
            execution_config: Some(WorkflowExecutionConfig {
                workflow_run_timeout: Some(prost_wkt_types::Duration {
                    seconds: 3600,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            workflow_execution_info: Some(WorkflowExecutionInfo {
                execution: Some(WorkflowExecution {
                    workflow_id: "order-123".into(),
                    run_id: "r2".into(),
                }),
                r#type: Some(WorkflowType {
                    name: "OrderWorkflow".into(),
                }),
                status: WorkflowExecutionStatus::Running as i32,
                task_queue: "orders".into(),
                first_run_id: "r1".into(),
                parent_execution: Some(WorkflowExecution {
                    workflow_id: "checkout-1".into(),
                    run_id: "c1".into(),
                }),
                memo: Some(Memo {
                    fields: [("note".to_string(), payload("\"vip\""))].into(),
                }),
                search_attributes: Some(SearchAttributes {
                    indexed_fields: [
                        ("tenant".to_string(), payload("\"acme\"")),
                        ("region".to_string(), payload("\"ap\"")),
                    ]
                    .into(),
                }),
                ..Default::default()
            }),
            pending_children: vec![PendingChildExecutionInfo {
                workflow_id: "ship-1".into(),
                workflow_type_name: "ShipWorkflow".into(),
                parent_close_policy: ProtoClosePolicy::Abandon as i32,
                ..Default::default()
            }],
            pending_workflow_task: Some(PendingWorkflowTaskInfo {
                state: ProtoTaskState::Started as i32,
                attempt: 2,
                ..Default::default()
            }),
            ..Default::default()
        });

        let info = d.info;
        assert_eq!(info.workflow_id, "order-123");
        assert_eq!(info.first_run_id, "r1");
        assert_eq!(info.status, tmprl_core::workflow::WorkflowStatus::Running);
        assert_eq!(info.run_timeout_ms, Some(3_600_000));
        assert_eq!(info.execution_timeout_ms, None);
        assert_eq!(
            info.parent.map(|p| p.workflow_id),
            Some("checkout-1".into())
        );
        assert_eq!(info.memo[0].0, "note");
        let keys: Vec<&str> = info
            .search_attributes
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(
            keys,
            ["region", "tenant"],
            "a proto map has no order of its own"
        );
        assert_eq!(info.pending_children[0].parent_close_policy, "abandon");
        assert_eq!(info.workflow_task.map(|t| t.state), Some("started"));
        assert!(d.pending.is_empty());
    }
}
