//! The workflow table.
//!
//! Status is carried by a glyph as well as a colour, so the column still reads on a
//! 16-colour terminal, under `NO_COLOR`, and for a colour-blind operator. The namespace
//! column appears only when the list is fanned out over more than one, because on a single
//! namespace it would be the same value on every row.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tmprl_core::WorkflowStatus;
use tmprl_core::workflow::humanize_age_ms;

use super::truncate;
use crate::app::App;
use crate::theme::Theme;
use crate::view::View;

/// Fixed column widths. The workflow id gets whatever is left, because it is the column
/// people actually read.
const GUTTER: usize = 5;
const STATUS: usize = 15;
const TYPE: usize = 22;
const AGE: usize = 5;
const NAMESPACE: usize = 16;

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let rows = view.workflow_rows();

    if rows.is_empty() {
        let msg = if view.workflows.is_loading() {
            "loading workflows…".to_string()
        } else if let Some(e) = view.workflows.error() {
            format!("{e} (R to retry)")
        } else if view.query.trim().is_empty() {
            "no workflows in this namespace".to_string()
        } else {
            // Distinguish "nothing here" from "your filter excluded everything", which is
            // the far more likely cause and the one you can act on.
            "no workflows match this query (i to edit it)".to_string()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), Style::new().fg(t.faint))),
            area,
        );
        return;
    }

    let show_ns = view.is_fanned_out();
    let fixed = GUTTER + STATUS + TYPE + AGE + if show_ns { NAMESPACE } else { 0 };
    // Never let the id column collapse to nothing on a narrow pane.
    let id_width = (area.width as usize).saturating_sub(fixed).max(8);

    let now = now_millis();
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
        .map(|(i, w)| {
            let focused = i == view.cursor;
            let base = if focused {
                Style::new().fg(t.fg).bg(t.sel).add_modifier(Modifier::BOLD)
            } else if view.is_selected(i) {
                Style::new().fg(t.fg).bg(t.sel)
            } else {
                Style::new().fg(t.fg)
            };

            let mut spans = vec![
                Span::styled(
                    super::gutter(i, view.cursor),
                    Style::new().fg(if focused { t.warn } else { t.faint }),
                ),
                Span::styled(
                    format!(
                        "{} {:<width$}",
                        w.status.glyph(),
                        truncate(w.status.query_name(), STATUS - 2),
                        width = STATUS - 2
                    ),
                    Style::new().fg(status_color(w.status, t)),
                ),
                Span::styled(
                    format!("{:<id_width$}", truncate(&w.workflow_id, id_width)),
                    base,
                ),
                Span::styled(
                    format!(
                        "{:<width$}",
                        truncate(&w.workflow_type, TYPE - 1),
                        width = TYPE
                    ),
                    Style::new().fg(t.dim),
                ),
            ];
            if show_ns {
                spans.push(Span::styled(
                    format!(
                        "{:<width$}",
                        truncate(&w.namespace, NAMESPACE - 1),
                        width = NAMESPACE
                    ),
                    Style::new().fg(t.accent),
                ));
            }
            spans.push(Span::styled(
                match w.start_time {
                    Some(started) => {
                        format!("{:>width$}", humanize_age_ms(now - started), width = AGE)
                    }
                    None => format!("{:>AGE$}", ","),
                },
                Style::new().fg(t.faint),
            ));
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// Colour reinforces the glyph; it never carries information on its own.
fn status_color(s: WorkflowStatus, t: &Theme) -> ratatui::style::Color {
    match s {
        WorkflowStatus::Running => t.accent,
        WorkflowStatus::Completed => t.ok,
        WorkflowStatus::Failed | WorkflowStatus::TimedOut => t.err,
        WorkflowStatus::Terminated | WorkflowStatus::Canceled => t.warn,
        WorkflowStatus::ContinuedAsNew | WorkflowStatus::Paused => t.dim,
        WorkflowStatus::Unspecified => t.faint,
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
