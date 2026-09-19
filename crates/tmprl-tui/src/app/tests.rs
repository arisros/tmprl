use super::yank::json_string;
use super::*;
use tmprl_core::{Key, WorkflowStatus};
use tokio::sync::mpsc::unbounded_channel;

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
fn the_editor_is_refused_away_from_a_history() {
    let mut app = app();
    four(&mut app);
    app.run("payload.edit", None);
    assert!(app.editing.is_none());
    let (msg, level) = app.note.clone().expect("should have said why");
    assert_eq!(level, Note::Warn);
    assert!(msg.contains("history"), "got: {msg}");
}

#[test]
fn the_editor_writes_the_payloads_and_leaves_a_request_behind() {
    // The reducer's whole job here: choose and write. Spawning belongs to the event
    // loop, which is why this test never runs an editor.
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);

    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    let mut started = hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order");
    started.payloads.push((
        "input".into(),
        Payload::new("json/plain", br#"{"amount":100}"#.to_vec()),
    ));
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((vec![started], Vec::new())),
    });

    app.run("payload.edit", None);
    let request = app
        .take_edit_request()
        .expect("a request should be waiting");
    let written = std::fs::read_to_string(&request.path).expect("file should exist");
    assert!(written.contains("amount"), "got: {written}");
    assert!(
        request.what.contains("not saved back"),
        "the copy must say it is a copy: {}",
        request.what
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&request.path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "decoded payloads must not be group/world readable"
        );
    }

    // Finishing removes the copy: it is not a document, and leaving it would pile up
    // readable payloads in the temp directory for the rest of the session.
    app.finish_edit(&request, None);
    assert!(
        !request.path.exists(),
        "the copy should have been cleaned up"
    );
}

#[test]
fn an_unreadable_payload_is_reported_on_the_request_not_as_a_note() {
    // The note set during `open_editor` is never seen: the loop takes the request
    // before the next draw and `finish_edit` overwrites the note afterwards.
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);

    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    let mut started = hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order");
    started.payloads.push((
        "input".into(),
        Payload::new("json/plain", br#"{"amount":100}"#.to_vec()),
    ));
    started.payloads.push((
        "secret".into(),
        Payload::new("binary/encrypted", vec![1, 2, 3]),
    ));
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((vec![started], Vec::new())),
    });

    app.run("payload.edit", None);
    let request = app
        .take_edit_request()
        .expect("a request should be waiting");
    assert!(
        request.what.contains("secret"),
        "the skipped payload must reach the user: {}",
        request.what
    );
    app.finish_edit(&request, None);
}

#[test]
fn taking_the_edit_request_clears_it() {
    // The loop must not open the same file twice on the next pass round.
    let mut app = app();
    app.editing = Some(EditRequest {
        path: std::path::PathBuf::from("/tmp/tmprl-nonexistent/x.json"),
        dir: std::path::PathBuf::from("/tmp/tmprl-nonexistent"),
        what: "test".into(),
    });
    assert!(app.take_edit_request().is_some());
    assert!(app.take_edit_request().is_none());
}

// ---- jumplist ----

#[test]
fn opening_a_workflow_and_jumping_back_returns_to_the_list() {
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);

    app.run("nav.jump-back", None);
    assert_eq!(app.view.screen, Screen::Workflows);
}

#[test]
fn jumping_forward_returns_to_the_workflow() {
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None);
    let opened = app.view.viewing.clone();

    app.run("nav.jump-back", None);
    assert_eq!(app.view.screen, Screen::Workflows);

    app.run("nav.jump-forward", None);
    assert_eq!(app.view.screen, Screen::History);
    assert_eq!(app.view.viewing, opened);
}

#[test]
fn jumping_back_with_nowhere_to_go_says_so() {
    let mut app = app();
    four(&mut app);
    app.run("nav.jump-back", None);
    let (msg, level) = app.note.clone().expect("should have explained itself");
    assert_eq!(level, Note::Warn);
    assert!(msg.contains("earlier"), "got: {msg}");
}

#[test]
fn ordinary_motion_is_not_a_jump() {
    // The whole point: a jumplist that recorded every `j` is a scroll history, and
    // `<C-o>` stops being worth pressing.
    let mut app = app();
    four(&mut app);
    app.run("motion.down", None);
    app.run("motion.down", None);
    app.run("nav.jump-back", None);
    assert!(app.jumps.is_empty(), "j must not have recorded anything");
}

#[test]
fn gg_is_a_jump_so_you_can_get_back_from_it() {
    let mut app = app();
    four(&mut app);
    app.run("motion.down", None);
    app.run("motion.down", None);
    let before = app.view.cursor;
    assert_eq!(before, 2);

    app.run("motion.top", None);
    assert_eq!(app.view.cursor, 0);

    app.run("nav.jump-back", None);
    assert_eq!(app.view.cursor, before, "back to where gg was pressed from");
}

#[test]
fn a_new_jump_discards_the_forward_history() {
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None); // workflows -> history
    app.run("nav.jump-back", None); // back to workflows
    assert_eq!(app.view.screen, Screen::Workflows);

    // A fresh jump from here. `<C-i>` must not offer the history any more.
    app.run("motion.bottom", None);
    app.run("nav.jump-forward", None);
    let (msg, _) = app.note.clone().expect("should have refused");
    assert!(msg.contains("later"), "got: {msg}");
}

#[test]
fn a_jump_back_to_a_workflow_list_refetches_rather_than_restoring_a_stale_one() {
    // The list is re-requested, so coming back shows what is there now. Asserting on
    // the generation because that is what a fetch bumps.
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None);
    let before = app.view.generation;
    app.run("nav.jump-back", None);
    assert_ne!(app.view.generation, before, "should have issued a fetch");
}

#[test]
fn the_problem_list_is_a_jump() {
    let mut app = app();
    four(&mut app);
    let query = app.view.query.clone();
    app.run("list.problems", None);
    assert_ne!(app.view.query, query);
    app.run("nav.jump-back", None);
    assert_eq!(app.view.query, query, "back to the query it replaced");
}

#[test]
fn tab_jumps_forward_because_it_is_the_same_key_as_ctrl_i() {
    // Ctrl+I is byte 0x09 on a terminal, which arrives as Tab. Binding only `<C-i>`
    // gives a key that never fires.
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None);
    app.run("nav.jump-back", None);
    assert_eq!(app.view.screen, Screen::Workflows);

    app.handle(Msg::Key(Chord::plain(Key::Tab)));
    assert_eq!(app.view.screen, Screen::History, "Tab should jump forward");
}

#[test]
fn failing_to_find_a_pane_leaves_the_tab_where_it_was() {
    // Finding a pane in another tab means rotating, because Tabs cannot be inspected
    // without being made current. A miss must undo the rotation.
    let mut app = app();
    four(&mut app);
    app.run("tab.new", None);
    app.run("tab.new", None);
    let before = app.tabs.index();

    app.focus_pane(ViewId(9999));
    assert_eq!(app.tabs.index(), before, "a miss must not move the tab");
    let (msg, _) = app.note.clone().expect("should have said so");
    assert!(msg.contains("gone"), "got: {msg}");
}

#[test]
fn jumping_back_to_a_list_returns_to_the_workflow_not_the_row_number() {
    // A live list grows at the top. Restoring a bare index would put the cursor on
    // whatever has since taken that slot.
    let mut app = app();
    four(&mut app);
    app.run("motion.down", None);
    let left = at_cursor(&app);

    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);

    app.run("nav.jump-back", None);
    // Two newer workflows arrive while we were away, pushing everything down.
    let mut rows = vec![wf("default", "r9", 900), wf("default", "r8", 800)];
    for run in ["r4", "r3", "r2", "r1"] {
        let start = 100 * run[1..].parse::<i64>().unwrap();
        rows.push(wf("default", run, start));
    }
    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: false,
        result: Ok((rows, vec![])),
    });

    assert_eq!(
        at_cursor(&app),
        left,
        "should have followed the workflow down"
    );
}

// ---- regressions from the review of #17 ----

#[test]
fn picking_a_workflow_from_inside_a_history_does_not_merge_the_two() {
    // The bug this pins: `open_workflow` set `viewing` and called `load_history`
    // without clearing the previous run's events or continuation token. `load_history`
    // then saw non-empty events, skipped bumping the generation, sent r1's page token
    // to r2, and merged the reply into r1's events, so one outline showed two
    // workflows.
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);

    // A paged history: a non-empty token is what made the old code send the wrong one.
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((history_events(), b"page-2".to_vec())),
    });
    assert!(!app.view.history_events.is_empty());
    assert!(!app.view.history_token.is_empty());
    let first = app.view.viewing.clone().unwrap().run_id;

    app.run("find.workflow", None);
    type_into_picker(&mut app, "r2");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));

    assert_ne!(app.view.viewing.clone().unwrap().run_id, first);
    assert!(
        app.view.history_events.is_empty(),
        "the previous run's events must not survive"
    );
    assert!(
        app.view.history_token.is_empty(),
        "the previous run's page token must not be sent to this one"
    );
    assert!(app.view.history_resume.is_empty());
}

#[test]
fn the_problem_list_clears_the_history_it_leaves_behind() {
    // `load_history` passes `history_token` unconditionally, so a token left over from
    // an abandoned history goes to whichever run is opened next.
    let mut app = app();
    four(&mut app);
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((history_events(), b"page-2".to_vec())),
    });

    app.run("list.problems", None);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert!(app.view.history_token.is_empty(), "token must not survive");
    assert!(app.view.history_resume.is_empty());
    assert!(app.view.history_events.is_empty());
}

#[test]
fn a_refused_dash_does_not_destroy_the_forward_jumps() {
    // `Jumplist::push` truncates everything ahead of the cursor, so marking a jump for
    // a move that turns out to be refused silently empties the `<C-i>` list.
    //
    // Getting to the state that shows it needs care: a *legal* `-` truncates the
    // forward entries too, and correctly so. What is needed is to be sitting on the
    // namespace list, where `-` is refused, with forward entries still ahead.
    let mut app = app();
    app.handle(Msg::Namespaces(Ok(vec![NamespaceInfo {
        name: "default".into(),
        state: "Registered".into(),
        retention_days: 3,
        description: String::new(),
    }])));
    app.view.screen = Screen::Namespaces;

    app.run("nav.open", None); // namespaces -> workflows
    four(&mut app);
    app.run("nav.open", None); // workflows -> history
    assert_eq!(app.view.screen, Screen::History);

    app.run("nav.jump-back", None); // -> workflows
    app.run("nav.jump-back", None); // -> namespaces, with two entries ahead
    assert_eq!(app.view.screen, Screen::Namespaces);

    let ahead = app.jumps.len();
    app.run("nav.up", None); // refused: already at the top level
    assert_eq!(
        app.jumps.len(),
        ahead,
        "a refused move must not touch the jumplist"
    );

    app.run("nav.jump-forward", None);
    assert_eq!(
        app.view.screen,
        Screen::Workflows,
        "the forward list should still lead back down"
    );
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

#[test]
fn json_strings_escape_control_characters() {
    assert_eq!(json_string(r#"a"b"#), r#""a\"b""#);
    assert_eq!(json_string("a\nb"), r#""a\nb""#);
    assert_eq!(json_string("a\u{1}b"), r#""a\u0001b""#);
}

#[test]
fn the_cursor_stays_on_its_workflow_when_a_newer_one_arrives() {
    // The whole reason the cursor is anchored to a run id. A live list grows at the
    // top; an index-based cursor would quietly select a different workflow.
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("motion.top", None);
    assert_eq!(app.workflow_rows()[app.view.cursor].run_id, "r1");

    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: false,
        result: Ok((
            vec![wf("default", "r9", 900), wf("default", "r1", 100)],
            vec![],
        )),
    });
    assert_eq!(
        app.view.cursor, 1,
        "cursor should have followed r1 down a row"
    );
    assert_eq!(app.workflow_rows()[app.view.cursor].run_id, "r1");
}

#[test]
fn a_reply_for_a_superseded_query_is_dropped() {
    // Type a new query while the old one is still in flight: the stale reply must not
    // repaint the table with rows the user is no longer looking at.
    let mut app = app();
    loaded(&mut app, vec![wf("default", "old", 100)], vec![]);
    let stale = app.view.generation;

    app.view.query = "WorkflowType = 'New'".into();
    app.load_workflows(false);
    assert_ne!(app.view.generation, stale);

    app.handle(Msg::Workflows {
        generation: stale,
        append: false,
        result: Ok((vec![wf("default", "stale", 1)], vec![])),
    });
    assert_eq!(
        app.workflow_rows()[0].run_id,
        "old",
        "a stale reply must not land"
    );
}

#[test]
fn a_failed_extra_page_keeps_the_rows_already_on_screen() {
    let mut app = app();
    loaded(
        &mut app,
        vec![wf("default", "r1", 100)],
        vec![("default".into(), vec![1])],
    );
    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: true,
        result: Err("connection reset".into()),
    });
    assert_eq!(app.workflow_rows().len(), 1, "rows must survive");
    assert!(matches!(app.note, Some((_, Note::Error))));
}

#[test]
fn a_failed_first_page_shows_the_error_state() {
    let mut app = app();
    app.view.screen = Screen::Workflows;
    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: false,
        result: Err("permission denied".into()),
    });
    assert_eq!(app.view.workflows.error(), Some("permission denied"));
}

#[test]
fn enter_opens_a_namespace_and_dash_goes_back() {
    let mut app = app();
    app.view.namespaces = Loadable::loaded(vec![
        NamespaceInfo {
            name: "alpha".into(),
            state: "Registered".into(),
            retention_days: 1,
            description: String::new(),
        },
        NamespaceInfo {
            name: "beta".into(),
            state: "Registered".into(),
            retention_days: 1,
            description: String::new(),
        },
    ]);
    app.run("motion.bottom", None);
    app.run("nav.open", None);

    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(
        app.view.scope,
        ["beta"],
        "the focused namespace becomes the scope"
    );

    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert_eq!(app.view.cursor, 1, "the namespace cursor is restored");
}

#[test]
fn a_visual_selection_of_namespaces_opens_a_fan_out() {
    let mut app = app();
    app.view.namespaces = Loadable::loaded(
        ["alpha", "beta", "gamma"]
            .iter()
            .map(|n| NamespaceInfo {
                name: (*n).into(),
                state: "Registered".into(),
                retention_days: 1,
                description: String::new(),
            })
            .collect(),
    );

    // Driven through the keymap, not by calling `run` directly: the binding is half of
    // the feature, and a test that skips it cannot tell you the key does nothing.
    for chord in [
        Chord::ch('g'),
        Chord::ch('g'),
        Chord::ch('V'),
        Chord::ch('j'),
        Chord::plain(Key::Enter),
    ] {
        app.handle(Msg::Key(chord));
    }

    assert_eq!(app.view.scope, ["alpha", "beta"]);
    assert!(
        app.view.is_fanned_out(),
        "rows must be tagged with their namespace"
    );
    assert_eq!(app.mode, Mode::Normal, "opening ends the selection");
    assert!(app.view.anchor.is_none());
}

#[test]
fn opening_without_a_selection_scopes_to_one_namespace() {
    let mut app = app();
    app.view.namespaces = Loadable::loaded(vec![NamespaceInfo {
        name: "alpha".into(),
        state: "Registered".into(),
        retention_days: 1,
        description: String::new(),
    }]);
    app.run("nav.open", None);
    assert_eq!(app.view.scope, ["alpha"]);
    assert!(!app.view.is_fanned_out());
}

#[test]
fn insert_mode_edits_the_query_on_the_workflow_screen() {
    let mut app = app();
    loaded(&mut app, vec![], vec![]);
    app.view.query = "A = 1".into();

    app.run("mode.insert", None);
    assert!(app.is_editing_query());
    assert_eq!(app.insert_buf, "A = 1", "the edit starts from the query");

    app.handle(Msg::Key(Chord::plain(Key::Backspace)));
    type_chars(&mut app, "2");
    assert_eq!(app.query_display(), "A = 2");

    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert_eq!(app.view.query, "A = 2", "Enter applies the query");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn escape_abandons_a_query_edit() {
    let mut app = app();
    loaded(&mut app, vec![], vec![]);
    app.view.query = "A = 1".into();

    app.run("mode.insert", None);
    type_chars(&mut app, "999");
    app.handle(Msg::Key(Chord::plain(Key::Esc)));

    assert_eq!(app.view.query, "A = 1", "Esc must not apply the edit");
    assert_eq!(app.query_display(), "A = 1");
}

#[test]
fn insert_mode_on_the_namespace_screen_is_not_the_query_bar() {
    let mut app = app();
    app.run("mode.insert", None);
    assert!(!app.is_editing_query());
    type_chars(&mut app, "xy");
    assert_eq!(app.insert_buf, "xy");
    assert_eq!(app.view.query, "", "the namespace screen has no query bar");
}

#[test]
fn a_saved_view_fills_the_query_bar_and_leaves_it_editable() {
    let mut app = app();
    let views = vec![SavedView {
        key: '1',
        name: "Broken".into(),
        query: "ExecutionStatus = 'Failed'".into(),
    }];
    app.registry.add_views(&views);
    app.views = views;

    app.run("view.1", None);
    assert_eq!(app.view.query, "ExecutionStatus = 'Failed'");
    assert_eq!(app.view.screen, Screen::Workflows);

    // Still text, still editable, a view is a bookmark, not a mode.
    app.run("mode.insert", None);
    assert_eq!(app.insert_buf, "ExecutionStatus = 'Failed'");
}

#[test]
fn scrolling_near_the_end_asks_for_the_next_page_once() {
    let mut app = app();
    app.view.page = 2;
    let rows: Vec<WorkflowRow> = (0..10)
        .map(|i| wf("default", &format!("r{i}"), 1000 - i))
        .collect();
    loaded(&mut app, rows, vec![("default".into(), vec![7])]);
    assert!(
        !app.view.loading_more,
        "a completed load clears the in-flight flag"
    );

    app.run("motion.bottom", None);
    assert!(
        app.view.loading_more,
        "reaching the end should request the next page"
    );

    // A second motion while that request is in flight must not queue another.
    app.run("motion.up", None);
    app.run("motion.bottom", None);
    assert!(app.view.loading_more);
}

#[test]
fn scrolling_does_not_page_when_the_list_is_complete() {
    let mut app = app();
    app.view.page = 2;
    let rows: Vec<WorkflowRow> = (0..5)
        .map(|i| wf("default", &format!("r{i}"), 1000 - i))
        .collect();
    loaded(&mut app, rows, vec![]);
    app.run("motion.bottom", None);
    assert!(
        !app.view.loading_more,
        "no token means nothing left to fetch"
    );
}

#[test]
fn yank_on_a_workflow_row_takes_the_workflow_id() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    assert_eq!(app.field_under_cursor(), "order-r1");

    let record = app.records_selected();
    assert!(
        record.contains(r#""workflowId":"order-r1""#),
        "got {record}"
    );
    assert!(record.contains(r#""status":"Running""#), "got {record}");
    assert!(record.contains(r#""namespace":"default""#), "got {record}");
}

#[test]
fn a_visual_selection_yanks_every_selected_workflow() {
    let mut app = app();
    loaded(
        &mut app,
        vec![wf("default", "r1", 300), wf("default", "r2", 200)],
        vec![],
    );
    app.run("motion.top", None);
    app.run("mode.visual", None);
    app.run("motion.down", None);

    let record = app.records_selected();
    assert!(record.starts_with('['), "a multi-row yank is an array");
    assert!(record.contains("order-r1") && record.contains("order-r2"));
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

#[test]
fn opening_a_workflow_reads_its_history() {
    let app = viewing_history();
    assert_eq!(app.view.viewing.as_ref().unwrap().run_id, "r1");
    // Three groups: the workflow and two activities. The workflow task is plumbing.
    assert_eq!(app.row_count(), 3);
}

#[test]
fn dash_returns_to_the_workflow_it_came_from() {
    let mut app = viewing_history();
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert!(app.view.viewing.is_none());
    assert!(
        app.view.history.value().is_none(),
        "leaving must drop the history rather than show a stale one on re-entry"
    );
}

#[test]
fn folding_a_group_shows_its_events_and_keeps_the_cursor_on_it() {
    let mut app = viewing_history();
    app.run("motion.down", None); // onto the "Charge" activity
    let before = app.row_count();
    let at = app.view.cursor;

    app.run("history.fold", None);
    assert_eq!(app.row_count(), before + 3, "its three events appeared");
    assert_eq!(
        app.view.cursor, at,
        "the cursor stays on the group's own line"
    );

    // Folding shut from *inside* the group must not strand the cursor past the end.
    app.run("motion.down", None);
    app.run("motion.down", None);
    app.run("history.fold", None);
    assert_eq!(app.row_count(), before);
    assert_eq!(app.view.cursor, at);
}

#[test]
fn expanding_everything_keeps_the_cursor_on_the_same_group() {
    let mut app = viewing_history();
    app.run("motion.bottom", None); // the failed "Ship" activity
    let group = app.group_under_cursor();

    app.run("history.expand-all", None);
    assert_eq!(
        app.group_under_cursor(),
        group,
        "expanding moves every row; the cursor must follow its group"
    );

    app.run("history.collapse-all", None);
    assert_eq!(app.group_under_cursor(), group);
}

#[test]
fn workflow_tasks_are_hidden_until_asked_for() {
    let mut app = viewing_history();
    assert_eq!(app.row_count(), 3);

    app.run("history.plumbing", None);
    assert_eq!(app.row_count(), 4, "the workflow-task group appeared");
    assert!(matches!(app.note, Some((_, Note::Info))));

    app.run("history.plumbing", None);
    assert_eq!(app.row_count(), 3);
}

#[test]
fn failures_are_reachable_by_key() {
    let mut app = viewing_history();
    app.run("motion.top", None);
    app.run("history.next-failure", None);

    let group = app.group_under_cursor().expect("on a group");
    let outline = app.view.history.value().unwrap();
    assert_eq!(outline.group(group).unwrap().subject, "Ship");

    // Saying so beats moving the cursor nowhere and looking broken.
    app.run("history.next-failure", None);
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn a_second_history_page_is_appended_and_regrouped() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);

    // First page stops mid-group: "Charge" is scheduled but has not finished.
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((history_events()[..5].to_vec(), vec![7])),
    });
    let charge = app.view.history.value().unwrap().group(2).unwrap().clone();
    assert!(charge.is_open(), "the group is incomplete on page one");

    // The rest arrives and completes it, which is why pages are re-grouped whole.
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((history_events()[5..].to_vec(), Vec::new())),
    });
    let charge = app.view.history.value().unwrap().group(2).unwrap();
    assert!(!charge.is_open(), "the second page closed the group");
    assert_eq!(app.row_count(), 3);
}

#[test]
fn a_stale_history_reply_is_dropped() {
    let mut app = viewing_history();
    let stale = app.view.generation;
    app.view.generation = app.view.generation.wrapping_add(1);

    app.handle(Msg::History {
        generation: stale,
        result: Ok((Vec::new(), Vec::new())),
    });
    assert_eq!(
        app.row_count(),
        3,
        "a reply for an abandoned read must not land"
    );
}

#[test]
fn yanking_a_history_row_takes_something_useful() {
    let mut app = viewing_history();
    app.run("motion.bottom", None);
    assert_eq!(app.field_under_cursor(), "Ship");

    let record = app.records_selected();
    assert!(record.contains(r#""group":"Ship""#), "got {record}");
    assert!(record.contains(r#""outcome":"Failed""#), "got {record}");
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

#[test]
fn follow_starts_and_stops_on_the_same_key() {
    let mut app = viewing_running();
    assert!(!app.view.following);

    app.run("history.follow", None);
    assert!(app.view.following, "F should start following");
    assert!(matches!(app.note, Some((_, Note::Info))));

    app.run("history.follow", None);
    assert!(!app.view.following, "F again should stop");
}

#[test]
fn follow_refuses_on_a_workflow_that_has_already_closed() {
    // Polling a closed workflow waits for events that can never arrive. "Closed" means
    // the *workflow* group has a terminal event, an activity finishing is not enough.
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);

    let mut events = history_events();
    events.push(hev(9, G::Workflow, R::Closes, C::Workflow).with_outcome(O::Completed));
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((events, Vec::new())),
    });

    assert!(app.view.history_token.is_empty());
    app.run("history.follow", None);

    assert!(!app.view.following, "there is nothing to follow");
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("closed"), "got {msg}");
}

#[test]
fn follow_is_not_offered_away_from_a_history() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("history.follow", None);
    assert!(!app.view.following);
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn an_empty_token_while_following_means_the_workflow_closed() {
    let mut app = viewing_running();
    app.run("history.follow", None);
    assert!(app.view.following);

    // The long poll returns the terminal event with no continuation token.
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((
            vec![hev(9, G::Workflow, R::Closes, C::Workflow).with_outcome(O::Completed)],
            Vec::new(),
        )),
    });

    assert!(
        !app.view.following,
        "follow must stop when the workflow closes"
    );
    let (msg, _) = app.note.clone().unwrap();
    assert!(msg.contains("closed"), "got {msg}");
}

#[test]
fn replayed_events_do_not_duplicate_when_follow_resumes() {
    // Follow resumes from the last non-empty token, which replays that page.
    let mut app = viewing_running();
    let before = app.view.history_events.len();

    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((running_history(), vec![9])),
    });
    assert_eq!(
        app.view.history_events.len(),
        before,
        "a replayed page must not be appended twice"
    );
}

#[test]
fn the_resume_token_is_the_last_non_empty_one() {
    // Paging leaves history_token empty once caught up; following from that would
    // restart the read at event 1.
    let mut app = viewing_running();
    assert_eq!(app.view.history_resume, vec![9]);

    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((Vec::new(), Vec::new())),
    });
    assert!(app.view.history_token.is_empty(), "caught up");
    assert_eq!(
        app.view.history_resume,
        vec![9],
        "the resume point is remembered"
    );
}

#[test]
fn leaving_the_history_stops_following() {
    let mut app = viewing_running();
    app.run("history.follow", None);
    assert!(app.view.following);

    app.run("nav.up", None);
    assert!(
        !app.view.following,
        "a poll must not outlive the screen it feeds"
    );
    assert!(app.view.history_resume.is_empty());
}

#[test]
fn yanking_the_result_unwraps_it() {
    let mut app = viewing_payloads();
    app.run("motion.down", None); // the Charge group
    app.run("yank.payload-result", None);
    let (note, level) = app.note.clone().expect("a yank should report");
    assert!(note.contains("yanked"), "{note}");
    assert!(matches!(level, Note::Info));
}

#[test]
fn yanking_the_input_skips_the_result() {
    let mut app = viewing_payloads();
    app.run("motion.down", None); // the Charge group
    let all = app.payloads_under_cursor();
    assert!(
        all.iter().any(|(l, _)| l == "input") && all.iter().any(|(l, _)| l == "result"),
        "fixture should carry both"
    );
    // The filter is what separates them; the clipboard itself is not reachable in a test.
    app.run("yank.payload-input", None);
    assert!(app.note.as_ref().unwrap().0.contains("yanked"));
}

#[test]
fn yanking_a_payload_off_a_history_is_refused() {
    let mut app = app();
    app.run("yank.payload", None);
    let (note, level) = app.note.clone().expect("a refusal should be reported");
    assert!(note.contains("workflow history"), "{note}");
    assert!(matches!(level, Note::Warn));
}

#[test]
fn yanking_an_absent_part_says_so() {
    let mut app = viewing_payloads();
    // Row 0 is the opening group, which carries no payloads at all.
    app.run("motion.top", None);
    app.run("yank.payload-result", None);
    let (note, level) = app.note.clone().unwrap();
    assert!(note.contains("no result"), "got {note}");
    assert_eq!(level, Note::Warn);
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

#[test]
fn the_pipe_prompt_gathers_a_group_s_input_and_result() {
    let mut app = viewing_payloads();
    app.run("motion.down", None); // the Charge group
    let payloads = app.payloads_under_cursor();
    let labels: Vec<&str> = payloads.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(
        labels,
        ["input", "result"],
        "a group's arguments and its result live on two different events"
    );
}

#[test]
fn the_pipe_prompt_opens_prefilled_with_jq() {
    let mut app = viewing_payloads();
    app.run("motion.down", None);
    app.run("payload.pipe", None);

    let p = app.prompt.clone().expect("a prompt should open");
    assert_eq!(p.kind, PromptKind::Pipe);
    assert_eq!(
        p.buf, "jq .",
        "an empty prompt means retyping jq every time"
    );
    assert_eq!(p.sigil(), "!");
}

#[test]
fn piping_is_refused_when_nothing_readable_is_under_the_cursor() {
    let mut app = viewing_payloads();
    app.run("motion.bottom", None); // the encrypted group
    app.run("payload.pipe", None);

    assert!(app.prompt.is_none(), "there is nothing worth piping");
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(
        msg.contains("encrypted"),
        "the reason should be given: {msg}"
    );
}

#[test]
fn piping_is_refused_away_from_a_history() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("payload.pipe", None);
    assert!(app.prompt.is_none());
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn a_filter_result_is_dropped_when_the_cursor_moves() {
    // The output belonged to the row it was run on; leaving it up under a different
    // heading would be a lie.
    let mut app = viewing_payloads();
    app.run("motion.down", None);
    app.view.piped = Some(Ok("{}".into()));
    app.run("motion.down", None);
    assert!(app.view.piped.is_none());
}

#[test]
fn a_pipe_result_message_opens_the_pane_and_lands() {
    let mut app = viewing_payloads();
    app.handle(Msg::Piped(Ok("42\n".into())));
    assert_eq!(app.view.piped, Some(Ok("42\n".into())));
    assert_eq!(app.view.detail_scroll, 0);
}

#[test]
fn both_prompts_edit_the_same_way() {
    // `:` and `!` share their editing; only Enter differs. This pins that they do.
    use tmprl_core::Key;
    for open in ["app.command-line", "payload.pipe"] {
        let mut app = viewing_payloads();
        app.run("motion.down", None);
        app.run(open, None);
        let start = app.prompt.clone().unwrap().buf.len();

        app.handle(Msg::Key(Chord::ch('x')));
        assert_eq!(app.prompt.clone().unwrap().buf.len(), start + 1, "{open}");
        app.handle(Msg::Key(Chord::plain(Key::Backspace)));
        assert_eq!(app.prompt.clone().unwrap().buf.len(), start, "{open}");
        app.handle(Msg::Key(Chord::plain(Key::Esc)));
        assert!(app.prompt.is_none(), "{open}: Esc should close");
        assert_eq!(app.mode, Mode::Normal, "{open}");
    }
}

#[test]
fn backspace_on_an_empty_prompt_closes_it() {
    use tmprl_core::Key;
    let mut app = viewing_payloads();
    app.run("app.command-line", None);
    app.handle(Msg::Key(Chord::plain(Key::Backspace)));
    assert!(app.prompt.is_none(), "as it does in vim");
}

/// The runner is IO, so it is exercised against real commands rather than mocked.
#[tokio::test]
async fn a_filter_receives_the_payloads_on_stdin() {
    let out = pipe_through("cat", br#"{"a":1}"#.to_vec()).await.unwrap();
    assert_eq!(out, r#"{"a":1}"#);
}

#[tokio::test]
async fn a_failing_filter_reports_the_command_s_own_stderr() {
    // When a jq expression is wrong, jq's message is the entire diagnosis; paraphrasing
    // it would lose the line and column.
    let err = pipe_through("echo 'boom' >&2; exit 3", Vec::new())
        .await
        .unwrap_err();
    assert!(err.contains("boom"), "got {err:?}");
}

#[tokio::test]
async fn a_filter_that_exits_silently_still_reports_failure() {
    let err = pipe_through("exit 1", Vec::new()).await.unwrap_err();
    assert!(err.contains("exited"), "got {err:?}");
}

#[tokio::test]
async fn a_filter_that_ignores_its_input_does_not_error() {
    // `head -1` closes the pipe early; writing to a closed pipe is not a failure.
    let out = pipe_through("echo done", vec![b'x'; 1_000_000])
        .await
        .unwrap();
    assert_eq!(out.trim(), "done");
}

#[test]
fn a_decoded_payload_replaces_the_encrypted_one_everywhere() {
    // Replacing in place is what lets the pane, `!` piping and yanking all read the
    // plaintext without knowing a codec exists.
    let mut app = viewing_payloads();
    app.run("motion.bottom", None); // the encrypted group

    let encrypted = app
        .payloads_under_cursor()
        .into_iter()
        .next()
        .map(|(_, p)| p)
        .expect("an encrypted payload");
    assert!(encrypted.needs_codec());
    let key = App::payload_key(&encrypted);

    app.handle(Msg::Decoded(Ok(vec![(
        key,
        Payload::new("json/plain", br#"{"secret":true}"#.to_vec()),
    )])));

    let (_, now) = app
        .payloads_under_cursor()
        .into_iter()
        .next()
        .expect("still a payload");
    assert!(!now.needs_codec(), "it should be plaintext now");
    assert_eq!(
        now.render(),
        tmprl_core::payload::Rendered::Text("{\n  \"secret\": true\n}".into())
    );
    // And it is pipeable, which it was not before.
    assert!(now.pipeable().is_some());
}

#[test]
fn a_decode_failure_is_reported_and_can_be_retried() {
    // Leaving the in-flight set populated would make one failure permanent for the
    // session, with no way to ask again.
    let mut app = viewing_payloads();
    app.decoding.insert(42);
    app.handle(Msg::Decoded(Err("codec server returned 502".into())));

    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("502"), "the server's own words: {msg}");
    assert!(app.decoding.is_empty(), "a retry must be possible");
}

#[test]
fn a_failed_decode_is_remembered_rather_than_flashed() {
    // The note is gone by the next keystroke, and the badge it leaves behind is
    // identical to a payload nothing was ever asked about, so the reader is told nothing.
    let mut app = viewing_payloads();
    let p = Payload::new("binary/aes_comp", vec![1, 2, 3]);
    app.codec = Some(Arc::new(Codec::new("http://127.0.0.1:1", None)));
    app.decoding.insert(App::payload_key(&p));
    app.handle(Msg::Decoded(Err("connection refused".into())));

    match app.decode_state(&p) {
        DecodeState::Failed(why) => assert!(why.contains("connection refused"), "{why}"),
        other => panic!("expected a recorded failure, got {other:?}"),
    }
    // `R` is the retry: a codec fixed outside tmprl needs some way back.
    app.run("app.refresh", None);
    assert_eq!(app.decode_state(&p), DecodeState::Idle);
}

#[test]
fn the_same_ciphertext_is_only_decoded_once() {
    let a = Payload::new("binary/encrypted", vec![1, 2, 3]);
    let b = Payload::new("binary/encrypted", vec![1, 2, 3]);
    let c = Payload::new("binary/encrypted", vec![9, 9, 9]);
    assert_eq!(App::payload_key(&a), App::payload_key(&b));
    assert_ne!(App::payload_key(&a), App::payload_key(&c));
}

#[test]
fn a_payload_key_distinguishes_encodings_with_identical_bytes() {
    let a = Payload::new("binary/encrypted", vec![1, 2, 3]);
    let b = Payload::new("binary/plain", vec![1, 2, 3]);
    assert_ne!(App::payload_key(&a), App::payload_key(&b));
}

#[test]
fn nothing_is_decoded_without_a_configured_codec() {
    // No endpoint means no request; the badge stays and says what is needed.
    let mut app = viewing_payloads();
    app.run("motion.bottom", None);
    app.run("history.detail", None);
    assert!(app.decoding.is_empty(), "there is nowhere to send it");
}

#[test]
fn a_config_without_a_codec_section_is_not_an_error() {
    let mut app = app();
    app.apply_config(None, None, Some("# nothing here\n"));
    assert!(app.note.is_none());
    assert!(app.codec.is_none());
}

#[test]
fn a_broken_config_is_surfaced() {
    let mut app = app();
    app.apply_config(None, None, Some("[codec]\nauth = \"x\"\n"));
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("codec.endpoint"), "got {msg}");
}

#[test]
fn a_configured_codec_is_used() {
    let mut app = app();
    app.apply_config(
        None,
        None,
        Some("[codec]\nendpoint = \"http://localhost:8081\"\n"),
    );
    assert!(app.note.is_none());
    assert!(app.codec.is_some());
}

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

fn on_a_workflow() -> App {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app
}

#[test]
fn a_mutation_key_only_opens_a_confirmation() {
    // Nothing reaches the cluster until the reader says yes.
    let mut app = on_a_workflow();
    app.run("workflow.terminate", None);

    let c = app.confirm.clone().expect("a confirmation should open");
    assert_eq!(c.first().verb(), "Terminate");
    assert_eq!(c.first().workflow_id(), "order-r1");
    assert_eq!(c.first().namespace(), "default");
}

#[test]
fn a_key_that_cannot_list_namespaces_still_lands_somewhere_usable() {
    // Temporal Cloud: the key is scoped to one namespace, so ListNamespaces is refused
    // while everything inside that namespace works. Stranding the reader on the opening
    // screen would make tmprl unusable against Cloud for no good reason.
    let mut app = App::detached("sit", "lora-sit.ixing", unbounded_channel().0);
    app.handle(Msg::Namespaces(Err(
        "code: 'The caller does not have permission to execute the specified operation', \
         message: \"Request unauthorized.\""
            .into(),
    )));

    let rows = app.namespace_rows();
    assert_eq!(rows.len(), 1, "the profile's own namespace is the fallback");
    assert_eq!(rows[0].name, "lora-sit.ixing");
    let (note, level) = app.note.clone().expect("the reader must be told why");
    assert!(note.contains("cannot list namespaces"), "{note}");
    assert!(note.contains("lora-sit.ixing"), "{note}");
    assert_eq!(
        level,
        Note::Info,
        "this is not an error, it is a scoped key"
    );
}

#[test]
fn a_real_namespace_failure_is_still_an_error() {
    // Only permission is special-cased; a transport failure must not be dressed up as a
    // one-row list, which would look like a cluster with one namespace.
    let mut app = App::detached("sit", "lora-sit.ixing", unbounded_channel().0);
    app.handle(Msg::Namespaces(Err("transport error".into())));

    assert!(app.namespace_rows().is_empty());
    assert_eq!(app.note.clone().unwrap().1, Note::Error);
}

#[test]
fn a_readonly_profile_refuses_before_a_confirmation_opens() {
    // The refusal costs one keystroke: a signal payload typed in full and then rejected
    // teaches the reader nothing useful.
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));
    assert!(
        app.readonly(),
        "config.toml should have marked prod read-only"
    );

    app.run("workflow.terminate", None);
    assert!(app.confirm.is_none(), "no confirmation may open");
    let (text, level) = app.note.clone().expect("a refusal should be reported");
    assert!(
        text.contains("prod"),
        "the refusal names the profile: {text}"
    );
    assert!(text.contains("read-only"), "{text}");
    assert!(matches!(level, Note::Warn));
}

#[test]
fn a_readonly_profile_refuses_at_the_wire_too() {
    // The guard that matters: whatever route a mutation took to get here, this is the
    // only path to the cluster.
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));

    // Batch and single share this path, so guarding it covers both.
    app.run_mutations(vec![Mutation::Terminate {
        namespace: "default".into(),
        workflow_id: "order-r1".into(),
        run_id: "r1".into(),
        reason: "x".into(),
    }]);
    let (text, _) = app.note.clone().expect("a refusal should be reported");
    assert!(text.contains("read-only"), "{text}");
}

#[test]
fn a_profile_without_a_readonly_flag_still_mutates() {
    let mut app = on_a_workflow();
    app.apply_config(None, None, Some("[profile.sit]\nreadonly = true"));
    assert!(
        !app.readonly(),
        "another profile's flag must not apply here"
    );

    app.run("workflow.terminate", None);
    assert!(app.confirm.is_some(), "a confirmation should still open");
}

#[test]
fn a_confirmation_owns_every_key_while_it_is_up() {
    // Nothing bound elsewhere may fire while a destructive action is pending.
    let mut app = on_a_workflow();
    let before = app.view.cursor;
    app.run("workflow.terminate", None);

    app.handle(Msg::Key(Chord::ch('j')));
    assert_eq!(app.view.cursor, before, "j must not move the cursor");
    assert!(
        app.confirm.is_some(),
        "and must not dismiss the confirmation"
    );

    app.handle(Msg::Key(Chord::ch(' ')));
    assert!(
        app.which_key.is_empty(),
        "the leader must not open which-key"
    );
}

#[test]
fn escape_always_backs_out() {
    let mut app = on_a_workflow();
    app.run("workflow.terminate", None);
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Esc)));

    assert!(app.confirm.is_none());
    let (msg, _) = app.note.clone().unwrap();
    assert_eq!(msg, "cancelled");
}

#[test]
fn deleting_costs_a_word_and_nearly_is_not_enough() {
    let mut app = on_a_workflow();
    app.run("workflow.delete", None);
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word.as_deref(), Some("delete"));

    // Enter with the word unfinished is not a refusal, just not yet.
    for ch in "delet".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));
    assert!(app.confirm.is_some(), "still waiting for the word");

    app.handle(Msg::Key(Chord::ch('e')));
    assert!(app.confirm.clone().unwrap().is_satisfied());
}

#[test]
fn a_mutation_needs_something_under_the_cursor() {
    let mut app = app(); // namespace screen, nothing selected
    app.run("workflow.terminate", None);
    assert!(app.confirm.is_none());
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn a_history_screen_mutates_the_workflow_it_is_showing() {
    let mut app = on_a_workflow();
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);

    app.run("workflow.cancel", None);
    let c = app
        .confirm
        .clone()
        .expect("the open workflow is the target");
    assert_eq!(c.first().workflow_id(), "order-r1");
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

#[test]
fn a_mutation_over_a_selection_covers_every_selected_row() {
    let mut app = on_four_workflows();
    select(&mut app, 3);
    app.run("workflow.cancel", None);

    let c = app.confirm.clone().expect("confirmed");
    assert_eq!(c.len(), 3);
    assert!(c.is_batch());
    let ids: Vec<&str> = c.mutations.iter().map(|m| m.workflow_id()).collect();
    assert_eq!(ids, ["order-r1", "order-r2", "order-r3"]);
    assert_eq!(c.first().verb(), "Cancel");
}

#[test]
fn without_a_selection_a_mutation_still_covers_one_row() {
    // The batch path and the single path are the same code, so they cannot drift.
    let mut app = on_four_workflows();
    app.run("workflow.cancel", None);
    let c = app.confirm.clone().expect("confirmed");
    assert_eq!(c.len(), 1);
    assert!(!c.is_batch());
    assert_eq!(c.first().workflow_id(), "order-r1");
}

#[test]
fn a_selection_upwards_covers_the_same_rows_as_one_downwards() {
    let mut app = on_four_workflows();
    app.handle(Msg::Key(Chord::ch('j')));
    app.handle(Msg::Key(Chord::ch('j')));
    app.run("mode.visual-line", None);
    app.handle(Msg::Key(Chord::ch('k')));
    app.run("workflow.cancel", None);

    let c = app.confirm.clone().unwrap();
    let ids: Vec<&str> = c.mutations.iter().map(|m| m.workflow_id()).collect();
    assert_eq!(ids, ["order-r2", "order-r3"]);
}

#[test]
fn a_destructive_batch_costs_the_count_rather_than_a_keypress() {
    // One key is too cheap to end three workflows at once.
    let mut app = on_four_workflows();
    select(&mut app, 3);
    app.run("workflow.terminate", None);

    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word.as_deref(), Some("3"));
    assert!(!c.is_satisfied(), "Enter alone does not go ahead");

    for ch in "3".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    assert!(app.confirm.clone().unwrap().is_satisfied());
}

#[test]
fn a_batch_that_destroys_histories_still_costs_the_word() {
    // Delete outranks the count: it destroys the record itself.
    let mut app = on_four_workflows();
    select(&mut app, 2);
    app.run("workflow.delete", None);
    assert_eq!(
        app.confirm.clone().unwrap().typed_word.as_deref(),
        Some("delete")
    );
}

#[test]
fn a_single_non_destructive_action_still_costs_nothing() {
    let mut app = on_four_workflows();
    app.run("workflow.cancel", None);
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word, None);
    assert!(c.is_satisfied());
}

#[test]
fn running_a_batch_spends_the_selection() {
    // The rows it covered are about to change, so a second batch must not be one
    // keypress away from a range that no longer means what it did.
    let mut app = on_four_workflows();
    select(&mut app, 2);
    assert!(app.view.selection().is_some());
    app.run("workflow.cancel", None);
    // A cancel over two rows is destructive, so it owes the count first.
    app.handle(Msg::Key(Chord::ch('2')));
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.view.selection().is_none());
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn a_signal_over_a_selection_sends_the_same_name_to_each() {
    let mut app = on_four_workflows();
    select(&mut app, 2);
    app.run("workflow.signal", None);
    for ch in "retry".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let c = app.confirm.clone().expect("confirmed");
    assert_eq!(c.len(), 2);
    assert!(c.mutations.iter().all(|m| m.cli().contains("--name retry")));
    assert_eq!(c.typed_word, None, "a signal is not a loss");
}

#[test]
fn a_batch_reports_progress_rather_than_each_row_in_turn() {
    let mut app = on_four_workflows();
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::Cancel {
            namespace: "default".into(),
            workflow_id: "order-r1".into(),
            run_id: "r1".into(),
        }),
        result: Ok(()),
        batch: Some((2, 3)),
    });
    let (msg, _) = app.note.clone().unwrap();
    assert_eq!(msg, "cancelled 2/3");
}

#[test]
fn a_signal_asks_for_its_name_before_confirming() {
    let mut app = on_a_workflow();
    app.run("workflow.signal", None);
    assert!(app.confirm.is_none(), "a signal needs a name first");
    assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Signal);

    for ch in "retry".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let c = app.confirm.clone().expect("now it can be confirmed");
    assert!(
        c.first().cli().contains("--name retry"),
        "{}",
        c.first().cli()
    );
    assert!(!c.first().is_destructive(), "a signal is not a loss");
}

#[test]
fn a_reset_resolves_to_a_workflow_task_the_cursor_is_not_on() {
    // The rows the server needs are exactly the ones the outline folds away, so "reset
    // to here" walks back, and the confirmation shows which id it landed on.
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
    let mut app = on_a_workflow();
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((
            vec![
                hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("W"),
                hev(2, G::Opened(2), R::Opens, C::WorkflowTask),
                hev(3, G::Opened(2), R::Closes, C::WorkflowTask).with_outcome(O::Completed),
                hev(4, G::Opened(4), R::Opens, C::Activity).with_subject("A"),
                hev(5, G::Opened(4), R::Closes, C::Activity).with_outcome(O::Completed),
            ],
            Vec::new(),
        )),
    });
    app.run("motion.bottom", None); // the activity group, not a workflow task

    app.run("workflow.reset", None);
    let c = app.confirm.clone().expect("a confirmation");
    assert!(
        c.first().cli().contains("--event-id 3"),
        "should resolve back to the completed workflow task: {}",
        c.first().cli()
    );
    assert!(c.first().is_destructive(), "a reset abandons work");
}

#[test]
fn a_reset_needs_a_history_and_says_so() {
    let mut app = on_a_workflow(); // still on the workflow list
    app.run("workflow.reset", None);
    assert!(app.confirm.is_none());
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("history"), "got {msg}");
}

#[test]
fn a_history_with_no_completed_task_cannot_be_reset() {
    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    let mut app = on_a_workflow();
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((
            vec![hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("W")],
            Vec::new(),
        )),
    });
    app.run("workflow.reset", None);
    assert!(app.confirm.is_none(), "there is nowhere valid to reset to");
}

#[test]
fn an_update_asks_for_its_name_and_is_not_destructive() {
    let mut app = on_a_workflow();
    app.run("workflow.update", None);
    assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Update);
    assert_eq!(app.prompt.clone().unwrap().sigil(), "update:");

    for ch in "setLimit".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let c = app.confirm.clone().expect("a confirmation");
    assert!(
        c.first().cli().contains("update execute"),
        "{}",
        c.first().cli()
    );
    assert!(c.first().cli().contains("--name setLimit"));
    assert!(
        !c.first().is_destructive(),
        "an update adds, it does not end"
    );
}

#[test]
fn a_signal_and_an_update_do_not_get_confused() {
    let mut app = on_a_workflow();
    app.run("workflow.signal", None);
    for ch in "ping".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));
    let cli = app.confirm.clone().unwrap().first().cli();
    assert!(cli.contains("workflow signal"), "{cli}");
    assert!(!cli.contains("update"), "{cli}");
}

#[test]
fn a_finished_mutation_reports_and_refreshes() {
    let mut app = on_a_workflow();
    let m = Mutation::Cancel {
        namespace: "default".into(),
        workflow_id: "order-r1".into(),
        run_id: "r1".into(),
    };
    app.handle(Msg::Mutated {
        mutation: Box::new(m),
        result: Ok(()),
        batch: None,
    });
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Info);
    assert!(msg.contains("order-r1"), "got {msg}");
}

#[test]
fn a_failed_mutation_shows_the_servers_reason() {
    let mut app = on_a_workflow();
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::Cancel {
            namespace: "default".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
        }),
        result: Err("PermissionDenied: not allowed".into()),
        batch: None,
    });
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("PermissionDenied"), "got {msg}");
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

#[test]
fn creating_a_schedule_collects_every_field_then_confirms() {
    let mut app = on_schedules();
    app.run("schedule.create", None);
    let form = app.form.clone().expect("a form opens");
    assert_eq!(form.cursor, 0);
    assert!(app.confirm.is_none(), "nothing is proposed yet");

    for (i, v) in ["nightly", "recon", "OrderWorkflow", "demo-tq", "0 2 * * *"]
        .iter()
        .enumerate()
    {
        for ch in v.chars() {
            app.handle(Msg::Key(Chord::ch(ch)));
        }
        if i < 4 {
            app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Tab)));
        }
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.form.is_none(), "the form closes once it is complete");
    let cli = app.confirm.clone().expect("confirmed").first().cli();
    assert!(cli.contains("--schedule-id nightly"), "{cli}");
    assert!(cli.contains("--workflow-id recon"), "{cli}");
    assert!(cli.contains("--type OrderWorkflow"), "{cli}");
    assert!(cli.contains("--task-queue demo-tq"), "{cli}");
    assert!(cli.contains("--cron '0 2 * * *'"), "{cli}");
    assert!(!cli.contains("--input"), "input was left empty: {cli}");
}

#[test]
fn an_incomplete_schedule_sends_the_caret_to_the_field_that_is_missing() {
    // Naming the field without going to it leaves the reader hunting for it.
    let mut app = on_schedules();
    app.run("schedule.create", None);
    for ch in "nightly".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.confirm.is_none(), "nothing is proposed");
    let form = app.form.clone().expect("the form stays open");
    assert_eq!(form.fields[form.cursor].label, "workflow id");
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("workflow id"), "got {msg}");
}

#[test]
fn backspace_on_an_empty_field_steps_back_rather_than_closing_the_form() {
    // A form is several fields deep, so losing all of them to one key would be a trap.
    let mut app = on_schedules();
    app.run("schedule.create", None);
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Tab)));
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Backspace)));

    let form = app.form.clone().expect("still open");
    assert_eq!(form.fields[form.cursor].label, "schedule id");
}

#[test]
fn a_schedule_form_takes_literal_keys_rather_than_running_commands() {
    // `j` and `q` are bound in Normal mode; inside a field they are text.
    let mut app = on_schedules();
    app.run("schedule.create", None);
    for ch in "jq".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    assert_eq!(app.form.clone().unwrap().get("schedule id"), "jq");
    assert!(!app.should_quit);
}

#[test]
fn a_backfill_asks_for_its_window_before_confirming() {
    let mut app = on_schedules();
    app.run("schedule.backfill", None);
    assert!(app.confirm.is_none(), "a backfill needs a window first");
    assert_eq!(app.prompt.clone().unwrap().kind, PromptKind::Backfill);

    for ch in "-1d..now".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    let m = app
        .confirm
        .clone()
        .expect("now it can be confirmed")
        .first()
        .clone();
    assert_eq!(m.verb(), "Backfill");
    assert_eq!(m.schedule_id(), Some("nightly"));
    let cli = m.cli();
    assert!(cli.contains("--overlap-policy BufferAll"), "{cli}");
    assert!(!m.is_destructive(), "it starts runs, it destroys nothing");
}

#[test]
fn an_unreadable_backfill_window_stops_at_the_prompt() {
    // The server's answer to a bad range is less specific than the parser's, and
    // confirming first would put a nonsense command in front of the reader.
    let mut app = on_schedules();
    app.run("schedule.backfill", None);
    for ch in "yesterday".chars() {
        app.handle(Msg::Key(Chord::ch(ch)));
    }
    app.handle(Msg::Key(Chord::plain(tmprl_core::Key::Enter)));

    assert!(app.confirm.is_none(), "nothing to confirm");
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("START..END"), "got {msg}");
}

#[test]
fn pausing_toggles_towards_the_opposite_of_now() {
    // One key does both, so the target state is whatever the schedule is not.
    let mut app = on_schedules();
    app.run("schedule.pause", None);
    let m = app.confirm.clone().unwrap().first().clone();
    assert_eq!(m.verb(), "Pause");
    assert!(m.cli().ends_with("--pause"));

    app.confirm = None;
    app.view.schedules.value_mut().unwrap()[0].paused = true;
    app.run("schedule.pause", None);
    let m = app.confirm.clone().unwrap().first().clone();
    assert_eq!(m.verb(), "Resume");
    assert!(m.cli().ends_with("--unpause"));
}

#[test]
fn a_paused_schedule_shows_the_new_state_before_the_list_catches_up() {
    // ListSchedules is eventually consistent, so a refresh straight after the patch can
    // return the old state and contradict the message beside it.
    let mut app = on_schedules();
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::PauseSchedule {
            namespace: "default".into(),
            schedule_id: "nightly".into(),
            paused: true,
        }),
        result: Ok(()),
        batch: None,
    });
    assert!(app.view.schedule_rows()[0].paused);
}

#[test]
fn deleting_a_schedule_costs_the_typed_word() {
    let mut app = on_schedules();
    app.run("schedule.delete", None);
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.typed_word.as_deref(), Some("delete"));
    assert!(c.first().cli().starts_with("temporal schedule delete "));
}

#[test]
fn schedule_keys_need_a_schedule_under_the_cursor() {
    let mut app = app(); // namespace screen
    app.run("schedule.trigger", None);
    assert!(app.confirm.is_none());
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn gs_and_gw_switch_lists_within_a_namespace() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    assert_eq!(app.view.screen, Screen::Workflows);

    app.run("nav.schedules", None);
    assert_eq!(app.view.screen, Screen::Schedules);
    app.run("nav.workflows", None);
    assert_eq!(app.view.screen, Screen::Workflows);
}

#[test]
fn switching_lists_is_refused_from_a_namespace_or_a_history() {
    // From a history the reader is inside one workflow; jumping sideways loses the place.
    let mut app = app();
    app.run("nav.schedules", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert!(matches!(app.note, Some((_, Note::Warn))));

    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::History);
    app.run("nav.schedules", None);
    assert_eq!(app.view.screen, Screen::History, "still in the history");
}

#[test]
fn dash_from_schedules_goes_back_to_namespaces() {
    let mut app = on_schedules();
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
}

#[test]
fn config_errors_are_surfaced_rather_than_swallowed() {
    let mut app = app();
    app.apply_config(Some("[normal]\n\"x\" = \"nope.nope\"\n"), None, None);
    let (msg, kind) = app.note.clone().expect("a bad binding must be reported");
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("nope.nope"), "got {msg}");
}

#[test]
fn views_from_config_become_commands_and_bindings() {
    let mut app = app();
    app.apply_config(
        None,
        Some(
            "[[view]]\nkey = \"1\"\nname = \"Running\"\nquery = \"ExecutionStatus = 'Running'\"\n",
        ),
        None,
    );
    assert!(app.note.is_none(), "a valid config must not warn");
    assert_eq!(app.registry.get("view.1").unwrap().title, "Running");

    app.handle(Msg::Key(Chord::ch(' ')));
    app.handle(Msg::Key(Chord::ch('1')));
    assert_eq!(app.view.query, "ExecutionStatus = 'Running'");
}

#[test]
fn opening_nothing_says_so_instead_of_changing_screen() {
    let mut app = app();
    app.run("nav.open", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert!(matches!(app.note, Some((_, Note::Warn))));
}

#[test]
fn going_up_from_the_top_level_says_so() {
    let mut app = app();
    app.run("nav.up", None);
    assert_eq!(app.view.screen, Screen::Namespaces);
    assert!(matches!(app.note, Some((_, Note::Warn))));
}
