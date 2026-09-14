//! Searching what is already on screen.
//!
//! Deliberately not the same thing as the visibility query, and the difference is worth
//! stating because the two look similar from the outside. The query is a filter the
//! *server* applies: it decides which workflows exist as far as this pane is concerned, it
//! costs a round trip, and it is written in Temporal's list-filter dialect. A search costs
//! nothing and changes nothing, it moves the cursor to the next row whose text matches, and
//! it works on screens the query cannot reach at all, the history outline most of all,
//! where there is no server-side filter to ask for.
//!
//! Neither substitutes for the other. You narrow to a few hundred workflows with the query
//! and then find the one you want with `/`.
//!
//! Pure: this module never learns what a row *is*, only the text one renders to. Deciding
//! that text is the view's job, which keeps this crate free of screens and keeps the
//! matching unit-testable without building an application.

/// A pattern, with its case sensitivity already decided.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Search {
    /// As typed. Kept verbatim so the statusline can echo the search back, the way vim's
    /// last-search register does.
    pattern: String,
    case_sensitive: bool,
}

/// Where a search landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub row: usize,
    /// The search ran off the end and resumed from the other one. Reported rather than
    /// silent: vim says "search hit BOTTOM, continuing at TOP", and without it a wrap looks
    /// like the cursor jumped at random.
    pub wrapped: bool,
}

impl Search {
    /// Compile a pattern. Empty is legal and matches nothing, which is what makes `n` with
    /// no previous search a no-op rather than a jump to row 0.
    pub fn new(pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        let case_sensitive = smartcase(&pattern);
        Self {
            pattern,
            case_sensitive,
        }
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.is_empty()
    }

    /// Whether this text contains the pattern anywhere.
    pub fn matches(&self, haystack: &str) -> bool {
        if self.pattern.is_empty() {
            return false;
        }
        haystack
            .char_indices()
            .any(|(at, _)| self.match_at(haystack, at).is_some())
    }

    /// Byte ranges of every match, for highlighting.
    ///
    /// Offsets are into `haystack` exactly as given, not into a lowercased copy. Case
    /// folding can change a string's byte length, `İ` lowercases to two chars, so folding
    /// first and reusing the resulting offsets would slice the original in the wrong place
    /// and eventually panic on a char boundary. Scanning char by char costs more and is
    /// always right, and this only ever runs over the rows actually on screen.
    pub fn spans(&self, haystack: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        if self.pattern.is_empty() {
            return out;
        }
        let starts: Vec<usize> = haystack.char_indices().map(|(i, _)| i).collect();
        let mut i = 0;
        while i < starts.len() {
            match self.match_at(haystack, starts[i]) {
                Some(end) => {
                    out.push((starts[i], end));
                    // Resume past the match. Overlapping spans would double-paint the
                    // overlap, which renders as a differently-coloured sliver.
                    while i < starts.len() && starts[i] < end {
                        i += 1;
                    }
                }
                None => i += 1,
            }
        }
        out
    }

    /// If `haystack` from `at` begins with the pattern, the byte offset just past it.
    fn match_at(&self, haystack: &str, at: usize) -> Option<usize> {
        let mut rest = haystack[at..].chars();
        for want in self.pattern.chars() {
            let got = rest.next()?;
            let same = if self.case_sensitive {
                got == want
            } else {
                got.to_lowercase().eq(want.to_lowercase())
            };
            if !same {
                return None;
            }
        }
        Some(haystack.len() - rest.as_str().len())
    }
}

/// Vim's `smartcase`: an all-lowercase pattern ignores case, one with any uppercase in it
/// does not.
///
/// This is the behaviour people have in their fingers, and it is the right default for the
/// data as well: workflow types and activity names are camel case, so typing `charge`
/// should find `ChargeCard`, while typing `ChargeCard` means you know what you are after.
fn smartcase(pattern: &str) -> bool {
    pattern.chars().any(char::is_uppercase)
}

/// The next matching row from `from`, wrapping once.
///
/// `from` is the cursor, and it is **excluded**: `n` on a match moves to the next one
/// rather than sitting still, which is what makes repeated `n` walk the results.
///
/// Wrapping is unconditional, unlike `]f`, which stops at the end. A failure jump is
/// "where did this go wrong", asked once; a search is "show me the next one", asked
/// repeatedly, and a `n` that silently stops at the last match reads as a broken key.
pub fn find(search: &Search, labels: &[String], from: usize, forward: bool) -> Option<Hit> {
    if search.is_empty() || labels.is_empty() {
        return None;
    }
    let n = labels.len();
    // Walk every row once, starting one past the cursor, so the cursor's own row is only
    // considered last, as the wrap case.
    (1..=n).find_map(|step| {
        let row = if forward {
            (from + step) % n
        } else {
            (from + n - (step % n)) % n
        };
        if !search.matches(&labels[row]) {
            return None;
        }
        let wrapped = if forward { row <= from } else { row >= from };
        Some(Hit { row, wrapped })
    })
}

/// Every matching row, for the count in the statusline.
pub fn count(search: &Search, labels: &[String]) -> usize {
    if search.is_empty() {
        return 0;
    }
    labels.iter().filter(|l| search.matches(l)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_lowercase_pattern_ignores_case() {
        let s = Search::new("charge");
        assert!(s.matches("ChargeCard"));
        assert!(s.matches("CHARGE"));
    }

    #[test]
    fn a_pattern_with_uppercase_is_case_sensitive() {
        let s = Search::new("Charge");
        assert!(s.matches("ChargeCard"));
        assert!(!s.matches("chargecard"));
    }

    #[test]
    fn an_empty_pattern_matches_nothing() {
        // Otherwise `n` with no previous search jumps to row 0, which reads as the key
        // being bound to something else entirely.
        let s = Search::new("");
        assert!(!s.matches("anything"));
        assert_eq!(find(&s, &labels(&["a", "b"]), 0, true), None);
    }

    #[test]
    fn spans_point_into_the_original_string() {
        let s = Search::new("ab");
        let hay = "xxabyyab";
        assert_eq!(s.spans(hay), vec![(2, 4), (6, 8)]);
        for (a, b) in s.spans(hay) {
            // The whole point of the byte offsets: they must be sliceable.
            assert_eq!(&hay[a..b], "ab");
        }
    }

    #[test]
    fn spans_survive_a_multibyte_haystack() {
        let s = Search::new("é");
        let hay = "aéb";
        let spans = s.spans(hay);
        assert_eq!(spans.len(), 1);
        let (a, b) = spans[0];
        assert_eq!(&hay[a..b], "é");
    }

    #[test]
    fn overlapping_matches_are_reported_once() {
        // "aa" in "aaa" starts at 0 and at 1; painting both would double-draw byte 1.
        let s = Search::new("aa");
        assert_eq!(s.spans("aaa"), vec![(0, 2)]);
    }

    #[test]
    fn find_skips_the_row_the_cursor_is_on() {
        let rows = labels(&["charge", "ship", "charge"]);
        let s = Search::new("charge");
        assert_eq!(find(&s, &rows, 0, true).unwrap().row, 2);
    }

    #[test]
    fn find_wraps_and_says_so() {
        let rows = labels(&["charge", "ship", "refund"]);
        let s = Search::new("charge");
        let hit = find(&s, &rows, 1, true).unwrap();
        assert_eq!(hit.row, 0);
        assert!(hit.wrapped, "going forward past the last match wraps");
    }

    #[test]
    fn find_backwards_wraps_too() {
        let rows = labels(&["charge", "ship", "refund"]);
        let s = Search::new("refund");
        let hit = find(&s, &rows, 0, false).unwrap();
        assert_eq!(hit.row, 2);
        assert!(hit.wrapped);
    }

    #[test]
    fn a_sole_match_is_found_from_itself_by_wrapping() {
        // `n` on the only match stays put, and reports the wrap rather than "not found",
        // which is what vim does.
        let rows = labels(&["charge", "ship"]);
        let s = Search::new("charge");
        let hit = find(&s, &rows, 0, true).unwrap();
        assert_eq!(hit.row, 0);
        assert!(hit.wrapped);
    }

    #[test]
    fn no_match_is_none_rather_than_a_jump_to_zero() {
        let rows = labels(&["charge", "ship"]);
        assert_eq!(find(&Search::new("nope"), &rows, 0, true), None);
    }

    #[test]
    fn count_reports_every_matching_row() {
        let rows = labels(&["charge", "ship", "Charge"]);
        assert_eq!(count(&Search::new("charge"), &rows), 2);
        assert_eq!(count(&Search::new("Charge"), &rows), 1);
    }
}
