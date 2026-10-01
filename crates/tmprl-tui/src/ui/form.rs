//! The multi-field overlay, for inputs a single prompt line cannot carry.

use ratatui::Frame;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tmprl_core::form::Form;

use crate::theme::Theme;

/// Width of the label column, so the values line up under each other.
const LABEL: usize = 15;

/// How wide the box is, borders included.
const WIDTH: u16 = 60;

pub fn render(frame: &mut Frame, form: &Form, t: &Theme) {
    // What a value has to itself: the box, less its borders and the label column.
    let room = (WIDTH.min(frame.area().width) as usize).saturating_sub(2 + 2 + LABEL);
    let mut lines = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let focused = i == form.cursor;
        let label = Span::styled(
            format!("  {:<LABEL$}", field.label),
            if focused { t.fg } else { t.dim },
        );
        let mut spans = vec![label];
        if focused {
            spans.extend(super::input::spans(&field.value, t.fg, room));
        } else {
            spans.push(Span::styled(field.value.text(), t.fg));
        }
        // An empty field shows what belongs in it. Faint, so a hint is never mistaken for a
        // value that is already there, and after the cursor on the field being typed in.
        if field.value.is_empty() {
            let gap = if focused { "  " } else { "" };
            spans.push(Span::styled(format!("{gap}{}", field.hint), t.faint));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  ⇥ next field   ⏎ review   Esc cancel",
        t.faint,
    )));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = super::centered(frame.area(), WIDTH, height);
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
                    t.accent.add_modifier(Modifier::BOLD),
                ))
                .border_style(t.accent),
        ),
        area,
    );
}
