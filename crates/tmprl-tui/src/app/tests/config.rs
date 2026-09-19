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
