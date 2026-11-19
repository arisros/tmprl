//! The logo: where the mark is drawn, where it is not, and the splash before the TUI.

use super::*;
use crate::theme::Theme;
use crate::ui::brand;
use ratatui::text::{Line, Span};
use tmprl_core::ColorDepth;
use tmprl_core::theme::ThemeOverrides;

fn empty_namespace(depth: ColorDepth) -> App {
    let mut app = app_with_workflows(&["default"]);
    app.view.workflows = Loadable::loaded(WorkflowList::default());
    app.apply_theme(depth, None);
    app
}

#[test]
fn a_namespace_with_no_workflows_shows_the_mark_and_how_to_start_one() {
    let mut app = empty_namespace(ColorDepth::TrueColor);
    let out = draw(&mut app, 110, 24);
    assert!(out.contains("▀▀▀▀▀▀▀▀▀▀"), "{out}");
    assert!(out.contains("no workflows in this namespace"), "{out}");
    assert!(
        out.contains("temporal workflow start --type Hello --task-queue demo"),
        "{out}"
    );
}

#[test]
fn the_mark_is_not_drawn_where_it_does_not_fit_or_there_is_no_colour() {
    for (depth, w, h) in [
        (ColorDepth::TrueColor, 60, 24),
        (ColorDepth::TrueColor, 110, 10),
        (ColorDepth::Mono, 110, 24),
    ] {
        let mut app = empty_namespace(depth);
        let out = draw(&mut app, w, h);
        assert!(!out.contains("▀▀▀▀"), "{depth:?} {w}x{h}:\n{out}");
        assert!(out.contains("no workflows in this namespace"), "{out}");
    }
}

#[test]
fn a_filter_that_matches_nothing_is_not_a_welcome() {
    let mut app = empty_namespace(ColorDepth::TrueColor);
    app.view.query = "ExecutionStatus = 'Failed'".into();
    let out = draw(&mut app, 110, 24);
    assert!(!out.contains("▀▀▀▀"), "{out}");
    assert!(out.contains("no workflows match this query"), "{out}");
}

#[test]
fn help_is_titled_with_the_wordmark_and_version() {
    let mut app = app_with_rows();
    app.show_help = true;
    let out = draw(&mut app, 110, 30);
    assert!(
        out.contains(&format!("tmprl▌ {} · help", env!("CARGO_PKG_VERSION"))),
        "{out}"
    );
}

#[test]
fn the_splash_is_the_mark_beside_where_it_connects_and_none_without_colour() {
    let status = || vec![Line::from(Span::raw("connecting…"))];
    let t = Theme::new(ColorDepth::TrueColor, &ThemeOverrides::default());
    let (width, lines) = brand::splash(&t, "127.0.0.1:7233", status(), 110).unwrap();
    assert_eq!(lines.len(), brand::MARK_HEIGHT);
    assert_eq!(width, brand::MARK_WIDTH + 3 + "127.0.0.1:7233".len());
    assert!(lines[2].contains("127.0.0.1:7233"), "{lines:?}");
    assert!(lines[4].contains("connecting…"), "{lines:?}");
    assert!(lines[0].contains("\x1b[38;2;44;51;64m"), "{lines:?}");

    let mono = Theme::new(ColorDepth::Mono, &ThemeOverrides::default());
    assert!(brand::splash(&mono, "127.0.0.1:7233", status(), 110).is_none());
}

#[test]
fn a_long_connect_error_wraps_beside_the_mark() {
    let t = Theme::new(ColorDepth::TrueColor, &ThemeOverrides::default());
    let error = "could not connect to Temporal: tcp connect error ".repeat(5);
    let status = vec![Line::from(Span::styled(error, t.err))];
    let (width, lines) = brand::splash(&t, "127.0.0.1:7299", status, 80).unwrap();
    assert!(width < 80, "{width}");
    assert!(lines.len() > brand::MARK_HEIGHT - 1, "{lines:?}");
}

fn splash(depth: ColorDepth, w: u16, h: u16) -> Vec<String> {
    let t = Theme::new(depth, &ThemeOverrides::default());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| crate::ui::splash::render(f, "profile sit", 2_300, &t))
        .unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn connecting_fills_the_screen_with_the_mark_twice_its_size_where_it_fits() {
    let rows = splash(ColorDepth::TrueColor, 120, 40);
    let out = rows.join("\n");
    let mark: Vec<&String> = rows.iter().filter(|r| r.contains("▀▀▀▀")).collect();
    assert_eq!(mark.len(), brand::MARK_HEIGHT * 2, "{out}");
    assert!(
        out.contains(&format!("tmprl▌ {}", env!("CARGO_PKG_VERSION"))),
        "{out}"
    );
    assert!(out.contains("profile sit"), "{out}");
    assert!(out.contains("connecting  2s"), "{out}");
    assert!(rows[39].trim_end().ends_with("Ctrl-C to quit"), "{out}");
    let line = rows.iter().find(|r| r.contains("connecting")).unwrap();
    let left = line.len() - line.trim_start().len();
    let right = line.len() - line.trim_end().len();
    assert!(left.abs_diff(right) <= 2, "centred: {line:?}");

    let small = splash(ColorDepth::TrueColor, 80, 22).join("\n");
    assert!(
        small.contains("▀▀▀▀") && !small.contains(&"▀".repeat(30)),
        "{small}"
    );

    let mono = splash(ColorDepth::Mono, 120, 40).join("\n");
    assert!(
        !mono.contains("▀▀▀▀") && mono.contains("connecting"),
        "{mono}"
    );
}

#[test]
fn gf_waits_behind_a_card_with_the_mark_the_stage_and_the_time() {
    let mut app = empty_namespace(ColorDepth::TrueColor);
    app.source_wait = Some(crate::app::SourceWait {
        subject: "create_task".into(),
        asking: Some("tmprl-source".into()),
        since_ms: crate::app::now_ms() - 3_000,
        request: Some(1),
    });
    let out = draw(&mut app, 110, 30);
    assert!(out.contains("▀▀▀▀▀▀▀▀▀▀"), "{out}");
    assert!(out.contains("finding the source of `create_task`"), "{out}");
    assert!(out.contains("asking tmprl-source  3s"), "{out}");
    assert!(out.contains("Esc to stop"), "{out}");

    app.source_wait.as_mut().unwrap().asking = None;
    assert!(draw(&mut app, 110, 30).contains("decoding the payloads"));

    app.apply_theme(ColorDepth::Mono, None);
    let out = draw(&mut app, 110, 30);
    assert!(out.contains("finding the source of `create_task`"), "{out}");
}
