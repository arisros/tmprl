//! The workflow bar and the info card `gi` opens under it.

use super::*;
use tmprl_core::info::WorkflowInfo;
use tmprl_core::payload::Payload;
use tmprl_core::pending::{PendingActivity, PendingState};

fn described() -> WorkflowInfo {
    WorkflowInfo {
        workflow_id: "order-1001".into(),
        run_id: "r-42".into(),
        workflow_type: "OrderWorkflow".into(),
        status: WorkflowStatus::Running,
        task_queue: "orders".into(),
        start_time: Some(0),
        run_timeout_ms: Some(3_600_000),
        task_timeout_ms: Some(10_000),
        memo: vec![(
            "note".into(),
            Payload::new("json/plain", b"\"vip\"".to_vec()),
        )],
        search_attributes: vec![(
            "Tenant".into(),
            Payload::new("json/plain", b"\"acme\"".to_vec()),
        )],
        ..Default::default()
    }
}

fn opened(width: u16, height: u16) -> String {
    let mut app = app_with_payloads();
    app.view.info = Loadable::loaded(described());
    app.run("history.info", None);
    draw(&mut app, width, height)
}

fn line_with<'a>(out: &'a str, needle: &str) -> &'a str {
    out.lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle}:\n{out}"))
}

#[test]
fn the_card_is_closed_until_gi() {
    let mut app = app_with_payloads();
    app.view.info = Loadable::loaded(described());
    let out = draw(&mut app, 120, 24);
    assert!(!out.contains("waiting on"), "{out}");
    assert!(out.contains("gi info"), "{out}");
}

#[test]
fn gi_opens_the_card_under_the_bar_and_it_starts_with_what_it_waits_on() {
    let out = opened(140, 30);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[1].contains("gi hide"), "{out}");
    assert!(lines[2].contains(" info "), "{out}");
    assert!(lines[2].contains("<Space>yR run"), "{out}");
    assert!(lines[3].contains("waiting on"), "{out}");
    assert!(lines[3].contains("nothing pending"), "{out}");
}

#[test]
fn the_card_lists_only_what_is_there_and_folds_the_rest_into_one_line() {
    let out = opened(140, 30);
    for want in [
        "r-42",
        "run 1h · workflow task 10s",
        "memo",
        "note \"vip\"",
        "search",
        "Tenant \"acme\"",
        "no parent · no children",
    ] {
        assert!(out.contains(want), "{want} missing:\n{out}");
    }
    assert!(!out.contains("this run"), "{out}");
    assert!(!out.contains("closed"), "{out}");
}

#[test]
fn the_card_does_not_repeat_the_bar() {
    let out = opened(140, 30);
    let card: String = out.lines().skip(3).take(8).collect::<Vec<_>>().join("\n");
    assert!(!card.contains("order-1001 "), "{card}");
    assert!(!card.contains("orders"), "{card}");
}

#[test]
fn a_wide_card_pairs_facts_and_a_narrow_one_stacks_them() {
    let wide = opened(140, 30);
    assert!(line_with(&wide, "r-42").contains("started"), "{wide}");
    let narrow = opened(90, 40);
    assert!(!line_with(&narrow, "r-42").contains("started"), "{narrow}");
}

#[test]
fn a_pending_activity_is_named_in_the_first_line() {
    let mut app = app_with_payloads();
    app.view.info = Loadable::loaded(described());
    app.view.pending = vec![PendingActivity {
        activity_type: "ChargeCard".into(),
        state: PendingState::Started,
        attempt: 2,
        maximum_attempts: 5,
        ..Default::default()
    }];
    app.run("history.info", None);
    let out = draw(&mut app, 140, 30);
    let first = line_with(&out, "waiting on");
    assert!(first.contains("activity ChargeCard"), "{out}");
    assert!(first.contains("attempt 2/5"), "{out}");

    let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
    term.draw(|f| render(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let (x, y) = (0..30u16)
        .find_map(|y| {
            let row: String = (0..140u16).map(|x| buf[(x, y)].symbol()).collect();
            row.find("activity ChargeCard")
                .map(|at| (row[..at].chars().count() as u16, y))
        })
        .unwrap();
    use ratatui::style::Modifier;
    assert!(
        buf[(x, y)].modifier.contains(Modifier::BOLD),
        "the name stands out"
    );
    let after = (x..140).find(|&x| buf[(x, y)].symbol() == "·").unwrap();
    assert!(
        !buf[(after + 2, y)].modifier.contains(Modifier::BOLD),
        "the rest does not"
    );
    assert_eq!(buf[(x - 2, y)].symbol(), "↻", "attempt 2 is a retry");
    assert_eq!(buf[(x, y)].fg, app.theme.warn.fg.unwrap_or_default());
}

#[test]
fn a_bar_whose_queue_is_the_type_names_it_once() {
    let mut app = app_with_history();
    let mut info = described();
    info.task_queue = info.workflow_type.clone();
    app.view.info = Loadable::loaded(info);
    let bar = draw(&mut app, 140, 12).lines().nth(1).unwrap().to_string();
    assert_eq!(bar.matches("OrderWorkflow").count(), 1, "{bar}");
}

#[test]
fn the_payload_pane_stays_open_beside_the_card() {
    let mut app = app_with_payloads();
    app.view.info = Loadable::loaded(described());
    app.run("motion.down", None);
    app.run("history.detail", None);
    app.run("history.info", None);
    let out = draw(&mut app, 140, 40);
    assert!(out.contains("waiting on"), "{out}");
    assert!(out.contains("payloads"), "{out}");
    assert!(out.contains("\"amount\": 100"), "{out}");
}

#[test]
fn the_card_says_it_is_describing_until_the_reply_lands() {
    let mut app = app_with_payloads();
    app.view.info = Loadable::Loading;
    app.run("history.info", None);
    let out = draw(&mut app, 120, 20);
    assert!(out.contains("describing"), "{out}");
}

#[test]
fn the_bar_shows_the_list_row_before_describe_answers() {
    let mut app = app_with_history();
    app.view.info = Loadable::Loading;
    let bar = draw(&mut app, 120, 12).lines().nth(1).unwrap().to_string();
    assert!(bar.contains("order-1001"), "{bar}");
}

#[test]
fn the_card_says_which_sdk_and_worker_run_the_workflow_and_who_polls_its_queue() {
    use tmprl_core::history::{Category, GroupRef, Outcome, Role};
    let mut app = app_with_payloads();
    app.view.info = Loadable::loaded(described());
    let mut task = tmprl_core::history::NormalizedEvent::new(
        4,
        "WORKFLOW_TASK_COMPLETED",
        Category::WorkflowTask,
        GroupRef::Opened(2),
        Role::Closes,
    );
    task.outcome = Outcome::Completed;
    task.fields = vec![
        ("identity", "81@worker-a@".into()),
        ("sdkName", "temporal-go".into()),
        ("sdkVersion", "1.26.0".into()),
    ];
    app.view.history_events.push(task);
    app.view.pollers = Some(Vec::new());
    app.run("history.info", None);

    let out = draw(&mut app, 140, 34);
    assert!(
        line_with(&out, "sdk").contains("Go · temporal-go 1.26.0"),
        "{out}"
    );
    assert!(!out.contains('\u{f07d3}'), "no icon unless asked:\n{out}");
    assert!(line_with(&out, "worker").contains("81@worker-a@"), "{out}");
    assert!(out.contains("none polling the task queue"), "{out}");

    app.icons = true;
    let out = draw(&mut app, 140, 34);
    assert!(line_with(&out, "sdk").contains("\u{f07d3} Go"), "{out}");
    assert!(
        line_with(&out, "gi hide").contains("OrderWorkflow  \u{f07d3}  order-1001"),
        "the bar carries the icon after the type:\n{out}"
    );
}

#[test]
fn the_card_colours_the_language_its_own_and_an_unpolled_queue_red() {
    use ratatui::style::Color;
    use tmprl_core::history::{Category, GroupRef, Outcome, Role};
    let mut app = app_with_payloads();
    app.apply_theme(tmprl_core::ColorDepth::TrueColor, None);
    app.view.info = Loadable::loaded(described());
    let mut task = tmprl_core::history::NormalizedEvent::new(
        4,
        "WORKFLOW_TASK_COMPLETED",
        Category::WorkflowTask,
        GroupRef::Opened(2),
        Role::Closes,
    );
    task.outcome = Outcome::Completed;
    task.fields = vec![("sdkName", "temporal-go".into())];
    app.view.history_events.push(task);
    app.view.pollers = Some(Vec::new());
    app.run("history.info", None);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 34)).unwrap();
    terminal.draw(|f| crate::ui::render(f, &mut app)).unwrap();
    let buf = terminal.backend().buffer();
    let fg_of = |needle: &str| {
        (0..34u16)
            .find_map(|y| {
                let row: String = (0..140u16).map(|x| buf[(x, y)].symbol()).collect();
                row.find(needle).map(|at| {
                    let x = row[..at].chars().count() as u16;
                    buf[(x, y)].fg
                })
            })
            .unwrap_or_else(|| panic!("no {needle}"))
    };
    assert_eq!(fg_of("Go · temporal-go"), Color::Rgb(0x00, 0xad, 0xd8));
    assert_eq!(fg_of("none polling"), app.theme.err.fg.unwrap());

    app.view.pollers = Some(vec![tmprl_core::taskqueue::Poller {
        identity: "7@poller-host@".into(),
        last_access_ms: None,
    }]);
    terminal.draw(|f| crate::ui::render(f, &mut app)).unwrap();
    let buf = terminal.backend().buffer();
    let (y, row) = (0..34u16)
        .map(|y| {
            (
                y,
                (0..140u16)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>(),
            )
        })
        .find(|(_, row)| row.contains("pollers"))
        .unwrap();
    let col = |needle: &str| row[..row.find(needle).unwrap()].chars().count() as u16;
    assert_eq!(
        buf[(col("1 · "), y)].fg,
        app.theme.ok.fg.unwrap(),
        "the count"
    );
    assert_eq!(
        buf[(col("7@poller"), y)].fg,
        app.theme.dim.fg.unwrap(),
        "who, as the worker row"
    );
}
