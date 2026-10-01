//! Line editing in every input, and pasted text.

use super::*;
use tmprl_core::Mods;

fn key(app: &mut App, key: Key) {
    app.handle(Msg::Key(Chord::plain(key)));
}

fn ctrl(app: &mut App, c: char) {
    app.handle(Msg::Key(Chord::ctrl(c)));
}

fn alt(app: &mut App, c: char) {
    app.handle(Msg::Key(Chord {
        key: Key::Char(c),
        mods: Mods {
            ctrl: false,
            alt: true,
            shift: false,
        },
    }));
}

fn paste(app: &mut App, text: &str) {
    app.handle(Msg::Paste(text.into()));
}

fn note(app: &App) -> &str {
    app.note.as_ref().map_or("", |(msg, _)| msg.as_str())
}

type Line = for<'a> fn(&'a App) -> &'a LineEdit;

/// Every text input there is, open and empty, with the way to read its line.
fn inputs() -> Vec<(&'static str, App, Line)> {
    let mut command = app();
    command.run("app.command-line", None);

    let mut query = on_a_workflow();
    query.run("mode.insert", None);

    let mut picker = on_four_workflows();
    picker.run("find.workflow", None);

    let mut form = on_schedules();
    form.run("schedule.create", None);

    let mut confirm = on_a_workflow();
    confirm.run("workflow.delete", None);

    vec![
        (":", command, |a| &a.prompt.as_ref().unwrap().buf),
        ("query bar", query, |a| &a.insert_buf),
        ("picker", picker, |a| &a.picker.as_ref().unwrap().prompt),
        ("form", form, |a| {
            let form = a.form.as_ref().unwrap();
            &form.fields[form.cursor].value
        }),
        ("confirmation", confirm, |a| {
            &a.confirm.as_ref().unwrap().entered
        }),
    ]
}

/// The line with its cursor drawn in, so an assertion reads as a picture.
fn show(line: &LineEdit) -> String {
    let (before, after) = line.split();
    format!("{before}|{after}")
}

#[test]
fn every_input_edits_at_its_cursor() {
    for (name, mut app, line) in inputs() {
        type_chars(&mut app, "ac");
        key(&mut app, Key::Left);
        type_chars(&mut app, "b");
        assert_eq!(show(line(&app)), "ab|c", "{name}");

        key(&mut app, Key::Home);
        key(&mut app, Key::Delete);
        assert_eq!(show(line(&app)), "|bc", "{name}");

        ctrl(&mut app, 'e');
        key(&mut app, Key::Backspace);
        assert_eq!(show(line(&app)), "b|", "{name}");
    }
}

#[test]
fn every_input_takes_the_readline_keys() {
    for (name, mut app, line) in inputs() {
        type_chars(&mut app, "one two three");
        alt(&mut app, 'b');
        assert_eq!(show(line(&app)), "one two |three", "{name}: <A-b>");
        ctrl(&mut app, 'k');
        assert_eq!(show(line(&app)), "one two |", "{name}: <C-k>");
        ctrl(&mut app, 'w');
        assert_eq!(show(line(&app)), "one |", "{name}: <C-w>");
        ctrl(&mut app, 'a');
        ctrl(&mut app, 'f');
        assert_eq!(show(line(&app)), "o|ne ", "{name}: <C-a> <C-f>");
        ctrl(&mut app, 'b');
        alt(&mut app, 'f');
        assert_eq!(show(line(&app)), "one| ", "{name}: <C-b> <A-f>");
        ctrl(&mut app, 'u');
        assert_eq!(show(line(&app)), "| ", "{name}: <C-u>");
    }
}

#[test]
fn emptying_a_line_does_not_close_the_input() {
    // Only a backspace with nothing left to take closes a prompt. `<C-u>` on a line that
    // was wrong from the first character is how most lines get retyped.
    for (name, mut app, line) in inputs() {
        type_chars(&mut app, "abc");
        ctrl(&mut app, 'u');
        assert_eq!(show(line(&app)), "|", "{name}");
    }
}

#[test]
fn backspace_at_the_start_of_a_line_with_text_closes_nothing() {
    for (name, mut app, line) in inputs() {
        type_chars(&mut app, "abc");
        key(&mut app, Key::Home);
        key(&mut app, Key::Backspace);
        assert_eq!(show(line(&app)), "|abc", "{name}");
    }
}

#[test]
fn backspace_on_an_empty_prompt_still_closes_it() {
    let mut app = app();
    app.run("app.command-line", None);
    key(&mut app, Key::Backspace);
    assert!(app.prompt.is_none());
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn a_prompt_submits_the_whole_line_wherever_the_cursor_is() {
    let mut app = app();
    app.run("app.command-line", None);
    type_chars(&mut app, "pp.quit");
    key(&mut app, Key::Home);
    type_chars(&mut app, "a");
    key(&mut app, Key::Enter);
    assert!(
        app.should_quit,
        "`app.quit` was typed, its first letter last"
    );
}

#[test]
fn the_picker_keeps_its_list_keys() {
    // `<C-n>` and `<C-p>` move the selection and are not readline's history keys here,
    // and a cursor move in the prompt leaves the selection where it was.
    let mut app = on_four_workflows();
    app.run("find.workflow", None);
    type_chars(&mut app, "order");
    ctrl(&mut app, 'n');
    ctrl(&mut app, 'n');
    ctrl(&mut app, 'p');
    assert_eq!(app.picker.as_ref().unwrap().cursor, 1);

    key(&mut app, Key::Left);
    ctrl(&mut app, 'a');
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.cursor, 1, "moving in the prompt is not typing in it");
    assert_eq!(show(&picker.prompt), "|order");
}

#[test]
fn the_form_keeps_tab_for_its_fields() {
    let mut app = on_schedules();
    app.run("schedule.create", None);
    type_chars(&mut app, "nightly");
    key(&mut app, Key::Left);
    key(&mut app, Key::Tab);
    type_chars(&mut app, "recon");
    key(&mut app, Key::BackTab);
    type_chars(&mut app, "-");

    let form = app.form.as_ref().unwrap();
    assert_eq!(form.get("schedule id"), "nightl-y", "its cursor was kept");
    assert_eq!(form.get("workflow id"), "recon");
}

#[test]
fn the_typed_word_can_be_corrected_in_the_middle() {
    let mut app = on_a_workflow();
    app.run("workflow.delete", None);
    type_chars(&mut app, "delte");
    assert!(!app.confirm.as_ref().unwrap().is_satisfied());
    key(&mut app, Key::Left);
    key(&mut app, Key::Left);
    type_chars(&mut app, "e");
    assert!(app.confirm.as_ref().unwrap().is_satisfied());
}

#[test]
fn jk_still_leaves_the_query_bar_and_a_lone_j_is_still_text() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    type_chars(&mut app, "aj");
    key(&mut app, Key::Left);
    assert_eq!(show(&app.insert_buf), "a|j", "the held `j` went in first");
    assert_eq!(app.mode, Mode::Insert);

    type_chars(&mut app, "jk");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn the_query_bar_keeps_its_completion_keys() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    type_chars(&mut app, "e");
    let first = app.completion.as_ref().unwrap().cursor();
    ctrl(&mut app, 'n');
    assert_ne!(app.completion.as_ref().unwrap().cursor(), first);
    ctrl(&mut app, 'p');
    assert_eq!(app.completion.as_ref().unwrap().cursor(), first);
    assert_eq!(show(&app.insert_buf), "e|", "neither moved the text cursor");
}

#[test]
fn tab_completes_the_clause_left_of_the_cursor_and_keeps_the_rest() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    type_chars(&mut app, " AND WorkflowType = 'Checkout'");
    key(&mut app, Key::Home);
    type_chars(&mut app, "runn");
    key(&mut app, Key::Tab);

    assert_eq!(
        show(&app.insert_buf),
        "ExecutionStatus = 'Running'| AND WorkflowType = 'Checkout'"
    );
    assert!(app.is_editing_query(), "Tab completes; it does not apply");
}

#[test]
fn offers_follow_typing_and_a_cursor_move_dismisses_them() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    type_chars(&mut app, "runn");
    assert!(app.completion.is_some());

    key(&mut app, Key::Left);
    assert!(
        app.completion.is_none(),
        "the offers were for another cursor"
    );
    assert_eq!(show(&app.insert_buf), "run|n", "the line is untouched");

    // `ru` is what is left of the cursor now, and what the offers come back for.
    key(&mut app, Key::Backspace);
    assert!(app.completion.is_some());
    key(&mut app, Key::Tab);
    assert_eq!(show(&app.insert_buf), "ExecutionStatus = 'Running'| n");
}

#[test]
fn a_paste_lands_at_the_cursor_of_whichever_input_is_open() {
    for (name, mut app, line) in inputs() {
        if name == "confirmation" {
            continue;
        }
        type_chars(&mut app, "ad");
        key(&mut app, Key::Left);
        paste(&mut app, "bc");
        assert_eq!(show(line(&app)), "abc|d", "{name}");
    }
}

#[test]
fn a_pasted_newline_never_submits() {
    // The line being pasted was copied off a terminal, so it ends in a newline. As keys,
    // that newline was an Enter.
    let mut app = app();
    app.run("app.command-line", None);
    paste(&mut app, "app.quit\n");
    assert!(!app.should_quit);
    let prompt = app.prompt.as_ref().expect("still open, still editable");
    assert_eq!(show(&prompt.buf), "app.quit|");

    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    paste(
        &mut app,
        "WorkflowType = 'Checkout'\nAND ExecutionStatus = 'Failed'\n",
    );
    assert!(app.is_editing_query(), "not applied");
    assert_eq!(app.view.query, "");
    assert_eq!(
        app.insert_buf,
        "WorkflowType = 'Checkout' AND ExecutionStatus = 'Failed'"
    );

    let mut app = on_schedules();
    app.run("schedule.create", None);
    paste(&mut app, "nightly\n");
    assert!(app.form.is_some(), "not sent for review");
    assert!(app.confirm.is_none());
}

#[test]
fn a_paste_into_the_picker_narrows_it_and_takes_nothing() {
    let mut app = on_four_workflows();
    app.run("find.workflow", None);
    paste(&mut app, "order-r2\n");

    assert_eq!(picker_labels(&app), ["order-r2"]);
    assert_eq!(app.view.screen, Screen::Workflows, "nothing was opened");
}

#[test]
fn a_paste_never_reaches_a_confirmation() {
    let mut app = on_a_workflow();
    app.run("workflow.delete", None);
    paste(&mut app, "delete\n");

    let confirm = app.confirm.as_ref().expect("still waiting");
    assert_eq!(confirm.entered, "", "the word has to be typed");
    assert!(!confirm.is_satisfied());
    assert!(note(&app).contains("paste ignored"), "{}", note(&app));

    key(&mut app, Key::Enter);
    assert!(app.confirm.is_some(), "and Enter still does nothing");
}

#[test]
fn a_paste_does_not_confirm_an_action_that_owes_no_word() {
    let mut app = on_a_workflow();
    app.run("workflow.terminate", None);
    paste(&mut app, "\n");
    assert!(app.confirm.is_some(), "a pasted newline is not an Enter");
}

#[test]
fn a_paste_with_no_input_open_is_ignored_and_says_so() {
    // Every character of this is a command in Normal mode: `j` moves, `G` jumps, `:`
    // opens the command line, and the newline would have run what followed it.
    let mut app = on_four_workflows();
    let before = app.view.cursor;
    paste(&mut app, "jjG:app.quit\n");

    assert_eq!(app.view.cursor, before);
    assert!(app.prompt.is_none());
    assert!(!app.should_quit);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.note.as_ref().map(|(_, kind)| *kind), Some(Note::Warn));
    assert!(note(&app).contains("paste ignored"), "{}", note(&app));
}

#[test]
fn a_paste_in_visual_mode_leaves_the_selection_alone() {
    let mut app = on_four_workflows();
    app.run("mode.visual", None);
    app.run("motion.down", None);
    let selected = app.selection();
    paste(&mut app, "jj\u{1b}");

    assert_eq!(app.mode, Mode::Visual);
    assert_eq!(app.selection(), selected);
}

#[test]
fn a_paste_behind_an_overlay_is_ignored_too() {
    let mut app = on_four_workflows();
    app.run("app.help", None);
    paste(&mut app, "q");
    assert!(app.show_help);
    assert!(!app.should_quit);
}

#[test]
fn a_pasted_jk_is_text_not_the_way_out_of_insert_mode() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    paste(&mut app, "WorkflowId = 'jk'");
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.insert_buf, "WorkflowId = 'jk'");
}

#[test]
fn a_j_held_before_a_paste_goes_in_before_it() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    type_chars(&mut app, "j");
    paste(&mut app, "kl");

    assert_eq!(app.insert_buf, "jkl");
    assert_eq!(app.mode, Mode::Insert, "a pasted `k` does not finish `jk`");
    assert!(app.pending.is_idle());
}

#[test]
fn a_paste_into_the_query_bar_brings_the_offers_up_to_date() {
    let mut app = on_a_workflow();
    app.run("mode.insert", None);
    paste(&mut app, "runn");
    assert!(app.completion.is_some());
    paste(&mut app, " ");
    assert!(app.completion.is_none(), "the clause is finished");
}
