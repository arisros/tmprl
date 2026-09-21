//! The payload pane and the `!` pipe.

use super::*;

#[test]
fn the_payload_pane_is_closed_until_asked_for() {
    let mut app = app_with_payloads();
    let out = draw(&mut app, 110, 20);
    assert!(
        !out.contains("payloads"),
        "the pane should start closed:\n{out}"
    );
    assert!(
        !out.contains("amount"),
        "no payload should be shown:\n{out}"
    );
}

#[test]
fn the_payload_pane_shows_input_and_result_for_a_group() {
    // A group's arguments live on the event that opened it and its result on the event
    // that closed it, so the pane has to gather from both.
    let mut app = app_with_payloads();
    app.run("motion.down", None); // the ChargeCard group
    app.run("history.detail", None);

    let out = draw(&mut app, 110, 20);
    assert!(out.contains("payloads"), "pane missing:\n{out}");
    assert!(out.contains("input"), "input label missing:\n{out}");
    assert!(out.contains("result"), "result label missing:\n{out}");
    assert!(
        out.contains("\"amount\": 100"),
        "JSON should be pretty-printed:\n{out}"
    );
    assert!(out.contains("charged"), "result value missing:\n{out}");
}

#[test]
fn an_encrypted_payload_says_it_needs_a_codec_rather_than_showing_bytes() {
    let mut app = app_with_payloads();
    app.run("motion.bottom", None); // the Secret group
    app.run("history.detail", None);

    let out = draw(&mut app, 110, 20);
    assert!(out.contains("encrypted"), "should be labelled:\n{out}");
    assert!(out.contains("codec"), "should say what is needed:\n{out}");
}

#[test]
fn the_payload_pane_opens_below_the_list_by_default() {
    let mut app = app_with_payloads();
    app.run("history.detail", None);
    let out = draw(&mut app, 120, 20);
    let (row, _) = position(&out, "payloads");
    assert!(row >= 6, "the pane should sit under the list:\n{out}");
}

#[test]
fn layout_payload_right_opens_the_pane_beside_the_list() {
    let mut app = app_with_payloads();
    app.apply_config(None, None, Some("[layout]\npayload = \"right\""));
    app.run("history.detail", None);
    let out = draw(&mut app, 120, 20);
    let (row, col) = position(&out, "payloads");
    assert!(row <= 2, "the pane should start at the top:\n{out}");
    assert!(col >= 40, "the pane should be on the right:\n{out}");

    // The list keeps its full height, and the cursor still drives the pane.
    app.run("motion.down", None);
    let out = draw(&mut app, 120, 20);
    assert!(
        out.contains("charged"),
        "pane should follow the cursor:\n{out}"
    );
}

#[test]
fn layout_payload_right_stacks_when_the_terminal_is_narrow() {
    let mut app = app_with_payloads();
    app.apply_config(None, None, Some("[layout]\npayload = \"right\""));
    app.run("history.detail", None);
    let out = draw(&mut app, 80, 20);
    let (row, _) = position(&out, "payloads");
    assert!(row >= 6, "too narrow to split sideways:\n{out}");
}

#[test]
fn a_row_carrying_nothing_says_so() {
    // A pane showing only a title reads as broken; plenty of events carry no payload.
    let mut app = app_with_payloads();
    app.run("motion.top", None); // the workflow group, no payloads in this fixture
    app.run("history.detail", None);
    let out = draw(&mut app, 110, 20);
    assert!(out.contains("no payloads on this group"), "{out}");
}

#[test]
fn a_tall_payload_scrolls_rather_than_clipping_silently() {
    use tmprl_core::payload::Payload;
    let mut app = app_with_payloads();
    // A deep value: taller than any pane on a normal terminal.
    let big: String = (0..60).map(|i| format!("\"k{i}\":{i},")).collect();
    let json = format!("{{{}\"last\":1}}", big);
    if let Some(o) = app.view.history.value_mut() {
        let mut events = o.events().to_vec();
        events[1].payloads = vec![(
            "input".into(),
            Payload::new("json/plain", json.into_bytes()),
        )];
        let groups = tmprl_core::history::group_events(&events);
        o.replace(events, groups);
    }
    app.run("motion.down", None);
    app.run("history.detail", None);

    let out = draw(&mut app, 110, 20);
    assert!(
        app.view.detail_max_scroll > 0,
        "the payload should overflow the pane"
    );
    assert!(
        out.contains("to scroll"),
        "an overflowing pane must say so:\n{out}"
    );

    // And it actually scrolls.
    let before = app.view.detail_scroll;
    app.run("history.detail-down", Some(5));
    assert!(app.view.detail_scroll > before);
}

#[test]
fn moving_the_cursor_restarts_the_payload_pane_at_the_top() {
    use tmprl_core::payload::Payload;
    let mut app = app_with_payloads();
    // Only a payload taller than the pane can be scrolled at all.
    let big: String = (0..60).map(|i| format!("\"k{i}\":{i},")).collect();
    if let Some(o) = app.view.history.value_mut() {
        let mut events = o.events().to_vec();
        events[1].payloads = vec![(
            "input".into(),
            Payload::new("json/plain", format!("{{{big}\"last\":1}}").into_bytes()),
        )];
        let groups = tmprl_core::history::group_events(&events);
        o.replace(events, groups);
    }
    app.run("motion.down", None);
    app.run("history.detail", None);
    let _ = draw(&mut app, 110, 20); // the renderer is what learns how far it can scroll
    app.run("history.detail-down", Some(2));
    assert!(app.view.detail_scroll > 0);

    app.run("motion.down", None);
    assert_eq!(
        app.view.detail_scroll, 0,
        "a different value must be shown from its start"
    );
}

#[test]
fn an_encrypted_payload_points_at_the_config_when_no_codec_is_set() {
    // "needs a codec server" is not actionable; naming the file and key is.
    let mut app = app_with_payloads();
    app.run("motion.bottom", None);
    app.run("history.detail", None);
    let out = draw(&mut app, 110, 20);
    assert!(out.contains("encrypted"), "{out}");
    assert!(
        out.contains("config.toml"),
        "should say where to set it:\n{out}"
    );
}

/// Async because opening the pane with a codec configured spawns the decode request.
#[tokio::test]
async fn a_configured_codec_stops_claiming_one_is_needed() {
    let mut app = app_with_payloads();
    app.apply_config(
        None,
        None,
        Some("[codec]\nendpoint = \"http://localhost:8081\"\n"),
    );
    app.run("motion.bottom", None);
    app.run("history.detail", None);

    let out = draw(&mut app, 110, 20);
    assert!(
        !out.contains("config.toml"),
        "a codec is configured; it must not still ask for one:\n{out}"
    );
}

#[test]
fn a_filter_result_replaces_the_payloads_in_the_pane() {
    // You asked to see the filtered value; showing it under the raw payloads would bury
    // the thing you asked for.
    let mut app = app_with_payloads();
    app.run("motion.down", None);
    app.run("history.detail", None);
    assert!(draw(&mut app, 110, 20).contains("amount"));

    app.handle(crate::app::Msg::Piped(Ok("\"charged\"".into())));
    let out = draw(&mut app, 110, 20);
    assert!(out.contains("filtered"), "pane should say so:\n{out}");
    assert!(out.contains("charged"), "output missing:\n{out}");
    assert!(
        !out.contains("amount"),
        "raw payloads should give way:\n{out}"
    );
}

#[test]
fn a_failed_filter_shows_the_commands_own_message() {
    let mut app = app_with_payloads();
    app.run("motion.down", None);
    app.run("history.detail", None);
    app.handle(crate::app::Msg::Piped(Err(
        "jq: error: syntax error, unexpected INVALID_CHARACTER".into(),
    )));

    let out = draw(&mut app, 110, 20);
    assert!(out.contains("filter failed"), "{out}");
    assert!(out.contains("syntax error"), "jq's own diagnosis:\n{out}");
}

#[test]
fn a_filter_with_no_output_says_so_rather_than_looking_broken() {
    let mut app = app_with_payloads();
    app.run("motion.down", None);
    app.run("history.detail", None);
    app.handle(crate::app::Msg::Piped(Ok(String::new())));
    assert!(draw(&mut app, 110, 20).contains("no output"));
}

#[test]
fn the_pipe_prompt_is_drawn_with_its_own_sigil() {
    let mut app = app_with_payloads();
    app.run("motion.down", None);
    app.run("payload.pipe", None);

    let out = draw(&mut app, 110, 20);
    assert!(out.contains("!jq ."), "the ! prompt should show:\n{out}");
    // `:` completions have no business appearing over a shell command.
    assert!(
        !out.contains("app.quit"),
        "a pipe prompt must not offer command completions:\n{out}"
    );
}

#[test]
fn k_on_a_retrying_activity_shows_where_it_is() {
    let mut app = app_with_retrying_activity();
    app.run("motion.bottom", None);
    app.run("history.detail", None);
    let out = draw(&mut app, 120, 20);
    assert!(out.contains("attempt 4/10"), "attempt missing:\n{out}");
    assert!(out.contains("worker-7@host"), "worker missing:\n{out}");
    assert!(out.contains("PaymentDeclined"), "failure missing:\n{out}");
}
