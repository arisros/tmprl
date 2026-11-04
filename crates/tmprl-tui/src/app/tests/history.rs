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

fn viewing(events: Vec<NormalizedEvent>) -> App {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);
    app.handle(Msg::History {
        generation: app.view.generation,
        result: Ok((events, vec![9])),
    });
    app
}

fn subject_under_cursor(app: &App) -> String {
    let group = app.group_under_cursor().expect("on a group");
    let outline = app.view.history.value().unwrap();
    outline.group(group).unwrap().subject.clone()
}

/// Two activities still open, `Charge` with id "1" and `Ship` with id "2".
fn two_open_activities() -> Vec<NormalizedEvent> {
    use tmprl_core::history::{Category as C, GroupRef as G, Role as R};
    let activity = |id: i64, subject: &str, activity_id: &str| {
        let mut e = hev(id, G::Opened(id), R::Opens, C::Activity).with_subject(subject);
        e.fields.push(("activityId", activity_id.to_string()));
        e
    };
    vec![
        hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
        activity(5, "Charge", "1"),
        activity(6, "Ship", "2"),
    ]
}

#[test]
fn a_failure_jump_stops_on_an_activity_between_tries() {
    let mut app = viewing(two_open_activities());
    app.view.pending = vec![
        PendingActivity {
            activity_id: "1".into(),
            attempt: 1,
            ..Default::default()
        },
        PendingActivity {
            activity_id: "2".into(),
            attempt: 4,
            ..Default::default()
        },
    ];
    app.run("motion.top", None);
    app.run("history.next-failure", None);
    assert_eq!(
        subject_under_cursor(&app),
        "Ship",
        "Charge is on its first try"
    );

    app.run("history.prev-failure", None);
    assert_eq!(
        app.note.as_ref().unwrap().0,
        "no failure above",
        "Charge, above, is not failing"
    );
    assert_eq!(subject_under_cursor(&app), "Ship");

    app.view.pending[0].attempt = 2;
    app.run("history.prev-failure", None);
    assert_eq!(subject_under_cursor(&app), "Charge");
}

#[test]
fn without_a_describe_an_open_activity_is_not_a_failure() {
    let mut app = viewing(two_open_activities());
    app.run("motion.top", None);
    app.run("history.next-failure", None);
    let (note, level) = app.note.clone().unwrap();
    assert_eq!((note.as_str(), level), ("no failure below", Note::Warn));
}

#[test]
fn a_failure_jump_shows_the_workflow_tasks_to_reach_a_failed_one() {
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
    let mut app = viewing(vec![
        hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order"),
        hev(2, G::Opened(2), R::Opens, C::WorkflowTask),
        hev(3, G::Opened(2), R::Closes, C::WorkflowTask).with_outcome(O::Failed),
    ]);
    assert_eq!(app.row_count(), 1, "workflow tasks start hidden");
    app.run("motion.top", None);
    app.run("history.next-failure", None);

    assert_eq!(app.row_count(), 2, "the workflow tasks were shown");
    let group = app.group_under_cursor().expect("on a group");
    let outline = app.view.history.value().unwrap();
    assert_eq!(outline.group(group).unwrap().category, C::WorkflowTask);
    let (note, level) = app.note.clone().unwrap();
    assert_eq!(level, Note::Warn);
    assert!(note.contains("zp"), "{note}");

    app.run("history.plumbing", None);
    assert_eq!(app.row_count(), 1, "zp still hides them");
}

#[test]
fn a_dropped_follow_says_it_is_retrying_and_keeps_following() {
    use tmprl_core::fault::{Code, Fault};
    let mut app = viewing_running();
    app.run("history.follow", None);

    app.handle(Msg::FollowRetrying {
        generation: app.view.generation,
        error: Fault::rpc(
            "GetWorkflowExecutionHistory",
            Code::Unavailable,
            "connection reset",
        ),
        wait: std::time::Duration::from_secs(4),
    });

    assert!(app.view.following);
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Warn);
    assert!(msg.contains("retrying in 4s"), "got {msg}");
}

#[test]
fn a_retry_notice_for_a_follow_already_stopped_is_dropped() {
    use tmprl_core::fault::{Code, Fault};
    let mut app = viewing_running();
    app.run("history.follow", None);
    app.run("history.follow", None);
    app.note = None;

    app.handle(Msg::FollowRetrying {
        generation: app.view.generation,
        error: Fault::rpc("GetWorkflowExecutionHistory", Code::Unavailable, ""),
        wait: std::time::Duration::from_secs(1),
    });

    assert!(app.note.is_none());
}

/// `events` long, three to an activity, every input carrying `payload_bytes` of JSON.
fn long_history(events: usize, payload_bytes: usize) -> Vec<NormalizedEvent> {
    use tmprl_core::history::{Category as C, GroupRef as G, Outcome as O, Role as R};
    let input = format!(r#"{{"blob":"{}"}}"#, "x".repeat(payload_bytes));
    let mut all = vec![hev(1, G::Workflow, R::Opens, C::Workflow).with_subject("Order")];
    let mut id = 2;
    while all.len() < events {
        let mut s = hev(id, G::Opened(id), R::Opens, C::Activity).with_subject("Charge");
        s.payloads.push((
            "input".into(),
            Payload::new("json/plain", input.clone().into_bytes()),
        ));
        all.push(s);
        all.push(hev(id + 1, G::Opened(id), R::Continues, C::Activity));
        all.push(hev(id + 2, G::Opened(id), R::Closes, C::Activity).with_outcome(O::Completed));
        id += 3;
    }
    all
}

fn load_in_pages(events: Vec<NormalizedEvent>) -> std::time::Duration {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("nav.open", None);
    let pages: Vec<Vec<_>> = events.chunks(500).map(<[_]>::to_vec).collect();
    let last = pages.len() - 1;
    let t = std::time::Instant::now();
    for (i, page) in pages.into_iter().enumerate() {
        let token = if i == last { Vec::new() } else { vec![1] };
        app.handle(Msg::History {
            generation: app.view.generation,
            result: Ok((page, token)),
        });
    }
    t.elapsed()
}

/// `cargo test --release -p tmprl -- --ignored bench_ --nocapture`. Timings, not asserts:
/// a machine's speed is not a property of the code.
#[test]
#[ignore]
fn bench_a_100k_event_history_loads_in_pages() {
    let took = load_in_pages(long_history(100_000, 64));
    eprintln!("100k events in 200 pages: {took:?}");
}

#[test]
#[ignore]
fn bench_a_history_at_temporals_50_mb_limit_loads_in_pages() {
    let took = load_in_pages(long_history(20_000, 7_500));
    eprintln!("20k events, 50 MB of inputs, in 40 pages: {took:?}");
}
