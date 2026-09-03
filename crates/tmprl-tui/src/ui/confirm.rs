//! The confirmation that stands in front of every destructive action.
//!
//! It shows **the equivalent `temporal` CLI command**, which is the whole design: you read
//! exactly what is about to happen rather than trusting a verb, and if you would rather not
//! trust a TUI with it you can copy the line and run it yourself.

use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use tmprl_core::mutation::Confirm;

use crate::theme::Theme;

pub fn render(frame: &mut Frame, confirm: &Confirm, t: &Theme) {
    let m = &confirm.mutation;
    // Destructive actions are outlined in the error colour. The border is doing real work
    // here: it is the difference between "this changes something" and "this ends it".
    let accent = if m.destroys_history() {
        t.err
    } else if m.is_destructive() {
        t.warn
    } else {
        t.accent
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("  {} {}", m.verb(), m.workflow_id()),
            Style::new().fg(t.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  in {}", m.namespace()),
            Style::new().fg(t.dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  the equivalent command:",
            Style::new().fg(t.faint),
        )),
    ];
    // Wrapped rather than truncated: a command you can only see half of is not one you can
    // check, and checking it is the point.
    for chunk in wrap(&m.cli(), 66) {
        lines.push(Line::from(Span::styled(
            format!("    {chunk}"),
            Style::new().fg(t.ok),
        )));
    }
    lines.push(Line::raw(""));

    if let Some(word) = &confirm.typed_word {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                "this destroys the history itself. ",
                Style::new().fg(t.err).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("type `{word}`:"), Style::new().fg(t.fg)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", Style::new()),
            Span::styled(confirm.entered.clone(), Style::new().fg(t.fg)),
            Span::styled("█", Style::new().fg(accent)),
        ]));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        format!("  {}", confirm.prompt()),
        Style::new().fg(t.faint),
    )));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = super::centered(frame.area(), 74, height);
    if area.height < 3 || area.width < 20 {
        return;
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" confirm — {} ", m.verb().to_lowercase()),
                    Style::new().fg(accent).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(accent)),
        ),
        area,
    );
}

/// Break a long command on spaces so it can be read across several lines.
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}
