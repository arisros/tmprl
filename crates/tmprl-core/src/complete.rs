//! Completing the clause being typed in the query bar.

use crate::filter::Clause;
use crate::fuzzy;

pub fn clause_start(query: &str) -> usize {
    let bytes = query.as_bytes();
    let mut in_quote = false;
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\'' {
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if in_quote {
            i += 1;
            continue;
        }
        for kw in ["AND", "OR"] {
            if at_keyword(query, i, kw) {
                start = i + kw.len();
            }
        }
        i += 1;
    }
    start
}

fn at_keyword(query: &str, i: usize, kw: &str) -> bool {
    let bytes = query.as_bytes();
    let end = i + kw.len();
    if end > bytes.len() {
        return false;
    }
    if !query[i..end].eq_ignore_ascii_case(kw) {
        return false;
    }
    let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
    let after_ok = end == bytes.len() || !is_word_byte(bytes[end]);
    before_ok && after_ok
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn fragment(query: &str) -> &str {
    query[clause_start(query)..].trim_start()
}

pub fn apply(query: &str, clause: &str) -> String {
    let start = clause_start(query);
    let head = &query[..start];
    if head.is_empty() {
        return clause.to_string();
    }
    format!("{} {clause}", head.trim_end())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    items: Vec<Clause>,
    cursor: usize,
}

pub const MAX_OFFERS: usize = 6;

impl Completion {
    pub fn new(query: &str, clauses: Vec<Clause>) -> Option<Self> {
        let tail = &query[clause_start(query)..];
        if tail.ends_with(char::is_whitespace) {
            return None;
        }
        let typed = tail.trim_start();
        if typed.is_empty() {
            return None;
        }
        let ranked = fuzzy::rank(typed, &clauses, |c| {
            if c.keywords.is_empty() {
                c.text.clone()
            } else {
                format!("{} {}", c.text, c.keywords)
            }
        });
        let items: Vec<Clause> = ranked
            .into_iter()
            .take(MAX_OFFERS)
            .map(|(at, _)| clauses[at].clone())
            .collect();
        if items.is_empty() {
            return None;
        }
        Some(Self { items, cursor: 0 })
    }

    pub fn items(&self) -> &[Clause] {
        &self.items
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&Clause> {
        self.items.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).rem_euclid(len) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_query_completes_from_the_start() {
        assert_eq!(clause_start("exec"), 0);
        assert_eq!(fragment("exec"), "exec");
        assert_eq!(
            apply("exec", "ExecutionStatus = 'Running'"),
            "ExecutionStatus = 'Running'"
        );
    }

    #[test]
    fn only_the_clause_being_typed_is_replaced() {
        let q = "WorkflowType = 'Checkout' AND exec";
        assert_eq!(fragment(q), "exec");
        assert_eq!(
            apply(q, "ExecutionStatus = 'Running'"),
            "WorkflowType = 'Checkout' AND ExecutionStatus = 'Running'"
        );
    }

    #[test]
    fn a_partly_typed_clause_is_replaced_whole_not_word_by_word() {
        let q = "ExecutionStatus = 'Run";
        assert_eq!(fragment(q), "ExecutionStatus = 'Run");
        assert_eq!(
            apply(q, "ExecutionStatus = 'Running'"),
            "ExecutionStatus = 'Running'"
        );
    }

    #[test]
    fn a_keyword_inside_quotes_is_not_a_split() {
        let q = "WorkflowId = 'send and forget'";
        assert_eq!(clause_start(q), 0, "the `and` is part of the id");
        assert_eq!(fragment(q), q);
    }

    #[test]
    fn order_by_is_not_mistaken_for_an_or() {
        let q = "ExecutionStatus = 'Running' ORDER BY Start";
        assert_eq!(fragment(q), q, "`ORDER` is not `OR`: {}", fragment(q));
    }

    #[test]
    fn a_word_ending_in_the_keyword_is_not_a_split() {
        for q in ["WorkflowType = 'Errand", "Type = 'brand", "SENSOR"] {
            assert_eq!(clause_start(q), 0, "{q} should not split");
        }
    }

    #[test]
    fn the_last_keyword_wins_when_a_query_has_several() {
        let q = "a = '1' AND b = '2' AND c";
        assert_eq!(fragment(q), "c");
        assert_eq!(apply(q, "C = '3'"), "a = '1' AND b = '2' AND C = '3'");
    }

    #[test]
    fn or_splits_as_readily_as_and() {
        let q = "a = '1' OR b";
        assert_eq!(fragment(q), "b");
        assert_eq!(apply(q, "B = '2'"), "a = '1' OR B = '2'");
    }

    #[test]
    fn lower_case_keywords_split_too() {
        let q = "a = '1' and b";
        assert_eq!(fragment(q), "b");
        assert_eq!(apply(q, "B = '2'"), "a = '1' and B = '2'");
    }

    #[test]
    fn a_query_ending_in_the_keyword_completes_after_it() {
        let q = "a = '1' AND ";
        assert_eq!(fragment(q), "");
        assert_eq!(apply(q, "B = '2'"), "a = '1' AND B = '2'");
    }

    #[test]
    fn an_unbalanced_quote_does_not_run_off_the_end() {
        let q = "WorkflowId = 'half";
        assert_eq!(clause_start(q), 0);
        assert_eq!(fragment(q), q);
    }
    fn catalogue() -> Vec<Clause> {
        use crate::clock::Clock;
        use crate::filter::{Facts, clauses};
        let clock = Clock::named("UTC").unwrap();
        clauses(&Facts {
            types: &["OrderWorkflow"],
            queues: &["orders"],
            attributes: &[],
            now_ms: 1_789_974_202_431,
            clock: &clock,
        })
    }

    #[test]
    fn nothing_is_offered_until_something_is_typed() {
        assert!(Completion::new("", catalogue()).is_none());
        assert!(Completion::new("a = '1' AND ", catalogue()).is_none());
    }

    #[test]
    fn a_finished_clause_is_left_alone() {
        assert!(Completion::new("ExecutionStatus = 'Running' ", catalogue()).is_none());
        assert!(Completion::new("ExecutionStatus = 'Runn", catalogue()).is_some());
    }

    #[test]
    fn what_is_typed_ranks_the_offers() {
        let c = Completion::new("orderw", catalogue()).expect("offers");
        assert_eq!(
            c.selected().map(|c| c.text.as_str()),
            Some("WorkflowType = 'OrderWorkflow'"),
            "got {:?}",
            c.items().iter().map(|c| &c.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_offer_is_reachable_by_words_its_text_does_not_contain() {
        let c = Completion::new("last hour", catalogue()).expect("offers");
        assert!(
            c.selected()
                .is_some_and(|c| c.text.starts_with("StartTime > ")),
            "got {:?}",
            c.selected()
        );
    }

    #[test]
    fn a_fragment_matching_nothing_offers_nothing() {
        assert!(Completion::new("zzzqqq", catalogue()).is_none());
    }

    #[test]
    fn the_list_is_capped_so_it_cannot_bury_the_rows() {
        let c = Completion::new("e", catalogue()).expect("offers");
        assert!(c.items().len() <= MAX_OFFERS, "{}", c.items().len());
    }

    #[test]
    fn the_selection_wraps_rather_than_stopping_dead() {
        let mut c = Completion::new("e", catalogue()).expect("offers");
        let n = c.items().len();
        c.move_cursor(-1);
        assert_eq!(c.cursor(), n - 1, "up from the top goes to the bottom");
        c.move_cursor(1);
        assert_eq!(c.cursor(), 0);
    }
}
