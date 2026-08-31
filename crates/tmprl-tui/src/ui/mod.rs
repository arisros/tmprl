//! Rendering. Every function here is a pure projection of `&App` onto a frame — no state
//! is mutated except the viewport height, which the layout is what determines.

mod cmdline;
mod help;
mod namespaces;
mod query;
mod statusline;
mod whichkey;
mod workflows;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::{App, Screen};
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    // The query bar is part of the workflow screen's chrome, not an overlay: it is always
    // on screen so the query is never something you have to go and open.
    let query_height = match app.screen {
        Screen::Workflows => 1,
        Screen::Namespaces => 0,
    };
    let [header, query_bar, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(query_height),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // The list needs to know how tall it is so that half-page motions mean something.
    app.page = body.height.saturating_sub(1) as usize;

    statusline::render_header(frame, header, app, &theme);
    match app.screen {
        Screen::Namespaces => namespaces::render(frame, body, app, &theme),
        Screen::Workflows => {
            query::render(frame, query_bar, app, &theme);
            workflows::render(frame, body, app, &theme);
        }
    }
    statusline::render_status(frame, status, app, &theme);

    // Overlays, outermost last.
    if app.cmdline.is_some() {
        cmdline::render(frame, app, &theme);
    }
    if !app.which_key.is_empty() {
        whichkey::render(frame, app, &theme);
    }
    if app.show_help {
        help::render(frame, app, &theme);
    }
}

/// The hybrid relative/absolute gutter, matching `set relativenumber number`: the cursor
/// row shows its own 1-based index, every other row its distance. That is what makes a
/// count like `7j` something you read off the screen rather than estimate.
fn gutter(i: usize, cursor: usize) -> String {
    if i == cursor {
        format!("{:>4} ", i + 1)
    } else {
        format!("{:>4} ", i.abs_diff(cursor))
    }
}

/// Shorten to `max` characters, on a character boundary, with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// A centred box `w` x `h`, clamped to the frame.
fn centered(area: ratatui::layout::Rect, w: u16, h: u16) -> ratatui::layout::Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    ratatui::layout::Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tmprl_client::NamespaceInfo;
    use tmprl_core::{Chord, Loadable, StatusCounts, WorkflowList, WorkflowRow, WorkflowStatus};
    use tokio::sync::mpsc::unbounded_channel;

    fn ns(name: &str, days: i64) -> NamespaceInfo {
        NamespaceInfo {
            name: name.into(),
            state: "Registered".into(),
            retention_days: days,
            description: String::new(),
        }
    }

    /// An app with data but no connection. Rendering must be testable without a server —
    /// that is the whole point of keeping IO out of the render path.
    fn app_with_rows() -> App {
        let (tx, _rx) = unbounded_channel();
        let mut app = App::detached("prod", "default", tx);
        app.namespaces = Loadable::loaded(vec![
            ns("default", 24),
            ns("payments", 30),
            ns("temporal-system", 7),
        ]);
        app
    }

    fn draw(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn wf(ns: &str, id: &str, status: WorkflowStatus, start: i64) -> WorkflowRow {
        WorkflowRow {
            namespace: ns.into(),
            workflow_id: id.into(),
            run_id: format!("run-{id}"),
            workflow_type: "CheckoutWorkflow".into(),
            task_queue: "orders".into(),
            status,
            start_time: Some(start),
            close_time: None,
            history_length: 7,
        }
    }

    /// A workflow screen with rows, no connection. Start times are relative to now so the
    /// age column renders something stable.
    fn app_with_workflows(scope: &[&str]) -> App {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let mut app = app_with_rows();
        app.screen = crate::app::Screen::Workflows;
        app.scope = scope.iter().map(|s| s.to_string()).collect();

        let mut list = WorkflowList::default();
        list.reset(
            vec![
                wf(
                    "default",
                    "order-1001",
                    WorkflowStatus::Running,
                    now - 45_000,
                ),
                wf(
                    "payments",
                    "charge-77",
                    WorkflowStatus::Failed,
                    now - 7_200_000,
                ),
            ],
            vec![],
        );
        app.workflows = Loadable::loaded(list);
        app.counts = Loadable::loaded(StatusCounts::new(
            2,
            [(WorkflowStatus::Running, 1), (WorkflowStatus::Failed, 1)],
        ));
        app
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
    fn the_workflow_table_shows_status_id_type_and_age() {
        let mut app = app_with_workflows(&["default"]);
        let out = draw(&mut app, 110, 12);
        assert!(out.contains("order-1001"), "workflow id missing:\n{out}");
        assert!(out.contains("CheckoutWorkflow"), "type missing:\n{out}");
        assert!(out.contains("Running"), "status missing:\n{out}");
        assert!(out.contains("Failed"), "status missing:\n{out}");
        // Status is legible without colour: the glyph carries it too.
        assert!(
            out.contains(WorkflowStatus::Running.glyph()),
            "status glyph missing:\n{out}"
        );
        assert!(out.contains("45s"), "age column missing:\n{out}");
        assert!(out.contains("2h"), "age column missing:\n{out}");
    }

    #[test]
    fn the_query_bar_is_always_on_screen() {
        // The raw query is the interface; it is never behind a keystroke to reveal.
        let mut app = app_with_workflows(&["default"]);
        assert!(
            draw(&mut app, 110, 12).contains("query"),
            "query bar must be visible with an empty query"
        );

        app.query = "ExecutionStatus = 'Failed'".into();
        let out = draw(&mut app, 110, 12);
        assert!(
            out.contains("ExecutionStatus = 'Failed'"),
            "the raw query text must be shown verbatim:\n{out}"
        );
    }

    #[test]
    fn editing_the_query_shows_the_live_text_not_the_applied_one() {
        let mut app = app_with_workflows(&["default"]);
        app.query = "A = 1".into();
        app.run("mode.insert", None);
        for c in "23".chars() {
            app.handle(crate::app::Msg::Key(Chord::ch(c)));
        }
        let out = draw(&mut app, 110, 12);
        assert!(out.contains("A = 123"), "live edit missing:\n{out}");
        assert!(out.contains("INSERT"), "mode should be INSERT:\n{out}");
    }

    #[test]
    fn the_header_shows_per_status_counts() {
        let mut app = app_with_workflows(&["default"]);
        let out = draw(&mut app, 110, 12);
        let header = out.lines().next().unwrap();
        assert!(header.contains("total"), "count total missing:\n{out}");
        assert!(
            header.contains(WorkflowStatus::Failed.glyph()),
            "the header should tally failures with the table's glyph:\n{out}"
        );
    }

    #[test]
    fn the_namespace_column_appears_only_in_a_fan_out() {
        // On one namespace it would be the same value on every row.
        let mut single = app_with_workflows(&["default"]);
        let out = draw(&mut single, 110, 12);
        let body: String = out.lines().skip(2).collect::<Vec<_>>().join("\n");
        assert!(
            !body.contains("payments"),
            "a single-namespace list should not carry a namespace column:\n{out}"
        );

        let mut fanned = app_with_workflows(&["default", "payments"]);
        let out = draw(&mut fanned, 110, 12);
        let body: String = out.lines().skip(2).collect::<Vec<_>>().join("\n");
        assert!(
            body.contains("payments"),
            "a fan-out must tag rows with their namespace:\n{out}"
        );
    }

    #[test]
    fn an_empty_result_distinguishes_no_data_from_a_filter() {
        let mut app = app_with_workflows(&["default"]);
        app.workflows = Loadable::loaded(WorkflowList::default());

        let out = draw(&mut app, 110, 12);
        assert!(out.contains("no workflows in this namespace"), "{out}");

        app.query = "ExecutionStatus = 'Failed'".into();
        let out = draw(&mut app, 110, 12);
        assert!(
            out.contains("no workflows match this query"),
            "an empty filtered list must say the filter is why:\n{out}"
        );
    }

    #[test]
    fn the_workflow_screen_renders_at_a_cramped_size() {
        let mut app = app_with_workflows(&["default", "payments"]);
        let _ = draw(&mut app, 20, 4);
        let _ = draw(&mut app, 8, 3);
        app.show_help = true;
        let _ = draw(&mut app, 20, 4);
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
        app.cursor = 1;
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
        let out = draw(&mut app, 90, 24);
        assert!(out.contains("Motion"), "group heading missing:\n{out}");
        assert!(out.contains("Quit"), "command title missing:\n{out}");
        // Bindings are rendered from the keymap, not written by hand.
        assert!(out.contains("<Space>q"), "leader binding missing:\n{out}");
        assert!(out.contains("jk"), "insert-escape binding missing:\n{out}");
        // Ids must not be truncated — they are what `:` and keys.toml consume.
        assert!(out.contains("app.command-line"), "id truncated:\n{out}");
        assert!(out.contains("mode.visual-line"), "id truncated:\n{out}");
    }

    #[test]
    fn the_help_overlay_scrolls_instead_of_clipping_silently() {
        // Adding commands must never push a group off the bottom with no sign of it —
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

        // The last group is reachable by scrolling.
        app.run("motion.bottom", None);
        let out = draw(&mut app, 90, 16);
        assert!(
            out.contains("list.more"),
            "the last group must be reachable:\n{out}"
        );
    }

    #[test]
    fn a_long_binding_list_never_runs_into_the_title_column() {
        // Three bindings on one command is ordinary once keys.toml adds to the defaults.
        let mut app = app_with_rows();
        app.apply_config(Some("[normal]\n\"ZZ\" = \"app.quit\"\n"), None);
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
        let out = draw(&mut app, 90, 60);
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
            app.cursor, 0,
            "help was open; the list cursor must not move"
        );

        app.run("app.cancel", None);
        app.run("motion.down", None);
        assert_eq!(app.cursor, 1);
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
}
