//! The header and status lines.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Note, Screen};
use crate::theme::Theme;

pub fn render_header(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let left = Line::from(vec![
        Span::styled(
            " tmprl ",
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("profile=", Style::new().fg(t.faint)),
        Span::styled(app.profile().to_string(), Style::new().fg(t.fg)),
        Span::styled("  ns=", Style::new().fg(t.faint)),
        Span::styled(scope_label(app), Style::new().fg(t.fg)),
    ]);
    frame.render_widget(Paragraph::new(left), area);

    let right = match app.screen {
        Screen::Namespaces => namespace_summary(app, t),
        Screen::Workflows => workflow_summary(app, t),
    };
    let w: usize = right.iter().map(|s| s.content.chars().count()).sum();
    if area.width as usize > w + 2 {
        let r = Rect {
            x: area.x + area.width - w as u16 - 1,
            y: area.y,
            width: w as u16,
            height: 1,
        };
        frame.render_widget(Paragraph::new(Line::from(right)), r);
    }
}

/// What the list is scoped to. A fan-out over several namespaces is summarised rather than
/// listed, because the header has one line and the rows carry the namespace anyway.
fn scope_label(app: &App) -> String {
    match app.scope.len() {
        0 | 1 => app.namespace().to_string(),
        n => format!("{} +{}", app.scope[0], n - 1),
    }
}

fn namespace_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    let text = if app.namespaces.is_loading() {
        "loading…".to_string()
    } else {
        format!("{} namespaces", app.namespace_rows().len())
    };
    vec![Span::styled(text, Style::new().fg(t.dim))]
}

/// Per-status counts, from one `CountWorkflowExecutions ... GROUP BY ExecutionStatus`.
///
/// Each tally is prefixed with the same glyph the table uses, so the header and the rows
/// are read the same way.
fn workflow_summary<'a>(app: &App, t: &Theme) -> Vec<Span<'a>> {
    if let Some(e) = app.counts.error() {
        return vec![Span::styled(format!("counts: {e}"), Style::new().fg(t.err))];
    }
    let Some(counts) = app.counts.value() else {
        return vec![Span::styled(
            if app.counts.is_loading() {
                "counting…"
            } else {
                ""
            },
            Style::new().fg(t.faint),
        )];
    };

    let mut spans = Vec::new();
    for (status, n) in counts.iter() {
        spans.push(Span::styled(
            format!("{} {n}  ", status.glyph()),
            Style::new().fg(status_color(status, t)),
        ));
    }
    // The total is the server's own, not the sum of the groups: grouped counts are
    // approximate, so summing them would understate the real number.
    spans.push(Span::styled(
        format!("{} total", counts.total),
        Style::new().fg(t.dim),
    ));
    spans
}

fn status_color(s: tmprl_core::WorkflowStatus, t: &Theme) -> ratatui::style::Color {
    use tmprl_core::WorkflowStatus as W;
    match s {
        W::Running => t.accent,
        W::Completed => t.ok,
        W::Failed | W::TimedOut => t.err,
        W::Terminated | W::Canceled => t.warn,
        W::ContinuedAsNew | W::Paused => t.dim,
        W::Unspecified => t.faint,
    }
}

pub fn render_status(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    // The command line takes over the status row while it is open.
    if let Some(buf) = &app.cmdline {
        let line = Line::from(vec![
            Span::styled(":", Style::new().fg(t.accent)),
            Span::styled(buf.clone(), Style::new().fg(t.fg)),
            Span::styled("█", Style::new().fg(t.accent)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mode = app.mode;
    let mut spans = vec![
        Span::styled(
            format!(" {} ", mode.label()),
            Style::new()
                .fg(ratatui::style::Color::Black)
                .bg(t.mode_color(mode))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    match &app.note {
        Some((msg, kind)) => {
            let c = match kind {
                Note::Info => t.ok,
                Note::Warn => t.warn,
                Note::Error => t.err,
            };
            spans.push(Span::styled(msg.clone(), Style::new().fg(c)));
        }
        None => {
            if let Some(sel) = app.selection() {
                let n = sel.1 - sel.0 + 1;
                spans.push(Span::styled(
                    format!("{n} selected"),
                    Style::new().fg(t.warn),
                ));
            } else if let Some(err) = app.namespaces.error() {
                spans.push(Span::styled(err.to_string(), Style::new().fg(t.err)));
            } else {
                spans.push(Span::styled(
                    "? help   : commands".to_string(),
                    Style::new().fg(t.faint),
                ));
            }
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // Pending keys, bottom right, like vim's pending-command indicator.
    let pend = app.pending.display();
    if !pend.is_empty() {
        let w = pend.len() as u16;
        if area.width > w + 2 {
            let r = Rect {
                x: area.x + area.width - w - 1,
                y: area.y,
                width: w,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Span::styled(pend, Style::new().fg(t.warn))),
                r,
            );
        }
    }
}
