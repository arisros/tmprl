//! The pickers and `/` search.

use super::*;

#[test]
fn slash_opens_a_prompt_that_says_it_is_a_search() {
    let mut app = app();
    four(&mut app);
    app.run("search.open", None);
    let prompt = app.prompt.as_ref().expect("/ should open a prompt");
    assert_eq!(prompt.kind, PromptKind::Search);
    assert_eq!(prompt.sigil(), "/");
    assert_eq!(prompt.buf, "", "a new search starts empty, not pre-filled");
}

#[test]
fn a_search_moves_the_cursor_to_the_first_match() {
    let mut app = app();
    four(&mut app);
    search_for(&mut app, "refund");
    assert_eq!(
        at_cursor(&app),
        "r4",
        "r4 is the first Refund in display order"
    );
}

#[test]
fn a_search_can_match_the_row_the_cursor_is_already_on() {
    // `/` is typed while looking at the screen. Skipping a match that is right there,
    // the way `n` deliberately does, would read as the search having failed.
    let mut app = app();
    four(&mut app);
    app.view.cursor = row_of(&app, "r2");
    search_for(&mut app, "refund");
    assert_eq!(
        at_cursor(&app),
        "r2",
        "should have stayed on the visible match"
    );
}

#[test]
fn n_walks_to_the_next_match_and_wraps() {
    let mut app = app();
    four(&mut app);
    search_for(&mut app, "refund");
    assert_eq!(at_cursor(&app), "r4");

    app.run("search.next", None);
    assert_eq!(at_cursor(&app), "r2", "the other Refund, further down");

    app.run("search.next", None);
    assert_eq!(at_cursor(&app), "r4", "wrapped back to the first");
    let (msg, _) = app.note.clone().expect("a wrap must announce itself");
    assert!(msg.contains("wrapped"), "got: {msg}");
}

#[test]
fn capital_n_walks_backwards() {
    let mut app = app();
    four(&mut app);
    search_for(&mut app, "refund");
    assert_eq!(at_cursor(&app), "r4");
    app.run("search.previous", None);
    assert_eq!(
        at_cursor(&app),
        "r2",
        "backwards from the topmost match wraps to the bottom one"
    );
}

#[test]
fn n_without_a_previous_search_says_so_rather_than_moving() {
    let mut app = app();
    four(&mut app);
    app.view.cursor = 2;
    app.run("search.next", None);
    assert_eq!(app.view.cursor, 2, "nothing should have moved");
    let (msg, level) = app.note.clone().expect("should have explained itself");
    assert_eq!(level, Note::Warn);
    assert!(msg.contains("/"), "got: {msg}");
}

#[test]
fn a_search_finds_a_run_id_that_is_not_on_screen() {
    // The reason labels are wider than the columns: a run id pasted out of a log is
    // exactly what you arrive with, and it is not one of the rendered fields.
    let mut app = app();
    four(&mut app);
    search_for(&mut app, "r3");
    assert_eq!(at_cursor(&app), "r3");
}

#[test]
fn a_failed_search_reports_it_and_leaves_the_cursor_alone() {
    let mut app = app();
    four(&mut app);
    app.view.cursor = 2;
    search_for(&mut app, "nothing-matches-this");
    assert_eq!(app.view.cursor, 2);
    let (msg, level) = app.note.clone().expect("should have said no match");
    assert_eq!(level, Note::Warn);
    assert!(msg.contains("no match"), "got: {msg}");
}

#[test]
fn the_match_count_is_reported() {
    let mut app = app();
    four(&mut app);
    search_for(&mut app, "refund");
    let (msg, _) = app.note.clone().expect("a search should report its count");
    assert!(msg.contains("2 match"), "got: {msg}");
}

#[test]
fn the_pattern_survives_so_n_keeps_working_after_a_refresh() {
    // The search register is session state, not view state. A reload replaces every row
    // in the pane, and having to retype the pattern afterwards is the friction this
    // avoids.
    let mut app = app();
    four(&mut app);
    search_for(&mut app, "refund");
    four(&mut app);
    assert_eq!(app.search.pattern(), "refund");
    app.run("search.next", None);
    assert!(
        app.workflow_rows()[app.view.cursor]
            .workflow_type
            .contains("Refund")
    );
}

// ---- pickers ----

#[test]
fn the_workflow_picker_lists_every_loaded_workflow() {
    let mut app = app();
    four(&mut app);
    app.run("find.workflow", None);
    assert_eq!(picker_labels(&app).len(), 4);
}

#[test]
fn the_picker_owns_the_keyboard_while_it_is_open() {
    // `j` must type a `j` into the prompt, not move the list underneath. A picker that
    // let motions leak through would scroll the thing it is covering.
    let mut app = app();
    four(&mut app);
    let before = app.view.cursor;
    app.run("find.workflow", None);
    app.handle(Msg::Key(Chord::ch('j')));
    assert_eq!(app.view.cursor, before, "the list must not have moved");
    assert_eq!(app.picker.as_ref().unwrap().prompt, "j");
}

#[test]
fn typing_narrows_the_workflow_picker() {
    let mut app = app();
    four(&mut app);
    app.run("find.workflow", None);
    type_into_picker(&mut app, "r2");
    let shown = picker_labels(&app);
    assert_eq!(shown, vec!["order-r2"], "got {shown:?}");
}

#[test]
fn accepting_a_workflow_opens_its_history() {
    let mut app = app();
    four(&mut app);
    app.run("find.workflow", None);
    type_into_picker(&mut app, "r2");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));

    assert!(app.picker.is_none(), "accepting closes the picker");
    assert_eq!(app.view.screen, Screen::History);
    assert_eq!(
        app.view.viewing.as_ref().map(|w| w.run_id.as_str()),
        Some("r2")
    );
}

#[test]
fn ctrl_n_and_ctrl_p_move_the_picker_cursor() {
    // Not `<C-j>` / `<C-k>`: tmux eats those before tmprl sees them.
    let mut app = app();
    four(&mut app);
    app.run("find.workflow", None);
    app.handle(Msg::Key(Chord::ctrl('n')));
    assert_eq!(app.picker.as_ref().unwrap().cursor, 1);
    app.handle(Msg::Key(Chord::ctrl('p')));
    assert_eq!(app.picker.as_ref().unwrap().cursor, 0);
}

#[test]
fn esc_closes_a_picker_without_taking_anything() {
    let mut app = app();
    four(&mut app);
    let before = app.view.screen;
    app.run("find.workflow", None);
    app.handle(Msg::Key(Chord::plain(Key::Esc)));
    assert!(app.picker.is_none());
    assert_eq!(app.view.screen, before, "nothing should have been opened");
}

#[test]
fn backspace_on_an_empty_picker_prompt_closes_it() {
    let mut app = app();
    four(&mut app);
    app.run("find.workflow", None);
    type_into_picker(&mut app, "r");
    app.handle(Msg::Key(Chord::plain(Key::Backspace)));
    assert!(app.picker.is_some(), "that backspace deleted the 'r'");
    app.handle(Msg::Key(Chord::plain(Key::Backspace)));
    assert!(app.picker.is_none(), "empty, so it closes");
}

#[test]
fn a_picker_with_nothing_to_show_says_why_instead_of_opening() {
    // On the namespace screen there are no workflows loaded yet. An empty box would
    // look broken; the reason is the useful answer.
    let mut app = app();
    app.view.screen = Screen::Namespaces;
    app.run("find.workflow", None);
    assert!(app.picker.is_none());
    let (msg, level) = app.note.clone().expect("should have explained itself");
    assert_eq!(level, Note::Warn);
    assert!(msg.contains("no workflows"), "got: {msg}");
}

#[test]
fn the_filter_builder_offers_the_types_actually_loaded() {
    // The point of building filters from the rows on screen: it offers `Refund` because
    // this namespace has one, not because someone hardcoded a list.
    let mut app = app();
    four(&mut app);
    app.run("find.filter", None);
    let offered = picker_labels(&app);
    assert!(
        offered.iter().any(|l| l == "WorkflowType = 'Refund'"),
        "got {offered:?}"
    );
    assert!(offered.iter().any(|l| l == "ExecutionStatus = 'Running'"));
}

#[test]
fn a_filter_clause_is_anded_onto_the_query_already_there() {
    let mut app = app();
    four(&mut app);
    app.view.query = "WorkflowType = 'Checkout'".into();
    app.run("find.filter", None);
    type_into_picker(&mut app, "Running");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert_eq!(
        app.view.query,
        "WorkflowType = 'Checkout' AND ExecutionStatus = 'Running'"
    );
}

#[test]
fn a_filter_clause_on_an_empty_query_stands_alone() {
    let mut app = app();
    four(&mut app);
    app.view.query.clear();
    app.run("find.filter", None);
    type_into_picker(&mut app, "Running");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert_eq!(app.view.query, "ExecutionStatus = 'Running'");
}

#[test]
fn an_order_by_clause_is_appended_rather_than_anded() {
    // `... AND ORDER BY StartTime DESC` is a query the server rejects.
    let mut app = app();
    four(&mut app);
    app.view.query = "ExecutionStatus = 'Running'".into();
    app.run("find.filter", None);
    type_into_picker(&mut app, "ORDER BY StartTime DESC");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert_eq!(
        app.view.query,
        "ExecutionStatus = 'Running' ORDER BY StartTime DESC"
    );
}

#[test]
fn the_command_picker_runs_what_it_accepts() {
    let mut app = app();
    four(&mut app);
    assert!(!app.show_help);
    app.run("find.command", None);
    type_into_picker(&mut app, "app.help");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert!(app.show_help, "accepting app.help should have run it");
}

#[test]
fn the_event_picker_needs_a_history() {
    let mut app = app();
    four(&mut app);
    app.run("find.event", None);
    assert!(app.picker.is_none());
    let (msg, _) = app.note.clone().expect("should have said why");
    assert!(msg.contains("no history"), "got: {msg}");
}

#[test]
fn the_pane_picker_is_not_offered_for_a_single_pane() {
    // With one window there is nothing to switch to, and a picker holding only the pane
    // you are already in is a keystroke that does nothing.
    let mut app = app();
    four(&mut app);
    app.run("find.pane", None);
    assert!(app.picker.is_none());
    let (msg, _) = app.note.clone().expect("should have said why");
    assert!(msg.contains("only this pane"), "got: {msg}");
}

#[test]
fn the_pane_picker_lists_both_halves_of_a_split() {
    let mut app = app();
    four(&mut app);
    app.run("window.split-right", None);
    app.run("find.pane", None);
    assert_eq!(picker_labels(&app).len(), 2);
}

#[test]
fn the_namespace_picker_switches_the_pane_to_the_one_chosen() {
    let mut app = app();
    app.handle(Msg::Namespaces(Ok(vec![
        NamespaceInfo {
            name: "default".into(),
            state: "Registered".into(),
            retention_days: 3,
            description: String::new(),
        },
        NamespaceInfo {
            name: "payments".into(),
            state: "Registered".into(),
            retention_days: 7,
            description: String::new(),
        },
    ])));
    app.run("find.namespace", None);
    type_into_picker(&mut app, "pay");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));

    assert_eq!(app.view.scope, vec!["payments".to_string()]);
    assert_eq!(app.view.screen, Screen::Workflows);
}

#[test]
fn switching_namespace_keeps_the_query() {
    // "the same question, over there" is the common case; retyping the filter every
    // time would cost more than the navigation the picker saves.
    let mut app = app();
    app.handle(Msg::Namespaces(Ok(vec![NamespaceInfo {
        name: "payments".into(),
        state: "Registered".into(),
        retention_days: 7,
        description: String::new(),
    }])));
    app.view.query = "ExecutionStatus = 'Running'".into();
    app.run("find.namespace", None);
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert_eq!(app.view.query, "ExecutionStatus = 'Running'");
}

#[test]
fn the_problem_list_lands_in_the_query_bar_where_it_can_be_edited() {
    // A preset, not a separate screen: the query stays visible and narrowing it further
    // is ordinary editing.
    let mut app = app();
    four(&mut app);
    app.run("list.problems", None);
    assert!(app.view.query.contains("Failed"), "got: {}", app.view.query);
    assert!(app.view.query.contains("TimedOut"));
    assert!(app.view.query.contains("Terminated"));
    assert_eq!(app.view.screen, Screen::Workflows);
}

#[test]
fn a_first_search_from_the_top_does_not_claim_to_have_wrapped() {
    // Row 0 is where a freshly loaded pane puts the cursor, so the old `cursor - 1`
    // seek made almost every first search report a wrap.
    let mut app = app();
    four(&mut app);
    assert_eq!(app.view.cursor, 0);
    search_for(&mut app, "refund");
    let (msg, _) = app.note.clone().expect("a search reports itself");
    assert!(!msg.contains("wrapped"), "got: {msg}");
}

#[test]
fn a_search_that_matches_nothing_does_not_record_a_jump() {
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None); // a real jump, so there is something to lose
    app.run("nav.jump-back", None);

    search_for(&mut app, "nothing-matches-this");
    app.run("nav.jump-forward", None);
    assert_eq!(
        app.view.screen,
        Screen::History,
        "a failed search must not have truncated the forward list"
    );
}

#[test]
fn the_command_picker_finds_a_command_by_its_title() {
    // `:` matches ids and titles; `<leader>fh` matched only ids, so `failed` found
    // `list.problems` in one and not the other.
    let mut app = app();
    four(&mut app);
    app.run("find.command", None);
    type_into_picker(&mut app, "failed");
    let shown = picker_labels(&app);
    assert!(
        shown.iter().any(|l| l.starts_with("list.problems")),
        "got {shown:?}"
    );
}
