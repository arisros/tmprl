//! What the dashboard tests share.

use super::*;
use crate::fault::{Code, Fault};
use crate::schedule::ScheduleRow;
use crate::workflow::{WorkflowRow, WorkflowStatus};

pub const NOW: i64 = 1_700_000_000_000;

pub fn scope() -> Vec<String> {
    vec!["default".to_string()]
}

pub fn wf(run: &str, kind: &str, queue: &str, start: i64) -> WorkflowRow {
    WorkflowRow {
        namespace: "default".into(),
        workflow_id: format!("wf-{run}"),
        run_id: run.into(),
        workflow_type: kind.into(),
        task_queue: queue.into(),
        status: WorkflowStatus::Failed,
        start_time: Some(start),
        close_time: None,
        history_length: 0,
    }
}

pub fn schedule(id: &str, paused: bool, next_run: Option<i64>) -> ScheduleRow {
    ScheduleRow {
        namespace: "default".into(),
        schedule_id: id.into(),
        workflow_type: "Nightly".into(),
        paused,
        notes: String::new(),
        spec: String::new(),
        next_run,
        recent_runs: 0,
    }
}

pub fn rows(rows: Vec<WorkflowRow>) -> Result<SourceData, Fault> {
    Ok(SourceData::Workflows { rows, more: false })
}

pub fn fault() -> Fault {
    Fault::rpc("ListWorkflowExecutions", Code::Unavailable, "down")
}

pub fn titles(layout: &Layout) -> Vec<Vec<&str>> {
    layout
        .rows
        .iter()
        .map(|row| row.panels.iter().map(PanelSpec::title).collect())
        .collect()
}
