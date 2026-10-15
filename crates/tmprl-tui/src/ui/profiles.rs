//! The profile list, one level above the namespaces.

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
    let rows = view.profile_rows();

    if rows.is_empty() {
        let msg = if view.profiles.is_loading() {
            "loading profiles…"
        } else if view.profiles.error().is_some() {
            "could not read temporal.toml (R to retry)"
        } else {
            "no profiles"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), t.faint)),
            area,
        );
        return;
    }

    let height = area.height as usize;
    let area = super::list_scrollbar(frame, area, view.cursor, rows.len(), t);
    let first = view
        .cursor
        .saturating_sub(height.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(height));

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, p)| {
            let focused = i == view.cursor;
            let base = if focused {
                t.fg.patch(t.sel).add_modifier(Modifier::BOLD)
            } else if view.is_selected(i) {
                t.fg.patch(t.sel)
            } else {
                t.fg
            };
            let address = p.address.as_deref().unwrap_or("localhost:7233");
            let namespace = p.namespace.as_deref().unwrap_or("default");
            Line::from(vec![
                Span::styled(
                    super::gutter(i, view.cursor),
                    if focused { t.warn } else { t.faint },
                ),
                Span::styled(format!("{:<20}", truncate(&p.name, 20)), base),
                Span::styled(format!("{:<36}", truncate(address, 36)), t.dim),
                Span::styled(format!("{:<20}", truncate(namespace, 20)), t.faint),
                Span::styled(
                    if p.name == app.profile() {
                        "connected"
                    } else {
                        ""
                    },
                    t.accent,
                ),
            ])
        })
        .map(|l| highlight(l, &app.search, match_style()))
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}
