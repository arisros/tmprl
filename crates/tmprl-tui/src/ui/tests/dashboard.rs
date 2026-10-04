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
        timed: false,
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
fn a_namespace_with_something_of_everything_draws_every_panel() {
    let out = draw(&mut app_with_dashboard(), 120, 30);
    for title in [
        "Status",
        "Recent failures",
        "Failing types",
        "Task queues",
        "Paused schedules",
        "Upcoming schedules",
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
    assert!(
        out.lines().next().unwrap().contains("6 panels  auto 30s"),
        "{out}"
    );
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
    assert!(
        out.contains("Failing types"),
        "a failed panel stays to say so:\n{out}"
    );
    assert!(
        !out.contains("Schedules"),
        "known to be empty, so not drawn:\n{out}"
    );
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
    assert!(out.contains("+4 hidden"), "{out}");
    assert!(!out.contains("schedules"), "{out}");
}

#[test]
fn the_dashboard_scrolls_to_the_panel_the_cursor_is_in() {
    let mut app = app_with_dashboard();
    app.run("motion.bottom", None);
    let out = draw(&mut app, 40, 8);
    assert!(out.contains("Upcoming schedules"), "{out}");
    assert!(out.contains("nightly-recon"), "{out}");
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

#[test]
fn a_healthy_namespace_shows_what_is_running_in_place_of_failures() {
    let mut app = empty_dashboard();
    reply(
        &mut app,
        1,
        Ok(SourceData::Workflows {
            rows: Vec::new(),
            more: false,
        }),
    );
    reply(
        &mut app,
        2,
        Ok(SourceData::Workflows {
            rows: vec![wf("default", "order-2000", WorkflowStatus::Running, now())],
            more: false,
        }),
    );
    reply(&mut app, 3, Ok(SourceData::Schedules(Vec::new())));
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("Running"), "{out}");
    assert!(out.contains("order-2000"), "{out}");
    assert!(!out.contains("Recent failures"), "{out}");
    assert!(!out.contains("Schedules"), "{out}");
}

#[test]
fn a_namespace_with_nothing_in_it_says_so_in_every_panel() {
    let mut app = empty_dashboard();
    let empty = || {
        Ok(SourceData::Workflows {
            rows: Vec::new(),
            more: false,
        })
    };
    reply(&mut app, 0, Ok(SourceData::Counts(StatusCounts::default())));
    reply(&mut app, 1, empty());
    reply(&mut app, 2, empty());
    reply(&mut app, 3, Ok(SourceData::Schedules(Vec::new())));
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("no workflows"), "{out}");
    assert!(out.contains("no schedules"), "{out}");
    assert!(out.contains("Recent failures"), "{out}");
}

#[test]
fn a_task_queue_says_whether_anything_is_taking_its_work() {
    use tmprl_core::taskqueue::QueueHealth;
    let mut app = app_with_dashboard();
    let out = draw(&mut app, 120, 30);
    assert!(!out.contains("pollers"), "not described yet:\n{out}");
    let queue = app
        .view
        .dashboard
        .as_ref()
        .unwrap()
        .sources()
        .iter()
        .position(|s| matches!(s, tmprl_core::dashboard::Source::Queue { .. }))
        .unwrap();

    reply(
        &mut app,
        queue,
        Ok(SourceData::Queue(QueueHealth {
            backlog: Some(12),
            backlog_age_ms: Some(240_000),
            pollers: 0,
            last_poll_ms: None,
        })),
    );
    let out = draw(&mut app, 120, 30);
    assert!(
        out.contains("backlog 12  no pollers"),
        "narrow, so no age:\n{out}"
    );
    let out = draw(&mut app, 200, 30);
    assert!(out.contains("backlog 12, oldest 4m  no pollers"), "{out}");

    reply(
        &mut app,
        queue,
        Ok(SourceData::Queue(QueueHealth {
            backlog: Some(0),
            pollers: 2,
            ..QueueHealth::default()
        })),
    );
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("2 pollers"), "{out}");
    assert!(!out.contains("backlog"), "{out}");
}

#[test]
fn a_list_windowed_on_close_time_says_how_long_ago_each_one_closed() {
    let mut app = app_with_rows();
    app.apply_dashboard(Some(
        "[[row]]\n[[row.panel]]\nkind = \"workflows\"\nsince = \"1d\"\nby = \"close\"\n",
    ));
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    let mut slow = wf(
        "default",
        "order-1001",
        WorkflowStatus::Failed,
        now() - 3 * 86_400_000,
    );
    slow.close_time = Some(now() - 300_000);
    reply(
        &mut app,
        0,
        Ok(SourceData::Workflows {
            rows: vec![slow],
            more: false,
        }),
    );
    let out = draw(&mut app, 100, 12);
    let row = out.lines().find(|l| l.contains("order-1001")).unwrap();
    assert!(row.ends_with("5m│"), "the start was 3d ago:\n{out}");
}

#[test]
fn a_tally_of_a_sample_is_marked_until_its_counts_arrive() {
    use tmprl_core::dashboard::Source;
    let mut app = app_with_dashboard();
    reply(&mut app, 1, Ok(failures(true)));
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("~2  CheckoutWorkflow"), "{out}");
    assert!(out.contains("of 3 sampled"), "{out}");

    let counting = |app: &App, name: &str| {
        app.view
            .dashboard
            .as_ref()
            .unwrap()
            .sources()
            .iter()
            .position(|s| matches!(s, Source::Counts { query, .. } if query.contains(name)))
            .unwrap()
    };
    for (name, n) in [("CheckoutWorkflow", 31_204), ("RefundWorkflow", 7)] {
        let source = counting(&app, name);
        reply(
            &mut app,
            source,
            Ok(SourceData::Counts(StatusCounts::new(n, []))),
        );
    }
    let out = draw(&mut app, 120, 30);
    assert!(out.contains("31204  CheckoutWorkflow"), "{out}");
    assert!(!out.contains('~'), "{out}");
    assert!(out.contains("Failing types names from 3"), "{out}");
    assert!(out.contains("Recent failures of 3 sampled"), "{out}");
}

#[test]
fn a_failure_says_why_once_its_closing_event_is_read() {
    use tmprl_core::dashboard::Source;
    let mut app = app_with_dashboard();
    let out = draw(&mut app, 140, 30);
    assert!(out.contains("CheckoutWorkflow"), "{out}");

    let closing = |app: &App, id: &str| {
        app.view
            .dashboard
            .as_ref()
            .unwrap()
            .sources()
            .iter()
            .position(|s| matches!(s, Source::Close { workflow_id, .. } if workflow_id == id))
    };
    assert!(
        closing(&app, "refund-9").is_none(),
        "a timeout has no reason to ask for"
    );
    let source = closing(&app, "order-1001").unwrap();
    reply(
        &mut app,
        source,
        Ok(SourceData::Close(Some(
            "ValidationError: nik not found".into(),
        ))),
    );
    let out = draw(&mut app, 200, 30);
    let row = out.lines().find(|l| l.contains("order-1001")).unwrap();
    assert!(row.contains("ValidationError: nik not found"), "{out}");
    let out = draw(&mut app, 140, 30);
    assert!(out.contains("ValidationError:…"), "cut to fit:\n{out}");

    let out = draw(&mut app, 70, 30);
    assert!(
        !out.contains("ValidationError"),
        "no room beside an id:\n{out}"
    );
}

#[test]
fn a_type_every_row_shares_is_not_repeated_down_the_list() {
    let mut app = app_with_dashboard();
    reply(
        &mut app,
        1,
        Ok(SourceData::Workflows {
            rows: vec![
                wf("default", "order-1", WorkflowStatus::Failed, now() - 1_000),
                wf("default", "order-2", WorkflowStatus::Failed, now() - 2_000),
            ],
            more: false,
        }),
    );
    let out = draw(&mut app, 140, 30);
    let row = out.lines().find(|l| l.contains("order-1")).unwrap();
    let list = row.split("││").next().unwrap();
    assert!(!list.contains("CheckoutWorkflow"), "{out}");
}

#[test]
fn a_row_with_little_to_show_leaves_its_room_to_a_row_with_more() {
    let mut app = app_with_rows();
    app.apply_dashboard(Some(
        "[[row]]\n[[row.panel]]\nkind = \"types\"\n\
         [[row]]\n[[row.panel]]\nkind = \"workflows\"\ntitle = \"Stuck\"\nquery = \"A = 'b'\"\nlimit = 50\n",
    ));
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    let top_of = |out: &str, title: &str| out.lines().position(|l| l.contains(title)).unwrap();

    let out = draw(&mut app, 100, 30);
    assert_eq!(top_of(&out, "┌ Stuck"), 15, "even, while loading:\n{out}");

    let rows = |n: i64| SourceData::Workflows {
        rows: (0..n)
            .map(|i| {
                wf(
                    "default",
                    &format!("order-{i}"),
                    WorkflowStatus::Running,
                    now() - i,
                )
            })
            .collect(),
        more: false,
    };
    reply(&mut app, 0, Ok(rows(2)));
    reply(&mut app, 1, Ok(rows(40)));
    let out = draw(&mut app, 100, 30);
    assert_eq!(
        top_of(&out, "┌ Stuck"),
        5,
        "one type, two lines kept:\n{out}"
    );
    assert!(out.lines().nth(28).unwrap().starts_with('└'), "{out}");

    reply(&mut app, 1, Ok(rows(3)));
    let out = draw(&mut app, 100, 30);
    assert_eq!(
        top_of(&out, "┌ Stuck"),
        15,
        "both have all they need, so what is over is shared:\n{out}"
    );
}
