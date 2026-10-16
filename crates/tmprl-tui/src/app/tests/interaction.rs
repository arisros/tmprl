//! Queries: the handler picker, its fallbacks, and the answer in the pane.

use super::*;
use tmprl_core::fault::Code;

fn metadata(json: &str) -> Msg {
    Msg::QueryHandlers {
        generation: 0,
        result: Ok(vec![Payload::new(
            "json/protobuf",
            json.as_bytes().to_vec(),
        )]),
    }
}

fn reply(app: &App, msg: Msg) -> Msg {
    match msg {
        Msg::QueryHandlers { result, .. } => Msg::QueryHandlers {
            generation: app.view.generation,
            result,
        },
        Msg::Queried { name, result, .. } => Msg::Queried {
            generation: app.view.generation,
            name,
            result,
        },
        other => other,
    }
}

fn refused(code: Code, message: &str) -> Msg {
    Msg::QueryHandlers {
        generation: 0,
        result: Err(Fault::rpc("QueryWorkflow", code, message)),
    }
}

#[test]
fn a_query_is_refused_away_from_a_history() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.run("workflow.query", None);
    assert!(matches!(app.note, Some((_, Note::Warn))));
    assert!(app.picker.is_none());
}

#[test]
fn the_metadata_opens_a_picker_of_the_workflow_s_own_handlers() {
    let mut app = viewing_payloads();
    let msg = metadata(
        r#"{"definition":{"queryDefinitions":[{"name":"getState","description":"loan stage"}]},
            "currentDetails":"waiting for approval"}"#,
    );
    app.handle(reply(&app, msg));

    let picker = app.picker.as_ref().expect("a picker should open");
    assert_eq!(picker.kind, picker::Kind::Queries);
    let items = app.query_items();
    let names: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(names, ["getState", "__stack_trace"]);
    assert_eq!(items[0].note, "loan stage");
    assert_eq!(
        app.note.as_ref().map(|(n, _)| n.as_str()),
        Some("waiting for approval"),
        "what the workflow says it is doing is worth reading before picking"
    );
}

#[test]
fn an_sdk_without_metadata_still_gives_a_picker_from_its_refusal() {
    let mut app = viewing_payloads();
    let msg = refused(
        Code::InvalidArgument,
        "unknown queryType __temporal_workflow_metadata. KnownQueryTypes=[__stack_trace getState]",
    );
    app.handle(reply(&app, msg));

    assert!(app.picker.is_some());
    let names: Vec<_> = app.query_items().into_iter().map(|i| i.label).collect();
    assert_eq!(names, ["getState", "__stack_trace"]);
}

#[test]
fn a_refusal_that_lists_nothing_asks_for_the_name() {
    let mut app = viewing_payloads();
    let msg = refused(Code::InvalidArgument, "query handler not found");
    app.handle(reply(&app, msg));

    assert!(app.picker.is_none());
    let p = app.prompt.clone().expect("a prompt should open");
    assert_eq!(p.kind, PromptKind::Query);
    assert_eq!(p.sigil(), "query:");
}

#[test]
fn no_worker_answering_is_an_error_not_a_prompt() {
    let mut app = viewing_payloads();
    let msg = refused(Code::DeadlineExceeded, "context deadline exceeded");
    app.handle(reply(&app, msg));

    assert!(app.picker.is_none());
    assert!(app.prompt.is_none());
    assert!(matches!(app.note, Some((_, Note::Error))));
}

#[test]
fn an_answer_replaces_the_pane_and_is_what_the_pipe_takes() {
    let mut app = viewing_payloads();
    app.run("motion.down", None);
    let msg = Msg::Queried {
        generation: 0,
        name: "getState".into(),
        result: Ok(vec![Payload::new(
            "json/plain",
            br#"{"stage":"approved"}"#.to_vec(),
        )]),
    };
    app.handle(reply(&app, msg));

    let labels: Vec<_> = app
        .payloads_under_cursor()
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(labels, ["getState"]);

    app.run("motion.down", None);
    assert!(
        app.view.queried.is_none(),
        "the answer belongs to the moment it was asked, not to the next row"
    );
}

#[test]
fn a_failed_query_says_which_handler() {
    let mut app = viewing_payloads();
    let msg = Msg::Queried {
        generation: 0,
        name: "getState".into(),
        result: Err(Fault::rpc(
            "QueryWorkflow",
            Code::InvalidArgument,
            "missing argument",
        )),
    };
    app.handle(reply(&app, msg));

    assert!(app.view.queried.is_none());
    let (note, level) = app.note.clone().unwrap();
    assert_eq!(level, Note::Error);
    assert!(note.contains("getState"), "{note}");
}

#[test]
fn an_answer_for_a_history_since_left_is_dropped() {
    let mut app = viewing_payloads();
    app.handle(Msg::Queried {
        generation: app.view.generation.wrapping_add(1),
        name: "getState".into(),
        result: Ok(vec![Payload::new("json/plain", b"1".to_vec())]),
    });
    assert!(app.view.queried.is_none());
}
