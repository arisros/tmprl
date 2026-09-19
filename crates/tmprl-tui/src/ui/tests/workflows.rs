//! The workflow table and its query bar.

use super::*;

#[test]
fn the_workflow_table_shows_status_id_type_and_age() {
    let mut app = app_with_workflows(&["default"]);
    let out = draw(&mut app, 110, 12);
    assert!(out.contains("order-1001"), "workflow id missing:\n{out}");
    assert!(out.contains("CheckoutWorkflow"), "type missing:\n{out}");
    assert!(out.contains("Running"), "status missing:\n{out}");
    assert!(out.contains("Failed"), "status missing:\n{out}");
    // Status is legible without colour: the glyph carries it too.
    assert!(
        out.contains(WorkflowStatus::Running.glyph()),
        "status glyph missing:\n{out}"
    );
    assert!(out.contains("45s"), "age column missing:\n{out}");
    assert!(out.contains("2h"), "age column missing:\n{out}");
}

#[test]
fn leader_t_swaps_the_age_column_for_a_clock_reading() {
    let mut app = app_with_workflows(&["default"]);
    // Fix the zone so the assertion does not depend on where the test runs.
    app.clock = tmprl_core::Clock::named("UTC").unwrap();
    assert!(
        draw(&mut app, 110, 12).contains("45s"),
        "age column missing"
    );

    app.run("app.times", None);
    let out = draw(&mut app, 110, 12);
    assert!(!out.contains(" 45s"), "age column should be gone:\n{out}");
    let started = app.view.workflow_rows()[0].start_time;
    let stamp = tmprl_core::Clock::named("UTC").unwrap().stamp(started);
    assert!(
        out.contains(&stamp),
        "clock reading {stamp} missing:\n{out}"
    );
    // Toggling says which way it went, and in which zone.
    assert!(
        out.contains("absolute"),
        "the note should name the mode:\n{out}"
    );

    app.run("app.times", None);
    assert!(
        draw(&mut app, 110, 12).contains("45s"),
        "toggling back should restore the age column"
    );
}

#[test]
fn a_closed_workflow_shows_when_it_finished_not_only_when_it_started() {
    let mut app = app_with_workflows(&["default"]);
    app.clock = tmprl_core::Clock::named("UTC").unwrap();
    let start = 1_789_628_602_431;
    let closed = Some(start + 90_000);
    let mut done = wf("default", "order-9000", WorkflowStatus::Completed, start);
    done.close_time = closed;
    let mut list = WorkflowList::default();
    list.reset(vec![done], vec![]);
    app.view.workflows = Loadable::loaded(list);
    app.run("app.times", None);
    let out = draw(&mut app, 110, 12);
    let stamp = app.clock.stamp(closed);
    assert!(out.contains(&stamp), "close time missing:\n{out}");
}

#[test]
fn the_query_bar_is_always_on_screen() {
    // The raw query is the interface; it is never behind a keystroke to reveal.
    let mut app = app_with_workflows(&["default"]);
    assert!(
        draw(&mut app, 110, 12).contains("query"),
        "query bar must be visible with an empty query"
    );

    app.view.query = "ExecutionStatus = 'Failed'".into();
    let out = draw(&mut app, 110, 12);
    assert!(
        out.contains("ExecutionStatus = 'Failed'"),
        "the raw query text must be shown verbatim:\n{out}"
    );
}

#[test]
fn editing_the_query_shows_the_live_text_not_the_applied_one() {
    let mut app = app_with_workflows(&["default"]);
    app.view.query = "A = 1".into();
    app.run("mode.insert", None);
    for c in "23".chars() {
        app.handle(crate::app::Msg::Key(Chord::ch(c)));
    }
    let out = draw(&mut app, 110, 12);
    assert!(out.contains("A = 123"), "live edit missing:\n{out}");
    assert!(out.contains("INSERT"), "mode should be INSERT:\n{out}");
}

#[test]
fn the_header_shows_per_status_counts() {
    let mut app = app_with_workflows(&["default"]);
    let out = draw(&mut app, 110, 12);
    let header = out.lines().next().unwrap();
    assert!(header.contains("total"), "count total missing:\n{out}");
    assert!(
        header.contains(WorkflowStatus::Failed.glyph()),
        "the header should tally failures with the table's glyph:\n{out}"
    );
}

#[test]
fn the_namespace_column_appears_only_in_a_fan_out() {
    // On one namespace it would be the same value on every row.
    let mut single = app_with_workflows(&["default"]);
    let out = draw(&mut single, 110, 12);
    let body: String = out.lines().skip(2).collect::<Vec<_>>().join("\n");
    assert!(
        !body.contains("payments"),
        "a single-namespace list should not carry a namespace column:\n{out}"
    );

    let mut fanned = app_with_workflows(&["default", "payments"]);
    let out = draw(&mut fanned, 110, 12);
    let body: String = out.lines().skip(2).collect::<Vec<_>>().join("\n");
    assert!(
        body.contains("payments"),
        "a fan-out must tag rows with their namespace:\n{out}"
    );
}

#[test]
fn an_empty_result_distinguishes_no_data_from_a_filter() {
    let mut app = app_with_workflows(&["default"]);
    app.view.workflows = Loadable::loaded(WorkflowList::default());

    let out = draw(&mut app, 110, 12);
    assert!(out.contains("no workflows in this namespace"), "{out}");

    app.view.query = "ExecutionStatus = 'Failed'".into();
    let out = draw(&mut app, 110, 12);
    assert!(
        out.contains("no workflows match this query"),
        "an empty filtered list must say the filter is why:\n{out}"
    );
}

#[test]
fn the_workflow_screen_renders_at_a_cramped_size() {
    let mut app = app_with_workflows(&["default", "payments"]);
    let _ = draw(&mut app, 20, 4);
    let _ = draw(&mut app, 8, 3);
    app.show_help = true;
    let _ = draw(&mut app, 20, 4);
}
