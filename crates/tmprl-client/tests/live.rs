//! Integration tests against a real Temporal frontend.
//!
//! These assert the contracts the rest of tmprl is built on — the ones that would
//! silently change under a `temporalio-client` bump. They need a server:
//!
//!     temporal server start-dev
//!     cargo test -p tmprl-client
//!
//! With no server reachable the tests **skip** rather than fail, so `cargo test` stays
//! green on a machine that has never run Temporal. Set `TMPRL_REQUIRE_SERVER=1` in CI to
//! turn that skip into a hard failure.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    enums::v1::HistoryEventFilterType,
    workflowservice::v1::{
        CountWorkflowExecutionsRequest, GetWorkflowExecutionHistoryRequest, ListNamespacesRequest,
        ListWorkflowExecutionsRequest,
    },
};
use tmprl_client::{Conn, ProfileRef};

/// Connect, or return `None` to skip. Keeps every test's preamble to one line.
async fn conn() -> Option<Conn> {
    match Conn::connect(&ProfileRef::default()).await {
        Ok(c) => Some(c),
        Err(e) => {
            if std::env::var("TMPRL_REQUIRE_SERVER").is_ok() {
                panic!("TMPRL_REQUIRE_SERVER is set but connecting failed: {e}");
            }
            eprintln!("SKIP: no Temporal server reachable ({e}). Run `temporal server start-dev`.");
            None
        }
    }
}

/// The profile loader resolves a namespace even with no config file present — tmprl must
/// never come up namespace-less, because every RPC needs one.
#[tokio::test]
async fn connects_and_resolves_a_namespace() {
    let Some(c) = conn().await else { return };
    assert!(
        !c.namespace().is_empty(),
        "namespace must never resolve to empty"
    );
}

/// The namespace switcher reads this. `default` is guaranteed on a dev server.
#[tokio::test]
async fn lists_namespaces() {
    let Some(c) = conn().await else { return };
    let resp = c
        .wf()
        .list_namespaces(Request::new(ListNamespacesRequest {
            page_size: 50,
            ..Default::default()
        }))
        .await
        .expect("ListNamespaces")
        .into_inner();

    let names: Vec<_> = resp
        .namespaces
        .iter()
        .filter_map(|n| n.namespace_info.as_ref().map(|i| i.name.as_str()))
        .collect();
    assert!(
        names.contains(&"default"),
        "expected `default` in {names:?}"
    );
}

/// The list header shows a count from `CountWorkflowExecutions` while the table itself
/// comes from `ListWorkflowExecutions`. If those two ever disagree about what an empty
/// query means, the header lies — so pin it.
#[tokio::test]
async fn count_agrees_with_list() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let count = c
        .wf()
        .count_workflow_executions(Request::new(CountWorkflowExecutionsRequest {
            namespace: ns.clone(),
            query: String::new(),
        }))
        .await
        .expect("CountWorkflowExecutions")
        .into_inner()
        .count;

    let listed = c
        .wf()
        .list_workflow_executions(Request::new(ListWorkflowExecutionsRequest {
            namespace: ns,
            page_size: 1000,
            ..Default::default()
        }))
        .await
        .expect("ListWorkflowExecutions")
        .into_inner()
        .executions
        .len();

    assert_eq!(
        count as usize, listed,
        "header count and table row count must agree for an empty query"
    );
}

/// Paging drives infinite scroll in the workflow table. A page smaller than the result set
/// must hand back a token; the last page must not.
#[tokio::test]
async fn paginates_with_a_token() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let total = c
        .wf()
        .count_workflow_executions(Request::new(CountWorkflowExecutionsRequest {
            namespace: ns.clone(),
            query: String::new(),
        }))
        .await
        .expect("count")
        .into_inner()
        .count;

    if total < 2 {
        eprintln!("SKIP: need >= 2 workflows to exercise paging, found {total}");
        return;
    }

    let first = c
        .wf()
        .list_workflow_executions(Request::new(ListWorkflowExecutionsRequest {
            namespace: ns.clone(),
            page_size: 1,
            ..Default::default()
        }))
        .await
        .expect("page 1")
        .into_inner();

    assert_eq!(first.executions.len(), 1);
    assert!(
        !first.next_page_token.is_empty(),
        "a partial page must return a continuation token"
    );

    let second = c
        .wf()
        .list_workflow_executions(Request::new(ListWorkflowExecutionsRequest {
            namespace: ns,
            page_size: 1,
            next_page_token: first.next_page_token,
            ..Default::default()
        }))
        .await
        .expect("page 2")
        .into_inner();

    assert_eq!(second.executions.len(), 1);
    let a = &first.executions[0].execution.as_ref().unwrap().run_id;
    let b = &second.executions[0].execution.as_ref().unwrap().run_id;
    assert_ne!(a, b, "page 2 must not repeat page 1");
}

/// The history view and follow mode both read this. Event 1 of every workflow is always
/// `WorkflowExecutionStarted` — that invariant is what lets the header render before the
/// rest of the history has paged in.
#[tokio::test]
async fn reads_history_starting_at_execution_started() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let list = c
        .wf()
        .list_workflow_executions(Request::new(ListWorkflowExecutionsRequest {
            namespace: ns.clone(),
            page_size: 1,
            ..Default::default()
        }))
        .await
        .expect("list")
        .into_inner();

    let Some(exec) = list.executions.first().and_then(|e| e.execution.clone()) else {
        eprintln!("SKIP: no workflows to read a history from");
        return;
    };

    let hist = c
        .wf()
        .get_workflow_execution_history(Request::new(GetWorkflowExecutionHistoryRequest {
            namespace: ns,
            execution: Some(exec),
            maximum_page_size: 100,
            // false, or this call long-polls and the test hangs. `true` is follow mode.
            wait_new_event: false,
            history_event_filter_type: HistoryEventFilterType::AllEvent as i32,
            ..Default::default()
        }))
        .await
        .expect("GetWorkflowExecutionHistory")
        .into_inner();

    let events = hist.history.map(|h| h.events).unwrap_or_default();
    assert!(!events.is_empty(), "history must not be empty");
    assert_eq!(events[0].event_id, 1, "history must start at event id 1");
    assert_eq!(
        events[0].event_type().as_str_name(),
        "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED",
        "event 1 is always WorkflowExecutionStarted"
    );
}
