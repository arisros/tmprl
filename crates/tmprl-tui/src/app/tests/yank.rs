//! What each yank copies.

use super::*;

#[test]
fn json_strings_escape_control_characters() {
    assert_eq!(json_string(r#"a"b"#), r#""a\"b""#);
    assert_eq!(json_string("a\nb"), r#""a\nb""#);
    assert_eq!(json_string("a\u{1}b"), r#""a\u0001b""#);
}

#[test]
fn yank_on_a_workflow_row_takes_the_workflow_id() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    assert_eq!(app.field_under_cursor(), "order-r1");

    let record = app.records_selected();
    assert!(
        record.contains(r#""workflowId":"order-r1""#),
        "got {record}"
    );
    assert!(record.contains(r#""status":"Running""#), "got {record}");
    assert!(record.contains(r#""namespace":"default""#), "got {record}");
}

#[test]
fn a_visual_selection_yanks_every_selected_workflow() {
    let mut app = app();
    loaded(
        &mut app,
        vec![wf("default", "r1", 300), wf("default", "r2", 200)],
        vec![],
    );
    app.run("motion.top", None);
    app.run("mode.visual", None);
    app.run("motion.down", None);

    let record = app.records_selected();
    assert!(record.starts_with('['), "a multi-row yank is an array");
    assert!(record.contains("order-r1") && record.contains("order-r2"));
}

#[test]
fn yanking_a_history_row_takes_something_useful() {
    let mut app = viewing_history();
    app.run("motion.bottom", None);
    assert_eq!(app.field_under_cursor(), "Ship");

    let record = app.records_selected();
    assert!(record.contains(r#""group":"Ship""#), "got {record}");
    assert!(record.contains(r#""outcome":"Failed""#), "got {record}");
}

#[test]
fn yanking_the_result_unwraps_it() {
    let mut app = viewing_payloads();
    app.run("motion.down", None); // the Charge group
    app.run("yank.payload-result", None);
    let (note, level) = app.note.clone().expect("a yank should report");
    assert!(note.contains("yanked"), "{note}");
    assert!(matches!(level, Note::Info));
}

#[test]
fn yanking_the_input_skips_the_result() {
    let mut app = viewing_payloads();
    app.run("motion.down", None); // the Charge group
    let all = app.payloads_under_cursor();
    assert!(
        all.iter().any(|(l, _)| l == "input") && all.iter().any(|(l, _)| l == "result"),
        "fixture should carry both"
    );
    // The filter is what separates them; the clipboard itself is not reachable in a test.
    app.run("yank.payload-input", None);
    assert!(app.note.as_ref().unwrap().0.contains("yanked"));
}

#[test]
fn yanking_a_payload_off_a_history_is_refused() {
    let mut app = app();
    app.run("yank.payload", None);
    let (note, level) = app.note.clone().expect("a refusal should be reported");
    assert!(note.contains("workflow history"), "{note}");
    assert!(matches!(level, Note::Warn));
}

#[test]
fn yanking_an_absent_part_says_so() {
    let mut app = viewing_payloads();
    // Row 0 is the opening group, which carries no payloads at all.
    app.run("motion.top", None);
    app.run("yank.payload-result", None);
    let (note, level) = app.note.clone().unwrap();
    assert!(note.contains("no result"), "got {note}");
    assert_eq!(level, Note::Warn);
}

fn yank_file_in(note: &str) -> std::path::PathBuf {
    let path = note
        .split("written to ")
        .nth(1)
        .expect("a path in the note");
    std::path::PathBuf::from(path)
}

#[test]
fn a_yank_over_the_limit_is_written_to_a_file() {
    let mut app = app();
    app.apply_config(None, None, Some("[yank]\nmax_bytes = 10"));
    let text = r#"{"order":"a value longer than ten bytes"}"#;
    app.yank(text.to_string());

    let (note, level) = app.note.clone().expect("a note");
    assert_eq!(level, Note::Warn);
    assert!(
        note.starts_with(&format!(
            "{} bytes is over the yank limit (10), written to ",
            text.len()
        )),
        "got {note}"
    );
    let path = yank_file_in(&note);
    assert_eq!(path.file_name().unwrap(), "yank.json");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn a_yank_at_the_limit_still_goes_to_the_clipboard() {
    let mut app = app();
    app.apply_config(None, None, Some("[yank]\nmax_bytes = 10"));
    app.yank("0123456789".to_string());
    let (note, level) = app.note.clone().expect("a note");
    assert_eq!(level, Note::Info);
    assert_eq!(note, "yanked 10 bytes to clipboard");
}

#[test]
fn a_yank_written_to_a_file_ends_the_selection() {
    let mut app = app();
    app.apply_config(None, None, Some("[yank]\nmax_bytes = 1"));
    loaded(
        &mut app,
        vec![wf("default", "r1", 100), wf("default", "r2", 90)],
        vec![],
    );
    app.mode = Mode::VisualLine;
    app.view.anchor = Some(0);
    app.yank("order-r1\norder-r2".to_string());

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view.anchor, None);
    let path = yank_file_in(&app.note.clone().unwrap().0);
    assert_eq!(path.file_name().unwrap(), "yank.txt");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn the_yank_file_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let path = write_yank_file("[1,2,3]").unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
