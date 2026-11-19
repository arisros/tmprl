//! `DescribeTaskQueue`: whether anything is polling a queue, and how far behind it is.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    enums::v1::TaskQueueType,
    taskqueue::v1::TaskQueue,
    workflowservice::v1::{DescribeTaskQueueRequest, DescribeTaskQueueResponse},
};
use tmprl_core::taskqueue::{Poller, QueueHealth, TaskQueueKind};

use super::OpError;
use super::workflow::epoch_millis;
use crate::Conn;

impl Conn {
    pub async fn describe_task_queue(
        &self,
        namespace: &str,
        name: &str,
        kind: TaskQueueKind,
    ) -> Result<QueueHealth, OpError> {
        let resp = self
            .wf()
            .describe_task_queue(Request::new(DescribeTaskQueueRequest {
                namespace: namespace.to_string(),
                task_queue: Some(TaskQueue {
                    name: name.to_string(),
                    ..Default::default()
                }),
                task_queue_type: match kind {
                    TaskQueueKind::Workflow => TaskQueueType::Workflow,
                    TaskQueueKind::Activity => TaskQueueType::Activity,
                } as i32,
                report_stats: true,
                ..Default::default()
            }))
            .await
            .map_err(|s| super::rpc("DescribeTaskQueue", s))?
            .into_inner();
        Ok(health_from(resp))
    }

    pub async fn workflow_pollers(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Vec<Poller>, OpError> {
        let resp = self
            .wf()
            .describe_task_queue(Request::new(DescribeTaskQueueRequest {
                namespace: namespace.to_string(),
                task_queue: Some(TaskQueue {
                    name: name.to_string(),
                    ..Default::default()
                }),
                task_queue_type: TaskQueueType::Workflow as i32,
                ..Default::default()
            }))
            .await
            .map_err(|s| super::rpc("DescribeTaskQueue", s))?
            .into_inner();
        Ok(pollers_from(resp))
    }
}

fn pollers_from(resp: DescribeTaskQueueResponse) -> Vec<Poller> {
    resp.pollers
        .into_iter()
        .map(|p| Poller {
            identity: p.identity,
            last_access_ms: p.last_access_time.map(epoch_millis),
        })
        .collect()
}

fn health_from(resp: DescribeTaskQueueResponse) -> QueueHealth {
    QueueHealth {
        backlog: resp.stats.as_ref().map(|s| s.approximate_backlog_count),
        backlog_age_ms: resp
            .stats
            .and_then(|s| s.approximate_backlog_age)
            .map(|d| d.seconds * 1_000 + i64::from(d.nanos) / 1_000_000),
        pollers: resp.pollers.len(),
        last_poll_ms: resp
            .pollers
            .into_iter()
            .filter_map(|p| p.last_access_time.map(epoch_millis))
            .max(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::taskqueue::v1::{PollerInfo, TaskQueueStats};

    fn poller(seconds: i64) -> PollerInfo {
        PollerInfo {
            last_access_time: Some(prost_wkt_types::Timestamp { seconds, nanos: 0 }),
            identity: "worker".into(),
            ..Default::default()
        }
    }

    #[test]
    fn each_poller_keeps_its_identity_and_when_it_last_polled() {
        let pollers = pollers_from(DescribeTaskQueueResponse {
            pollers: vec![poller(100)],
            ..Default::default()
        });
        assert_eq!(
            pollers,
            vec![Poller {
                identity: "worker".into(),
                last_access_ms: Some(100_000),
            }]
        );
    }

    #[test]
    fn a_response_becomes_backlog_age_and_pollers() {
        let health = health_from(DescribeTaskQueueResponse {
            pollers: vec![poller(100), poller(250)],
            stats: Some(TaskQueueStats {
                approximate_backlog_count: 12,
                approximate_backlog_age: Some(prost_wkt_types::Duration {
                    seconds: 90,
                    nanos: 500_000_000,
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(
            health,
            QueueHealth {
                backlog: Some(12),
                backlog_age_ms: Some(90_500),
                pollers: 2,
                last_poll_ms: Some(250_000),
            }
        );
    }

    #[test]
    fn a_server_that_reports_no_stats_is_not_a_queue_with_no_backlog() {
        let health = health_from(DescribeTaskQueueResponse::default());
        assert_eq!(health.backlog, None);
        assert_eq!(health.pollers, 0);
        assert!(!health.stuck());
    }
}
