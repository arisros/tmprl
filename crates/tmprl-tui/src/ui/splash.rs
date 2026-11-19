//! The whole screen while tmprl connects: the mark, where it connects, and for how long.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::brand;
use crate::theme::Theme;

const BIG: usize = 2;

pub fn render(frame: &mut Frame, place: &str, elapsed_ms: i64, t: &Theme) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let text = vec![
        Line::from(
            [
                brand::wordmark(t).to_vec(),
                vec![Span::styled(
                    format!(" {}", env!("CARGO_PKG_VERSION")),
                    t.faint,
                )],
            ]
            .concat(),
        ),
        Line::from(Span::styled(place.to_string(), t.faint)),
        Line::default(),
        Line::from(vec![
            Span::styled(super::waiting::dots(elapsed_ms), t.accent),
            Span::styled("  connecting", t.fg),
            Span::styled(format!("  {}s", elapsed_ms.max(0) / 1000), t.faint),
        ]),
    ];
    let room = area.height.saturating_sub(text.len() as u16 + 3);
    let mark = [BIG, 1]
        .into_iter()
        .filter(|&scale| {
            (brand::MARK_HEIGHT * scale) as u16 <= room
                && (brand::MARK_WIDTH * scale) as u16 <= area.width
        })
        .find_map(|scale| brand::big_mark(t, scale))
        .unwrap_or_default();

    let gap = u16::from(!mark.is_empty()) * 2;
    let height = mark.len() as u16 + gap + text.len() as u16;
    let top = area.y + area.height.saturating_sub(height) / 2;
    let rows = |y: u16, h: u16| Rect::new(area.x, y.min(area.bottom()), area.width, h);

    let mark_rows = mark.len() as u16;
    frame.render_widget(
        Paragraph::new(mark).alignment(Alignment::Center),
        rows(top, mark_rows),
    );
    let below = (top + mark_rows + gap).min(area.bottom());
    frame.render_widget(
        Paragraph::new(text).alignment(Alignment::Center),
        rows(below, area.bottom() - below),
    );
    if area.height > height + 2 {
        frame.render_widget(
            Paragraph::new(Span::styled("Ctrl-C to quit  ", t.faint)).alignment(Alignment::Right),
            rows(area.bottom() - 1, 1),
        );
    }
}
