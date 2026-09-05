//! The schedule list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tmprl_core::schedule::time_until;

use super::truncate;
use crate::app::App;
use crate::theme::Theme;
use crate::view::View;

const GUTTER: usize = 5;
const STATE: usize = 10;
const TYPE: usize = 20;
const SPEC: usize = 26;
const NEXT: usize = 7;

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let rows = view.schedule_rows();

    if rows.is_empty() {
        let msg = if view.schedules.is_loading() {
            "loading schedules…".to_string()
        } else if let Some(e) = view.schedules.error() {
            format!("{e} (R to retry)")
        } else {
            "no schedules in this namespace (gw for workflows)".to_string()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), Style::new().fg(t.faint))),
            area,
        );
        return;
    }

    let now = now_millis();
    let height = area.height as usize;
    let first = view
        .cursor
        .saturating_sub(height.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(height));
    let fixed = GUTTER + STATE + TYPE + SPEC + NEXT;
    let id_width = (area.width as usize).saturating_sub(fixed).max(8);

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, s)| {
            let focused = i == view.cursor;
            let base = if focused {
                Style::new().fg(t.fg).bg(t.sel).add_modifier(Modifier::BOLD)
            } else if view.is_selected(i) {
                Style::new().fg(t.fg).bg(t.sel)
            } else {
                Style::new().fg(t.fg)
            };
            Line::from(vec![
                Span::styled(
                    super::gutter(i, view.cursor),
                    Style::new().fg(if focused { t.warn } else { t.faint }),
                ),
                Span::styled(
                    format!(
                        "{} {:<width$}",
                        s.glyph(),
                        if s.paused { "paused" } else { "running" },
                        width = STATE - 2
                    ),
                    Style::new().fg(if s.paused { t.warn } else { t.accent }),
                ),
                Span::styled(
                    format!("{:<id_width$}", truncate(&s.schedule_id, id_width)),
                    base,
                ),
                Span::styled(
                    format!("{:<TYPE$}", truncate(&s.workflow_type, TYPE - 1)),
                    Style::new().fg(t.dim),
                ),
                Span::styled(
                    format!("{:<SPEC$}", truncate(&s.spec, SPEC - 1)),
                    Style::new().fg(t.ok),
                ),
                Span::styled(
                    match time_until(s.next_run, now) {
                        // A paused schedule still has future times: the server computes them
                        // from the spec, not from whether it will act on them.
                        Some(_) if s.paused => format!("{:>NEXT$}", ""),
                        Some(next) => format!("{next:>NEXT$}"),
                        None => format!("{:>NEXT$}", ""),
                    },
                    Style::new().fg(t.faint),
                ),
            ])
        })
        .collect();

    let _ = app;
    frame.render_widget(Paragraph::new(lines), area);
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
