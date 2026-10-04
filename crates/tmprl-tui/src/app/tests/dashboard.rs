//! The dashboard: opening it, replies finding their pane, and what `<CR>` opens.

use super::*;
use tmprl_core::fault::Code;
use tmprl_core::query::PROBLEMS;

fn on_dashboard() -> App {
    let mut app = app();
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    app
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

fn failures() -> SourceData {
    let mut refund = wf("default", "r3", 100);
    refund.workflow_type = "Refund".into();
    SourceData::Workflows {
        rows: vec![wf("default", "r1", 300), wf("default", "r2", 200), refund],
        more: false,
    }
}

fn counts() -> SourceData {
    SourceData::Counts(StatusCounts::new(
        7,
        [(WorkflowStatus::Running, 4), (WorkflowStatus::Failed, 3)],
    ))
}

fn schedule(id: &str) -> ScheduleRow {
    ScheduleRow {
        namespace: "default".into(),
        schedule_id: id.into(),
        workflow_type: "Reconcile".into(),
        paused: false,
        notes: String::new(),
        spec: "0 2 * * *".into(),
        next_run: None,
        recent_runs: 0,
    }
}

/// Every panel of the builtin layout with something in it: two statuses, three failures,
/// two types, one queue, two schedules.
fn filled() -> App {
    let mut app = on_dashboard();
    reply(&mut app, 0, Ok(counts()));
    reply(&mut app, 1, Ok(failures()));
    reply(
        &mut app,
        2,
        Ok(SourceData::Workflows {
            rows: vec![wf("default", "r8", 50), wf("default", "r9", 40)],
            more: false,
        }),
    );
    reply(
        &mut app,
        3,
        Ok(SourceData::Schedules(vec![
            schedule("nightly"),
            schedule("weekly"),
        ])),
    );
    app
}

fn down() -> Fault {
    Fault::rpc("ListWorkflowExecutions", Code::Unavailable, "down")
}

fn board(app: &App) -> &Board {
    app.view.dashboard.as_ref().unwrap()
}

#[test]
fn gd_opens_the_dashboard_and_asks_for_every_source() {
    let app = on_dashboard();
    assert_eq!(app.view.screen, Screen::Dashboard);
    assert_eq!(board(&app).sources().len(), 4);
    assert!((0..5).all(|p| board(&app).state(p).unwrap().is_loading()));
    assert_eq!(app.row_count(), 0);
}

#[test]
fn gd_from_the_namespace_list_takes_the_namespaces_under_the_cursor() {
    let namespace = |name: &str| NamespaceInfo {
        name: name.into(),
        state: "Registered".into(),
        retention_days: 1,
        description: String::new(),
    };
    let mut app = app();
    app.view.namespaces = Loadable::loaded(vec![namespace("alpha"), namespace("beta")]);
    app.run("motion.down", None);
    app.run("nav.dashboard", None);
    assert_eq!(app.view.scope, ["beta"]);
    assert_eq!(app.view.screen, Screen::Dashboard);

    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert_eq!(app.view.cursor, 1);

    app.run("motion.top", None);
    select(&mut app, 2);
    app.run("nav.dashboard", None);
    assert_eq!(app.view.scope, ["alpha", "beta"]);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn gd_is_refused_inside_a_history() {
    let mut app = viewing_history();
    app.run("nav.dashboard", None);
    assert_eq!(app.view.screen, Screen::History);
    assert!(app.note.as_ref().unwrap().0.contains("go up"));
}

#[test]
fn a_reply_fills_the_panels_that_read_from_it() {
    let mut app = on_dashboard();
    reply(&mut app, 1, Ok(failures()));
    assert_eq!(board(&app).items(2).len(), 3);
    assert_eq!(board(&app).items(3).len(), 2, "the tally of the same rows");
    assert_eq!(app.row_count(), 5);
}

#[test]
fn a_reply_for_a_parked_dashboard_lands_in_it_and_nowhere_else() {
    let mut app = on_dashboard();
    let parked = app.tabs.current().focused();
    app.run("window.split-right", None);
    assert_ne!(app.tabs.current().focused(), parked);
    assert_eq!(app.view.screen, Screen::Dashboard);

    let generation = app.parked_view(parked).unwrap().generation;
    app.handle(Msg::Dashboard {
        view: parked,
        generation,
        source: 0,
        timed: false,
        result: Ok(counts()),
    });
    let there = app.parked_view(parked).unwrap().dashboard.as_ref().unwrap();
    assert_eq!(there.items(0).len(), 2);
    assert!(
        board(&app).items(0).is_empty(),
        "the focused pane was not sent this"
    );
}

#[test]
fn a_reply_nobody_is_waiting_for_is_dropped() {
    let mut app = on_dashboard();
    app.handle(Msg::Dashboard {
        view: app.tabs.current().focused(),
        generation: app.view.generation.wrapping_sub(1),
        source: 0,
        timed: false,
        result: Ok(counts()),
    });
    app.handle(Msg::Dashboard {
        view: ViewId(99),
        generation: app.view.generation,
        source: 0,
        timed: false,
        result: Ok(counts()),
    });
    assert!(board(&app).items(0).is_empty());
}

#[test]
fn a_failed_refresh_keeps_the_panel_and_reports_the_failure() {
    let mut app = filled();
    app.run("app.refresh", None);
    assert_eq!(app.row_count(), 10, "a refresh does not blank the screen");
    reply(&mut app, 1, Err(down()));
    assert_eq!(board(&app).items(2).len(), 3);
    assert_eq!(board(&app).fault(2), Some(&down()));
    assert_eq!(app.note.as_ref().unwrap().1, Note::Error);
}

#[test]
fn a_refresh_makes_the_replies_already_out_stale() {
    let mut app = on_dashboard();
    let before = app.view.generation;
    app.run("app.refresh", None);
    assert_ne!(app.view.generation, before);
}

#[test]
fn enter_on_a_status_opens_the_workflows_behind_it_and_c_o_comes_back() {
    let mut app = filled();
    app.run("motion.right", None);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(app.view.query, "ExecutionStatus = 'Failed'");
    assert_eq!(app.view.cursor, 0);

    app.run("nav.jump-back", None);
    assert_eq!(app.view.screen, Screen::Dashboard);
    assert_eq!(app.view.cursor, 1);
    assert_eq!(app.row_count(), 10, "the board it left is still there");
}

#[test]
fn enter_on_a_workflow_opens_its_history() {
    let mut app = filled();
    app.set_cursor(2);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);
    assert_eq!(app.view.viewing.as_ref().unwrap().run_id, "r1");
}

#[test]
fn enter_on_a_type_or_a_queue_narrows_the_panels_own_query() {
    let mut app = filled();
    app.set_cursor(5);
    app.run("nav.open", None);
    assert!(app.view.query.starts_with(PROBLEMS), "{}", app.view.query);
    assert!(
        app.view.query.ends_with("AND WorkflowType = 'Checkout'"),
        "{}",
        app.view.query
    );
    assert!(
        app.view.query.contains("StartTime > '"),
        "{}",
        app.view.query
    );

    let mut app = filled();
    app.set_cursor(7);
    app.run("nav.open", None);
    assert_eq!(
        app.view.query,
        "ExecutionStatus = 'Running' AND TaskQueue = 'tq'"
    );
}

#[test]
fn enter_on_a_schedule_opens_the_schedule_list_on_it() {
    let mut app = filled();
    app.run("motion.bottom", None);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::Schedules);
    app.handle(Msg::Schedules {
        generation: app.view.generation,
        result: Ok(vec![
            schedule("hourly"),
            schedule("nightly"),
            schedule("weekly"),
        ]),
    });
    assert_eq!(app.view.cursor, 2);
}

#[test]
fn enter_on_an_empty_dashboard_says_there_is_nothing_to_open() {
    let mut app = on_dashboard();
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::Dashboard);
    assert!(app.note.as_ref().unwrap().0.contains("nothing to open"));
}

#[test]
fn the_bracket_keys_move_by_panel() {
    let mut app = filled();
    app.handle(Msg::Key(Chord::ch(']')));
    app.handle(Msg::Key(Chord::ch('p')));
    assert_eq!(app.view.cursor, 2);
    app.run("dashboard.next-panel", None);
    assert_eq!(app.view.cursor, 5);
    app.run("dashboard.prev-panel", None);
    app.run("dashboard.prev-panel", None);
    assert_eq!(app.view.cursor, 0);

    app.run("nav.workflows", None);
    app.run("dashboard.next-panel", None);
    assert!(app.note.as_ref().unwrap().0.contains("gd"));
}

#[test]
fn the_cursor_stays_on_its_item_when_a_panel_above_it_fills() {
    let mut app = on_dashboard();
    reply(&mut app, 1, Ok(failures()));
    app.run("motion.down", None);
    reply(&mut app, 0, Ok(counts()));
    assert_eq!(app.view.cursor, 3, "still on the second failure");
}

#[test]
fn a_cursor_never_moved_stays_at_the_top() {
    let mut app = on_dashboard();
    reply(&mut app, 1, Ok(failures()));
    reply(&mut app, 0, Ok(counts()));
    assert_eq!(app.view.cursor, 0);
}

#[test]
fn nothing_on_the_dashboard_is_a_mutation_target() {
    let mut app = filled();
    app.run("motion.down", Some(2));
    app.run("workflow.terminate", None);
    assert!(app.confirm.is_none());
    assert!(app.note.as_ref().unwrap().0.contains("no workflow"));

    app.run("motion.bottom", None);
    app.run("schedule.pause", None);
    assert!(app.confirm.is_none());
}

#[test]
fn yank_copies_the_name_and_the_item_as_json() {
    let mut app = filled();
    assert_eq!(app.field_under_cursor(), "Running");
    assert_eq!(app.records_selected(), r#"{"status":"Running","count":4}"#);
    app.set_cursor(5);
    assert_eq!(app.field_under_cursor(), "Checkout");
    assert_eq!(
        app.records_selected(),
        r#"{"workflowType":"Checkout","count":2}"#
    );
    app.set_cursor(7);
    assert_eq!(
        app.records_selected(),
        r#"{"namespace":"default","taskQueue":"tq","running":2}"#
    );
}

#[test]
fn search_runs_over_every_panels_items() {
    let mut app = filled();
    search_for(&mut app, "weekly");
    assert_eq!(app.view.cursor, 9);
}

#[test]
fn the_lists_are_one_key_away_and_keep_the_scope() {
    let mut app = filled();
    app.run("nav.workflows", None);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(app.view.scope, ["default"]);
    app.run("nav.dashboard", None);
    app.run("nav.schedules", None);
    assert_eq!(app.view.screen, Screen::Schedules);
}

#[test]
fn a_saved_view_leaves_the_dashboard_for_the_list_it_fills() {
    let mut app = filled();
    app.apply_config(
        None,
        Some("[[view]]\nkey = \"1\"\nname = \"Running\"\nquery = \"ExecutionStatus = 'Running'\""),
        None,
    );
    app.run("view.1", None);
    assert_eq!(app.view.screen, Screen::Workflows);
}

#[test]
fn a_split_dashboard_asks_for_its_own_data() {
    let mut app = filled();
    app.run("window.split-down", None);
    assert_eq!(app.view.screen, Screen::Dashboard);
    assert_eq!(app.row_count(), 0);
    assert!(board(&app).state(0).unwrap().is_loading());
}

const ONE_PANEL: &str = "[[row]]\n[[row.panel]]\nkind = \"workflows\"\ntitle = \"Stuck\"\nquery = \"ExecutionStatus = 'Running'\"\n";

#[test]
fn a_dashboard_file_replaces_the_builtin_layout() {
    let mut app = app();
    app.apply_dashboard(Some(ONE_PANEL));
    assert!(app.note.is_none());
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    assert_eq!(board(&app).panel_count(), 1);
    assert_eq!(board(&app).spec(0).unwrap().title(), "Stuck");
    assert_eq!(board(&app).sources().len(), 1);
}

#[test]
fn a_dashboard_file_that_does_not_parse_is_reported_and_set_aside() {
    let mut app = app();
    app.apply_dashboard(Some(
        "[[row]]\n[[row.panel]]\nkind = \"workflows\"\nqeury = \"x\"\n",
    ));
    let (message, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Error);
    assert!(
        message.contains("dashboard.toml") && message.contains("qeury"),
        "{message}"
    );

    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    assert_eq!(board(&app).panel_count(), 5, "the builtin layout stands in");
}

#[test]
fn an_empty_or_absent_dashboard_file_is_the_builtin_layout() {
    for file in [None, Some(""), Some("# later\n")] {
        let mut app = app();
        app.apply_dashboard(file);
        assert!(app.note.is_none());
        app.view.screen = Screen::Workflows;
        app.run("nav.dashboard", None);
        assert_eq!(board(&app).panel_count(), 5);
    }
}

#[test]
fn a_key_error_is_the_one_left_on_the_note_line() {
    let mut app = app();
    app.apply_dashboard(Some("rows = 1"));
    app.apply_config(Some("[normal]\n\"x\" = \"nope.nope\"\n"), None, None);
    assert!(app.note.as_ref().unwrap().0.contains("nope.nope"));
}

fn titles(app: &App) -> Vec<String> {
    board(app)
        .layout()
        .panels()
        .map(|p| p.title().to_string())
        .collect()
}

#[test]
fn without_a_dashboard_file_the_panels_follow_what_the_namespace_shows() {
    let mut app = on_dashboard();
    assert_eq!(titles(&app).len(), 5, "everything, while nothing is known");

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
            rows: vec![wf("default", "r8", 50)],
            more: false,
        }),
    );
    reply(&mut app, 3, Ok(SourceData::Schedules(Vec::new())));
    assert_eq!(titles(&app), ["Status", "Running", "Task queues"]);
}

#[test]
fn a_panel_that_empties_goes_only_when_the_dashboard_is_asked_for_again() {
    let mut app = filled();
    let empty = || {
        Ok(SourceData::Workflows {
            rows: Vec::new(),
            more: false,
        })
    };
    reply(&mut app, 1, empty());
    assert!(titles(&app).contains(&"Recent failures".to_string()));

    app.run("app.refresh", None);
    assert!(!titles(&app).contains(&"Recent failures".to_string()));
    assert!(titles(&app).contains(&"Running".to_string()));
}

#[test]
fn a_dashboard_file_is_never_rearranged() {
    let mut app = app();
    app.apply_dashboard(Some(ONE_PANEL));
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    reply(
        &mut app,
        0,
        Ok(SourceData::Workflows {
            rows: Vec::new(),
            more: false,
        }),
    );
    app.run("app.refresh", None);
    assert_eq!(titles(&app), ["Stuck"]);
}

const INTERVAL: i64 = 30_000;

fn timed_reply(app: &mut App, source: usize, result: Result<SourceData, Fault>) {
    app.handle(Msg::Dashboard {
        view: app.tabs.current().focused(),
        generation: app.view.generation,
        source,
        timed: true,
        result,
    });
}

fn out(view: &View) -> Vec<bool> {
    (0..4).map(|s| view.dashboard_pacer.in_flight(s)).collect()
}

#[test]
fn nothing_is_asked_for_again_before_the_interval_is_up() {
    let mut app = filled();
    assert_eq!(out(&app.view), [false; 4], "every answer is in");
    app.tick_dashboards(now_ms() + INTERVAL - 2_000);
    assert_eq!(out(&app.view), [false; 4]);
}

#[test]
fn once_the_interval_is_up_everything_is_asked_for_again_once() {
    let mut app = filled();
    let generation = app.view.generation;
    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    assert_eq!(out(&app.view), [true; 4]);
    assert_eq!(
        app.view.generation, generation,
        "so a slow answer to the last round still lands"
    );
    assert_eq!(app.row_count(), 10, "and nothing on screen was blanked");

    timed_reply(&mut app, 0, Ok(counts()));
    assert_eq!(out(&app.view), [false, true, true, true]);
    app.tick_dashboards(now_ms() + 2_000);
    assert_eq!(out(&app.view), [false, true, true, true]);
}

#[test]
fn a_dashboard_left_in_a_split_is_refreshed_and_one_in_another_tab_is_not() {
    let mut app = filled();
    let parked = app.tabs.current().focused();
    app.run("window.split-right", None);
    app.run("nav.workflows", None);
    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    assert_eq!(out(app.parked_view(parked).unwrap()), [true; 4]);

    let mut app = filled();
    let parked = app.tabs.current().focused();
    app.run("tab.new", None);
    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    assert_eq!(out(app.parked_view(parked).unwrap()), [false; 4]);
}

#[test]
fn a_dashboard_that_was_left_is_not_refreshed() {
    let mut app = filled();
    app.run("nav.workflows", None);
    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    assert_eq!(out(&app.view), [false; 4]);
}

#[test]
fn refresh_can_be_turned_off() {
    let mut app = filled();
    app.apply_config(None, None, Some("[refresh]\ndashboard = \"off\""));
    app.tick_dashboards(now_ms() + INTERVAL * 100);
    assert_eq!(out(&app.view), [false; 4]);
}

#[test]
fn the_timer_reports_a_source_going_bad_once() {
    let mut app = filled();
    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    timed_reply(&mut app, 1, Err(down()));
    let logged = app.messages.len();
    assert_eq!(app.note.as_ref().unwrap().1, Note::Error);
    assert_eq!(board(&app).items(2).len(), 3, "the old rows stay");

    app.tick_dashboards(now_ms() + INTERVAL * 10);
    assert!(out(&app.view)[1], "it is tried again");
    timed_reply(&mut app, 1, Err(down()));
    assert_eq!(
        app.messages.len(),
        logged,
        "and failing again is not said again"
    );

    app.run("app.refresh", None);
    reply(&mut app, 1, Err(down()));
    assert_eq!(app.messages.len(), logged + 1, "R always answers");
}

#[test]
fn a_source_the_server_refuses_is_left_alone_until_r() {
    let mut app = filled();
    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    timed_reply(
        &mut app,
        3,
        Err(Fault::rpc("ListSchedules", Code::PermissionDenied, "no")),
    );
    app.tick_dashboards(now_ms() + INTERVAL * 1_000);
    assert!(!out(&app.view)[3]);

    app.run("app.refresh", None);
    assert!(out(&app.view)[3]);
}

#[test]
fn a_queue_found_on_running_workflows_is_described_and_kept_fresh() {
    use tmprl_core::taskqueue::QueueHealth;
    let mut app = filled();
    let sources = board(&app).sources();
    let queue = sources
        .iter()
        .position(|s| matches!(s, Source::Queue { .. }))
        .expect("queue `tq`");
    assert_eq!(
        sources
            .iter()
            .filter(|s| matches!(s, Source::Queue { .. }))
            .count(),
        1
    );
    assert!(app.view.dashboard_pacer.in_flight(queue));

    reply(
        &mut app,
        queue,
        Ok(SourceData::Queue(QueueHealth {
            backlog: Some(12),
            pollers: 0,
            ..QueueHealth::default()
        })),
    );
    let Item::Queue(described) = &board(&app).items(4)[0] else {
        panic!("{:?}", board(&app).items(4));
    };
    assert!(described.health.as_ref().unwrap().stuck());
    assert_eq!(app.row_count(), 10, "health adds no rows");

    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    assert!(app.view.dashboard_pacer.in_flight(queue));
}

#[test]
fn h_j_k_l_move_over_the_panels_as_they_sit_and_take_a_count() {
    let mut app = filled();
    assert_eq!(app.field_under_cursor(), "Running");
    app.run("motion.right", None);
    assert_eq!(app.field_under_cursor(), "Failed");
    app.run("motion.down", Some(2));
    let second = app.field_under_cursor();
    app.run("motion.up", None);
    assert_ne!(app.field_under_cursor(), second);
    app.run("motion.up", Some(9));
    assert_eq!(
        app.field_under_cursor(),
        "Running",
        "back at the top, and no further"
    );

    app.view.screen = Screen::Workflows;
    let before = app.view.cursor;
    app.run("motion.left", None);
    app.run("motion.right", None);
    assert_eq!(app.view.cursor, before, "a list has no sideways");
}

#[test]
fn a_workflow_a_retrying_panel_stopped_looking_into_is_not_asked_about_again() {
    let mut app = app();
    app.apply_dashboard(Some(
        "[[row]]\n[[row.panel]]\nkind = \"retrying\"\nscan = 1\n",
    ));
    app.view.screen = Screen::Workflows;
    app.run("nav.dashboard", None);
    let running = |run: &str, start: i64| {
        let mut row = wf("default", run, start);
        row.status = WorkflowStatus::Running;
        row
    };
    let page = |rows: Vec<WorkflowRow>| Ok(SourceData::Workflows { rows, more: false });
    reply(&mut app, 0, page(vec![running("first", 10)]));
    reply(&mut app, 1, Ok(SourceData::Pending(Vec::new())));
    assert!(!board(&app).dormant(1));

    reply(&mut app, 0, page(vec![running("second", 5)]));
    assert!(board(&app).dormant(1), "`first` has left the page");
    reply(&mut app, 2, Ok(SourceData::Pending(Vec::new())));

    app.tick_dashboards(now_ms() + INTERVAL + 1_000);
    assert!(!app.view.dashboard_pacer.in_flight(1), "left to sleep");
    assert!(
        app.view.dashboard_pacer.in_flight(2),
        "`second` is asked about"
    );
}
