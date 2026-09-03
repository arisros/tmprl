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

use super::truncate;
use crate::app::App;
use crate::theme::Theme;
use crate::view::View;

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let rows = view.namespace_rows();

    if rows.is_empty() {
        let msg = if view.namespaces.is_loading() {
            "loading namespaces…"
        } else if view.namespaces.error().is_some() {
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
        .view
        .cursor
        .saturating_sub(height.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(height));

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, ns)| {
            let focused = i == view.cursor;
            let gutter = super::gutter(i, view.cursor);

            let base = if focused {
                Style::new().fg(t.fg).bg(t.sel).add_modifier(Modifier::BOLD)
            } else if view.is_selected(i) {
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
