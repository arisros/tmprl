//! Painting the search pattern where it shows up.
//!
//! Applied to a row *after* it has been built, rather than woven into each screen's
//! renderer. The four list screens lay out their columns very differently, padding,
//! truncating and colouring per field, and teaching each of them to split a column around a
//! match would be the same fiddly code four times, in the places most likely to be edited
//! for unrelated reasons. Re-splitting the finished spans is one implementation and it
//! cannot fall out of step with the layout, because it runs on whatever the layout produced.
//!
//! Two consequences worth knowing, both deliberate:
//!
//! * A row can match without anything on it lighting up. Labels are wider than the columns,
//!   a run id is searchable but not rendered, so the cursor lands on a row with no visible
//!   highlight. The statusline's match count is what explains that, and a search that could
//!   not find a run id pasted from a log would be the worse trade.
//! * A match straddling a truncated column is highlighted only as far as the column goes.
//!   The rest of the value is not on screen to paint.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tmprl_core::search::Search;

/// Repaint every occurrence of `search` inside an already-built row.
///
/// The highlight is patched over each span's existing style rather than replacing it, so a
/// match inside a failure message stays red and a match on the selected row keeps its
/// selection background. Replacing outright would make a matched row lose the colour that
/// says what it is.
pub(crate) fn highlight<'a>(line: Line<'a>, search: &Search, style: Style) -> Line<'a> {
    if search.is_empty() {
        return line;
    }

    let mut out: Vec<Span<'a>> = Vec::with_capacity(line.spans.len());
    for span in line.spans {
        let hits = search.spans(&span.content);
        if hits.is_empty() {
            out.push(span);
            continue;
        }
        let base = span.style;
        let text = span.content.into_owned();
        let mut at = 0;
        for (a, b) in hits {
            if a > at {
                out.push(Span::styled(text[at..a].to_string(), base));
            }
            out.push(Span::styled(text[a..b].to_string(), base.patch(style)));
            at = b;
        }
        if at < text.len() {
            out.push(Span::styled(text[at..].to_string(), base));
        }
    }

    Line::from(out)
        .style(line.style)
        .alignment(line.alignment.unwrap_or_default())
}

/// How a match is painted.
///
/// `REVERSED` rather than a colour of its own, and that is the whole trick: reversing swaps
/// the cell's foreground and background, so the match stands out against every row without
/// discarding the colour the row already had. Painting matches yellow would make a failed
/// activity's red message turn yellow exactly when you searched for it, which is the moment
/// the red was carrying the most information.
pub(crate) fn match_style() -> Style {
    Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn hl() -> Style {
        Style::new().add_modifier(Modifier::REVERSED)
    }

    /// The concatenated text of a line, which must survive highlighting unchanged: the
    /// highlight repaints, it never rewrites.
    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn an_empty_search_leaves_the_line_alone() {
        let line = Line::from(vec![Span::raw("ChargeCard")]);
        let out = highlight(line, &Search::new(""), hl());
        assert_eq!(text(&out), "ChargeCard");
        assert_eq!(out.spans.len(), 1);
    }

    #[test]
    fn a_match_is_split_out_without_changing_the_text() {
        let line = Line::from(vec![Span::raw("xxChargexx")]);
        let out = highlight(line, &Search::new("charge"), hl());
        assert_eq!(text(&out), "xxChargexx", "text must be identical");
        assert_eq!(out.spans.len(), 3, "before, match, after");
        assert_eq!(out.spans[1].content.as_ref(), "Charge");
        assert!(out.spans[1].style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn the_underlying_style_is_kept_under_the_highlight() {
        // A match inside a failure message must stay red, otherwise highlighting a search
        // hides the thing the colour was telling you.
        let red = Style::new().fg(Color::Red);
        let line = Line::from(vec![Span::styled("timed out", red)]);
        let out = highlight(line, &Search::new("timed"), hl());
        assert_eq!(out.spans[0].style.fg, Some(Color::Red));
        assert!(out.spans[0].style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_match_spanning_two_spans_lights_up_in_each() {
        // Columns are separate spans. "charge" typed against a row whose id and type both
        // contain it should light up twice, not once.
        let line = Line::from(vec![Span::raw("charge-1  "), Span::raw("ChargeCard")]);
        let out = highlight(line, &Search::new("charge"), hl());
        let lit: Vec<&str> = out
            .spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(lit, vec!["charge", "Charge"]);
    }

    #[test]
    fn a_multibyte_row_is_sliced_on_char_boundaries() {
        // The glyph columns are non-ASCII; slicing a row on a byte offset from a lowercased
        // copy would panic here rather than in a test.
        let line = Line::from(vec![Span::raw("✓ activity  Chargé")]);
        let out = highlight(line, &Search::new("chargé"), hl());
        assert_eq!(text(&out), "✓ activity  Chargé");
    }
}
