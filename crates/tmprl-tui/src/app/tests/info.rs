//! The workflow info card: `gi`, and the describe reply behind it.

use super::*;
use tmprl_core::info::WorkflowInfo;

fn info() -> WorkflowInfo {
    WorkflowInfo {
        workflow_id: "r1".into(),
        run_id: "r1".into(),
        workflow_type: "OrderWorkflow".into(),
        status: WorkflowStatus::Running,
        task_queue: "orders".into(),
        memo: vec![(
            "note".into(),
            Payload::new("json/plain", b"\"vip\"".to_vec()),
        )],
        search_attributes: vec![(
            "tenant".into(),
            Payload::new("json/plain", b"\"acme\"".to_vec()),
        )],
        ..Default::default()
    }
}

fn reply(app: &mut App, info: WorkflowInfo) {
    app.handle(Msg::Described {
        generation: app.view.generation,
        result: Ok(Box::new(Described {
            info,
            pending: Vec::new(),
        })),
    });
}

#[test]
fn gi_off_a_history_says_where_it_works() {
    let mut app = app();
    app.run("history.info", None);
    assert!(!app.view.show_info);
    let (note, level) = app.note.clone().expect("should say why");
    assert_eq!(level, Note::Warn);
    assert!(note.contains("history"), "{note}");
}

#[test]
fn gi_opens_the_info_card_and_a_second_gi_closes_it() {
    let mut app = viewing_history();
    app.run("history.info", None);
    assert!(app.view.show_info);
    assert!(!app.view.show_detail, "the payload pane is its own toggle");

    app.run("history.info", None);
    assert!(!app.view.show_info);
}

#[test]
fn gi_leaves_an_open_payload_pane_open() {
    let mut app = viewing_history();
    app.run("history.detail", None);
    app.run("history.info", None);
    assert!(app.view.show_detail && app.view.show_info);
    app.run("history.info", None);
    assert!(app.view.show_detail);
}

#[test]
fn k_leaves_the_info_card_alone() {
    let mut app = viewing_history();
    app.run("history.info", None);
    app.run("history.detail", None);
    app.run("history.detail", None);
    assert!(app.view.show_info);
}

#[test]
fn the_card_stays_open_from_one_history_to_the_next() {
    let mut app = viewing_history();
    app.run("history.info", None);
    app.run("app.refresh", None);
    assert!(app.view.show_info);
}

#[test]
fn the_info_stays_while_the_cursor_moves() {
    let mut app = viewing_history();
    reply(&mut app, info());
    app.run("history.info", None);
    app.run("motion.down", None);
    assert!(
        app.view.show_info,
        "info is about the workflow, not the row"
    );
}

#[test]
fn a_describe_reply_fills_the_info_and_the_pending_activities() {
    let mut app = viewing_history();
    app.handle(Msg::Described {
        generation: app.view.generation,
        result: Ok(Box::new(Described {
            info: info(),
            pending: vec![PendingActivity::default()],
        })),
    });
    assert_eq!(
        app.view.info.value().map(|i| i.task_queue.as_str()),
        Some("orders")
    );
    assert_eq!(app.view.pending.len(), 1);
}

#[test]
fn a_failed_first_describe_shows_in_the_card_and_a_later_one_keeps_what_was_known() {
    let mut app = viewing_history();
    app.handle(Msg::Described {
        generation: app.view.generation,
        result: Err("permission denied".into()),
    });
    assert!(app.view.info.error().is_some());

    reply(&mut app, info());
    app.handle(Msg::Described {
        generation: app.view.generation,
        result: Err("unavailable".into()),
    });
    assert!(app.view.info.value().is_some());
}

#[test]
fn leaving_a_history_forgets_its_info() {
    let mut app = viewing_history();
    reply(&mut app, info());
    app.run("nav.up", None);
    assert!(app.view.info.value().is_none());
}

#[test]
fn a_run_that_finished_after_the_list_loaded_reads_completed_back_on_the_list() {
    let mut app = viewing_history();
    let mut done = info();
    done.status = WorkflowStatus::Completed;
    done.close_time = Some(900);
    reply(&mut app, done);
    app.run("nav.up", None);
    let row = &app.view.workflows.value().unwrap().rows()[0];
    assert_eq!(
        (row.status, row.close_time),
        (WorkflowStatus::Completed, Some(900))
    );
}
