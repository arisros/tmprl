//! Moving between screens and rows, the jumplist, and the lists themselves.

use super::*;

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
