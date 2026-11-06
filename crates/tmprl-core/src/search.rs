//! Searching what is already on screen.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Search {
    pattern: String,
    case_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub row: usize,
    pub wrapped: bool,
}

impl Search {
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

    pub fn for_pane(&self) -> Search {
        match self.pattern.rsplit_once('=') {
            Some((_, value)) if !value.is_empty() => Search {
                pattern: value.to_string(),
                case_sensitive: self.case_sensitive,
            },
            _ => self.clone(),
        }
    }

    pub fn matches(&self, haystack: &str) -> bool {
        if self.pattern.is_empty() {
            return false;
        }
        haystack
            .char_indices()
            .any(|(at, _)| self.match_at(haystack, at).is_some())
    }

    // Case folding can change byte length, so offsets are into the original string.
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
                    while i < starts.len() && starts[i] < end {
                        i += 1;
                    }
                }
                None => i += 1,
            }
        }
        out
    }

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

fn smartcase(pattern: &str) -> bool {
    pattern.chars().any(char::is_uppercase)
}

pub fn find(
    search: &Search,
    labels: &[String],
    from: usize,
    forward: bool,
    inclusive: bool,
) -> Option<Hit> {
    if search.is_empty() || labels.is_empty() {
        return None;
    }
    let n = labels.len();
    let offset = if inclusive { 0 } else { 1 };

    (0..n).find_map(|i| {
        let step = i + offset;
        let (row, wrapped) = if forward {
            ((from + step) % n, from + step >= n)
        } else {
            ((from + n - (step % n)) % n, step > from)
        };
        search.matches(&labels[row]).then_some(Hit { row, wrapped })
    })
}

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
    fn the_pane_is_searched_for_the_value_of_a_path_pattern() {
        assert_eq!(Search::new("customer.id=8812").for_pane().pattern(), "8812");
        assert_eq!(Search::new("8812").for_pane().pattern(), "8812");
        assert_eq!(Search::new("status=").for_pane().pattern(), "status=");
    }

    #[test]
    fn the_pane_pattern_keeps_the_case_the_whole_pattern_chose() {
        let pane = Search::new("Status=approved").for_pane();
        assert!(
            !pane.matches("APPROVED"),
            "an uppercase path made it case sensitive"
        );
        assert!(pane.matches("approved"));
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
        let s = Search::new("");
        assert!(!s.matches("anything"));
        assert_eq!(find(&s, &labels(&["a", "b"]), 0, true, false), None);
    }

    #[test]
    fn spans_point_into_the_original_string() {
        let s = Search::new("ab");
        let hay = "xxabyyab";
        assert_eq!(s.spans(hay), vec![(2, 4), (6, 8)]);
        for (a, b) in s.spans(hay) {
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
        let s = Search::new("aa");
        assert_eq!(s.spans("aaa"), vec![(0, 2)]);
    }

    #[test]
    fn find_skips_the_row_the_cursor_is_on() {
        let rows = labels(&["charge", "ship", "charge"]);
        let s = Search::new("charge");
        assert_eq!(find(&s, &rows, 0, true, false).unwrap().row, 2);
    }

    #[test]
    fn find_wraps_and_says_so() {
        let rows = labels(&["charge", "ship", "refund"]);
        let s = Search::new("charge");
        let hit = find(&s, &rows, 1, true, false).unwrap();
        assert_eq!(hit.row, 0);
        assert!(hit.wrapped, "going forward past the last match wraps");
    }

    #[test]
    fn find_backwards_wraps_too() {
        let rows = labels(&["charge", "ship", "refund"]);
        let s = Search::new("refund");
        let hit = find(&s, &rows, 0, false, false).unwrap();
        assert_eq!(hit.row, 2);
        assert!(hit.wrapped);
    }

    #[test]
    fn a_sole_match_is_found_from_itself_by_wrapping() {
        let rows = labels(&["charge", "ship"]);
        let s = Search::new("charge");
        let hit = find(&s, &rows, 0, true, false).unwrap();
        assert_eq!(hit.row, 0);
        assert!(hit.wrapped);
    }

    #[test]
    fn no_match_is_none_rather_than_a_jump_to_zero() {
        let rows = labels(&["charge", "ship"]);
        assert_eq!(find(&Search::new("nope"), &rows, 0, true, false), None);
    }

    #[test]
    fn an_inclusive_search_from_row_zero_does_not_claim_to_have_wrapped() {
        let rows = labels(&["charge", "ship", "refund"]);
        let hit = find(&Search::new("charge"), &rows, 0, true, true).unwrap();
        assert_eq!(hit.row, 0);
        assert!(!hit.wrapped, "row 0 is where we started, not a wrap");
    }

    #[test]
    fn an_inclusive_search_matches_the_row_it_starts_on() {
        let rows = labels(&["charge", "ship", "charge"]);
        let hit = find(&Search::new("charge"), &rows, 2, true, true).unwrap();
        assert_eq!(hit.row, 2, "the cursor's own row is a candidate");
        assert!(!hit.wrapped);
    }

    #[test]
    fn an_inclusive_search_still_reports_a_real_wrap() {
        let rows = labels(&["charge", "ship", "refund"]);
        let hit = find(&Search::new("charge"), &rows, 1, true, true).unwrap();
        assert_eq!(hit.row, 0);
        assert!(hit.wrapped, "going past the end to reach it is a wrap");
    }

    #[test]
    fn an_exclusive_search_from_row_zero_reports_a_backward_wrap() {
        let rows = labels(&["charge", "ship", "refund"]);
        let hit = find(&Search::new("refund"), &rows, 0, false, false).unwrap();
        assert_eq!(hit.row, 2);
        assert!(hit.wrapped);
    }

    #[test]
    fn count_reports_every_matching_row() {
        let rows = labels(&["charge", "ship", "Charge"]);
        assert_eq!(count(&Search::new("charge"), &rows), 2);
        assert_eq!(count(&Search::new("Charge"), &rows), 1);
    }
}
