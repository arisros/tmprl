//! The completion list under the query bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tmprl_core::complete::Completion;

use super::truncate;
use crate::theme::Theme;

const CHROME: usize = 6;

pub fn render(frame: &mut Frame, below: Rect, pane: Rect, c: &Completion, t: &Theme) {
    let items = c.items();
    if items.is_empty() || pane.width < 20 {
        return;
    }

    let widest = items
        .iter()
        .map(|i| i.text.chars().count() + i.note.chars().count())
        .max()
        .unwrap_or(0);
    let width = (widest + CHROME).min(pane.width as usize) as u16;
    let rows = (items.len() as u16 + 2).min(below.height.max(2));
    let area = Rect {
        x: below.x,
        y: below.y,
        width,
        height: rows,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.faint)
        .title(Span::styled(" ⇥ accept ", t.accent));
    let inner = block.inner(area);

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, item)| {
            let selected = i == c.cursor();
            let base = if selected {
                t.fg.patch(t.sel).add_modifier(Modifier::BOLD)
            } else {
                t.fg
            };
            let room = inner.width as usize;
            let note_width = item.note.chars().count() + 2;
            let text = truncate(&item.text, room.saturating_sub(note_width).max(1));
            let pad = room
                .saturating_sub(text.chars().count())
                .saturating_sub(item.note.chars().count());
            Line::from(vec![
                Span::styled(text, base),
                Span::styled(" ".repeat(pad), base),
                Span::styled(item.note.to_string(), t.dim),
            ])
        })
        .collect();

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}
