//! Integration tests against a real Temporal frontend.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    enums::v1::HistoryEventFilterType,
    workflowservice::v1::{
        CountWorkflowExecutionsRequest, GetWorkflowExecutionHistoryRequest, ListNamespacesRequest,
        ListWorkflowExecutionsRequest,
    },
};
use tmprl_client::{Conn, ProfileRef};

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

#[tokio::test]
async fn connects_and_resolves_a_namespace() {
    let Some(c) = conn().await else { return };
    assert!(
        !c.namespace().is_empty(),
        "namespace must never resolve to empty"
    );
}

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

    assert!(
        page.rows.len() <= 10,
        "page_size must be respected: asked for 10, got {}",
        page.rows.len()
    );
}

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
             stays correct, but server-side ordering is now available if wanted."
        ),
    }
}

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
        "total is {} but no status group decoded, the GROUP BY payload \
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

#[tokio::test]
async fn paging_a_fan_out_terminates_and_visits_each_row_once() {
    let Some(c) = conn().await else { return };

    let mut namespaces = Vec::new();
    let mut total = 0usize;
    let mut biggest = 0usize;
    for ns in c.list_namespaces().await.expect("list_namespaces") {
        let n = c
            .count_workflows_by_status(&ns.name, "")
            .await
            .expect("count")
            .total as usize;
        if n > 0 {
            namespaces.push(ns.name);
            total += n;
            biggest = biggest.max(n);
        }
    }
    if namespaces.len() < 2 || total < 3 {
        eprintln!(
            "SKIP: need workflows in >= 2 namespaces to exercise the fan-out, \
             found {} namespace(s) holding {total}",
            namespaces.len()
        );
        return;
    }

    const ROUNDS: usize = 6;
    let page_size = biggest.div_ceil(ROUNDS).max(1) as i32;

    let mut seen: Vec<(String, String)> = Vec::new();
    let (mut rows, mut tokens) = c
        .list_workflows_across(&namespaces, "", page_size)
        .await
        .expect("first page");

    let limit = ROUNDS * 4 + 16;
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
            "paging did not terminate after {rounds} rounds for {total} workflows at \
             page size {page_size}, an exhausted namespace is being restarted"
        );
        let next = c
            .continue_workflows_across(&tokens, "", page_size)
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

#[tokio::test]
async fn typed_history_normalises_a_real_workflow() {
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let page = c
        .list_workflows(&ns, "", 1, Vec::new())
        .await
        .expect("list");
    let Some(row) = page.rows.first() else {
        eprintln!("SKIP: no workflows to read a history from");
        return;
    };

    let hist = c
        .get_history(&ns, &row.workflow_id, &row.run_id, 100, Vec::new())
        .await
        .expect("get_history");

    assert!(!hist.events.is_empty(), "a history is never empty");

    let first = &hist.events[0];
    assert_eq!(first.id, 1, "history starts at event 1");
    assert_eq!(
        first.name, "WORKFLOW_EXECUTION_STARTED",
        "event 1 is always WorkflowExecutionStarted"
    );
    assert_eq!(first.group, tmprl_core::history::GroupRef::Workflow);
    assert_eq!(first.role, tmprl_core::history::Role::Opens);
    assert!(
        !first.subject.is_empty(),
        "the start event names the workflow type"
    );
    assert_eq!(first.subject, row.workflow_type);

    let ids: Vec<i64> = hist.events.iter().map(|e| e.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "events must arrive in history order");

    assert!(
        hist.events.iter().all(|e| e.time.is_some()),
        "every history event is timestamped"
    );
}

#[tokio::test]
async fn a_real_history_groups_consistently_with_the_list() {
    use tmprl_core::history::{GroupRef, Outcome, group_events};

    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let page = c
        .list_workflows(&ns, "ExecutionStatus = 'Terminated'", 1, Vec::new())
        .await
        .expect("list terminated");
    let Some(row) = page.rows.first() else {
        eprintln!("SKIP: no terminated workflow to check a terminal outcome against");
        return;
    };

    let hist = c
        .get_history(&ns, &row.workflow_id, &row.run_id, 1000, Vec::new())
        .await
        .expect("get_history");
    let groups = group_events(&hist.events);

    let wf = groups
        .iter()
        .find(|g| g.key == GroupRef::Workflow)
        .expect("every history has a workflow group");
    assert_eq!(
        wf.outcome,
        Outcome::Terminated,
        "the grouped outcome must match the status the list reported"
    );
    assert!(!wf.is_open(), "a terminated workflow is not still running");
    assert_eq!(wf.first_event(), Some(1));

    let mut grouped: Vec<i64> = groups
        .iter()
        .flat_map(|g| g.events.iter().copied())
        .collect();
    grouped.sort_unstable();
    let mut all: Vec<i64> = hist.events.iter().map(|e| e.id).collect();
    all.sort_unstable();
    assert_eq!(
        grouped, all,
        "grouping must account for every event exactly once"
    );
}

#[tokio::test]
async fn a_task_queue_nothing_polls_is_still_described() {
    use tmprl_core::taskqueue::TaskQueueKind;
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();
    for kind in [TaskQueueKind::Workflow, TaskQueueKind::Activity] {
        let health = c
            .describe_task_queue(&ns, "tmprl-live-nothing-polls-this", kind)
            .await
            .expect("DescribeTaskQueue");
        assert_eq!(health.pollers, 0);
        assert_eq!(health.last_poll_ms, None);
        match health.backlog {
            Some(n) => assert_eq!(n, 0, "nothing was ever sent to it"),
            None => eprintln!("NOTE: this server reports no task queue stats"),
        }
    }
}

#[tokio::test]
async fn the_close_event_is_the_one_event_a_closed_workflow_ended_on() {
    use tmprl_core::history::{GroupRef, Outcome, Role};

    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();

    let closed = c
        .list_workflows(&ns, "ExecutionStatus = 'Terminated'", 1, Vec::new())
        .await
        .expect("list terminated");
    match closed.rows.first() {
        Some(row) => {
            let event = c
                .close_event(&ns, &row.workflow_id, &row.run_id)
                .await
                .expect("close_event")
                .expect("a terminated workflow has a closing event");
            assert_eq!(event.group, GroupRef::Workflow);
            assert_eq!(event.role, Role::Closes);
            assert_eq!(event.outcome, Outcome::Terminated);
        }
        None => eprintln!("SKIP: no terminated workflow to read a closing event from"),
    }

    let open = c
        .list_workflows(&ns, "ExecutionStatus = 'Running'", 1, Vec::new())
        .await
        .expect("list running");
    match open.rows.first() {
        Some(row) => assert_eq!(
            c.close_event(&ns, &row.workflow_id, &row.run_id)
                .await
                .expect("close_event"),
            None,
            "a running workflow has not closed"
        ),
        None => eprintln!("SKIP: no running workflow to check for the absence of one"),
    }
}

#[tokio::test]
async fn a_query_nothing_answers_fails_within_the_deadline() {
    use tmprl_core::fault::Code;
    let Some(c) = conn().await else { return };
    let ns = c.namespace().to_string();
    let page = c
        .list_workflows(&ns, "ExecutionStatus = 'Running'", 1, Vec::new())
        .await
        .expect("ListWorkflowExecutions");
    let Some(row) = page.rows.first() else {
        eprintln!("SKIP: no running workflow to query. Seed one with `temporal workflow start`.");
        return;
    };

    let started = std::time::Instant::now();
    let err = c
        .query_workflow(&ns, &row.workflow_id, &row.run_id, "__stack_trace")
        .await
        .expect_err("no worker polls the CI task queue");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(
        err.code,
        Code::DeadlineExceeded,
        "a missing worker must not read as a missing handler, or as a cancel: {err}"
    );
}

#[tokio::test]
async fn every_schedule_is_listed_across_pages() {
    let Some(c) = conn().await else { return };
    let namespace = c.namespace().to_string();
    let one_page = c
        .list_schedules(&namespace, 1000, Vec::new())
        .await
        .expect("list schedules");
    if one_page.rows.len() < 2 {
        eprintln!(
            "SKIP: need >= 2 schedules to exercise paging, found {}",
            one_page.rows.len()
        );
        return;
    }
    let paged = c
        .list_all_schedules(&[namespace], 1)
        .await
        .expect("list every schedule");
    let ids = |rows: &[tmprl_core::ScheduleRow]| {
        let mut ids: Vec<String> = rows.iter().map(|r| r.schedule_id.clone()).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&paged), ids(&one_page.rows));
}
