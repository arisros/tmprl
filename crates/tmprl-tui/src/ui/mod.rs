//! Rendering. Every function here is a pure projection of `&App` onto a frame — no state
//! is mutated except the viewport height, which the layout is what determines.

mod cmdline;
mod help;
mod list;
mod statusline;
mod whichkey;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::App;
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // The list needs to know how tall it is so that half-page motions mean something.
    app.page = body.height.saturating_sub(1) as usize;

    statusline::render_header(frame, header, app, &theme);
    list::render(frame, body, app, &theme);
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
    use tmprl_core::{Chord, Loadable};
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
