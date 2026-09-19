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
