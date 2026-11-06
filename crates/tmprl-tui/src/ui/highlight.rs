//! Painting the search pattern where it shows up.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tmprl_core::search::Search;

pub(crate) fn highlight<'a>(line: Line<'a>, search: &Search, style: Style) -> Line<'a> {
    const REVERSED: Modifier = Modifier::REVERSED;

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
        let lit = if line.style.patch(base).add_modifier.contains(REVERSED) {
            base.patch(style)
                .remove_modifier(REVERSED)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            base.patch(style)
        };
        let text = span.content.into_owned();
        let mut at = 0;
        for (a, b) in hits {
            if a > at {
                out.push(Span::styled(text[at..a].to_string(), base));
            }
            out.push(Span::styled(text[a..b].to_string(), lit));
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
        let red = Style::new().fg(Color::Red);
        let line = Line::from(vec![Span::styled("timed out", red)]);
        let out = highlight(line, &Search::new("timed"), hl());
        assert_eq!(out.spans[0].style.fg, Some(Color::Red));
        assert!(out.spans[0].style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_match_on_a_reversed_row_is_the_part_that_is_not() {
        let reversed = Style::new().add_modifier(Modifier::REVERSED);
        let on_span = Line::from(vec![Span::styled("order-1001", reversed)]);
        let on_line = Line::from(vec![Span::raw("order-1001")]).style(reversed);

        for line in [on_span, on_line] {
            let out = highlight(line, &Search::new("1001"), match_style());
            assert_eq!(text(&out), "order-1001");
            let lit = out.spans[1].style;
            assert!(lit.sub_modifier.contains(Modifier::REVERSED), "{lit:?}");
            assert!(!lit.add_modifier.contains(Modifier::REVERSED), "{lit:?}");
            assert!(lit.add_modifier.contains(Modifier::UNDERLINED), "{lit:?}");
            assert!(out.spans[0].style.sub_modifier.is_empty());
        }
    }

    #[test]
    fn a_match_spanning_two_spans_lights_up_in_each() {
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
        let line = Line::from(vec![Span::raw("✓ activity  Chargé")]);
        let out = highlight(line, &Search::new("chargé"), hl());
        assert_eq!(text(&out), "✓ activity  Chargé");
    }
}
