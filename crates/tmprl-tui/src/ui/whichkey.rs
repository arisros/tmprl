//! The which-key popup.
//!
//! Its contents come straight from the keymap's pending candidates, so it cannot advertise a
//! binding that does not exist or omit one that does.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::App;
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &App, t: &Theme) {
    let area = frame.area();
    if area.height < 4 {
        return;
    }

    let lines: Vec<Line> = app
        .which_key
        .iter()
        .map(|e| {
            let (label, style) = match e.command {
                Some(id) => (
                    app.registry
                        .get(id)
                        .map(|c| c.title)
                        .unwrap_or(id)
                        .to_string(),
                    Style::new().fg(t.fg),
                ),
                // A prefix that opens more bindings, shown the way which-key shows groups.
                None => (
                    format!("+{} more", e.bindings),
                    Style::new().fg(t.dim).add_modifier(Modifier::ITALIC),
                ),
            };
            Line::from(vec![
                Span::styled(
                    format!(" {:>7} ", e.next.to_string()),
                    Style::new().fg(t.warn),
                ),
                Span::styled("→ ", Style::new().fg(t.faint)),
                Span::styled(label, style),
            ])
        })
        .collect();

    let height = (lines.len() as u16 + 2)
        .min(area.height.saturating_sub(1))
        .max(3);
    let width = 40u16.min(area.width);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width),
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Span::styled(
                    format!(" {} ", app.pending.display()),
                    Style::new().fg(t.accent),
                ))
                .border_style(Style::new().fg(t.faint)),
        ),
        popup,
    );
}
