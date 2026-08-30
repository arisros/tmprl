//! The header and status lines.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Note};
use crate::theme::Theme;

pub fn render_header(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let count = app.rows().len();
    let right = if app.namespaces.is_loading() {
        "loading…".to_string()
    } else {
        format!("{count} namespaces")
    };

    let left = Line::from(vec![
        Span::styled(
            " tmprl ",
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("profile=", Style::new().fg(t.faint)),
        Span::styled(app.profile().to_string(), Style::new().fg(t.fg)),
        Span::styled("  ns=", Style::new().fg(t.faint)),
        Span::styled(app.namespace().to_string(), Style::new().fg(t.fg)),
    ]);

    frame.render_widget(Paragraph::new(left), area);
    let w = right.len() as u16;
    if area.width > w + 2 {
        let r = Rect {
            x: area.x + area.width - w - 1,
            y: area.y,
            width: w,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(right, Style::new().fg(t.dim))),
            r,
        );
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
