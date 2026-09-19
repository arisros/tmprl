//! Tests for `App`, one file per `app/` file they exercise. The fixtures every file
//! shares are here.

use super::yank::json_string;
use super::*;
use tmprl_core::{Key, WorkflowStatus};
use tokio::sync::mpsc::unbounded_channel;

mod config;
mod find;
mod history;
mod mutate;
mod nav;
mod payload;
mod windows;
mod yank;

fn app() -> App {
    let (tx, _rx) = unbounded_channel();
    App::detached("prod", "default", tx)
}

fn wf(ns: &str, run: &str, start: i64) -> WorkflowRow {
    WorkflowRow {
        namespace: ns.into(),
        workflow_id: format!("order-{run}"),
        run_id: run.into(),
        workflow_type: "Checkout".into(),
        task_queue: "tq".into(),
        status: WorkflowStatus::Running,
        start_time: Some(start),
        close_time: None,
        history_length: 4,
    }
}

fn loaded(app: &mut App, rows: Vec<WorkflowRow>, tokens: Tokens) {
    app.view.screen = Screen::Workflows;
    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: false,
        result: Ok((rows, tokens)),
    });
}

fn type_chars(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle(Msg::Key(Chord::ch(c)));
    }
}

/// Type a pattern into an open `/` prompt and submit it.
fn search_for(app: &mut App, pattern: &str) {
    app.run("search.open", None);
    type_chars(app, pattern);
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
}

/// Four workflows, two of them `Refund`.
///
/// The list renders newest first, so the display order is the reverse of the order
/// written here: **r4, r3, r2, r1**, which puts the two Refunds on rows 0 and 2. The
/// assertions below name run ids rather than indices wherever they can, because that
/// reversal is exactly the kind of thing a reader gets wrong.
fn four(app: &mut App) {
    let mut rows = vec![
        wf("default", "r1", 100),
        wf("default", "r2", 200),
        wf("default", "r3", 300),
        wf("default", "r4", 400),
    ];
    rows[1].workflow_type = "Refund".into();
    rows[3].workflow_type = "Refund".into();
    loaded(app, rows, vec![]);
}

/// The run id under the cursor, which is what the search assertions are really about.
fn at_cursor(app: &App) -> String {
    app.workflow_rows()[app.view.cursor].run_id.clone()
}

/// Row index of a run id in display order.
fn row_of(app: &App, run: &str) -> usize {
    app.workflow_rows()
        .iter()
        .position(|w| w.run_id == run)
        .expect("run should be in the list")
}

fn type_into_picker(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle(Msg::Key(Chord::ch(c)));
    }
}

fn picker_labels(app: &App) -> Vec<String> {
    app.picker
        .as_ref()
        .expect("a picker should be open")
        .rows()
        .map(|(i, _)| i.label.clone())
        .collect()
}

fn hev(
    id: i64,
    group: tmprl_core::history::GroupRef,
    role: tmprl_core::history::Role,
    cat: tmprl_core::history::Category,
) -> NormalizedEvent {
    NormalizedEvent::new(id, "E", cat, group, role).with_time(Some(id * 1000))
}

/// A workflow, a workflow task (plumbing) and two activities, the second of which
/// failed.
fn history_events() -> Vec<NormalizedEvent> {
    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    vec![
        hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
        hev(2, G::Opened(2), R::Opens, C::WorkflowTask),
        hev(3, G::Opened(2), R::Closes, C::WorkflowTask),
        hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("Charge"),
        hev(5, G::Opened(4), R::Continues, C::Activity),
        hev(6, G::Opened(4), R::Closes, C::Activity)
            .with_outcome(tmprl_core::history::Outcome::Completed),
        hev(7, G::Opened(7), R::Opens, C::Activity).with_subject("Ship"),
        hev(8, G::Opened(7), R::Closes, C::Activity)
            .with_outcome(tmprl_core::history::Outcome::Failed),
    ]
}

fn viewing_history() -> App {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((history_events(), Vec::new())),
    });
    app
}

/// A history whose workflow has *not* closed: no terminal event.
fn running_history() -> Vec<NormalizedEvent> {
    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    vec![
        hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
        hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("Charge"),
    ]
}

fn viewing_running() -> App {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        // A non-empty token is what a running workflow returns.
        result: Ok((running_history(), vec![9])),
    });
    app
}

/// A history whose activity carries a JSON input and result.
fn viewing_payloads() -> App {
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};

    let mut scheduled = hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("Charge");
    scheduled.payloads.push((
        "input".into(),
        Payload::new("json/plain", br#"{"amount":100}"#.to_vec()),
    ));
    let mut completed = hev(6, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed);
    completed.payloads.push((
        "result".into(),
        Payload::new("json/plain", b"\"charged\"".to_vec()),
    ));
    let mut secret = hev(7, G::Opened(7), R::Opens, C::Activity).with_subject("Secret");
    secret.payloads.push((
        "input".into(),
        Payload::new("binary/encrypted", vec![0u8; 16]),
    ));

    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((
            vec![
                hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
                scheduled,
                completed,
                secret,
            ],
            Vec::new(),
        )),
    });
    app
}

fn on_a_workflow() -> App {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app
}

/// Four running workflows, cursor on the first.
fn on_four_workflows() -> App {
    let mut app = app();
    loaded(
        &mut app,
        vec![
            wf("default", "r1", 400),
            wf("default", "r2", 300),
            wf("default", "r3", 200),
            wf("default", "r4", 100),
        ],
        vec![],
    );
    app
}

/// Select `count` rows downwards from the cursor with `V` and `j`.
fn select(app: &mut App, count: usize) {
    app.run("mode.visual-line", None);
    for _ in 1..count {
        app.handle(Msg::Key(Chord::ch('j')));
    }
}

fn on_schedules() -> App {
    let mut app = app();
    app.view.screen = Screen::Schedules;
    app.view.scope = vec!["default".into()];
    app.view.schedules = Loadable::loaded(vec![ScheduleRow {
        namespace: "default".into(),
        schedule_id: "nightly".into(),
        workflow_type: "Reconcile".into(),
        paused: false,
        notes: String::new(),
        spec: "0 2 * * *".into(),
        next_run: None,
        recent_runs: 0,
    }]);
    app
}
