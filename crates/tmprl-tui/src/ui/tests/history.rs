//! The history outline.

use super::*;

#[test]
fn the_history_shows_one_row_per_group_not_per_event() {
    let mut app = app_with_history();
    let out = draw(&mut app, 110, 12);

    assert!(out.contains("ChargeCard"), "activity missing:\n{out}");
    assert!(out.contains("ShipOrder"), "activity missing:\n{out}");
    assert!(out.contains("activity"), "category missing:\n{out}");
    // Three groups on screen, not eight events: the body rows below the header.
    let rows = out
        .lines()
        .filter(|l| l.contains("activity") || l.contains("workflow "))
        .count();
    assert_eq!(rows, 3, "expected one row per group:\n{out}");
}

#[test]
fn a_retried_group_shows_its_attempt_count_and_failure() {
    let mut app = app_with_history();
    let out = draw(&mut app, 110, 12);
    assert!(
        out.contains("×3"),
        "the retry count must be visible:\n{out}"
    );
    assert!(out.contains("card declined"), "failure missing:\n{out}");
}

#[test]
fn k_shows_the_whole_failure_chain_the_row_had_no_room_for() {
    use tmprl_core::history::Failure;

    // What a worker sends: a wrapper whose message says nothing, over the failure that
    // names the class, over the exception it came from.
    let failure = Failure {
        message: "activity task failed".into(),
        cause: Some(Box::new(Failure {
            message: "card declined".into(),
            kind: Some("PaymentDeclined".into()),
            source: Some("JavaSDK".into()),
            non_retryable: true,
            stack_trace: Some("at com.example.shop.Charge.run(Charge.java:42)".into()),
            cause: Some(Box::new(Failure::new("Read timed out"))),
        })),
        ..Failure::default()
    };
    let mut app = app_with_failure(failure);

    app.run("motion.bottom", None);
    app.run("history.detail", None);
    let out = draw(&mut app, 110, 24);

    assert!(out.contains("PaymentDeclined"), "type missing:\n{out}");
    assert!(out.contains("caused by"), "the chain is the point:\n{out}");
    assert!(out.contains("Read timed out"), "root cause missing:\n{out}");
    assert!(
        out.contains("not retryable"),
        "retryability missing:\n{out}"
    );
    assert!(out.contains("JavaSDK"), "source missing:\n{out}");
    assert!(
        out.contains("Charge.java:42"),
        "stack trace missing:\n{out}"
    );
}

#[test]
fn folding_a_group_open_reveals_its_events_indented() {
    let mut app = app_with_history();
    app.run("motion.down", None); // the ChargeCard group
    app.run("history.fold", None);
    let out = draw(&mut app, 110, 14);

    // Event ids appear only once the group is unfolded.
    assert!(out.contains("EVENT"), "event rows missing:\n{out}");
    assert!(out.contains('▾'), "an open fold marker is expected:\n{out}");
}

#[test]
fn the_history_header_names_the_workflow_and_counts_failures() {
    let mut app = app_with_history();
    let out = draw(&mut app, 110, 12);
    let header = out.lines().next().unwrap();
    assert!(header.contains("order-1001"), "workflow id missing:\n{out}");
    assert!(header.contains("failed"), "failure tally missing:\n{out}");
}

#[test]
fn a_history_of_nothing_but_plumbing_says_so() {
    use tmprl_core::history::{Category as C, GroupRef as G, NormalizedEvent, Role as R};
    let events = vec![
        NormalizedEvent::new(1, "E", C::WorkflowTask, G::Opened(1), R::Opens),
        NormalizedEvent::new(2, "E", C::WorkflowTask, G::Opened(1), R::Closes),
    ];
    let groups = tmprl_core::history::group_events(&events);
    let mut app = app_with_history();
    app.view.history = Loadable::loaded(tmprl_core::outline::Outline::new(events, groups));

    let out = draw(&mut app, 110, 12);
    assert!(
        out.contains("nothing but workflow tasks"),
        "an empty pane would look broken:\n{out}"
    );
}

#[test]
fn a_long_scope_never_runs_into_the_header_summary() {
    // A workflow id can be a UUID, and the history header shows it. Rendered at full
    // length it overwrites the right-hand tallies with no separator.
    let mut app = app_with_history();
    app.view.viewing = Some(wf(
        "default",
        "a24368a8-fcaf-4c19-bc07-0334f59ee9b1-and-then-some-more",
        WorkflowStatus::Running,
        0,
    ));
    for width in [60u16, 80, 110] {
        let out = draw(&mut app, width, 10);
        let header = out.lines().next().unwrap();
        assert!(
            header.contains("  ") || header.trim().is_empty(),
            "header should keep a gap at width {width}:\n{header}"
        );
        assert!(
            !header.contains("failed") || header.contains(" failed"),
            "the summary must not be run into by the scope at width {width}:\n{header}"
        );
    }
}

#[test]
fn following_is_announced_in_the_statusline() {
    // A view that rewrites itself while you read it must say so, or a changing screen
    // reads as a glitch.
    let mut app = app_with_history();
    assert!(!draw(&mut app, 110, 12).contains("FOLLOW"));

    app.view.following = true;
    let out = draw(&mut app, 110, 12);
    assert!(out.contains("FOLLOW"), "follow indicator missing:\n{out}");
    assert!(out.contains("NORMAL"), "the mode is still shown:\n{out}");
}

#[test]
fn the_history_screen_renders_at_a_cramped_size() {
    let mut app = app_with_history();
    let _ = draw(&mut app, 20, 4);
    let _ = draw(&mut app, 8, 3);
    app.run("history.expand-all", None);
    let _ = draw(&mut app, 20, 4);
}

#[test]
fn a_retry_in_progress_shows_on_its_row() {
    let mut app = app_with_retrying_activity();
    let out = draw(&mut app, 120, 8);
    assert!(out.contains("×4/10"), "live attempt missing:\n{out}");
    assert!(out.contains("retry in"), "backoff missing:\n{out}");
    assert!(
        out.contains("card declined"),
        "last failure missing:\n{out}"
    );
}

#[test]
fn a_pending_entry_for_another_activity_changes_nothing() {
    let mut app = app_with_retrying_activity();
    app.view.pending[0].activity_id = "2".into();
    let out = draw(&mut app, 120, 8);
    assert!(!out.contains("×4"), "matched the wrong activity:\n{out}");
    assert!(!out.contains("card declined"), "{out}");
}
