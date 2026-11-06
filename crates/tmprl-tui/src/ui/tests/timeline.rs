//! The timeline view.

use super::*;

#[test]
fn the_timeline_draws_each_group_as_dots_on_one_axis() {
    let mut app = app_with_history();
    app.run("history.timeline", None);
    let out = draw(&mut app, 110, 12);

    assert!(out.contains("ChargeCard"), "label missing:\n{out}");
    assert!(out.contains("ShipOrder"), "label missing:\n{out}");
    assert!(
        out.contains('●') && out.contains('━'),
        "no dots or lines:\n{out}"
    );
    assert!(out.contains('◆'), "the workflow row is drawn apart:\n{out}");
    assert!(
        out.contains("3s ") && out.contains("ms"),
        "axis missing:\n{out}"
    );
    assert!(out.contains("↻ 3 • ChargeCard"), "attempts missing:\n{out}");
}

#[test]
fn the_timeline_colours_a_line_by_how_the_group_ended() {
    let mut app = app_with_history();
    app.run("history.timeline", None);
    assert_eq!(
        fg_on_row(&mut app, 110, 12, "ShipOrder", "━"),
        Some(Color::Rgb(0xce, 0x2c, 0x31))
    );
    assert_eq!(
        fg_on_row(&mut app, 110, 12, "OrderWorkflow", "╍"),
        Some(Color::Rgb(0x00, 0x90, 0xff))
    );
}

#[test]
fn idle_time_is_folded_until_zg_unfolds_it() {
    let mut app = app_with_history();
    app.run("history.timeline", None);
    assert!(draw(&mut app, 110, 12).contains('≀'), "gap not folded");

    app.run("history.gaps", None);
    let out = draw(&mut app, 110, 12);
    assert!(!out.contains('≀'), "gap still folded:\n{out}");
}

#[test]
fn the_timeline_is_refused_off_a_history() {
    let mut app = app_with_workflows(&["default"]);
    app.run("history.timeline", None);
    assert!(!app.view.timeline);
    assert!(draw(&mut app, 110, 12).contains("timeline is of a workflow history"));
}

#[test]
fn the_timeline_is_the_same_outline_row_for_row() {
    let mut app = app_with_history();
    app.run("history.timeline", None);
    app.run("motion.bottom", None);
    app.run("history.fold", None);
    let out = draw(&mut app, 110, 14);
    assert_eq!(out.matches("EVENT").count(), 2, "{out}");
}
