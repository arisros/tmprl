//! Drawing a line of input: its text, and the cursor in it.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use tmprl_core::LineEdit;

/// `line` in at most `width` cells, with the character under the cursor in reverse video.
///
/// Reverse video rather than a colour or the terminal's own cursor. A colour is nothing
/// under `NO_COLOR`, and the terminal's cursor would have to be placed by whichever of five
/// renderers owns the keyboard this frame and hidden by the rest. Past the end of the text
/// the cursor is a reversed space, so an empty input still shows where typing will land.
///
/// A line wider than its room is windowed around the cursor rather than clipped at the
/// right, which would put the cursor off screen exactly when the line is long enough to
/// need one. A third of the room is kept for what follows the cursor, so moving left
/// through a long line shows where it is going.
pub fn spans(line: &LineEdit, style: Style, width: usize) -> Vec<Span<'_>> {
    let (before, after) = line.split();
    let (under, rest) = match after.chars().next() {
        Some(c) => after.split_at(c.len_utf8()),
        None => (" ", ""),
    };

    let room = width.saturating_sub(cells(under));
    let (before, rest) = if cells(before) + cells(rest) <= room {
        (before, rest)
    } else {
        let ahead = cells(rest).min(room / 3);
        let before = fitting_end(before, room - ahead);
        (before, fitting_start(rest, room - cells(before)))
    };

    vec![
        Span::styled(before, style),
        Span::styled(under, style.add_modifier(Modifier::REVERSED)),
        Span::styled(rest, style),
    ]
}

fn cells(s: &str) -> usize {
    Span::raw(s).width()
}

/// The longest prefix of `s` that fits in `room` cells.
fn fitting_start(s: &str, room: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += cells(&s[i..i + c.len_utf8()]);
        if used > room {
            return &s[..i];
        }
    }
    s
}

/// The longest suffix of `s` that fits in `room` cells.
fn fitting_end(s: &str, room: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices().rev() {
        used += cells(&s[i..i + c.len_utf8()]);
        if used > room {
            return &s[i + c.len_utf8()..];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(marked: &str) -> LineEdit {
        let cursor = marked.find('|').expect("mark the cursor with |");
        let mut l = LineEdit::default();
        l.set(marked.replace('|', ""), cursor);
        l
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn reversed<'a>(spans: &'a [Span]) -> Vec<&'a str> {
        spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn the_cursor_is_the_character_it_sits_on() {
        let l = line("ab|cd");
        let s = spans(&l, Style::new(), 20);
        assert_eq!(text(&s), "abcd", "drawing a cursor must not move the text");
        assert_eq!(reversed(&s), ["c"]);
    }

    #[test]
    fn past_the_end_the_cursor_is_a_cell_of_its_own() {
        let l = line("ab|");
        let s = spans(&l, Style::new(), 20);
        assert_eq!(text(&s), "ab ");
        assert_eq!(reversed(&s), [" "]);
        assert_eq!(reversed(&spans(&line("|"), Style::new(), 20)), [" "]);
    }

    #[test]
    fn the_cursor_needs_no_colour_to_show() {
        // The style handed in under `NO_COLOR` is empty. The cursor must not depend on it.
        let l = line("a|b");
        let s = spans(&l, Style::new(), 20);
        let cursor = &s[1];
        assert_eq!(cursor.style.fg, None);
        assert!(cursor.style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_long_line_keeps_the_cursor_on_screen_at_either_end() {
        let long = "0123456789".repeat(5);
        for marked in [format!("{long}|"), format!("|{long}"), {
            let mut m = long.clone();
            m.insert(25, '|');
            m
        }] {
            let l = line(&marked);
            let s = spans(&l, Style::new(), 12);
            assert!(text(&s).chars().count() <= 12, "{marked}: {}", text(&s));
            assert_eq!(reversed(&s).len(), 1, "{marked}");
        }
    }

    #[test]
    fn a_long_line_shows_some_of_what_follows_the_cursor() {
        let mut marked = "0123456789".repeat(5);
        marked.insert(25, '|');
        let l = line(&marked);
        let s = spans(&l, Style::new(), 13);
        // Twelve cells beside the cursor: a third ahead of it, the rest behind.
        assert_eq!(text(&s), "7890123456789");
        assert_eq!(reversed(&s), ["5"]);
    }

    #[test]
    fn a_wide_character_is_never_cut_in_half() {
        // `日` is two cells. With three cells of room behind the cursor only one fits.
        let l = line("日日日|");
        let s = spans(&l, Style::new(), 4);
        assert_eq!(text(&s), "日 ");
    }
}
