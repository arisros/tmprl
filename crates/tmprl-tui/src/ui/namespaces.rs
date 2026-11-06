//! The namespace list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::truncate;
use super::{highlight, match_style};
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
            "could not load namespaces (R to retry)"
        } else {
            "no namespaces"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), t.faint)),
            area,
        );
        return;
    }

    let height = area.height as usize;
    let area = super::list_scrollbar(frame, area, view.cursor, rows.len(), t);
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
                t.fg.patch(t.sel).add_modifier(Modifier::BOLD)
            } else if view.is_selected(i) {
                t.fg.patch(t.sel)
            } else {
                t.fg
            };

            Line::from(vec![
                Span::styled(gutter, if focused { t.warn } else { t.faint }),
                Span::styled(format!("{:<28}", truncate(&ns.name, 28)), base),
                Span::styled(format!("{:<14}", ns.state), t.dim),
                Span::styled(format!("{:>4}d", ns.retention_days), t.faint),
            ])
        })
        .map(|l| highlight(l, &app.search, match_style()))
        .collect();

    view.rows_drawn.set(Some(crate::view::RowsDrawn {
        area: tmprl_ui::Rect::new(area.x, area.y, area.width, lines.len() as u16),
        first,
    }));
    frame.render_widget(Paragraph::new(lines), area);
}
