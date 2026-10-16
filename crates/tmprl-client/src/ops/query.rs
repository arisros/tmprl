//! `QueryWorkflow`, answered by a worker rather than by the server.

use std::time::Duration;

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::WorkflowExecution, query::v1::WorkflowQuery,
    workflowservice::v1::QueryWorkflowRequest,
};
use tmprl_core::fault::Code;
use tmprl_core::payload::Payload;

use super::OpError;
use super::history::convert;
use super::workflow::status_from_proto;
use crate::Conn;

/// With no worker polling the task queue the server holds a query until its own deadline,
/// which is longer than anyone waits at a keyboard.
const QUERY_DEADLINE: Duration = Duration::from_secs(15);

impl Conn {
    /// Ask one handler, with no arguments.
    pub async fn query_workflow(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
        query_type: &str,
    ) -> Result<Vec<Payload>, OpError> {
        let mut req = Request::new(QueryWorkflowRequest {
            namespace: namespace.to_string(),
            execution: Some(WorkflowExecution {
                workflow_id: workflow_id.to_string(),
                run_id: run_id.to_string(),
            }),
            query: Some(WorkflowQuery {
                query_type: query_type.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });
        req.set_timeout(QUERY_DEADLINE);
        let resp = self
            .wf()
            .query_workflow(req)
            .await
            .map_err(|s| {
                // tonic reports its own deadline as `Cancelled`, which reads as if someone
                // had cancelled the query.
                if s.code() == temporalio_client::tonic::Code::Cancelled
                    && s.message() == "Timeout expired"
                {
                    OpError::rpc(
                        "QueryWorkflow",
                        Code::DeadlineExceeded,
                        format!(
                            "no worker answered within {}s, is one polling this workflow's task queue?",
                            QUERY_DEADLINE.as_secs()
                        ),
                    )
                } else {
                    super::rpc("QueryWorkflow", s)
                }
            })?
            .into_inner();
        if let Some(rejected) = resp.query_rejected {
            return Err(OpError::rpc(
                "QueryWorkflow",
                Code::FailedPrecondition,
                format!(
                    "rejected, the workflow is {:?}",
                    status_from_proto(rejected.status())
                ),
            ));
        }
        Ok(resp
            .query_result
            .map(|p| p.payloads.into_iter().map(convert).collect())
            .unwrap_or_default())
    }
}
