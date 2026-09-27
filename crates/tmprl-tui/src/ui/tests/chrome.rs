//! Everything around the lists: header, statusline, gutter, splits, help, which-key, `:`.

use super::*;

#[test]
fn each_pane_draws_its_own_view_not_the_focused_one_twice() {
    // The bug this guards: a render loop that falls back to the focused view paints the
    // same pane twice, and a split looks like it worked while showing nothing new.
    let mut app = app_with_rows(); // namespaces loaded
    app.run("window.split-right", None);
    assert_eq!(app.tabs.current().len(), 2);

    let out = draw(&mut app, 120, 16);
    let body: Vec<&str> = out.lines().skip(1).collect();

    // The focused (new) pane is empty; the parked one still lists namespaces. So exactly
    // one side of each row should carry a namespace name.
    let with_default = body.iter().filter(|l| l.contains("default")).count();
    assert!(
        with_default > 0,
        "the pane we split from should still show its namespaces:\n{out}"
    );
    assert!(
        body.iter().any(|l| l.contains("no namespaces")),
        "the new pane should be empty until it loads:\n{out}"
    );
}

#[test]
fn truncation_respects_character_boundaries() {
    assert_eq!(truncate("short", 10), "short");
    assert_eq!(truncate("abcdefghij", 5), "abcd…");
    // Multi-byte input must not be sliced mid-character.
    assert_eq!(truncate("日本語テスト", 3), "日本…");
}

#[test]
fn the_gutter_is_absolute_on_the_cursor_and_relative_elsewhere() {
    assert_eq!(
        gutter(4, 4).trim(),
        "5",
        "cursor row shows its 1-based index"
    );
    assert_eq!(gutter(1, 4).trim(), "3");
    assert_eq!(gutter(7, 4).trim(), "3");
}

#[test]
fn header_shows_profile_and_namespace() {
    let mut app = app_with_rows();
    let out = draw(&mut app, 90, 12);
    assert!(out.contains("prod"), "profile missing:\n{out}");
    assert!(out.contains("default"), "namespace missing:\n{out}");
    assert!(out.contains('3'), "row count missing:\n{out}");
}

#[test]
fn a_readonly_profile_says_so_in_the_header() {
    // Colour alone is not the signal: this has to read on a 16-colour terminal and for
    // a colour-blind reader, so the marker is text.
    let mut app = app_with_rows();
    app.apply_config(None, None, Some("[profile.prod]\nreadonly = true"));
    let out = draw(&mut app, 90, 12);
    assert!(
        out.contains("prod [ro]"),
        "read-only marker missing:\n{out}"
    );
}

#[test]
fn a_writable_profile_carries_no_marker() {
    let mut app = app_with_rows();
    let out = draw(&mut app, 90, 12);
    assert!(!out.contains("[ro]"), "unexpected marker:\n{out}");
}

#[test]
fn statusline_shows_the_mode() {
    let mut app = app_with_rows();
    assert!(draw(&mut app, 90, 12).contains("NORMAL"));
    app.run("mode.insert", None);
    assert!(draw(&mut app, 90, 12).contains("INSERT"));
    app.run("mode.visual", None);
    assert!(draw(&mut app, 90, 12).contains("VISUAL"));
}

#[test]
fn rows_are_listed_with_a_hybrid_relative_gutter() {
    let mut app = app_with_rows();
    app.view.cursor = 1;
    let out = draw(&mut app, 90, 12);
    assert!(out.contains("payments"), "rows missing:\n{out}");

    // Skip the header, which also mentions the namespace by name.
    let body: Vec<&str> = out.lines().skip(1).collect();
    let gutter = |needle: &str| -> String {
        body.iter()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no row for {needle}:\n{out}"))
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect()
    };
    // The cursor row shows its own 1-based index; the others show distance.
    assert_eq!(
        gutter("payments"),
        "2",
        "absolute number on cursor row:\n{out}"
    );
    assert_eq!(gutter("default"), "1", "distance one row above:\n{out}");
    assert_eq!(
        gutter("temporal-system"),
        "1",
        "distance one row below:\n{out}"
    );
}

#[test]
fn which_key_popup_appears_after_the_leader() {
    let mut app = app_with_rows();
    app.handle(crate::app::Msg::Key(Chord::ch(' ')));
    let out = draw(&mut app, 90, 14);
    assert!(
        !app.which_key.is_empty(),
        "leader should open a pending state"
    );
    assert!(
        out.contains("Quit"),
        "which-key should list <leader>q:\n{out}"
    );
}

#[test]
fn help_overlay_lists_command_groups() {
    let mut app = app_with_rows();
    app.run("app.help", None);
    // Tall enough to reach the Mode group without scrolling: this is a test about ids
    // being rendered whole, not about where the fold falls. Scrolling has its own.
    let out = draw(&mut app, 90, 40);
    assert!(out.contains("Motion"), "group heading missing:\n{out}");
    assert!(out.contains("Quit"), "command title missing:\n{out}");
    // Bindings are rendered from the keymap, not written by hand.
    assert!(out.contains("<Space>q"), "leader binding missing:\n{out}");
    assert!(out.contains("jk"), "insert-escape binding missing:\n{out}");
    // Ids must not be truncated, they are what `:` and keys.toml consume.
    assert!(out.contains("app.command-line"), "id truncated:\n{out}");
    assert!(out.contains("mode.visual-line"), "id truncated:\n{out}");
}

#[test]
fn the_help_overlay_scrolls_instead_of_clipping_silently() {
    // Adding commands must never push a group off the bottom with no sign of it,
    // that is how a reader concludes a command does not exist.
    let mut app = app_with_rows();
    app.run("app.help", None);

    let out = draw(&mut app, 90, 16);
    assert!(
        app.help_max_scroll > 0,
        "the overlay should overflow at 16 rows"
    );
    assert!(
        out.contains("j/k to scroll"),
        "an overflowing overlay must say so:\n{out}"
    );
    assert!(out.contains("Application"), "first group missing:\n{out}");

    // The last group is reachable by scrolling. Whatever group is registered last, the
    // point is that adding commands must not push one off the bottom unreachably.
    app.run("motion.bottom", None);
    let out = draw(&mut app, 90, 16);
    let last_group = app.registry.groups().last().copied().unwrap();
    let last_id = app
        .registry
        .all()
        .iter()
        .rfind(|c| c.group == last_group)
        .unwrap()
        .id;
    assert!(
        out.contains(last_id),
        "the last command ({last_id}) must be reachable:\n{out}"
    );
}

#[test]
fn a_long_binding_list_never_runs_into_the_title_column() {
    // Three bindings on one command is ordinary once keys.toml adds to the defaults.
    let mut app = app_with_rows();
    app.apply_config(Some("[normal]\n\"ZZ\" = \"app.quit\"\n"), None, None);
    app.run("app.help", None);

    let out = draw(&mut app, 90, 60);
    let line = out
        .lines()
        .find(|l| l.contains("app.quit"))
        .unwrap_or_else(|| panic!("no app.quit row:\n{out}"));
    assert!(
        line.contains("ZZ") && line.contains("Quit"),
        "both should render:\n{line}"
    );
    assert!(
        !line.contains("ZZQuit"),
        "the key column must not collide with the title:\n{line}"
    );
}

#[test]
fn a_help_overlay_that_fits_says_nothing_about_scrolling() {
    let mut app = app_with_rows();
    app.run("app.help", None);
    // Tall enough that the whole registry fits, the overlay has grown with every
    // milestone, so the height here is "definitely more than enough", not a magic number.
    let out = draw(&mut app, 90, 200);
    assert_eq!(app.help_max_scroll, 0);
    assert!(!out.contains("j/k to scroll"), "{out}");
    assert!(
        out.contains("nav.open") && out.contains("yank.record"),
        "{out}"
    );
}

#[test]
fn motions_move_the_cursor_again_once_help_is_closed() {
    let mut app = app_with_rows();
    app.run("app.help", None);
    app.run("motion.down", None);
    assert_eq!(
        app.view.cursor, 0,
        "help was open; the list cursor must not move"
    );

    app.run("app.cancel", None);
    app.run("motion.down", None);
    assert_eq!(app.view.cursor, 1);
    assert_eq!(app.help_scroll, 0, "closing help resets its scroll");
}

#[test]
fn command_line_shows_completions() {
    let mut app = app_with_rows();
    app.run("app.command-line", None);
    for c in "motion".chars() {
        app.handle(crate::app::Msg::Key(Chord::ch(c)));
    }
    let out = draw(&mut app, 90, 16);
    assert!(out.contains(":motion"), "command line missing:\n{out}");
    assert!(out.contains("motion.down"), "completions missing:\n{out}");
}

#[test]
fn empty_state_renders_without_panicking() {
    let (tx, _rx) = unbounded_channel();
    let mut app = App::detached("p", "n", tx);
    let out = draw(&mut app, 80, 10);
    assert!(out.contains("NORMAL"));
}

#[test]
fn renders_at_a_cramped_terminal_size() {
    // A layout that assumes room is a layout that panics on someone's split pane.
    let mut app = app_with_rows();
    app.show_help = true;
    let _ = draw(&mut app, 20, 4);
    app.show_help = false;
    app.handle(crate::app::Msg::Key(Chord::ch(' ')));
    let _ = draw(&mut app, 20, 4);
}

#[test]
fn a_list_longer_than_the_pane_grows_a_scrollbar() {
    // The thumb is the only thing that says "there is more below" on a list whose rows all
    // look alike.
    let mut app = app_with_workflows(&["default"]);
    let start = 1_789_628_602_431;
    let mut list = WorkflowList::default();
    list.reset(
        (0..80)
            .map(|i| {
                wf(
                    "default",
                    &format!("order-{i}"),
                    WorkflowStatus::Running,
                    start,
                )
            })
            .collect(),
        vec![],
    );
    app.view.workflows = Loadable::loaded(list);

    let out = draw(&mut app, 110, 12);
    assert!(
        out.contains('█') || out.contains('║') || out.contains('▐'),
        "a scrollbar thumb should be on screen:\n{out}"
    );
}

#[test]
fn a_list_that_fits_draws_no_scrollbar() {
    // A pane with nothing to scroll must look exactly as it did before.
    let mut app = app_with_workflows(&["default"]);
    let out = draw(&mut app, 110, 20);
    assert!(
        !out.contains('█'),
        "two rows in a twenty-row pane need no thumb:\n{out}"
    );
}

#[test]
fn the_scrollbar_does_not_eat_the_column_beside_it() {
    // The bar takes its own column rather than painting over the rightmost one, which on
    // the workflow list is the age.
    let mut app = app_with_workflows(&["default"]);
    let start = 1_789_628_602_431;
    let mut list = WorkflowList::default();
    list.reset(
        (0..80)
            .map(|i| {
                wf(
                    "default",
                    &format!("order-{i}"),
                    WorkflowStatus::Running,
                    start,
                )
            })
            .collect(),
        vec![],
    );
    app.view.workflows = Loadable::loaded(list);

    let out = draw(&mut app, 110, 12);
    assert!(out.contains("order-0"), "the rows still render:\n{out}");
    assert!(
        out.contains("Running"),
        "and so does every column before the bar:\n{out}"
    );
}
