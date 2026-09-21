//! `DescribeWorkflowExecution`, for what the history cannot say yet.
//!
//! Only the pending activities are read today. See `tmprl_core::pending` for why the
//! history alone cannot show a retry in progress.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::WorkflowExecution, enums::v1::PendingActivityState as ProtoState,
    workflow::v1::PendingActivityInfo, workflowservice::v1::DescribeWorkflowExecutionRequest,
};
use tmprl_core::pending::{PendingActivity, PendingState};

use super::OpError;
use super::history::normalize_failure;
use super::workflow::epoch_millis;
use crate::Conn;

impl Conn {
    pub async fn pending_activities(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
    ) -> Result<Vec<PendingActivity>, OpError> {
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
            .map_err(|s| OpError::rpc("DescribeWorkflowExecution", s))?
            .into_inner();
        Ok(resp
            .pending_activities
            .into_iter()
            .map(pending_from)
            .collect())
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
}
