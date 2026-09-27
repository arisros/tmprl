//! The completion list under the query bar.
//!
//! Docked directly beneath the bar and over the top of the rows, the way a completion popup
//! sits under the line being typed in an editor. It is only ever on screen while Insert mode
//! owns the bar and something has been typed, so the rows it covers are rows nobody is
//! reading at that moment.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tmprl_core::complete::Completion;

use super::truncate;
use crate::theme::Theme;

/// Room for the border either side, plus a gap between clause and note.
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
    // Never taller than the space under the bar: a list that ran off the pane would be
    // clipped by the frame rather than by anything that knows what it is doing.
    let rows = (items.len() as u16 + 2).min(below.height.max(2));
    let area = Rect {
        x: below.x,
        y: below.y,
        width,
        height: rows,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(t.faint))
        .title(Span::styled(" ⇥ accept ", Style::new().fg(t.accent)));
    let inner = block.inner(area);

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, item)| {
            let selected = i == c.cursor();
            let base = if selected {
                Style::new().fg(t.fg).bg(t.sel).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(t.fg)
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
                Span::styled(item.note.to_string(), Style::new().fg(t.dim)),
            ])
        })
        .collect();

    // The rows underneath would otherwise show through the gaps.
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}
