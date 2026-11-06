//! The confirmation that stands in front of every destructive action.

use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use tmprl_core::mutation::Confirm;

use crate::theme::Theme;

const LISTED: usize = 8;

pub fn render(frame: &mut Frame, confirm: &Confirm, t: &Theme) {
    let m = confirm.first();
    let accent = if m.destroys_history() {
        t.err
    } else if m.is_destructive() {
        t.warn
    } else {
        t.accent
    };

    let heading = if confirm.is_batch() {
        format!("  {} {} {}", m.verb(), confirm.len(), m.subject_plural())
    } else {
        format!("  {} {}", m.verb(), m.workflow_id())
    };
    let mut lines = vec![
        Line::from(Span::styled(heading, t.fg.add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(format!("  in {}", m.namespace()), t.dim)),
    ];

    if confirm.is_batch() {
        lines.push(Line::raw(""));
        for target in confirm.mutations.iter().take(LISTED) {
            lines.push(Line::from(Span::styled(
                format!("    {}", target.workflow_id()),
                t.dim,
            )));
        }
        if confirm.len() > LISTED {
            lines.push(Line::from(Span::styled(
                format!("    and {} more", confirm.len() - LISTED),
                t.faint,
            )));
        }
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        if confirm.is_batch() {
            "  the equivalent command, for the first:"
        } else {
            "  the equivalent command:"
        },
        t.faint,
    )));
    for chunk in super::wrap(&m.cli(), 66) {
        lines.push(Line::from(Span::styled(format!("    {chunk}"), t.ok)));
    }
    lines.push(Line::raw(""));

    if let Some(word) = &confirm.typed_word {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                format!("{} ", confirm.caution().unwrap_or_default()),
                t.err.add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("type `{word}`:"), t.fg),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", Style::new()),
            Span::styled(confirm.entered.clone(), t.fg),
            Span::styled("█", accent),
        ]));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        format!("  {}", confirm.prompt()),
        t.faint,
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
                    format!(" confirm, {} ", m.verb().to_lowercase()),
                    accent.add_modifier(Modifier::BOLD),
                ))
                .border_style(accent),
        ),
        area,
    );
}
