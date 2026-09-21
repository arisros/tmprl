//! Rendering tests: the app drawn into ratatui's `TestBackend` and read back as text. One
//! file per screen; the fixtures they share are here.

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use tmprl_client::NamespaceInfo;
use tmprl_core::{Chord, Loadable, StatusCounts, WorkflowList, WorkflowRow, WorkflowStatus};
use tokio::sync::mpsc::unbounded_channel;

mod chrome;
mod detail;
mod history;
mod schedules;
mod timeline;
mod workflows;

fn ns(name: &str, days: i64) -> NamespaceInfo {
    NamespaceInfo {
        name: name.into(),
        state: "Registered".into(),
        retention_days: days,
        description: String::new(),
    }
}

/// An app with data but no connection. Rendering must be testable without a server,
/// that is the whole point of keeping IO out of the render path.
fn app_with_rows() -> App {
    let (tx, _rx) = unbounded_channel();
    let mut app = App::detached("prod", "default", tx);
    app.view.namespaces = Loadable::loaded(vec![
        ns("default", 24),
        ns("payments", 30),
        ns("temporal-system", 7),
    ]);
    app
}

fn draw(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn wf(ns: &str, id: &str, status: WorkflowStatus, start: i64) -> WorkflowRow {
    WorkflowRow {
        namespace: ns.into(),
        workflow_id: id.into(),
        run_id: format!("run-{id}"),
        workflow_type: "CheckoutWorkflow".into(),
        task_queue: "orders".into(),
        status,
        start_time: Some(start),
        close_time: None,
        history_length: 7,
    }
}

/// A workflow screen with rows, no connection. Start times are relative to now so the
/// age column renders something stable.
fn app_with_workflows(scope: &[&str]) -> App {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut app = app_with_rows();
    app.view.screen = crate::app::Screen::Workflows;
    app.view.scope = scope.iter().map(|s| s.to_string()).collect();

    let mut list = WorkflowList::default();
    list.reset(
        vec![
            wf(
                "default",
                "order-1001",
                WorkflowStatus::Running,
                now - 45_000,
            ),
            wf(
                "payments",
                "charge-77",
                WorkflowStatus::Failed,
                now - 7_200_000,
            ),
        ],
        vec![],
    );
    app.view.workflows = Loadable::loaded(list);
    app.view.counts = Loadable::loaded(StatusCounts::new(
        2,
        [(WorkflowStatus::Running, 1), (WorkflowStatus::Failed, 1)],
    ));
    app
}

/// A running workflow whose only activity is backing off before its fourth attempt. The
/// history has nothing but the scheduling event; the rest came from describe.
fn app_with_retrying_activity() -> App {
    use tmprl_core::history::{Category as C, Failure, GroupRef as G, NormalizedEvent, Role as R};
    use tmprl_core::pending::{PendingActivity, PendingState};

    let mut scheduled = NormalizedEvent::new(
        5,
        "ActivityTaskScheduled",
        C::Activity,
        G::Opened(5),
        R::Opens,
    )
    .with_subject("ChargeCard")
    .with_time(Some(5_000));
    scheduled.fields.push(("activityId", "1".into()));
    let events = vec![
        NormalizedEvent::new(1, "EVENT", C::Workflow, G::Workflow, R::Opens)
            .with_subject("OrderWorkflow"),
        scheduled,
    ];

    let mut app = app_with_workflows(&["default"]);
    app.view.screen = crate::app::Screen::History;
    app.view.viewing = Some(wf("default", "order-1001", WorkflowStatus::Running, 0));
    let groups = tmprl_core::history::group_events(&events);
    app.view.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));
    app.view.pending = vec![PendingActivity {
        activity_id: "1".into(),
        activity_type: "ChargeCard".into(),
        state: PendingState::Scheduled,
        attempt: 4,
        maximum_attempts: 10,
        last_failure: Some(Failure {
            message: "card declined".into(),
            kind: Some("PaymentDeclined".into()),
            ..Failure::default()
        }),
        next_attempt_at: Some(crate::app::now_ms() + 30_000),
        last_worker: Some("worker-7@host".into()),
        ..PendingActivity::default()
    }];
    app
}

/// A history screen with a workflow, a hidden workflow task, and two activities, the
/// second of which failed after a retry.
fn app_with_history() -> App {
    app_with_failure(tmprl_core::history::Failure::new("card declined"))
}

/// The same history, with the failure the last activity closed with given by the caller.
fn app_with_failure(failure: tmprl_core::history::Failure) -> App {
    use tmprl_core::history::{
        Category as C, GroupRef as G, NormalizedEvent, Outcome as O, Role as R,
    };

    let e = |id: i64, g: G, r: R, c: C| {
        NormalizedEvent::new(id, "EVENT", c, g, r).with_time(Some(id * 1_000))
    };
    let mut started = e(5, G::Opened(4), R::Continues, C::Activity);
    started.attempt = Some(3);
    let mut failed = e(8, G::Opened(7), R::Closes, C::Activity).with_outcome(O::Failed);
    failed.failure = Some(failure);

    let events = vec![
        e(1, G::Workflow, R::Opens, C::Workflow).with_subject("OrderWorkflow"),
        e(2, G::Opened(2), R::Opens, C::WorkflowTask),
        e(3, G::Opened(2), R::Closes, C::WorkflowTask),
        e(4, G::Opened(4), R::Opens, C::Activity).with_subject("ChargeCard"),
        started,
        e(6, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed),
        e(7, G::Opened(7), R::Opens, C::Activity).with_subject("ShipOrder"),
        failed,
    ];

    let mut app = app_with_workflows(&["default"]);
    app.view.screen = crate::app::Screen::History;
    app.view.viewing = Some(wf("default", "order-1001", WorkflowStatus::Running, 0));
    let groups = tmprl_core::history::group_events(&events);
    app.view.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));
    app
}

/// The foreground of the first cell on the row containing `row_text` that shows `symbol`.
fn fg_on_row(app: &mut App, w: u16, h: u16, row_text: &str, symbol: &str) -> Option<Color> {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height).find_map(|y| {
        let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        if !line.contains(row_text) {
            return None;
        }
        (0..buf.area.width)
            .map(|x| &buf[(x, y)])
            .find(|c| c.symbol() == symbol)
            .map(|c| c.fg)
    })
}

/// A history whose activity carries a JSON input and result.
fn app_with_payloads() -> App {
    use tmprl_core::history::{
        Category as C, GroupRef as G, NormalizedEvent, Outcome as O, Role as R,
    };
    use tmprl_core::payload::Payload;

    let mut scheduled = NormalizedEvent::new(
        4,
        "ACTIVITY_TASK_SCHEDULED",
        C::Activity,
        G::Opened(4),
        R::Opens,
    )
    .with_time(Some(4_000))
    .with_subject("ChargeCard");
    scheduled.payloads.push((
        "input".into(),
        Payload::new("json/plain", br#"{"amount":100,"currency":"GBP"}"#.to_vec()),
    ));
    let mut completed = NormalizedEvent::new(
        6,
        "ACTIVITY_TASK_COMPLETED",
        C::Activity,
        G::Opened(4),
        R::Closes,
    )
    .with_time(Some(6_000))
    .with_outcome(O::Completed);
    completed.payloads.push((
        "result".into(),
        Payload::new("json/plain", b"\"charged\"".to_vec()),
    ));
    let mut secret = NormalizedEvent::new(
        7,
        "ACTIVITY_TASK_SCHEDULED",
        C::Activity,
        G::Opened(7),
        R::Opens,
    )
    .with_time(Some(7_000))
    .with_subject("Secret");
    secret.payloads.push((
        "input".into(),
        Payload::new("binary/encrypted", vec![0u8; 32]),
    ));

    let events = vec![
        NormalizedEvent::new(
            1,
            "WORKFLOW_EXECUTION_STARTED",
            C::Workflow,
            G::Workflow,
            R::Opens,
        )
        .with_time(Some(1_000))
        .with_subject("OrderWorkflow"),
        scheduled,
        completed,
        secret,
    ];
    let groups = tmprl_core::history::group_events(&events);
    let mut app = app_with_history();
    app.view.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));
    app
}

/// Row and column of the first line mentioning `needle`.
fn position(out: &str, needle: &str) -> (usize, usize) {
    out.lines()
        .enumerate()
        .find_map(|(row, line)| line.find(needle).map(|b| (row, line[..b].chars().count())))
        .unwrap_or_else(|| panic!("`{needle}` not drawn:\n{out}"))
}
