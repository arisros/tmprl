//! Editing a line: the prompts, the `:` line and the query bar.

use super::*;
use tmprl_core::Key;

fn key(app: &mut App, k: Key) {
    app.handle(Msg::Key(Chord::plain(k)));
}

#[test]
fn a_prompt_edits_at_its_cursor() {
    let mut app = app();
    app.run("search.open", None);
    type_chars(&mut app, "ac");
    key(&mut app, Key::Left);
    type_chars(&mut app, "b");
    assert_eq!(app.prompt.as_ref().unwrap().buf, "abc");
}

#[test]
fn up_recalls_what_was_entered_at_the_same_prompt() {
    let mut app = app();
    search_for(&mut app, "charge");
    app.run("search.open", None);
    key(&mut app, Key::Up);
    assert_eq!(app.prompt.as_ref().unwrap().buf, "charge");
    key(&mut app, Key::Down);
    assert_eq!(app.prompt.as_ref().unwrap().buf, "");
}

#[test]
fn each_prompt_keeps_its_own_history() {
    let mut app = app();
    search_for(&mut app, "charge");
    app.run("app.command-line", None);
    key(&mut app, Key::Up);
    assert_eq!(
        app.prompt.as_ref().unwrap().buf,
        "",
        "a search is not a command"
    );
}

#[test]
fn tab_fills_in_the_completions_in_turn() {
    let mut app = app();
    app.run("app.command-line", None);
    type_chars(&mut app, "yank");
    let offered: Vec<&str> = app.cmdline_matches().iter().map(|c| c.id).collect();
    assert!(offered.len() > 1, "{offered:?}");

    key(&mut app, Key::Tab);
    assert_eq!(app.prompt.as_ref().unwrap().buf, offered[0]);
    key(&mut app, Key::Tab);
    assert_eq!(app.prompt.as_ref().unwrap().buf, offered[1]);
    key(&mut app, Key::BackTab);
    assert_eq!(app.prompt.as_ref().unwrap().buf, offered[0]);
    let still: Vec<&str> = app.cmdline_matches().iter().map(|c| c.id).collect();
    assert_eq!(still, offered, "the list is still for what was typed");
}

#[test]
fn colon_q_quits() {
    let mut app = app();
    app.run("app.command-line", None);
    type_chars(&mut app, "q");
    key(&mut app, Key::Enter);
    assert!(app.should_quit);
}

#[test]
fn a_paste_goes_into_the_open_prompt_on_one_line() {
    let mut app = app();
    app.run("search.open", None);
    app.handle(Msg::Paste("order-42\n".into()));
    assert_eq!(app.prompt.as_ref().unwrap().buf, "order-42");
}

#[test]
fn a_paste_with_nothing_open_finds_the_workflow_instead_of_running_keys() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    app.handle(Msg::Paste("order-r1\n".into()));
    let picker = app.picker.as_ref().expect("the workflow finder");
    assert_eq!(picker.prompt, "order-r1");
    assert_eq!(app.view.screen, Screen::Workflows, "nothing ran as a key");
}

#[test]
fn the_query_bar_remembers_the_queries_applied() {
    let mut app = app();
    loaded(&mut app, vec![wf("default", "r1", 100)], vec![]);
    for q in ["A = 1", "B = 2"] {
        app.run("mode.insert", None);
        app.handle(Msg::Key(Chord::ctrl('u')));
        type_chars(&mut app, q);
        key(&mut app, Key::Enter);
    }
    app.run("mode.insert", None);
    app.handle(Msg::Key(Chord::ctrl('u')));
    key(&mut app, Key::Up);
    assert_eq!(app.insert_buf, "B = 2");
    key(&mut app, Key::Up);
    assert_eq!(app.insert_buf, "A = 1");
}
