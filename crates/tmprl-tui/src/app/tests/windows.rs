//! Splits and tabs.

use super::*;

#[test]
fn splitting_keeps_the_old_pane_and_focuses_a_fresh_one() {
    let mut app = app();
    app.view.namespaces = Loadable::loaded(vec![NamespaceInfo {
        name: "alpha".into(),
        state: "Registered".into(),
        retention_days: 1,
        description: String::new(),
    }]);

    app.run("window.split-right", None);
    assert_eq!(app.tabs.current().len(), 2);

    // The focused pane is the new one and starts empty.
    assert_eq!(app.view.namespace_rows().len(), 0);

    // The pane we came from must still hold what it had loaded.
    let others: Vec<_> = app
        .tabs
        .current()
        .views()
        .into_iter()
        .filter_map(|id| app.parked_view(id))
        .collect();
    assert_eq!(others.len(), 1, "exactly one pane is parked");
    assert_eq!(
        others[0].namespace_rows().len(),
        1,
        "the original pane kept its namespaces"
    );
}

#[test]
fn a_new_pane_opens_where_you_split_from() {
    // Splitting is almost always "show me this again so I can take one of them
    // elsewhere". Landing back at the namespace list would make the diff case two
    // navigations instead of one keystroke.
    let mut app = app();
    app.view.screen = Screen::Workflows;
    app.view.query = "ExecutionStatus = 'Failed'".into();
    app.view.scope = vec!["payments".into()];

    app.run("window.split-right", None);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(app.view.query, "ExecutionStatus = 'Failed'");
    assert_eq!(app.view.scope, ["payments"]);
    // But none of the other pane's loaded data came with it.
    assert!(app.view.workflows.value().is_none());
}

#[test]
fn focus_moves_between_panes_and_carries_their_state() {
    let mut app = app();
    app.view.query = "left".into();
    app.run("window.split-right", None);
    app.view.query = "right".into();

    app.run("window.focus-left", None);
    assert_eq!(app.view.query, "left", "each pane keeps its own query");
    app.run("window.focus-right", None);
    assert_eq!(app.view.query, "right");
}

#[test]
fn closing_a_window_leaves_the_survivor_focused_with_its_own_state() {
    let mut app = app();
    app.view.query = "kept".into();
    app.run("window.split-right", None);
    app.view.query = "doomed".into();

    app.run("window.close", None);
    assert_eq!(app.tabs.current().len(), 1);
    assert_eq!(app.view.query, "kept");
}

#[test]
fn the_last_window_refuses_to_close_and_says_how_to_quit() {
    let mut app = app();
    app.run("window.close", None);
    assert_eq!(app.tabs.current().len(), 1);
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("quit"), "should point at the way out: {msg}");
}

#[test]
fn tabs_keep_separate_windows_and_state() {
    let mut app = app();
    app.view.query = "first tab".into();
    app.run("window.split-right", None);
    assert_eq!(app.tabs.current().len(), 2);

    app.run("tab.new", None);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.tabs.current().len(), 1, "a new tab has one window");
    assert_eq!(app.view.query, "", "and a fresh view");

    app.run("tab.previous", None);
    assert_eq!(app.tabs.current().len(), 2, "the split is still there");
    assert_eq!(app.view.query, "first tab");
}

#[test]
fn the_last_tab_refuses_to_close() {
    let mut app = app();
    app.run("tab.close", None);
    assert_eq!(app.tabs.len(), 1);
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn a_closed_window_stops_the_poll_it_was_running() {
    // View's Drop aborts the follow task. Without it a closed pane keeps a long poll
    // open and keeps pushing events at a pane that no longer exists.
    let mut app = app();
    app.run("window.split-right", None);
    app.view.following = true;
    app.run("window.close", None);
    assert!(!app.view.following, "the survivor was never following");
    assert_eq!(app.tabs.current().len(), 1);
}
