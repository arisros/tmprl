//! The `:messages` overlay, and what a failed list says about itself.

use super::*;
use crate::app::Msg;
use tmprl_core::fault::{Code, Fault};

fn failed_list(app: &mut App, fault: Fault) {
    app.view.screen = crate::app::Screen::Workflows;
    app.handle(Msg::Workflows {
        generation: app.view.generation,
        append: false,
        result: Err(fault),
    });
}

#[test]
fn a_failed_list_says_what_to_try() {
    let mut app = app_with_rows();
    failed_list(
        &mut app,
        Fault::rpc(
            "ListWorkflowExecutions",
            Code::Unavailable,
            "transport error",
        ),
    );

    let out = draw(&mut app, 140, 12);
    assert!(out.contains("failed (unavailable)"), "code missing:\n{out}");
    assert!(out.contains("R retries"), "hint missing:\n{out}");
    assert!(
        !out.contains("(R to retry)"),
        "the hint already names the key, saying it twice is noise:\n{out}"
    );
}

#[test]
fn a_failure_with_no_hint_still_names_the_retry_key() {
    let mut app = app_with_rows();
    failed_list(
        &mut app,
        Fault::rpc("ListWorkflowExecutions", Code::Internal, "boom"),
    );

    let out = draw(&mut app, 140, 12);
    assert!(out.contains("(R to retry)"), "{out}");
}

#[test]
fn messages_shows_the_grpc_name_and_the_hint_under_a_failure() {
    let mut app = app_with_rows();
    failed_list(
        &mut app,
        Fault::rpc(
            "ListWorkflowExecutions",
            Code::PermissionDenied,
            "Request unauthorized.",
        ),
    );
    app.run("app.messages", None);

    let out = draw(&mut app, 120, 20);
    assert!(out.contains("messages"), "title missing:\n{out}");
    assert!(out.contains("error"), "level missing:\n{out}");
    assert!(
        out.contains("ListWorkflowExecutions · PermissionDenied"),
        "the name a server log uses must be shown:\n{out}"
    );
    assert!(
        out.contains("not allowed to do that here"),
        "hint missing:\n{out}"
    );
}

#[test]
fn a_long_message_wraps_rather_than_running_off_the_edge() {
    let mut app = app_with_rows();
    let tail = "the-part-at-the-end-that-matters";
    failed_list(
        &mut app,
        Fault::rpc(
            "ListWorkflowExecutions",
            Code::InvalidArgument,
            format!("{} {tail}", "word ".repeat(40)),
        ),
    );
    app.run("app.messages", None);

    let out = draw(&mut app, 80, 24);
    assert!(
        out.contains(tail),
        "the end of the message was clipped:\n{out}"
    );
}

#[test]
fn messages_opens_on_the_newest_entry() {
    let mut app = app_with_rows();
    for i in 0..40 {
        failed_list(
            &mut app,
            Fault::rpc(
                "ListWorkflowExecutions",
                Code::Internal,
                format!("failure {i}"),
            ),
        );
    }
    app.run("app.messages", None);

    let out = draw(&mut app, 120, 16);
    assert!(
        out.contains("failure 39"),
        "the newest must be in view:\n{out}"
    );
    assert!(
        !out.contains("failure 0 "),
        "the oldest should have scrolled off:\n{out}"
    );
    assert!(out.contains("j/k to scroll"), "{out}");
}

#[test]
fn an_empty_log_says_so() {
    let mut app = app_with_rows();
    app.run("app.messages", None);
    let out = draw(&mut app, 100, 12);
    assert!(out.contains("nothing has been said yet"), "{out}");
}

#[test]
fn a_failure_with_no_rpc_behind_it_is_filed_under_its_kind() {
    // A codec refusal has no call name to show, and an empty one would print as a
    // dangling separator.
    let mut app = app_with_rows();
    app.handle(Msg::Decoded(Err(Fault::codec("codec server returned 502"))));
    app.run("app.messages", None);

    let out = draw(&mut app, 100, 12);
    assert!(out.contains("codec server returned 502"), "{out}");
    assert!(out.contains("Codec"), "the kind is missing:\n{out}");
    assert!(
        !out.contains(" · Codec"),
        "nothing to separate it from:\n{out}"
    );
}

#[test]
fn the_overlay_scrolls_with_the_ordinary_motions() {
    let mut app = app_with_rows();
    for i in 0..40 {
        failed_list(
            &mut app,
            Fault::rpc(
                "ListWorkflowExecutions",
                Code::Internal,
                format!("failure {i}"),
            ),
        );
    }
    app.run("app.messages", None);
    draw(&mut app, 120, 16);
    let end = app.overlay_max_scroll;
    assert!(end > 0);
    assert_eq!(app.overlay_scroll, end, "it opens at the end");

    app.run("motion.top", None);
    assert_eq!(app.overlay_scroll, 0);
    let out = draw(&mut app, 120, 16);
    assert!(
        out.contains("failure 0"),
        "the oldest is at the top:\n{out}"
    );

    app.run("motion.half-down", None);
    assert!(app.overlay_scroll > 0 && app.overlay_scroll < end);
    app.run("motion.half-up", None);
    assert_eq!(app.overlay_scroll, 0);

    app.run("motion.bottom", None);
    assert_eq!(app.overlay_scroll, end);
    app.run("motion.up", None);
    assert_eq!(app.overlay_scroll, end - 1);
}

#[test]
fn a_terminal_too_small_for_the_overlay_draws_without_it() {
    let mut app = app_with_rows();
    app.run("app.messages", None);
    let out = draw(&mut app, 18, 4);
    assert!(!out.contains("messages"), "{out}");
}
