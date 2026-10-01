//! The note log behind `:messages`, and the failures it keeps the detail of.

use super::*;
use tmprl_core::fault::Code;

fn unavailable() -> Fault {
    Fault::rpc(
        "ListWorkflowExecutions",
        Code::Unavailable,
        "transport error",
    )
}

fn failed_list(app: &mut App, fault: Fault) {
    app.view.screen = Screen::Workflows;
    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: false,
        result: Err(fault),
    });
}

#[test]
fn a_failure_is_logged_with_its_code_and_the_call_it_came_from() {
    let mut app = app();
    failed_list(&mut app, unavailable());

    let logged = app.messages.back().expect("the failure must be logged");
    assert_eq!(logged.level, Note::Error);
    assert_eq!(
        logged.text,
        "ListWorkflowExecutions failed (unavailable): transport error"
    );
    let fault = logged.fault.as_ref().expect("the detail must be kept");
    assert_eq!(fault.code, Code::Unavailable);
    assert_eq!(fault.operation, "ListWorkflowExecutions");
}

#[test]
fn the_log_outlives_the_note_line() {
    // The note is gone by the next key. That is exactly when someone wants to read it
    // again, so the log must not go with it.
    let mut app = app();
    failed_list(&mut app, unavailable());
    app.handle(Msg::Key(Chord::ch('j')));

    assert!(app.note.is_none(), "a key still dismisses the note");
    assert_eq!(app.messages.len(), 1);
}

#[test]
fn a_note_is_logged_once_however_long_it_stays_up() {
    // Ticks and server replies leave the note where it is. Logging on every message that
    // finds a note present would fill the log with one line a second.
    let mut app = app();
    failed_list(&mut app, unavailable());
    for _ in 0..5 {
        app.handle(Msg::Tick);
    }

    assert!(app.note.is_some(), "a tick must not dismiss the note");
    assert_eq!(app.messages.len(), 1);
}

#[test]
fn the_same_note_twice_is_two_entries() {
    // Two refusals are two things that happened, even when they read the same.
    let mut app = app();
    failed_list(&mut app, unavailable());
    failed_list(&mut app, unavailable());
    assert_eq!(app.messages.len(), 2);
}

#[test]
fn an_ordinary_note_is_logged_without_a_failure_attached() {
    let mut app = app();
    app.handle(Msg::Key(Chord::ch(' ')));
    app.handle(Msg::Key(Chord::ch('T')));

    let logged = app
        .messages
        .back()
        .expect("an info note is still a message");
    assert_eq!(logged.level, Note::Info);
    assert!(logged.fault.is_none());
}

#[test]
fn a_failure_does_not_lend_its_detail_to_the_next_note() {
    let mut app = app();
    failed_list(&mut app, unavailable());
    app.handle(Msg::Key(Chord::ch(' ')));
    app.handle(Msg::Key(Chord::ch('T')));

    assert_eq!(app.messages.len(), 2);
    assert!(app.messages.back().unwrap().fault.is_none());
}

#[test]
fn the_log_is_bounded() {
    let mut app = app();
    for _ in 0..250 {
        failed_list(&mut app, unavailable());
    }
    assert_eq!(app.messages.len(), 200);
}

#[test]
fn a_refusal_to_list_namespaces_is_recognised_by_its_code() {
    // The wording differs between a self-hosted server and Temporal Cloud; the code does
    // not, and before the code survived to here only the wording could be matched.
    let mut app = app();
    app.handle(Msg::Namespaces(Err(Fault::rpc(
        "ListNamespaces",
        Code::PermissionDenied,
        "nope",
    ))));

    assert_eq!(app.namespace_rows().len(), 1, "the profile's namespace");
    assert_eq!(app.note.clone().unwrap().1, Note::Info);
}

#[test]
fn messages_opens_from_the_command_line_and_closes_on_escape() {
    let mut app = app();
    app.handle(Msg::Key(Chord::ch(':')));
    type_chars(&mut app, "messages");
    app.handle(Msg::Key(Chord::plain(Key::Enter)));
    assert!(app.show_messages, "{:?}", app.note);

    app.handle(Msg::Key(Chord::plain(Key::Esc)));
    assert!(!app.show_messages);
}

#[test]
fn only_one_overlay_is_open_at_a_time() {
    let mut app = app();
    app.run("app.help", None);
    app.run("app.messages", None);
    assert!(app.show_messages && !app.show_help);

    app.run("app.help", None);
    assert!(app.show_help && !app.show_messages);
}

#[test]
fn a_failed_schedule_list_is_logged_with_its_call() {
    let mut app = app();
    app.view.screen = Screen::Schedules;
    app.handle(Msg::Schedules {
        generation: app.view.generation,
        result: Err(Fault::rpc(
            "ListSchedules",
            Code::Unavailable,
            "transport error",
        )),
    });

    assert!(app.view.schedules.error().is_some());
    let fault = app.messages.back().unwrap().fault.clone().unwrap();
    assert_eq!(fault.operation, "ListSchedules");
}

#[test]
fn a_warning_worded_by_the_caller_still_keeps_the_failure_behind_it() {
    // "pending activities: ..." is the note's wording; the log must still know which code
    // it was, or the wording is all that is left to search a server log with.
    let mut app = app();
    app.handle(Msg::Pending {
        generation: app.view.generation,
        result: Err(Fault::rpc(
            "DescribeWorkflowExecution",
            Code::DeadlineExceeded,
            "slow",
        )),
    });

    let logged = app.messages.back().unwrap();
    assert_eq!(logged.level, Note::Warn);
    assert!(
        logged.text.starts_with("pending activities:"),
        "{}",
        logged.text
    );
    assert_eq!(logged.fault.as_ref().unwrap().code, Code::DeadlineExceeded);
}

#[test]
fn a_failed_mutation_is_logged_as_a_failure() {
    let mut app = app();
    four(&mut app);
    app.handle(Msg::Mutated {
        mutation: Box::new(Mutation::Cancel {
            namespace: "default".into(),
            workflow_id: "order-r1".into(),
            run_id: "r1".into(),
        }),
        result: Err(Fault::rpc(
            "RequestCancelWorkflowExecution",
            Code::PermissionDenied,
            "not allowed",
        )),
        batch: None,
    });

    let logged = app.messages.back().unwrap();
    assert_eq!(logged.level, Note::Error);
    assert_eq!(logged.fault.as_ref().unwrap().code, Code::PermissionDenied);
}
