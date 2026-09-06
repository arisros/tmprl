//! The multi-field overlay, for inputs a single prompt line cannot carry.

use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tmprl_core::form::Form;

use crate::theme::Theme;

/// Width of the label column, so the values line up under each other.
const LABEL: usize = 15;

pub fn render(frame: &mut Frame, form: &Form, t: &Theme) {
    let mut lines = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let focused = i == form.cursor;
        let label = Span::styled(
            format!("  {:<LABEL$}", field.label),
            Style::new().fg(if focused { t.fg } else { t.dim }),
        );
        // An empty field shows what belongs in it. Faint, so a hint is never mistaken for a
        // value that is already there.
        let value = if field.value.is_empty() && !focused {
            Span::styled(field.hint.to_string(), Style::new().fg(t.faint))
        } else {
            Span::styled(field.value.clone(), Style::new().fg(t.fg))
        };
        let mut spans = vec![label, value];
        if focused {
            spans.push(Span::styled("█", Style::new().fg(t.accent)));
            if field.value.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", field.hint),
                    Style::new().fg(t.faint),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  ⇥ next field   ⏎ review   Esc cancel",
        Style::new().fg(t.faint),
    )));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = super::centered(frame.area(), 60, height);
    if area.height < 3 || area.width < 20 {
        return;
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" {} ", form.title),
                    Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(t.accent)),
        ),
        area,
    );
}
