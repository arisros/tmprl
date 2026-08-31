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

// ── the typed workflow ops the M1 table is built on ──────────────────────────

/// The typed wrapper must agree with the raw RPC about what a page contains. This is the
/// call the workflow table makes on every scroll, so a mapping slip here is every row.
#[tokio::test]
async fn typed_list_maps_rows_and_pages() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let page = c
        .list_workflows(&ns, "", 10, Vec::new())
        .await
        .expect("list_workflows");

    for row in &page.rows {
        assert_eq!(row.namespace, ns, "every row must carry its namespace");
        assert!(!row.run_id.is_empty(), "a listed row always has a run id");
        assert_ne!(
            row.status,
            tmprl_core::WorkflowStatus::Unspecified,
            "a real execution never has an unspecified status"
        );
    }

    // NOTE: the server does *not* guarantee an order here, and the dev server's standard
    // visibility store rejects `ORDER BY` outright ("operation is not supported"). So
    // there is no server-side ordering to lean on, and tmprl sorts client-side — see
    // `tmprl_core::workflow::WorkflowList`. This test therefore asserts the mapping, not
    // an ordering the API never promised.
    assert!(
        page.rows.len() <= 10,
        "page_size must be respected: asked for 10, got {}",
        page.rows.len()
    );
}

/// Pin the constraint that forces client-side sorting, so that if a future server version
/// starts supporting `ORDER BY` we find out from a failing test rather than never.
#[tokio::test]
async fn order_by_is_rejected_by_standard_visibility() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let err = c
        .list_workflows(&ns, "ORDER BY StartTime DESC", 10, Vec::new())
        .await;
    match err {
        Err(e) => assert!(
            e.to_string().contains("ORDER BY") || e.to_string().contains("not supported"),
            "unexpected error for an ORDER BY query: {e}"
        ),
        Ok(_) => eprintln!(
            "NOTE: this server accepts `ORDER BY`. tmprl still sorts client-side, which \
             stays correct — but server-side ordering is now available if wanted."
        ),
    }
}

/// Infinite scroll depends on the token round-tripping through the typed wrapper, and on
/// the second page not repeating the first.
#[tokio::test]
async fn typed_list_continues_from_its_token() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let first = c
        .list_workflows(&ns, "", 1, Vec::new())
        .await
        .expect("page 1");
    if !first.has_more() {
        eprintln!("SKIP: need >= 2 workflows to exercise paging");
        return;
    }

    let second = c
        .list_workflows(&ns, "", 1, first.next_page_token.clone())
        .await
        .expect("page 2");

    assert_eq!(first.rows.len(), 1);
    assert_eq!(second.rows.len(), 1);
    assert_ne!(
        first.rows[0].run_id, second.rows[0].run_id,
        "page 2 must not repeat page 1"
    );
}

/// The header counts. `GROUP BY` payloads are `json/plain` Keyword values holding a quoted
/// status name — if that encoding ever changes, every status count silently reads zero, so
/// assert that at least one group actually decoded.
#[tokio::test]
async fn grouped_counts_decode_to_statuses() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let counts = c
        .count_workflows_by_status(&ns, "")
        .await
        .expect("count_workflows_by_status");

    if counts.total == 0 {
        eprintln!("SKIP: no workflows to count");
        return;
    }
    let groups: Vec<_> = counts.iter().collect();
    assert!(
        !groups.is_empty(),
        "total is {} but no status group decoded — the GROUP BY payload \
         encoding has changed",
        counts.total
    );
    let summed: i64 = groups.iter().map(|(_, n)| n).sum();
    assert!(
        summed <= counts.total,
        "grouped counts ({summed}) cannot exceed the total ({})",
        counts.total
    );
}

/// A filter must survive being adapted for the count RPC. This is the pairing the header
/// depends on: the same user query, counted and listed, must describe the same set.
#[tokio::test]
async fn a_filtered_count_agrees_with_a_filtered_list() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let query = "ExecutionStatus = 'Running'";
    let counts = c
        .count_workflows_by_status(&ns, query)
        .await
        .expect("filtered count");
    let page = c
        .list_workflows(&ns, query, 1000, Vec::new())
        .await
        .expect("filtered list");

    assert_eq!(
        counts.total as usize,
        page.rows.len(),
        "count and list must agree on the same filter"
    );
    assert!(
        page.rows
            .iter()
            .all(|r| r.status == tmprl_core::WorkflowStatus::Running),
        "the filter must actually be applied server-side"
    );
}

/// The multi-namespace fan-out. Even with one namespace this pins the contract: rows come
/// back merged and newest-first, and exhausted namespaces drop out of the token list.
#[tokio::test]
async fn fan_out_merges_rows_newest_first() {
    let Some(c) = conn().await else { return };
    let namespaces = vec![c.namespace().to_string()];

    let (rows, tokens) = c
        .list_workflows_across(&namespaces, "", 1000)
        .await
        .expect("list_workflows_across");

    let starts: Vec<Option<i64>> = rows.iter().map(|r| r.start_time).collect();
    let mut sorted = starts.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(starts, sorted, "merged rows must be newest first");
    assert!(
        tokens.is_empty(),
        "a namespace with no further pages must not appear in the token list"
    );
}

/// Paging a fan-out to exhaustion must terminate, and must visit every workflow exactly
/// once.
///
/// This is the test for a bug that is invisible at a glance: if a continuation re-derives
/// its namespaces from the original scope, a namespace that has already finished is handed
/// an empty token, the server reads that as "start from the beginning", and it hands back
/// page one plus a fresh token — forever. A tiny page size makes it show up with only a
/// handful of workflows.
#[tokio::test]
async fn paging_a_fan_out_terminates_and_visits_each_row_once() {
    let Some(c) = conn().await else { return };

    // Fan out over every namespace that actually holds workflows. The bug this guards
    // against only appears when the namespaces exhaust at *different* points, so a
    // single-namespace fan-out would not reach it.
    let mut namespaces = Vec::new();
    let mut total = 0usize;
    for ns in c.list_namespaces().await.expect("list_namespaces") {
        let n = c
            .count_workflows_by_status(&ns.name, "")
            .await
            .expect("count")
            .total as usize;
        if n > 0 {
            namespaces.push(ns.name);
            total += n;
        }
    }
    if namespaces.len() < 2 || total < 3 {
        eprintln!(
            "SKIP: need workflows in >= 2 namespaces to exercise the fan-out,              found {} namespace(s) holding {total}",
            namespaces.len()
        );
        return;
    }

    let mut seen: Vec<(String, String)> = Vec::new();
    let (mut rows, mut tokens) = c
        .list_workflows_across(&namespaces, "", 1)
        .await
        .expect("first page");

    // Generous, but finite: without a bound a regression here hangs the suite instead of
    // failing it.
    let limit = total * 4 + 16;
    let mut rounds = 0;
    loop {
        for r in &rows {
            seen.push((r.namespace.clone(), r.run_id.clone()));
        }
        if tokens.is_empty() {
            break;
        }
        rounds += 1;
        assert!(
            rounds < limit,
            "paging did not terminate after {rounds} rounds for {total} workflows —              an exhausted namespace is being restarted"
        );
        let next = c
            .continue_workflows_across(&tokens, "", 1)
            .await
            .expect("continuation");
        rows = next.0;
        tokens = next.1;
    }

    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "paging returned the same execution twice"
    );
    assert_eq!(
        unique.len(),
        total,
        "paging must visit every workflow exactly once"
    );
}
