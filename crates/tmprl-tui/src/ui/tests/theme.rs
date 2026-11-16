//! Colour depth: what reaches the terminal at sixteen colours and at none.

use super::*;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::Modifier;
use tmprl_core::ColorDepth;
use tmprl_core::search::Search;

fn buffer(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    term.backend().buffer().clone()
}

fn cells_of(buf: &Buffer, text: &str) -> Vec<Cell> {
    (0..buf.area.height)
        .find_map(|y| {
            let symbols: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            let wanted: Vec<String> = text.chars().map(String::from).collect();
            let at = symbols
                .windows(wanted.len())
                .position(|w| w.iter().zip(&wanted).all(|(a, b)| a == b))?;
            Some(
                (at..at + wanted.len())
                    .map(|x| buf[(x as u16, y)].clone())
                    .collect(),
            )
        })
        .unwrap_or_else(|| panic!("`{text}` is not on screen"))
}

fn every_screen(depth: ColorDepth) -> Vec<Buffer> {
    let themed = |mut app: App| {
        app.apply_config(None, None, Some("[profile.prod]\naccent = \"red\""));
        app.apply_theme(depth, None);
        app
    };
    let mut out = Vec::new();

    out.push(buffer(&mut themed(app_with_rows()), 100, 14));

    let mut workflows = themed(app_with_workflows(&["default", "payments"]));
    workflows.search = Search::new("order");
    out.push(buffer(&mut workflows, 120, 14));
    workflows.run("window.split-right", None);
    out.push(buffer(&mut workflows, 120, 14));

    let mut history = themed(app_with_history());
    history.view.following = true;
    out.push(buffer(&mut history, 120, 16));
    history.run("history.timeline", None);
    out.push(buffer(&mut history, 120, 16));

    let mut payloads = themed(app_with_payloads());
    payloads.run("motion.down", None);
    payloads.run("history.detail", None);
    out.push(buffer(&mut payloads, 120, 24));

    let mut dashboard = themed(dashboard::app_with_dashboard());
    dashboard.run("motion.down", Some(2));
    out.push(buffer(&mut dashboard, 120, 30));
    out.push(buffer(&mut themed(dashboard::empty_dashboard()), 120, 30));

    let mut help = themed(app_with_rows());
    help.show_help = true;
    out.push(buffer(&mut help, 100, 30));

    out
}

#[test]
fn no_color_puts_no_colour_on_any_screen() {
    for buf in every_screen(ColorDepth::Mono) {
        for cell in buf.content() {
            assert_eq!(
                (cell.fg, cell.bg),
                (Color::Reset, Color::Reset),
                "`{}` is coloured",
                cell.symbol()
            );
        }
    }
}

#[test]
fn sixteen_colours_put_no_rgb_on_any_screen() {
    for buf in every_screen(ColorDepth::Ansi16) {
        for cell in buf.content() {
            assert!(
                !matches!(cell.fg, Color::Rgb(..)) && !matches!(cell.bg, Color::Rgb(..)),
                "`{}` is {:?} on {:?}",
                cell.symbol(),
                cell.fg,
                cell.bg
            );
        }
    }
}

#[test]
fn truecolor_still_draws_the_palette() {
    let rgb = every_screen(ColorDepth::TrueColor)
        .iter()
        .flat_map(|buf| buf.content())
        .filter(|c| matches!(c.fg, Color::Rgb(..)))
        .count();
    assert!(rgb > 100, "only {rgb} truecolor cells");
}

#[test]
fn without_colour_status_is_still_its_glyph() {
    let mut app = app_with_workflows(&["default"]);
    app.apply_theme(ColorDepth::Mono, None);
    let out = draw(&mut app, 120, 12);
    assert!(out.contains("● Running"), "{out}");
    assert!(out.contains("✗ Failed"), "{out}");
}

#[test]
fn without_colour_the_cursor_row_is_reverse_video() {
    for depth in [ColorDepth::Mono, ColorDepth::Ansi16] {
        let mut app = app_with_workflows(&["default"]);
        app.apply_theme(depth, None);
        let buf = buffer(&mut app, 120, 12);

        let cursor = cells_of(&buf, "order-1001");
        assert!(
            cursor
                .iter()
                .all(|c| c.modifier.contains(Modifier::REVERSED)),
            "{depth:?}: the cursor row is not marked"
        );
        let other = cells_of(&buf, "charge-77");
        assert!(
            other
                .iter()
                .all(|c| !c.modifier.contains(Modifier::REVERSED)),
            "{depth:?}: a row off the cursor is marked"
        );
    }
}

#[test]
fn without_colour_a_selection_is_marked_and_the_cursor_is_bolder() {
    let mut app = app_with_workflows(&["default"]);
    app.apply_theme(ColorDepth::Mono, None);
    app.run("mode.visual-line", None);
    app.run("motion.down", None);
    let buf = buffer(&mut app, 120, 12);

    let selected = cells_of(&buf, "order-1001");
    let cursor = cells_of(&buf, "charge-77");
    for cell in selected.iter().chain(&cursor) {
        assert!(cell.modifier.contains(Modifier::REVERSED));
    }
    assert!(cursor.iter().all(|c| c.modifier.contains(Modifier::BOLD)));
    assert!(
        selected
            .iter()
            .all(|c| !c.modifier.contains(Modifier::BOLD))
    );
}

#[test]
fn without_colour_a_match_shows_on_the_cursor_row_and_off_it() {
    let mut app = app_with_workflows(&["default"]);
    app.apply_theme(ColorDepth::Mono, None);
    app.search = Search::new("r-1");
    let buf = buffer(&mut app, 120, 12);
    let id = cells_of(&buf, "order-1001");
    for (i, cell) in id.iter().enumerate() {
        let matched = (4..7).contains(&i);
        assert_eq!(
            cell.modifier.contains(Modifier::REVERSED),
            !matched,
            "cell {i} of the cursor row"
        );
        assert_eq!(cell.modifier.contains(Modifier::UNDERLINED), matched);
    }

    app.search = Search::new("77");
    let buf = buffer(&mut app, 120, 12);
    let hit = cells_of(&buf, "77");
    assert!(hit.iter().all(|c| c.modifier.contains(Modifier::REVERSED)));
    let rest = cells_of(&buf, "charge-");
    assert!(
        rest.iter()
            .all(|c| !c.modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn without_colour_the_focused_pane_has_the_heavier_border() {
    let mut app = app_with_rows();
    app.apply_theme(ColorDepth::Mono, None);
    app.run("window.split-right", None);
    let buf = buffer(&mut app, 120, 12);

    let corners: Vec<&Cell> = (0..buf.area.width)
        .map(|x| &buf[(x, 1)])
        .filter(|c| c.symbol() == "┌")
        .collect();
    assert_eq!(corners.len(), 2, "expected two panes");
    assert!(corners[0].modifier.contains(Modifier::DIM));
    assert!(!corners[0].modifier.contains(Modifier::BOLD));
    assert!(corners[1].modifier.contains(Modifier::BOLD));
    assert!(!corners[1].modifier.contains(Modifier::DIM));
}

#[test]
fn a_frame_is_drawn_with_the_themes_colours() {
    let mut app = app_with_workflows(&["default"]);
    app.apply_theme(ColorDepth::TrueColor, Some("err = \"#010203\""));
    assert!(app.note.is_none());
    assert_eq!(
        fg_on_row(&mut app, 120, 12, "charge-77", "✗"),
        Some(Color::Rgb(1, 2, 3))
    );
    assert_eq!(
        fg_on_row(&mut app, 120, 12, "order-1001", "●"),
        Theme::default().accent.fg
    );
}

#[test]
fn a_hex_in_the_theme_is_a_named_colour_on_sixteen_colours() {
    let mut app = app_with_workflows(&["default"]);
    app.apply_theme(ColorDepth::Ansi16, Some("err = \"#e05252\""));
    assert_eq!(
        fg_on_row(&mut app, 120, 12, "charge-77", "✗"),
        Some(Color::Red)
    );
}

#[test]
fn a_profile_accent_fills_the_pill_and_reverses_without_colour() {
    let pill = |depth, config: &str| {
        let mut app = app_with_rows();
        app.apply_config(None, None, Some(config));
        app.apply_theme(depth, None);
        let buf = buffer(&mut app, 100, 12);
        cells_of(&buf, "PROD")[0].clone()
    };
    let chosen = "[profile.prod]\naccent = \"cyan\"";
    let ansi = pill(ColorDepth::Ansi16, chosen);
    assert_eq!(
        ansi.bg,
        Color::Cyan,
        "sixteen colours follow the terminal's palette"
    );
    assert_eq!(ansi.fg, Color::Black);
    assert!(ansi.modifier.contains(Modifier::BOLD));
    let truecolor = pill(ColorDepth::TrueColor, chosen);
    assert_eq!(
        truecolor.bg,
        Color::Rgb(0x56, 0xb6, 0xc2),
        "tmprl's own palette"
    );
    assert!(truecolor.modifier.contains(Modifier::BOLD));
    let mono = pill(ColorDepth::Mono, chosen);
    assert_eq!(mono.bg, Color::Reset);
    assert!(mono.modifier.contains(Modifier::REVERSED));
}

#[test]
fn a_profile_without_an_accent_gets_one_from_its_name() {
    let mut app = app_with_rows();
    app.apply_theme(ColorDepth::Ansi16, None);
    let buf = buffer(&mut app, 100, 12);
    assert_eq!(cells_of(&buf, "PROD")[0].bg, Color::Red);
}

#[test]
fn production_is_red_from_tmprls_palette_in_truecolor() {
    let mut app = app_with_rows();
    app.apply_theme(ColorDepth::TrueColor, None);
    let buf = buffer(&mut app, 100, 12);
    assert_eq!(cells_of(&buf, "PROD")[0].bg, Color::Rgb(0xe0, 0x52, 0x52));
}

#[test]
fn read_only_is_a_neutral_pill_of_its_own() {
    let mut app = app_with_rows();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));
    app.apply_theme(ColorDepth::TrueColor, None);
    let buf = buffer(&mut app, 100, 12);
    let profile = cells_of(&buf, "PROD")[0].bg;
    let lock = cells_of(&buf, " RO ")[1].bg;
    assert_ne!(
        profile, lock,
        "read-only must not take the profile's colour"
    );
    assert_eq!(lock, Color::Rgb(0x3a, 0x40, 0x4b));
}

#[test]
fn the_brand_ends_in_the_logos_pink_cursor() {
    for (depth, pink) in [
        (ColorDepth::TrueColor, Some(Color::Rgb(0xff, 0x00, 0x7c))),
        (ColorDepth::Ansi16, Some(Color::Magenta)),
        (ColorDepth::Mono, None),
    ] {
        let mut app = app_with_rows();
        app.apply_theme(depth, None);
        let buf = buffer(&mut app, 100, 12);
        let cursor = &cells_of(&buf, "tmprl▌")[5];
        assert_eq!(
            (cursor.fg != Color::Reset).then_some(cursor.fg),
            pink,
            "{depth:?}"
        );
    }
}

#[test]
fn without_colour_the_pill_is_bracketed() {
    let mut app = app_with_rows();
    app.apply_theme(ColorDepth::Mono, None);
    let buf = buffer(&mut app, 100, 12);
    cells_of(&buf, "[PROD]");
}

#[test]
fn the_timeline_keeps_its_hues_on_sixteen_colours() {
    let mut app = app_with_history();
    app.apply_theme(ColorDepth::Ansi16, None);
    app.run("history.timeline", None);
    assert_eq!(
        fg_on_row(&mut app, 110, 12, "ShipOrder", "━"),
        Some(Color::Red)
    );
    assert_eq!(
        fg_on_row(&mut app, 110, 12, "OrderWorkflow", "╍"),
        Some(Color::LightBlue)
    );
}
