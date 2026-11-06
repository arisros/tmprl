//! Listing and counting workflow executions.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    enums::v1::WorkflowExecutionStatus as ProtoStatus,
    workflow::v1::WorkflowExecutionInfo,
    workflowservice::v1::{CountWorkflowExecutionsRequest, ListWorkflowExecutionsRequest},
};
use tmprl_core::query::count_query;
use tmprl_core::workflow::{StatusCounts, WorkflowRow, WorkflowStatus, merge_by_start_time};

use super::OpError;
use crate::Conn;

pub type Continuation = Vec<(String, Vec<u8>)>;

#[derive(Debug, Clone, Default)]
pub struct WorkflowPage {
    pub rows: Vec<WorkflowRow>,
    pub next_page_token: Vec<u8>,
}

impl WorkflowPage {
    pub fn has_more(&self) -> bool {
        !self.next_page_token.is_empty()
    }
}

impl Conn {
    pub async fn list_workflows(
        &self,
        namespace: &str,
        query: &str,
        page_size: i32,
        next_page_token: Vec<u8>,
    ) -> Result<WorkflowPage, OpError> {
        let resp = self
            .wf()
            .list_workflow_executions(Request::new(ListWorkflowExecutionsRequest {
                namespace: namespace.to_string(),
                page_size,
                next_page_token,
                query: query.to_string(),
            }))
            .await
            .map_err(|s| super::rpc("ListWorkflowExecutions", s))?
            .into_inner();

        Ok(WorkflowPage {
            rows: resp
                .executions
                .into_iter()
                .map(|e| row_from(namespace, e))
                .collect(),
            next_page_token: resp.next_page_token,
        })
    }

    pub async fn list_workflows_across(
        &self,
        namespaces: &[String],
        query: &str,
        page_size: i32,
    ) -> Result<(Vec<WorkflowRow>, Continuation), OpError> {
        let starting: Continuation = namespaces
            .iter()
            .map(|ns| (ns.clone(), Vec::new()))
            .collect();
        self.fetch_pages(&starting, query, page_size).await
    }

    // Only namespaces with a token: an exhausted one given an empty token starts again.
    pub async fn continue_workflows_across(
        &self,
        tokens: &Continuation,
        query: &str,
        page_size: i32,
    ) -> Result<(Vec<WorkflowRow>, Continuation), OpError> {
        self.fetch_pages(tokens, query, page_size).await
    }

    async fn fetch_pages(
        &self,
        tokens: &Continuation,
        query: &str,
        page_size: i32,
    ) -> Result<(Vec<WorkflowRow>, Continuation), OpError> {
        let pages = futures_util::future::try_join_all(tokens.iter().map(|(ns, token)| {
            let token = token.clone();
            async move {
                self.list_workflows(ns, query, page_size, token)
                    .await
                    .map(|p| (ns.clone(), p))
            }
        }))
        .await?;

        let mut next = Continuation::new();
        let mut all = Vec::new();
        for (ns, page) in pages {
            if page.has_more() {
                next.push((ns, page.next_page_token));
            }
            all.push(page.rows);
        }
        Ok((merge_by_start_time(all), next))
    }

    pub async fn count_workflows_by_status(
        &self,
        namespace: &str,
        query: &str,
    ) -> Result<StatusCounts, OpError> {
        let resp = self
            .wf()
            .count_workflow_executions(Request::new(CountWorkflowExecutionsRequest {
                namespace: namespace.to_string(),
                query: count_query(query),
            }))
            .await
            .map_err(|s| super::rpc("CountWorkflowExecutions", s))?
            .into_inner();

        let counts = resp.groups.into_iter().filter_map(|g| {
            let status = g.group_values.first().and_then(status_from_payload)?;
            Some((status, g.count))
        });
        Ok(StatusCounts::new(resp.count, counts))
    }
}

impl Conn {
    pub async fn count_workflows_across(
        &self,
        namespaces: &[String],
        query: &str,
    ) -> Result<StatusCounts, OpError> {
        let per_ns = futures_util::future::try_join_all(
            namespaces
                .iter()
                .map(|ns| self.count_workflows_by_status(ns, query)),
        )
        .await?;

        let mut total = 0;
        let mut summed: Vec<(WorkflowStatus, i64)> = Vec::new();
        for counts in &per_ns {
            total += counts.total;
            for (status, n) in counts.iter() {
                match summed.iter_mut().find(|(s, _)| *s == status) {
                    Some((_, acc)) => *acc += n,
                    None => summed.push((status, n)),
                }
            }
        }
        Ok(StatusCounts::new(total, summed))
    }
}

fn row_from(namespace: &str, e: WorkflowExecutionInfo) -> WorkflowRow {
    let status = status_from_proto(e.status());
    let (workflow_id, run_id) = e
        .execution
        .map(|x| (x.workflow_id, x.run_id))
        .unwrap_or_default();

    WorkflowRow {
        namespace: namespace.to_string(),
        workflow_id,
        run_id,
        workflow_type: e.r#type.map(|t| t.name).unwrap_or_default(),
        task_queue: e.task_queue,
        status,
        start_time: e.start_time.map(epoch_millis),
        close_time: e.close_time.map(epoch_millis),
        history_length: e.history_length,
    }
}

// Exhaustive on purpose: a new status should stop the build.
pub(crate) fn status_from_proto(s: ProtoStatus) -> WorkflowStatus {
    match s {
        ProtoStatus::Unspecified => WorkflowStatus::Unspecified,
        ProtoStatus::Running => WorkflowStatus::Running,
        ProtoStatus::Completed => WorkflowStatus::Completed,
        ProtoStatus::Failed => WorkflowStatus::Failed,
        ProtoStatus::Canceled => WorkflowStatus::Canceled,
        ProtoStatus::Terminated => WorkflowStatus::Terminated,
        ProtoStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
        ProtoStatus::TimedOut => WorkflowStatus::TimedOut,
        ProtoStatus::Paused => WorkflowStatus::Paused,
    }
}

fn status_from_payload(
    p: &temporalio_common::protos::temporal::api::common::v1::Payload,
) -> Option<WorkflowStatus> {
    WorkflowStatus::parse(std::str::from_utf8(&p.data).ok()?)
}

pub(crate) fn epoch_millis(t: prost_wkt_types::Timestamp) -> i64 {
    t.seconds * 1000 + i64::from(t.nanos) / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::common::v1::{
        Payload, WorkflowExecution, WorkflowType,
    };
    use tmprl_core::workflow::WorkflowStatus;

    fn payload(body: &str) -> Payload {
        Payload {
            metadata: [
                ("encoding".to_string(), b"json/plain".to_vec()),
                ("type".to_string(), b"Keyword".to_vec()),
            ]
            .into_iter()
            .collect(),
            data: body.as_bytes().to_vec(),
            external_payloads: Vec::new(),
        }
    }

    #[test]
    fn every_proto_status_maps_to_a_domain_status() {
        let all = [
            ProtoStatus::Unspecified,
            ProtoStatus::Running,
            ProtoStatus::Completed,
            ProtoStatus::Failed,
            ProtoStatus::Canceled,
            ProtoStatus::Terminated,
            ProtoStatus::ContinuedAsNew,
            ProtoStatus::TimedOut,
            ProtoStatus::Paused,
        ];
        let mut mapped: Vec<WorkflowStatus> = all.iter().copied().map(status_from_proto).collect();
        mapped.sort_unstable();
        mapped.dedup();
        assert_eq!(mapped.len(), all.len(), "two proto statuses collapsed");
    }

    #[test]
    fn group_payloads_decode_to_a_status() {
        assert_eq!(
            status_from_payload(&payload("\"Running\"")),
            Some(WorkflowStatus::Running)
        );
        assert_eq!(
            status_from_payload(&payload("\"ContinuedAsNew\"")),
            Some(WorkflowStatus::ContinuedAsNew)
        );
        assert_eq!(status_from_payload(&payload("\"Nonsense\"")), None);
    }

    #[test]
    fn timestamps_convert_to_epoch_millis() {
        let t = prost_wkt_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 500_000_000,
        };
        assert_eq!(epoch_millis(t), 1_700_000_000_500);
    }

    #[test]
    fn a_row_without_an_execution_still_maps() {
        let row = row_from("ns", WorkflowExecutionInfo::default());
        assert_eq!(row.namespace, "ns");
        assert!(row.workflow_id.is_empty() && row.run_id.is_empty());
        assert_eq!(row.status, WorkflowStatus::Unspecified);
        assert_eq!(row.start_time, None);
    }

    #[test]
    fn a_populated_row_carries_its_namespace() {
        let info = WorkflowExecutionInfo {
            execution: Some(WorkflowExecution {
                workflow_id: "wf-1".into(),
                run_id: "run-1".into(),
            }),
            r#type: Some(WorkflowType {
                name: "Greeter".into(),
            }),
            task_queue: "tq".into(),
            status: ProtoStatus::Completed as i32,
            history_length: 12,
            start_time: Some(prost_wkt_types::Timestamp {
                seconds: 100,
                nanos: 0,
            }),
            ..Default::default()
        };
        let row = row_from("payments", info);
        assert_eq!(row.namespace, "payments");
        assert_eq!(row.workflow_id, "wf-1");
        assert_eq!(row.workflow_type, "Greeter");
        assert_eq!(row.status, WorkflowStatus::Completed);
        assert_eq!(row.start_time, Some(100_000));
        assert_eq!(row.history_length, 12);
    }

    #[test]
    fn a_page_knows_whether_more_exist() {
        assert!(!WorkflowPage::default().has_more());
        assert!(
            WorkflowPage {
                next_page_token: vec![1],
                ..Default::default()
            }
            .has_more()
        );
    }
}
