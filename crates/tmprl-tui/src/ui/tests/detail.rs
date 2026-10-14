//! The payload pane and the `!` pipe.

use super::*;

/// The pane stacked under the list, for tests about scrolling it rather than placing it.
const BOTTOM: &str = "[layout]\npayload = \"bottom\"";

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
fn layout_payload_bottom_opens_the_pane_below_the_list() {
    let mut app = app_with_payloads();
    app.apply_config(None, None, Some(BOTTOM));
    app.run("history.detail", None);
    let out = draw(&mut app, 120, 20);
    let (row, _) = position(&out, "payloads");
    assert!(row >= 6, "the pane should sit under the list:\n{out}");
}

#[test]
fn the_payload_pane_opens_beside_the_list_by_default() {
    let mut app = app_with_payloads();
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
    app.apply_config(None, None, Some(BOTTOM));
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
    app.apply_config(None, None, Some(BOTTOM));
    app.run("motion.bottom", None);
    app.run("history.detail", None);
    let out = draw(&mut app, 120, 20);
    assert!(out.contains("attempt 4/10"), "attempt missing:\n{out}");
    assert!(out.contains("worker-7@host"), "worker missing:\n{out}");
    assert!(out.contains("PaymentDeclined"), "failure missing:\n{out}");
}

/// Whether the cell showing `symbol` on the row containing `row_text` is painted as a match.
fn lit_on_row(app: &mut App, w: u16, h: u16, row_text: &str, symbol: &str) -> bool {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height).any(|y| {
        let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        line.contains(row_text)
            && (0..buf.area.width).map(|x| &buf[(x, y)]).any(|c| {
                c.symbol() == symbol && c.modifier.contains(ratatui::style::Modifier::REVERSED)
            })
    })
}

#[test]
fn a_search_lights_up_its_value_in_the_payload_pane() {
    let mut app = app_with_payloads();
    app.run("motion.down", None);
    app.run("history.detail", None);
    app.search = tmprl_core::search::Search::new("GBP");
    assert!(lit_on_row(&mut app, 110, 20, "\"currency\"", "G"));
}

#[test]
fn a_path_search_lights_up_the_value_the_pane_shows() {
    // `currency=GBP` is the flattened form, never on screen; the value is.
    let mut app = app_with_payloads();
    app.run("motion.down", None);
    app.run("history.detail", None);
    app.search = tmprl_core::search::Search::new("currency=GBP");
    assert!(lit_on_row(&mut app, 110, 20, "\"currency\"", "G"));
}

#[test]
fn a_search_that_lands_scrolls_the_pane_to_the_match_once() {
    let mut app = app_with_payloads();
    app.apply_config(None, None, Some(BOTTOM));
    app.run("motion.down", None);
    app.run("history.detail", None);
    app.search = tmprl_core::search::Search::new("charged");

    draw(&mut app, 110, 12);
    assert_eq!(app.view.detail_scroll, 0, "no search landed, nothing moves");

    app.view.detail_seek = true;
    draw(&mut app, 110, 12);
    assert!(app.view.detail_scroll > 0, "the result sits below the fold");
    assert!(!app.view.detail_seek, "the seek is spent on the draw");

    app.run("history.detail-up", None);
    let after = app.view.detail_scroll;
    draw(&mut app, 110, 12);
    assert_eq!(
        app.view.detail_scroll, after,
        "later draws leave the scroll alone"
    );
}

/// The second row carrying a payload far taller than any pane, with the pane open on it
/// and drawn once, since the renderer is what learns how far the pane can scroll.
fn app_on_a_tall_payload(config: Option<&str>, width: u16) -> App {
    use tmprl_core::payload::Payload;
    let mut app = app_with_payloads();
    app.apply_config(None, None, config);
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
    let _ = draw(&mut app, width, 20);
    app
}

#[test]
fn the_focused_payload_pane_takes_the_motions() {
    let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
    let row = app.view.cursor;
    let max = app.view.detail_max_scroll;

    app.run("window.focus-down", None);
    assert!(
        app.view.detail_focus,
        "the pane sits below, so down enters it"
    );

    app.run("motion.down", Some(3));
    assert_eq!(app.view.detail_scroll, 3);
    app.run("motion.up", None);
    assert_eq!(app.view.detail_scroll, 2);
    app.run("motion.bottom", None);
    assert_eq!(app.view.detail_scroll, max);
    app.run("motion.down", None);
    assert_eq!(app.view.detail_scroll, max, "it stops at the end");
    app.run("motion.top", None);
    assert_eq!(app.view.detail_scroll, 0);
    app.run("motion.half-down", None);
    assert_eq!(
        app.view.detail_scroll,
        app.view.detail_page / 2,
        "half of the pane, not half of the list"
    );
    app.run("motion.top", None);
    app.run("motion.page-down", None);
    assert_eq!(app.view.detail_scroll, app.view.detail_page);
    app.run("motion.page-up", None);
    assert_eq!(app.view.detail_scroll, 0);
    assert_eq!(app.view.cursor, row, "the row under the pane never moved");

    let out = draw(&mut app, 110, 20);
    assert!(
        out.contains("<Esc> back"),
        "a focused pane says how to leave:\n{out}"
    );
}

#[test]
fn leaving_the_payload_pane_gives_the_motions_back_to_the_list() {
    for leave in ["window.focus-up", "app.cancel", "history.detail"] {
        let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
        let row = app.view.cursor;
        app.run("window.focus-down", None);
        app.run(leave, None);
        assert!(!app.view.detail_focus, "{leave} should leave the pane");

        app.run("motion.down", None);
        assert_eq!(app.view.cursor, row + 1, "after {leave}");
    }
}

#[test]
fn a_pane_beside_the_list_is_entered_sideways() {
    let beside = Some("[layout]\npayload = \"right\"");
    let mut app = app_on_a_tall_payload(beside, 120);
    app.run("window.focus-down", None);
    assert!(!app.view.detail_focus, "nothing is below the list");
    app.run("window.focus-right", None);
    assert!(app.view.detail_focus);
    app.run("window.focus-left", None);
    assert!(!app.view.detail_focus);

    // Too narrow to sit side by side, so the pane is stacked and down is the way in.
    let mut app = app_on_a_tall_payload(beside, 80);
    app.run("window.focus-right", None);
    assert!(!app.view.detail_focus);
    app.run("window.focus-down", None);
    assert!(app.view.detail_focus);
}

#[test]
fn a_payload_that_fits_is_not_given_the_focus() {
    let mut app = app_with_payloads();
    app.apply_config(None, None, Some(BOTTOM));
    app.run("history.detail", None);
    let _ = draw(&mut app, 110, 40);
    assert_eq!(app.view.detail_max_scroll, 0);

    app.run("window.focus-down", None);
    assert!(!app.view.detail_focus, "j and k would do nothing in it");
    assert!(
        app.note.clone().unwrap().0.contains("nothing to scroll"),
        "and the key says why it did nothing"
    );
}

#[test]
fn moving_to_another_window_leaves_the_payload_pane() {
    let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
    app.run("window.split-right", None);
    app.run("window.focus-left", None);
    let _ = draw(&mut app, 110, 20);
    app.run("window.focus-down", None);
    assert!(app.view.detail_focus);

    app.run("window.focus-right", None);
    app.run("window.focus-left", None);
    assert!(!app.view.detail_focus, "coming back lands on the list");
}

#[test]
fn a_full_page_moves_the_list_by_what_it_shows() {
    let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
    app.run("motion.top", None);
    let rows = app.view.page;

    app.run("motion.page-down", None);
    assert!(app.view.cursor > 0 && app.view.cursor <= rows);
    app.run("motion.page-up", None);
    assert_eq!(app.view.cursor, 0);
}

use crate::app::{Mouse, Msg};

fn wheel(app: &mut App, down: bool, column: u16, row: u16) {
    app.handle(Msg::Mouse(Mouse::Wheel { down, column, row }));
}

fn click(app: &mut App, column: u16, row: u16) {
    app.handle(Msg::Mouse(Mouse::Click { column, row }));
}

#[test]
fn the_wheel_scrolls_whichever_of_the_list_and_the_pane_it_is_over() {
    let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
    let row = app.view.cursor;
    let pane = app.view.detail_area;

    wheel(&mut app, true, pane.x + 2, pane.y + 2);
    assert_eq!(app.view.detail_scroll, 3);
    assert_eq!(app.view.cursor, row, "the list stayed where it was");
    wheel(&mut app, false, pane.x + 2, pane.y + 2);
    assert_eq!(app.view.detail_scroll, 0);

    wheel(&mut app, false, 2, pane.y - 2);
    assert!(app.view.cursor < row, "over the list it moves the row");
    assert!(!app.view.detail_focus, "the wheel never takes the focus");
}

#[test]
fn a_click_focuses_what_it_lands_on() {
    let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
    let pane = app.view.detail_area;

    click(&mut app, pane.x + 2, pane.y + 2);
    assert!(app.view.detail_focus);
    click(&mut app, 2, pane.y - 2);
    assert!(!app.view.detail_focus);

    app.run("window.split-right", None);
    let _ = draw(&mut app, 110, 20);
    let here = app.tabs.current().focused();
    click(&mut app, 2, 5);
    assert_ne!(app.tabs.current().focused(), here, "the window on the left");
}

#[test]
fn the_mouse_is_ignored_while_a_prompt_owns_the_keyboard() {
    let mut app = app_on_a_tall_payload(Some(BOTTOM), 110);
    let pane = app.view.detail_area;
    app.run("search.open", None);

    wheel(&mut app, true, pane.x + 2, pane.y + 2);
    click(&mut app, pane.x + 2, pane.y + 2);
    assert_eq!(app.view.detail_scroll, 0);
    assert!(!app.view.detail_focus);
}

#[test]
fn a_click_puts_the_cursor_on_the_row_under_it() {
    let mut app = app_with_rows();
    let out = draw(&mut app, 110, 20);
    let (row, _) = position(&out, "temporal-system");

    click(&mut app, 5, row as u16);
    assert_eq!(app.view.cursor, 2);
    assert_eq!(
        app.view.screen,
        Screen::Namespaces,
        "one click only selects"
    );

    click(&mut app, 5, 18);
    assert_eq!(
        app.view.cursor, 2,
        "below the last row there is nothing to select"
    );
}

#[test]
fn a_second_click_on_the_same_row_opens_it() {
    let mut app = app_with_rows();
    let out = draw(&mut app, 110, 20);
    let (payments, _) = position(&out, "payments");
    let (system, _) = position(&out, "temporal-system");

    click(&mut app, 5, payments as u16);
    click(&mut app, 5, system as u16);
    assert_eq!(
        app.view.screen,
        Screen::Namespaces,
        "two rows are two selections"
    );

    click(&mut app, 5, system as u16);
    assert_eq!(app.view.screen, Screen::Workflows);
    assert_eq!(app.view.scope, vec!["temporal-system".to_string()]);
}

#[test]
fn layout_payload_size_sets_the_panes_share() {
    let app = app_on_a_tall_payload(Some("[layout]\npayload_size = 30"), 120);
    let body = app.view.body_area;
    let pane = app.view.detail_area;
    assert!(app.view.detail_beside);
    assert_eq!(pane.width, body.width * 30 / 100);
}

#[test]
fn the_payload_pane_resizes_like_a_window() {
    let mut app = app_on_a_tall_payload(None, 120);
    assert_eq!(app.payload_size(), 50);

    app.run("window.grow-right", None);
    assert_eq!(app.payload_size(), 45, "from the list, the list grows");
    app.run("window.grow-up", None);
    assert_eq!(
        app.payload_size(),
        45,
        "a pane beside the list has no height to give"
    );

    app.run("window.focus-right", None);
    assert!(app.view.detail_focus);
    for _ in 0..10 {
        app.run("window.grow-right", None);
    }
    assert_eq!(
        app.payload_size(),
        tmprl_core::config::PAYLOAD_SIZE_MAX,
        "the list keeps a share"
    );
    let _ = draw(&mut app, 120, 20);
    assert!(app.view.detail_area.width > app.view.body_area.width / 2);
}

#[test]
fn dragging_the_panes_top_rule_resizes_it() {
    let mut app = app_on_a_tall_payload(None, 120);
    let body = app.view.body_area;
    let pane = app.view.detail_area;

    app.handle(Msg::Mouse(Mouse::Drag {
        column: body.x + 10,
        row: pane.y,
    }));
    assert_eq!(
        app.payload_size(),
        50,
        "a drag that did not start on the rule"
    );

    click(&mut app, pane.x + 3, pane.y);
    app.handle(Msg::Mouse(Mouse::Drag {
        column: body.right() - body.width * 30 / 100,
        row: pane.y,
    }));
    assert_eq!(app.payload_size(), 30);

    app.handle(Msg::Mouse(Mouse::Release));
    app.handle(Msg::Mouse(Mouse::Drag {
        column: body.x + 10,
        row: pane.y,
    }));
    assert_eq!(
        app.payload_size(),
        30,
        "released, the pointer is free again"
    );
}
