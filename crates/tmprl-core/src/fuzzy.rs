//! Fuzzy matching, for the pickers and the `:` completion list.
//!
//! Subsequence matching on its own is not enough once a picker is ranking a thousand
//! workflows rather than eighty command ids. `ord` is a subsequence of nearly every string
//! in a list of Temporal ids, so without a score the useful hit is somewhere in the middle
//! of nine hundred equally-valid ones and the picker is worse than scrolling.
//!
//! So this scores. The weights are not tuned to a benchmark, they encode three things people
//! actually do when they type at a picker:
//!
//! * They type the **start of a word**: `oc` should find `order-checkout` before it finds
//!   `prOCess`, and the first character they type is the one they aim hardest with.
//! * They type **runs**, not scattered letters: `chec` reads as one piece, and a haystack
//!   that contains it contiguously beats one that spells it out across four words.
//! * They type the **beginning** of the thing. Earlier matches beat later ones, and a
//!   shorter haystack beats a longer one holding the same match.
//!
//! Case follows the same `smartcase` rule as `/`, for one less thing to remember. See
//! [`crate::search`], which is the *other* kind of finding: that one walks the rows already
//! on screen, this one reorders a list by how well it matches.

/// A scored hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Higher is better. Only comparable between matches on the same needle.
    pub score: i32,
    /// Byte offsets of the matched characters, ascending, for highlighting. Byte rather
    /// than char offsets so a renderer can slice the haystack directly.
    pub positions: Vec<usize>,
}

/// Score `haystack` against `needle`, or `None` if it does not match at all.
///
/// An empty needle matches everything with score 0, which is what makes a picker that has
/// not been typed into yet show the list in its natural order rather than empty.
///
/// The weights follow fzf's shape, which is worth copying rather than re-deriving: every
/// matched character earns a flat amount, position earns a bonus on top, and gaps are
/// charged with the *first* skipped character costing more than the rest. That last part is
/// what makes a solid run beat a scattered one. An earlier attempt here scored a word start
/// almost as highly as a consecutive character, which ranked `c-h-e-c` above `checkout` for
/// `chec`, because every letter in the former sits after a separator.
pub fn match_score(needle: &str, haystack: &str) -> Option<Match> {
    /// Earned by every matched character, so a longer match always beats a shorter one.
    const MATCHED: i32 = 16;
    /// A word start: after a separator, or the capital in camel case.
    const BOUNDARY: i32 = 8;
    /// Directly after the previously matched character.
    const CONSECUTIVE: i32 = 8;
    /// Opening a gap, charged once per run of skipped characters.
    const GAP_START: i32 = -3;
    /// Each further skipped character in the same gap.
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

        // The very first character of the haystack is a word start by definition; after
        // that it takes a separator or a camel-case hump.
        let starts_word = index == 0 || is_boundary(hay[index - 1].1, c);
        let mut bonus = if starts_word { BOUNDARY } else { 0 };
        // The first character of the needle is the one people aim hardest with, so its
        // position bonus counts double. This is what puts `order-checkout` above
        // `retry-of-order` for `order`.
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

    // A shorter haystack holding the same match is the better hit: `order-1` beats
    // `order-1-retry-shipping` for `order`. Deliberately small, it only ever breaks ties.
    score -= (haystack.len() as i32) / 16;
    Some(Match { score, positions })
}

/// Whether this haystack matches at all, without paying for the score.
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

/// Whether `c` begins a word, given the character before it.
fn is_boundary(before: char, c: char) -> bool {
    matches!(before, '-' | '_' | '.' | '/' | ':' | ' ' | '@')
        || (before.is_lowercase() && c.is_uppercase())
}

/// Rank `items` by how well their text matches `needle`, best first.
///
/// `text` projects an item onto the string to match, so a caller can rank workflows by id
/// or commands by title without this module learning what either is. Non-matches are
/// dropped.
///
/// The sort is stable and falls back to the input order, so an empty needle leaves the list
/// exactly as it arrived, newest-first for workflows and registration order for commands.
/// A picker that reshuffled the moment it opened would be unreadable.
pub fn rank<T>(needle: &str, items: &[T], text: impl Fn(&T) -> String) -> Vec<(usize, Match)> {
    let mut hits: Vec<(usize, Match)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| match_score(needle, &text(item)).map(|m| (i, m)))
        .collect();
    // Stable, so equal scores keep their original relative order. `Reverse` rather than
    // swapping the operands, which is the same thing said more obviously.
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
        // What makes a freshly opened picker show the whole list in its natural order.
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
            // Must be sliceable: a byte offset landing mid-character would panic.
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
        // A picker that reshuffled the list the moment it opened, before anything was
        // typed, would throw away the newest-first ordering the list arrived in.
        let items = ["c", "a", "b"];
        let hits = rank("", &items, |s| s.to_string());
        let got: Vec<&str> = hits.iter().map(|(i, _)| items[*i]).collect();
        assert_eq!(got, vec!["c", "a", "b"]);
    }
}
