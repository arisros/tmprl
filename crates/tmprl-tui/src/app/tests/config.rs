//! Configuration reaching the app.

use super::*;

#[test]
fn a_config_without_a_codec_section_is_not_an_error() {
    let mut app = app();
    app.apply_config(None, None, Some("# nothing here\n"));
    assert!(app.note.is_none());
    assert!(app.codec.is_none());
}

#[test]
fn a_broken_config_is_surfaced() {
    let mut app = app();
    app.apply_config(None, None, Some("[codec]\nauth = \"x\"\n"));
    let (msg, kind) = app.note.clone().unwrap();
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("codec.endpoint"), "got {msg}");
}

#[test]
fn a_configured_codec_is_used() {
    let mut app = app();
    app.apply_config(
        None,
        None,
        Some("[codec]\nendpoint = \"http://localhost:8081\"\n"),
    );
    assert!(app.note.is_none());
    assert!(app.codec.is_some());
}

#[test]
fn config_errors_are_surfaced_rather_than_swallowed() {
    let mut app = app();
    app.apply_config(Some("[normal]\n\"x\" = \"nope.nope\"\n"), None, None);
    let (msg, kind) = app.note.clone().expect("a bad binding must be reported");
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("nope.nope"), "got {msg}");
}

#[test]
fn views_from_config_become_commands_and_bindings() {
    let mut app = app();
    app.apply_config(
        None,
        Some(
            "[[view]]\nkey = \"1\"\nname = \"Running\"\nquery = \"ExecutionStatus = 'Running'\"\n",
        ),
        None,
    );
    assert!(app.note.is_none(), "a valid config must not warn");
    assert_eq!(app.registry.get("view.1").unwrap().title, "Running");

    app.handle(Msg::Key(Chord::ch(' ')));
    app.handle(Msg::Key(Chord::ch('1')));
    assert_eq!(app.view.query, "ExecutionStatus = 'Running'");
}

#[test]
fn a_theme_with_an_unknown_slot_is_surfaced_and_not_half_applied() {
    let mut app = app();
    app.apply_theme(
        ColorDepth::TrueColor,
        Some("err = \"#010203\"\nacent = \"red\"\n"),
    );
    let (msg, kind) = app.note.clone().expect("a bad theme must be reported");
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("theme.toml") && msg.contains("acent"), "{msg}");
    assert_eq!(app.theme, Theme::default());
}

#[test]
fn a_theme_with_an_unparseable_colour_is_surfaced() {
    let mut app = app();
    app.apply_theme(ColorDepth::TrueColor, Some("accent = \"#12345\"\n"));
    let (msg, kind) = app.note.clone().expect("a bad colour must be reported");
    assert_eq!(kind, Note::Error);
    assert!(msg.contains("#12345"), "{msg}");
}

#[test]
fn a_valid_theme_is_applied_without_a_word() {
    let mut app = app();
    app.apply_theme(ColorDepth::TrueColor, Some("accent = \"cyan\"\n"));
    assert!(app.note.is_none());
    assert_ne!(app.theme, Theme::default());
}

#[test]
fn no_theme_file_is_the_default_palette_for_the_depth() {
    let mut app = app();
    app.apply_theme(ColorDepth::Ansi16, None);
    assert!(app.note.is_none());
    assert_eq!(
        app.theme,
        Theme::new(ColorDepth::Ansi16, &ThemeOverrides::default())
    );
    assert_ne!(app.theme, Theme::default());
}

#[test]
fn a_theme_error_is_still_reported_under_no_color() {
    // The file is not used there, but it is still wrong, and saying so only once
    // NO_COLOR is unset would look like the theme had broken by itself.
    let mut app = app();
    app.apply_theme(ColorDepth::Mono, Some("nope = \"red\"\n"));
    assert!(app.note.is_some());
}
