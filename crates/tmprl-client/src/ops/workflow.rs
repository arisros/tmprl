//! Listing and counting workflow executions.
//!
//! Two RPCs back the workflow table. `ListWorkflowExecutions` returns the rows, one page at
//! a time; `CountWorkflowExecutions` with a `GROUP BY` returns the header tallies in a
//! single cheap call, instead of the table having to page to exhaustion to know how many
//! workflows are failing.
//!
//! Both take an explicit namespace rather than using the connection's own. A connection is
//! bound to one namespace, but clones share a single HTTP/2 channel, so a fan-out across
//! namespaces is one connection and N requests — see `list_workflows_across`.

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

/// One page of the workflow table, plus the token that fetches the next.
#[derive(Debug, Clone, Default)]
pub struct WorkflowPage {
    pub rows: Vec<WorkflowRow>,
    /// Empty on the last page. This is opaque server state — never construct one.
    pub next_page_token: Vec<u8>,
}

impl WorkflowPage {
    /// Whether another page exists. Infinite scroll stops asking when this goes false.
    pub fn has_more(&self) -> bool {
        !self.next_page_token.is_empty()
    }
}

impl Conn {
    /// One page of executions matching `query`, newest first.
    ///
    /// This deliberately does *not* page to exhaustion the way `list_namespaces` does. A
    /// namespace list is tens of rows; a workflow list is unbounded, and draining it would
    /// hang the interface on any real cluster.
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
            .map_err(|s| OpError::rpc("ListWorkflowExecutions", s))?
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

    /// The same page from several namespaces at once, merged newest-first.
    ///
    /// Returned alongside the rows is each namespace's continuation token, because the
    /// namespaces exhaust at different points and a single merged token cannot express
    /// that. A namespace whose token comes back empty has no more pages.
    pub async fn list_workflows_across(
        &self,
        namespaces: &[String],
        query: &str,
        page_size: i32,
        tokens: &[(String, Vec<u8>)],
    ) -> Result<(Vec<WorkflowRow>, Vec<(String, Vec<u8>)>), OpError> {
        let token_for = |ns: &str| -> Vec<u8> {
            tokens
                .iter()
                .find(|(n, _)| n == ns)
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        };

        // One request per namespace, in flight together. They share the connection's
        // channel, so this costs N streams rather than N connections.
        let pages = futures_util::future::try_join_all(namespaces.iter().map(|ns| {
            let token = token_for(ns);
            async move {
                self.list_workflows(ns, query, page_size, token)
                    .await
                    .map(|p| (ns.clone(), p))
            }
        }))
        .await?;

        let mut next = Vec::new();
        let mut all = Vec::new();
        for (ns, page) in pages {
            if page.has_more() {
                next.push((ns, page.next_page_token));
            }
            all.push(page.rows);
        }
        Ok((merge_by_start_time(all), next))
    }

    /// Per-status counts for the list header.
    ///
    /// One `GROUP BY` call rather than one call per status. The grouped counts are
    /// approximate by design on large clusters — Temporal says so — which is why the total
    /// is taken from the response rather than summed from the groups.
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
            .map_err(|s| OpError::rpc("CountWorkflowExecutions", s))?
            .into_inner();

        let counts = resp.groups.into_iter().filter_map(|g| {
            let status = g.group_values.first().and_then(status_from_payload)?;
            Some((status, g.count))
        });
        Ok(StatusCounts::new(resp.count, counts))
    }
}

impl Conn {
    /// Header counts summed over a fan-out.
    ///
    /// A header that counted only the first of several namespaces would be quietly wrong,
    /// which is worse than having no header at all.
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

/// Map the protobuf row onto the domain row.
fn row_from(namespace: &str, e: WorkflowExecutionInfo) -> WorkflowRow {
    // `status()` borrows `e`, so resolve it before the string fields are moved out.
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

/// Exhaustive on purpose. When Temporal adds an execution status this stops compiling,
/// which is the moment we want to hear about it — a `_` arm would render the new status as
/// `Unspecified` and nobody would notice for a release or two.
fn status_from_proto(s: ProtoStatus) -> WorkflowStatus {
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

/// A `GROUP BY ExecutionStatus` group value is a `json/plain` Keyword payload whose data is
/// the quoted status name, e.g. `"Running"`. Parsing tolerates the quotes.
fn status_from_payload(
    p: &temporalio_common::protos::temporal::api::common::v1::Payload,
) -> Option<WorkflowStatus> {
    WorkflowStatus::parse(std::str::from_utf8(&p.data).ok()?)
}

fn epoch_millis(t: prost_wkt_types::Timestamp) -> i64 {
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
        // Pairwise distinct: a copy-paste slip in the match would collapse two statuses.
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
        // The exact bytes a dev server returns for `GROUP BY ExecutionStatus`.
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
        // Defensive: every field of WorkflowExecutionInfo is optional on the wire, and a
        // panic here would take down the whole table for one malformed row.
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
