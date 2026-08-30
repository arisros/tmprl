//! M0 spike: prove the RPC shapes the rest of tmprl is built on.
//!
//! Run against a local dev server:
//!     temporal server start-dev
//!     cargo run -p tmprl-client --example spike
//!
//! It deliberately exercises the four calls that M1 and M2 depend on:
//!   - ListNamespaces          (namespace switcher)
//!   - CountWorkflowExecutions (status counts in the list header)
//!   - ListWorkflowExecutions  (the workflow table + its paging token)
//!   - GetWorkflowExecutionHistory (history view; `wait_new_event` is follow mode)

use temporalio_client::tonic::Request;
use tmprl_client::{Conn, ProfileRef};

// The generated protos live under temporalio_common, re-exported through the client's deps.
use temporalio_common::protos::temporal::api::{
    common::v1::WorkflowExecution,
    enums::v1::HistoryEventFilterType,
    workflowservice::v1::{
        CountWorkflowExecutionsRequest, GetWorkflowExecutionHistoryRequest, ListNamespacesRequest,
        ListWorkflowExecutionsRequest,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let profile = ProfileRef {
        name: std::env::args().nth(1),
        config_file: None,
    };

    let conn = Conn::connect(&profile).await?;
    println!(
        "connected  profile={}  namespace={}",
        conn.profile(),
        conn.namespace()
    );

    // ── namespaces ───────────────────────────────────────────────────────────
    let mut wf = conn.wf();
    let ns = wf
        .list_namespaces(Request::new(ListNamespacesRequest {
            page_size: 50,
            ..Default::default()
        }))
        .await?
        .into_inner();
    println!("\nnamespaces ({}):", ns.namespaces.len());
    for n in &ns.namespaces {
        if let Some(info) = &n.namespace_info {
            println!("  - {}", info.name);
        }
    }

    // ── counts, the cheap header stat ────────────────────────────────────────
    let count = wf
        .count_workflow_executions(Request::new(CountWorkflowExecutionsRequest {
            namespace: conn.namespace().to_string(),
            query: String::new(),
        }))
        .await?
        .into_inner();
    println!(
        "\ntotal workflows in `{}`: {}",
        conn.namespace(),
        count.count
    );

    // ── the workflow table ───────────────────────────────────────────────────
    let list = wf
        .list_workflow_executions(Request::new(ListWorkflowExecutionsRequest {
            namespace: conn.namespace().to_string(),
            page_size: 10,
            query: String::new(),
            ..Default::default()
        }))
        .await?
        .into_inner();

    println!("\nworkflows ({} shown):", list.executions.len());
    let mut first: Option<WorkflowExecution> = None;
    for e in &list.executions {
        let exec = e.execution.clone().unwrap_or_default();
        println!(
            "  {:<10} {:<28} {}",
            format!("{:?}", e.status()),
            e.r#type.as_ref().map(|t| t.name.as_str()).unwrap_or("?"),
            exec.workflow_id
        );
        first.get_or_insert(exec);
    }
    println!("  next_page_token: {} bytes", list.next_page_token.len());

    // ── history: the shape follow mode long-polls on ─────────────────────────
    if let Some(exec) = first {
        let hist = wf
            .get_workflow_execution_history(Request::new(GetWorkflowExecutionHistoryRequest {
                namespace: conn.namespace().to_string(),
                execution: Some(exec.clone()),
                maximum_page_size: 100,
                // `wait_new_event: true` is what turns this into `tail -f`. Left false
                // here so the spike terminates.
                wait_new_event: false,
                history_event_filter_type: HistoryEventFilterType::AllEvent as i32,
                ..Default::default()
            }))
            .await?
            .into_inner();

        let events = hist.history.map(|h| h.events).unwrap_or_default();
        println!(
            "\nhistory of {} ({} events):",
            exec.workflow_id,
            events.len()
        );
        for ev in events.iter().take(15) {
            println!("  {:>4}  {:?}", ev.event_id, ev.event_type());
        }
    }

    Ok(())
}
