//! The dashboard.

use super::*;
use crate::app::{Msg, Screen};
use tmprl_core::ScheduleRow;
use tmprl_core::dashboard::SourceData;
use tmprl_core::fault::{Code, Fault};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn reply(app: &mut App, source: usize, result: Result<SourceData, Fault>) {
    app.handle(Msg::Dashboard {
        view: app.tabs.current().focused(),
        generation: app.view.generation,
        source,
        result,
    });
}

fn failures(more: bool) -> SourceData {
    let mut refund = wf(
        "default",
        "refund-9",
        WorkflowStatus::TimedOut,
        now() - 60_000,
    );
    refund.workflow_type = "RefundWorkflow".into();
    SourceData::Workflows {
        rows: vec![
            wf(
                "default",
                "order-1001",
                WorkflowStatus::Failed,
                now() - 45_000,
            ),
            wf(
                "default",
                "order-1002",
                WorkflowStatus::Failed,
                now() - 7_200_000,
            ),
            refund,
        ],
        more,
    }
}

pub(super) fn empty_dashboard() -> App {
    let mut app = app_with_rows();
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    app
}

pub(super) fn app_with_dashboard() -> App {
    let mut app = empty_dashboard();
    reply(
        &mut app,
        0,
        Ok(SourceData::Counts(StatusCounts::new(
            7,
            [(WorkflowStatus::Running, 4), (WorkflowStatus::Failed, 3)],
        ))),
    );
    reply(&mut app, 1, Ok(failures(false)));
    reply(
        &mut app,
        2,
        Ok(SourceData::Workflows {
            rows: vec![wf("default", "order-2000", WorkflowStatus::Running, now())],
            more: false,
        }),
    );
    reply(
        &mut app,
        3,
        Ok(SourceData::Schedules(vec![
            ScheduleRow {
                namespace: "default".into(),
                schedule_id: "nightly-recon".into(),
                workflow_type: "Reconcile".into(),
                paused: false,
                notes: String::new(),
                spec: "0 2 * * *".into(),
                next_run: Some(now() + 3_600_000),
                recent_runs: 2,
            },
            ScheduleRow {
                namespace: "default".into(),
                schedule_id: "held".into(),
                workflow_type: "Reconcile".into(),
                paused: true,
                notes: String::new(),
                spec: "every 1h".into(),
                next_run: None,
                recent_runs: 0,
            },
        ])),
    );
    app
}

#[test]
fn the_builtin_layout_draws_every_panel_with_its_items() {
    let out = draw(&mut app_with_dashboard(), 120, 30);
    for title in [
        "Status",
        "Recent failures",
        "Failing types",
        "Task queues",
        "Schedules",
    ] {
        assert!(out.contains(title), "no `{title}` panel:\n{out}");
    }
    assert!(out.contains("● 4 Running"), "{out}");
    assert!(out.contains("✗ 3 Failed"), "{out}");
    assert!(out.contains("7 total"), "{out}");
    assert!(out.contains("order-1001"), "{out}");
    assert!(out.contains("2  CheckoutWorkflow"), "the tally:\n{out}");
    assert!(out.contains("1  orders"), "the queue:\n{out}");
    assert!(out.contains("nightly-recon"), "{out}");
    assert!(out.contains("paused"), "{out}");
    assert!(!out.contains("hidden"), "{out}");
    assert!(out.lines().next().unwrap().contains("5 panels"), "{out}");
}

#[test]
fn the_newest_failure_is_listed_first() {
    let out = draw(&mut app_with_dashboard(), 120, 30);
    let (first, _) = position(&out, "order-1001");
    let (later, _) = position(&out, "order-1002");
    assert!(first < later, "{out}");
}

#[test]
fn a_panel_says_whether_it_is_waiting_failed_or_empty() {
    let mut app = empty_dashboard();
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("loading…"), "{out}");

    reply(
        &mut app,
        1,
        Err(Fault::rpc(
            "ListWorkflowExecutions",
            Code::Unavailable,
            "transport error",
        )),
    );
    reply(&mut app, 3, Ok(SourceData::Schedules(Vec::new())));
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("ListWorkflowExecutions failed"), "{out}");
    assert!(out.contains("no schedules"), "{out}");
}

#[test]
fn a_panel_showing_an_old_answer_or_a_sample_says_so() {
    let mut app = app_with_dashboard();
    reply(&mut app, 1, Ok(failures(true)));
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("of 3 sampled"), "{out}");

    reply(
        &mut app,
        1,
        Err(Fault::rpc(
            "ListWorkflowExecutions",
            Code::Unavailable,
            "down",
        )),
    );
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("stale"), "{out}");
    assert!(out.contains("order-1001"), "the old rows stay:\n{out}");
}

#[test]
fn panels_that_do_not_fit_are_counted_not_squeezed() {
    let mut app = app_with_dashboard();
    let out = draw(&mut app, 40, 8);
    assert!(out.contains("Status"), "{out}");
    assert!(out.contains("+3 hidden"), "{out}");
    assert!(!out.contains("Schedules"), "{out}");
}

#[test]
fn the_dashboard_scrolls_to_the_panel_the_cursor_is_in() {
    let mut app = app_with_dashboard();
    app.run("motion.bottom", None);
    let out = draw(&mut app, 40, 8);
    assert!(out.contains("Schedules"), "{out}");
    assert!(out.contains("held"), "{out}");
}

#[test]
fn a_pane_too_small_for_a_box_shows_the_cursors_panel_bare() {
    let mut app = app_with_dashboard();
    app.run("motion.down", Some(2));
    let out = draw(&mut app, 22, 6);
    assert!(out.contains("order-1"), "{out}");
    assert!(!out.contains('┌'), "{out}");
}

#[test]
fn the_cursor_row_is_marked_in_its_panel() {
    let mut app = app_with_dashboard();
    app.run("motion.down", Some(2));
    let buf = {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| render(f, &mut app)).unwrap();
        term.backend().buffer().clone()
    };
    let out = draw(&mut app, 120, 30);
    let (row, col) = position(&out, "order-1001");
    let (other_row, other_col) = position(&out, "order-1002");
    assert_ne!(
        buf[(col as u16, row as u16)].style(),
        buf[(other_col as u16, other_row as u16)].style()
    );
}

#[test]
fn a_dashboard_in_a_split_draws_beside_another_screen() {
    let mut app = app_with_dashboard();
    app.run("window.split-right", None);
    app.run("nav.workflows", None);
    let out = draw(&mut app, 160, 30);
    assert!(out.contains("Recent failures"), "{out}");
    assert!(out.contains("order-1001"), "{out}");
}
