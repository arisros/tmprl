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
