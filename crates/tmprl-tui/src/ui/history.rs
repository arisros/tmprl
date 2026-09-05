//! The workflow history: a collapsible outline of groups.
//!
//! One row per group by default, an activity that was scheduled, started and completed is
//! one line, not three. Folding a group open shows the events underneath it, indented.
//!
//! Only the visible rows are ever built. The outline answers "what is row N" without
//! walking the rows before it, so scrolling a hundred-thousand-event history moves an index
//! rather than rebuilding a list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tmprl_core::history::{Category, Outcome};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::workflow::humanize_age_ms;

use super::truncate;
use crate::app::App;
use crate::theme::Theme;
use crate::view::View;

pub fn render(frame: &mut Frame, area: Rect, view: &View, app: &App, t: &Theme) {
    if area.height == 0 {
        return;
    }
    let Some(outline) = view.history.value() else {
        let msg = match view.history.error() {
            Some(e) => format!("{e} (R to retry)"),
            None => "loading history…".to_string(),
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), Style::new().fg(t.faint))),
            area,
        );
        return;
    };

    if outline.is_empty() {
        let msg = if outline.show_plumbing() {
            "no events"
        } else {
            // Everything was workflow tasks, which are hidden by default. Saying so beats
            // an empty pane that looks broken.
            "nothing but workflow tasks (zp to show them)"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!("  {msg}"), Style::new().fg(t.faint))),
            area,
        );
        return;
    }

    let height = area.height as usize;
    let first = app
        .view
        .cursor
        .saturating_sub(height.saturating_sub(1) / 2)
        .min(outline.len().saturating_sub(height));

    // The one call that matters: only these rows are built.
    let lines: Vec<Line> = outline
        .slice(first, height)
        .into_iter()
        .enumerate()
        .map(|(n, row)| render_row(outline, row, first + n, view, t))
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_row<'a>(
    outline: &'a Outline,
    row: Row,
    index: usize,
    view: &View,
    t: &Theme,
) -> Line<'a> {
    let focused = index == view.cursor;
    let base = if focused {
        Style::new().fg(t.fg).bg(t.sel).add_modifier(Modifier::BOLD)
    } else if view.is_selected(index) {
        Style::new().fg(t.fg).bg(t.sel)
    } else {
        Style::new().fg(t.fg)
    };
    let gutter = Span::styled(
        super::gutter(index, view.cursor),
        Style::new().fg(if focused { t.warn } else { t.faint }),
    );

    match row {
        Row::Group { group, expanded } => {
            let Some(g) = outline.group(group) else {
                return Line::from(gutter);
            };
            let mut spans = vec![
                gutter,
                // A fold marker only where there is something to unfold.
                Span::styled(
                    if g.events.len() > 1 {
                        if expanded { "▾ " } else { "▸ " }
                    } else {
                        "  "
                    },
                    Style::new().fg(t.faint),
                ),
                Span::styled(
                    format!("{} ", outcome_glyph(g.outcome)),
                    Style::new().fg(outcome_color(g.outcome, t)),
                ),
                Span::styled(
                    format!("{:<14}", category_label(g.category)),
                    Style::new().fg(t.dim),
                ),
                Span::styled(format!("{:<32}", truncate(&g.subject, 31)), base),
            ];
            // Attempts are only worth the space when something was actually retried.
            if g.attempts > 1 {
                spans.push(Span::styled(
                    format!("×{} ", g.attempts),
                    Style::new().fg(t.warn).add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::styled(
                match g.duration_ms() {
                    Some(d) => format!("{:>6}", humanize_age_ms(d)),
                    None if g.is_open() => format!("{:>6}", "…"),
                    None => format!("{:>6}", ""),
                },
                Style::new().fg(t.faint),
            ));
            if let Some(f) = &g.failure {
                spans.push(Span::styled(
                    format!("  {}", truncate(f, 48)),
                    Style::new().fg(t.err),
                ));
            }
            Line::from(spans)
        }

        Row::Event { event, .. } => {
            let Some(e) = outline.event(event) else {
                return Line::from(gutter);
            };
            let mut spans = vec![
                gutter,
                // Indented under the group it belongs to.
                Span::styled(format!("    {:>5}  ", e.id), Style::new().fg(t.faint)),
                Span::styled(format!("{:<38}", truncate(e.name, 37)), base),
            ];
            let detail = e
                .fields
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ");
            if !detail.is_empty() {
                spans.push(Span::styled(truncate(&detail, 40), Style::new().fg(t.dim)));
            }
            if let Some(f) = &e.failure {
                spans.push(Span::styled(
                    format!("  {}", truncate(f, 40)),
                    Style::new().fg(t.err),
                ));
            }
            Line::from(spans)
        }
    }
}

/// Status as shape, not colour alone, the same rule the workflow table follows.
fn outcome_glyph(o: Outcome) -> char {
    match o {
        Outcome::Pending => '●',
        Outcome::Completed => '✓',
        Outcome::Failed => '✗',
        Outcome::Canceled => '⊘',
        Outcome::TimedOut => '◔',
        Outcome::Terminated => '■',
        Outcome::ContinuedAsNew => '↻',
        Outcome::Rejected => '⊗',
    }
}

fn outcome_color(o: Outcome, t: &Theme) -> ratatui::style::Color {
    match o {
        Outcome::Pending => t.accent,
        Outcome::Completed => t.ok,
        Outcome::Failed | Outcome::TimedOut | Outcome::Rejected => t.err,
        Outcome::Terminated | Outcome::Canceled => t.warn,
        Outcome::ContinuedAsNew => t.dim,
    }
}

fn category_label(c: Category) -> &'static str {
    match c {
        Category::Workflow => "workflow",
        Category::WorkflowTask => "task",
        Category::Activity => "activity",
        Category::Timer => "timer",
        Category::ChildWorkflow => "child",
        Category::ExternalWorkflow => "external",
        Category::Update => "update",
        Category::Nexus => "nexus",
        Category::Marker => "marker",
        Category::SearchAttributes => "search-attr",
    }
}
