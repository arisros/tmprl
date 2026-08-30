//! The namespace list.
//!
//! The gutter is hybrid relative/absolute, matching `set relativenumber number`: the cursor
//! row shows its own index, every other row shows its distance. That is what makes a count
//! like `7j` something you read off the screen rather than estimate.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let rows = app.rows();

    if rows.is_empty() {
        let msg = if app.namespaces.is_loading() {
            "loading namespaces…"
        } else if app.namespaces.error().is_some() {
            "could not load namespaces — R to retry"
        } else {
            "no namespaces"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), Style::new().fg(t.faint))),
            area,
        );
        return;
    }

    // Scroll so the cursor stays on screen.
    let height = area.height as usize;
    let first = app
        .cursor
        .saturating_sub(height.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(height));

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, ns)| {
            let focused = i == app.cursor;
            let gutter = if focused {
                format!("{:>4} ", i + 1)
            } else {
                format!("{:>4} ", i.abs_diff(app.cursor))
            };

            let base = if focused {
                Style::new().fg(t.fg).bg(t.sel).add_modifier(Modifier::BOLD)
            } else if app.is_selected(i) {
                Style::new().fg(t.fg).bg(t.sel)
            } else {
                Style::new().fg(t.fg)
            };

            Line::from(vec![
                Span::styled(
                    gutter,
                    Style::new().fg(if focused { t.warn } else { t.faint }),
                ),
                Span::styled(format!("{:<28}", truncate(&ns.name, 28)), base),
                Span::styled(format!("{:<14}", ns.state), Style::new().fg(t.dim)),
                Span::styled(
                    format!("{:>4}d", ns.retention_days),
                    Style::new().fg(t.faint),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncation_respects_character_boundaries() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
        // Multi-byte input must not be sliced mid-character.
        assert_eq!(truncate("日本語テスト", 3), "日本…");
    }
}
