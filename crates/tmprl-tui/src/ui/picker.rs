//! The picker overlay: Telescope's `ivy` layout.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tmprl_core::picker::Picker;

use crate::theme::Theme;

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
    frame.render_widget(Clear, area);

    let [prompt_area, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    render_prompt(frame, prompt_area, picker, t);

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
            t.accent.add_modifier(Modifier::BOLD),
        ),
        Span::styled("> ", t.accent),
        Span::styled(picker.prompt.clone(), t.fg),
        Span::styled("▏", t.accent),
        Span::styled(format!("  {counts}"), t.faint),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_list(frame: &mut Frame, area: Rect, picker: &Picker, t: &Theme) {
    let block = Block::default().borders(Borders::TOP).border_style(t.faint);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if picker.is_empty() {
        frame.render_widget(Paragraph::new(Span::styled("  no matches", t.faint)), inner);
        return;
    }

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
                t.fg.add_modifier(Modifier::BOLD)
            } else {
                t.dim
            };
            let mut spans = vec![Span::styled(if selected { "▸ " } else { "  " }, t.accent)];
            spans.extend(marked(&item.label, &m.positions, base, t));
            if item.lookup {
                spans.push(Span::styled("  found by id", t.accent));
            }
            if !item.note.is_empty() {
                spans.push(Span::styled(format!("  {}", item.note), t.faint));
            }
            let line = Line::from(spans);
            if selected { line.style(t.sel) } else { line }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn marked<'a>(label: &'a str, positions: &[usize], base: Style, t: &Theme) -> Vec<Span<'a>> {
    if positions.is_empty() {
        return vec![Span::styled(label, base)];
    }
    let hit = t.hit(base);
    let mut spans = Vec::new();
    let mut at = 0;
    for p in positions {
        let p = *p;
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
        .border_style(t.faint)
        .title(Span::styled(" preview ", t.faint));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(item) = picker.selected() else {
        return;
    };
    let lines: Vec<Line> = item
        .preview
        .lines()
        .map(|l| Line::from(Span::styled(format!(" {l}"), t.dim)))
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
            .filter(|s| s.style.fg == t.accent.fg)
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
        let t = theme();
        let spans = marked("abc", &[99], Style::new(), &t);
        assert_eq!(text(&spans), "abc");
    }

    #[test]
    fn a_multibyte_label_marks_whole_characters() {
        let t = theme();
        let label = "café";
        let spans = marked(label, &[3], Style::new(), &t);
        assert_eq!(text(&spans), label);
        let lit: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.fg == t.accent.fg)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(lit, vec!["é"]);
    }
}
