//! The help overlay.
//!
//! Generated from the command registry and the keymap rather than written by hand, so it is
//! incapable of going stale: a command with no binding shows as unbound, and a binding to a
//! command that does not exist is impossible.

use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::App;
use crate::theme::Theme;

/// Width of the key column. One wider than the longest string it will render, so a long
/// binding list can never touch the title beside it.
const KEYS: usize = 22;

pub fn render(frame: &mut Frame, app: &mut App, t: &Theme) {
    // Wide enough that command ids are not truncated — the ids are the part a reader
    // needs verbatim, since they are what `:` and keys.toml take.
    let area = super::centered(
        frame.area(),
        78,
        frame.area().height.saturating_sub(2).max(3),
    );
    if area.height < 3 || area.width < 20 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for group in app.registry.groups() {
        lines.push(Line::from(Span::styled(
            group.to_string(),
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        )));
        for cmd in app.registry.all().iter().filter(|c| c.group == group) {
            let keys = app.keymap.keys_for(cmd.id);
            let rendered = if keys.is_empty() {
                "—".to_string()
            } else {
                let mut seen: Vec<String> = Vec::new();
                for b in keys {
                    let s = b.seq.to_string();
                    if !seen.contains(&s) {
                        seen.push(s);
                    }
                }
                seen.join(" / ")
            };
            // Truncated, not just padded: a command with several bindings (a keys.toml
            // addition on top of two defaults) would otherwise run into the title column.
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<KEYS$}", super::truncate(&rendered, KEYS - 1)),
                    Style::new().fg(t.warn),
                ),
                Span::styled(format!("{:<26}", cmd.title), Style::new().fg(t.fg)),
                Span::styled(cmd.id.to_string(), Style::new().fg(t.faint)),
            ]));
        }
        lines.push(Line::raw(""));
    }

    // The overlay is taller than most terminals, so it scrolls. The reducer owns the
    // offset; the renderer is the only thing that knows how far it can usefully go.
    let visible = area.height.saturating_sub(2) as usize;
    app.help_max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.help_scroll.min(app.help_max_scroll);
    app.help_scroll = scroll;

    let title = if app.help_max_scroll == 0 {
        " help — Esc to close ".to_string()
    } else {
        // Say that there is more, and how to get to it. A silently clipped list is how a
        // reader concludes a command does not exist.
        format!(
            " help — j/k to scroll ({}/{}) — Esc to close ",
            scroll + 1,
            app.help_max_scroll + 1
        )
    };

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)).block(
            Block::bordered()
                .title(Span::styled(title, Style::new().fg(t.accent)))
                .border_style(Style::new().fg(t.faint)),
        ),
        area,
    );
}
