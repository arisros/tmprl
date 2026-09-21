//! The picker overlay: Telescope's `ivy` layout.
//!
//! Docked along the bottom rather than floating in the middle, and that is the layout
//! decision worth defending. A centred box covers the list you opened it from, so the
//! moment it appears you lose the context you were reading. Docked, the top half of the
//! screen keeps showing the workflows or the history, and the picker is a drawer over the
//! part you were not using.
//!
//! Split left and right: the candidates, and a preview of the one under the cursor. The
//! preview is what makes a workflow picker usable, an id alone does not tell you whether it
//! is the run you want, and opening it to find out is the thing being avoided.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tmprl_core::picker::Picker;

use crate::theme::Theme;

/// How much of the screen the drawer takes. Enough rows to be worth filtering, not so many
/// that the list behind it stops being context.
const SHARE: u16 = 55;

pub fn render(frame: &mut Frame, picker: &Picker, t: &Theme) {
    let screen = frame.area();
    if screen.height < 6 || screen.width < 20 {
        return;
    }

    let height = (screen.height * SHARE / 100).max(5);
    let area = Rect {
        x: screen.x,
        y: screen.y + screen.height - height,
        width: screen.width,
        height,
    };
    // The drawer is opaque: whatever the pane drew underneath must not show through.
    frame.render_widget(Clear, area);

    let [prompt_area, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    render_prompt(frame, prompt_area, picker, t);

    // A preview needs room to be worth having. On a narrow terminal the list wins, since a
    // preview squeezed into twenty columns is unreadable either way.
    let previewing = body.width >= 80 && picker.selected().is_some_and(|i| !i.preview.is_empty());
    if previewing {
        let [list, preview] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(body);
        render_list(frame, list, picker, t);
        render_preview(frame, preview, picker, t);
    } else {
        render_list(frame, body, picker, t);
    }
}

fn render_prompt(frame: &mut Frame, area: Rect, picker: &Picker, t: &Theme) {
    let counts = format!("{}/{}", picker.shown(), picker.total());
    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", picker.kind.title()),
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("> ", Style::new().fg(t.accent)),
        Span::styled(picker.prompt.clone(), Style::new().fg(t.fg)),
        // A block caret, since the terminal's own is parked in the statusline.
        Span::styled("▏", Style::new().fg(t.accent)),
        Span::styled(format!("  {counts}"), Style::new().fg(t.faint)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_list(frame: &mut Frame, area: Rect, picker: &Picker, t: &Theme) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(t.faint));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if picker.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  no matches", Style::new().fg(t.faint))),
            inner,
        );
        return;
    }

    // Keep the cursor on screen without a scroll offset of its own: the list is rebuilt on
    // every keystroke and the cursor is pinned to the top on each one, so the window only
    // ever needs to follow `<C-n>` downwards.
    let height = inner.height as usize;
    let first = picker.cursor.saturating_sub(height.saturating_sub(1));

    let lines: Vec<Line> = picker
        .rows()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, (item, m))| {
            let selected = i == picker.cursor;
            let base = if selected {
                Style::new().fg(t.fg).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(t.dim)
            };
            let mut spans = vec![Span::styled(
                if selected { "▸ " } else { "  " },
                Style::new().fg(t.accent),
            )];
            spans.extend(marked(&item.label, &m.positions, base, t));
            if item.lookup {
                // Nothing in the prompt is underlined on this row, because what matched is
                // a field the label does not show. Saying where it came from is the
                // difference between "I had missed it" and "the server was asked".
                spans.push(Span::styled("  found by id", Style::new().fg(t.accent)));
            }
            if !item.note.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", item.note),
                    Style::new().fg(t.faint),
                ));
            }
            let line = Line::from(spans);
            if selected {
                line.style(Style::new().bg(t.sel))
            } else {
                line
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Split a label so the characters that actually matched stand out.
///
/// This is what tells you *why* an entry is in the list: with a fuzzy match, a row can be a
/// hit for reasons that are not obvious from looking at it, and underlining the matched
/// characters is the difference between trusting the ranking and fighting it.
fn marked<'a>(label: &'a str, positions: &[usize], base: Style, t: &Theme) -> Vec<Span<'a>> {
    if positions.is_empty() {
        return vec![Span::styled(label, base)];
    }
    let hit = base.fg(t.accent).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut at = 0;
    for p in positions {
        let p = *p;
        // Defensive: a position from a stale match could be past the end after an edit.
        if p < at || p >= label.len() || !label.is_char_boundary(p) {
            continue;
        }
        let end = label[p..]
            .chars()
            .next()
            .map(|c| p + c.len_utf8())
            .unwrap_or(p);
        if p > at {
            spans.push(Span::styled(&label[at..p], base));
        }
        spans.push(Span::styled(&label[p..end], hit));
        at = end;
    }
    if at < label.len() {
        spans.push(Span::styled(&label[at..], base));
    }
    spans
}

fn render_preview(frame: &mut Frame, area: Rect, picker: &Picker, t: &Theme) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::LEFT)
        .border_style(Style::new().fg(t.faint))
        .title(Span::styled(" preview ", Style::new().fg(t.faint)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(item) = picker.selected() else {
        return;
    };
    let lines: Vec<Line> = item
        .preview
        .lines()
        .map(|l| Line::from(Span::styled(format!(" {l}"), Style::new().fg(t.dim))))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn theme() -> Theme {
        Theme::default()
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn marking_preserves_the_label_exactly() {
        let t = theme();
        let spans = marked("order-checkout", &[0, 6], Style::new(), &t);
        assert_eq!(text(&spans), "order-checkout");
    }

    #[test]
    fn marked_characters_are_the_ones_that_matched() {
        let t = theme();
        let spans = marked("order-checkout", &[0, 6], Style::new(), &t);
        let lit: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.fg == Some(t.accent))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(lit, vec!["o", "c"]);
    }

    #[test]
    fn an_unmatched_label_is_one_span() {
        let t = theme();
        let spans = marked("order", &[], Style::new().fg(Color::Red), &t);
        assert_eq!(spans.len(), 1);
        assert_eq!(text(&spans), "order");
    }

    #[test]
    fn a_position_past_the_end_is_ignored_rather_than_panicking() {
        // Positions and label come from the same match today, but a future caller holding a
        // match across an edit should get a wrong highlight, never a crash in a renderer.
        let t = theme();
        let spans = marked("abc", &[99], Style::new(), &t);
        assert_eq!(text(&spans), "abc");
    }

    #[test]
    fn a_multibyte_label_marks_whole_characters() {
        let t = theme();
        let label = "café";
        // Byte 3 is the start of 'é', which is two bytes wide.
        let spans = marked(label, &[3], Style::new(), &t);
        assert_eq!(text(&spans), label);
        let lit: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.fg == Some(t.accent))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(lit, vec!["é"]);
    }
}
