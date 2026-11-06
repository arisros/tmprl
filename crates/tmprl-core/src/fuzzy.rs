//! Fuzzy matching, for the pickers and the `:` completion list.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

pub fn match_score(needle: &str, haystack: &str) -> Option<Match> {
    const MATCHED: i32 = 16;
    const BOUNDARY: i32 = 8;
    const CONSECUTIVE: i32 = 8;
    const GAP_START: i32 = -3;
    const GAP_EXTEND: i32 = -1;

    if needle.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    let fold = !needle.chars().any(char::is_uppercase);

    let hay: Vec<(usize, char)> = haystack.char_indices().collect();
    let mut positions = Vec::new();
    let mut score = 0i32;
    let mut at = 0usize;
    let mut previous: Option<usize> = None;

    for (nth, want) in needle.chars().enumerate() {
        let found = hay[at..].iter().position(|(_, c)| same(*c, want, fold))?;
        let index = at + found;
        let (byte, c) = hay[index];

        let starts_word = index == 0 || is_boundary(hay[index - 1].1, c);
        let mut bonus = if starts_word { BOUNDARY } else { 0 };
        if nth == 0 {
            bonus *= 2;
        }
        if previous.is_some_and(|p| index == p + 1) {
            bonus += CONSECUTIVE;
        }

        let gap = if found == 0 {
            0
        } else {
            GAP_START + GAP_EXTEND * (found as i32 - 1)
        };

        score += MATCHED + bonus + gap;
        positions.push(byte);
        previous = Some(index);
        at = index + 1;
    }

    score -= (haystack.len() as i32) / 16;
    Some(Match { score, positions })
}

pub fn matches(needle: &str, haystack: &str) -> bool {
    match_score(needle, haystack).is_some()
}

fn same(a: char, b: char, fold: bool) -> bool {
    if fold {
        a.to_lowercase().eq(b.to_lowercase())
    } else {
        a == b
    }
}

fn is_boundary(before: char, c: char) -> bool {
    matches!(before, '-' | '_' | '.' | '/' | ':' | ' ' | '@')
        || (before.is_lowercase() && c.is_uppercase())
}

pub fn rank<T>(needle: &str, items: &[T], text: impl Fn(&T) -> String) -> Vec<(usize, Match)> {
    let mut hits: Vec<(usize, Match)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| match_score(needle, &text(item)).map(|m| (i, m)))
        .collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.1.score));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(needle: &str, haystack: &str) -> i32 {
        match_score(needle, haystack)
            .unwrap_or_else(|| panic!("{needle:?} should match {haystack:?}"))
            .score
    }

    fn better(needle: &str, winner: &str, loser: &str) {
        let w = score(needle, winner);
        let l = score(needle, loser);
        assert!(
            w > l,
            "{needle:?}: expected {winner:?} ({w}) to beat {loser:?} ({l})"
        );
    }

    #[test]
    fn a_non_subsequence_does_not_match() {
        assert_eq!(match_score("xyz", "order-checkout"), None);
    }

    #[test]
    fn an_empty_needle_matches_everything_neutrally() {
        let m = match_score("", "anything").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.positions.is_empty());
    }

    #[test]
    fn word_starts_beat_letters_in_the_middle() {
        better("oc", "order-checkout", "processor");
    }

    #[test]
    fn a_contiguous_run_beats_a_scattered_one() {
        better("chec", "checkout", "c-h-e-c");
    }

    #[test]
    fn an_earlier_match_beats_a_later_one() {
        better("order", "order-1", "retry-of-order-1");
    }

    #[test]
    fn a_shorter_haystack_wins_when_the_match_is_equal() {
        better("order", "order-1", "order-1-retry-shipping-attempt-2");
    }

    #[test]
    fn camel_case_counts_as_a_word_boundary() {
        better("cc", "ChargeCard", "cucumber");
    }

    #[test]
    fn smartcase_applies_here_too() {
        assert!(matches("charge", "ChargeCard"), "lowercase folds");
        assert!(
            !matches("CHARGE", "ChargeCard"),
            "an uppercase needle is literal"
        );
    }

    #[test]
    fn positions_are_byte_offsets_into_the_haystack() {
        let hay = "order-checkout";
        let m = match_score("oc", hay).unwrap();
        assert_eq!(m.positions.len(), 2);
        for (p, want) in m.positions.iter().zip(['o', 'c']) {
            assert_eq!(hay[*p..].chars().next(), Some(want));
        }
    }

    #[test]
    fn positions_survive_a_multibyte_haystack() {
        let hay = "café-checkout";
        let m = match_score("éc", hay).unwrap();
        for p in &m.positions {
            assert!(hay.is_char_boundary(*p), "offset {p} is not a boundary");
        }
    }

    #[test]
    fn rank_drops_non_matches_and_orders_best_first() {
        let items = ["processor", "order-checkout", "shipping"];
        let hits = rank("oc", &items, |s| s.to_string());
        assert_eq!(hits.len(), 2, "shipping does not match");
        assert_eq!(items[hits[0].0], "order-checkout");
    }

    #[test]
    fn rank_with_an_empty_needle_keeps_the_input_order() {
        let items = ["c", "a", "b"];
        let hits = rank("", &items, |s| s.to_string());
        let got: Vec<&str> = hits.iter().map(|(i, _)| items[*i]).collect();
        assert_eq!(got, vec!["c", "a", "b"]);
    }
}
