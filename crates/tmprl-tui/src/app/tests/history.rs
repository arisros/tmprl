//! Folds, follow mode, and history paging.

use super::*;

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
fn a_pending_reply_for_a_history_since_left_is_dropped() {
    let mut app = app();
    app.view.screen = Screen::History;
    app.view.viewing = Some(wf("default", "r1", 0));
    app.load_history();
    let stale = app.view.generation.wrapping_sub(1);

    app.handle(Msg::Pending {
        generation: stale,
        result: Ok(vec![PendingActivity::default()]),
    });
    assert!(app.view.pending.is_empty());

    app.handle(Msg::Pending {
        generation: app.view.generation,
        result: Ok(vec![PendingActivity::default()]),
    });
    assert_eq!(app.view.pending.len(), 1);
}

#[test]
fn opening_another_run_forgets_the_last_ones_pending_activities() {
    // SDKs number activity ids from "1" in every run, so a list left over from the
    // previous run would attach itself to this one's rows.
    let mut app = app();
    app.view.screen = Screen::History;
    app.view.viewing = Some(wf("default", "r1", 0));
    app.view.pending = vec![PendingActivity::default()];
    app.run("app.refresh", None);
    assert!(app.view.pending.is_empty());
}

#[test]
fn a_failed_describe_warns_and_keeps_what_was_known() {
    let mut app = app();
    app.view.pending = vec![PendingActivity::default()];
    app.handle(Msg::Pending {
        generation: app.view.generation,
        result: Err("permission denied".into()),
    });
    assert_eq!(app.view.pending.len(), 1);
    let (note, level) = app.note.clone().expect("should warn");
    assert_eq!(level, Note::Warn);
    assert!(note.contains("permission denied"), "{note}");
}
