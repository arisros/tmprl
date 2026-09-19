//! Decoding, the `!` pipe and `$EDITOR`.

use super::*;

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
