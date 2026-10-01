//! The cursor in each text input: where it is drawn, and that it needs no colour.

use super::*;
use crate::app::Msg;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use tmprl_core::{ColorDepth, Key};

fn buffer(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    term.backend().buffer().clone()
}

fn type_chars(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle(Msg::Key(Chord::ch(c)));
    }
}

fn key(app: &mut App, key: Key) {
    app.handle(Msg::Key(Chord::plain(key)));
}

/// Every cell in reverse video, as its position and what it shows.
///
/// At full colour nothing else on these screens is reversed, the cursor row is a
/// background, so what this finds is the text cursor.
fn reversed(buf: &Buffer) -> Vec<(u16, u16, String)> {
    let mut found = Vec::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            if cell.modifier.contains(Modifier::REVERSED) {
                found.push((x, y, cell.symbol().to_string()));
            }
        }
    }
    found
}

/// Every text input there is, open and empty.
fn inputs() -> Vec<(&'static str, App)> {
    let mut command = app_with_rows();
    command.run("app.command-line", None);

    let mut query = app_with_workflows(&["default"]);
    query.run("mode.insert", None);

    let mut picker = app_with_workflows(&["default"]);
    picker.run("find.workflow", None);

    let mut form = app_with_rows();
    form.view.screen = crate::app::Screen::Schedules;
    form.run("schedule.create", None);

    let mut confirm = app_with_workflows(&["default"]);
    confirm.run("workflow.delete", None);

    vec![
        (":", command),
        ("query bar", query),
        ("picker", picker),
        ("form", form),
        ("confirmation", confirm),
    ]
}

#[test]
fn every_input_draws_its_cursor_on_the_character_it_is_at() {
    for (name, mut app) in inputs() {
        type_chars(&mut app, "abc");
        key(&mut app, Key::Left);
        let buf = buffer(&mut app, 120, 30);

        let cursor = reversed(&buf);
        assert_eq!(cursor.len(), 1, "{name}: one cursor, got {cursor:?}");
        let (x, y, symbol) = &cursor[0];
        assert_eq!(symbol, "c", "{name}");
        assert_eq!(buf[(x - 1, *y)].symbol(), "b", "{name}");
        assert_eq!(buf[(x - 2, *y)].symbol(), "a", "{name}");
    }
}

#[test]
fn at_the_end_of_the_line_the_cursor_is_the_cell_after_the_text() {
    for (name, mut app) in inputs() {
        type_chars(&mut app, "ab");
        let buf = buffer(&mut app, 120, 30);

        let cursor = reversed(&buf);
        assert_eq!(cursor.len(), 1, "{name}: one cursor, got {cursor:?}");
        let (x, y, symbol) = &cursor[0];
        assert_eq!(symbol, " ", "{name}");
        assert_eq!(buf[(x - 1, *y)].symbol(), "b", "{name}");
    }
}

#[test]
fn an_empty_input_still_shows_where_typing_will_land() {
    for (name, mut app) in inputs() {
        let buf = buffer(&mut app, 120, 30);
        assert_eq!(reversed(&buf).len(), 1, "{name}");
    }
}

#[test]
fn without_colour_the_cursor_is_still_drawn() {
    for (name, mut app) in inputs() {
        app.apply_theme(ColorDepth::Mono, None);
        type_chars(&mut app, "abc");
        key(&mut app, Key::Left);
        key(&mut app, Key::Left);
        let buf = buffer(&mut app, 120, 30);

        // The list's own cursor row is reversed too at this depth, so look for the one
        // cell that is reversed between two that are not.
        let lone: Vec<_> = reversed(&buf)
            .into_iter()
            .filter(|(x, y, _)| {
                let plain = |x: u16| !buf[(x, *y)].modifier.contains(Modifier::REVERSED);
                *x > 0 && x + 1 < buf.area.width && plain(x - 1) && plain(x + 1)
            })
            .collect();
        assert_eq!(lone.len(), 1, "{name}: got {lone:?}");
        assert_eq!(lone[0].2, "b", "{name}");
    }
}

#[test]
fn a_prompt_longer_than_the_terminal_keeps_its_cursor_on_screen() {
    let mut app = app_with_rows();
    app.run("app.command-line", None);
    type_chars(&mut app, &"0123456789".repeat(6));
    let buf = buffer(&mut app, 30, 12);
    let cursor = reversed(&buf);
    assert_eq!(cursor.len(), 1, "at the end: {cursor:?}");

    for _ in 0..35 {
        key(&mut app, Key::Left);
    }
    let buf = buffer(&mut app, 30, 12);
    let cursor = reversed(&buf);
    assert_eq!(cursor.len(), 1, "in the middle: {cursor:?}");
    assert_eq!(cursor[0].2, "5", "sixty digits, thirty-five back");
}

#[test]
fn the_query_bar_gives_up_its_hint_before_any_of_the_query() {
    let mut app = app_with_workflows(&["default"]);
    app.run("mode.insert", None);
    type_chars(&mut app, "abc");
    assert!(draw(&mut app, 60, 12).contains("⏎ apply"));

    type_chars(&mut app, " WorkflowType = 'Checkout' OR x = 'yz'");
    let out = draw(&mut app, 60, 12);
    assert!(
        !out.contains("⏎ apply"),
        "the hint should have gone:\n{out}"
    );
    assert!(
        out.contains("abc WorkflowType = 'Checkout' OR x = 'yz'"),
        "the whole query fits without it:\n{out}"
    );
}
