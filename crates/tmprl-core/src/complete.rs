//! Completing the clause being typed in the query bar.
//!
//! What gets completed is a whole *clause*, not a word. `ExecutionStatus = 'Running'` has
//! two spaces in it, so completing the word under the cursor would leave the field name
//! behind and produce `ExecutionStatus = ExecutionStatus = 'Running'`. The unit that can be
//! replaced by an offer from the catalogue is everything since the last `AND` or `OR`.
//!
//! "Being typed" means left of the cursor. The clause runs from the last keyword before the
//! cursor up to the cursor, that is what ranks the offers, and that is all an offer
//! replaces. Text right of the cursor is never touched: it may be the rest of this clause
//! or the whole of the next one, the grammar cannot say which until an `AND` is typed
//! between them, and a completion that guessed wrong would delete a clause nobody asked it
//! to. A shell completes the word left of the cursor for the same reason.

use crate::filter::Clause;
use crate::fuzzy;

/// Where the clause being typed begins, as a byte offset into `query`.
///
/// After the last `AND` or `OR` outside quotes, else the start of the string. Quoting is
/// respected because a workflow id may well contain the word: `WorkflowId = 'send and
/// forget'` is one clause, not two.
fn clause_start(query: &str) -> usize {
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

/// Whether `kw` sits at `i`, case-insensitively, with a boundary either side.
///
/// The boundary check is what keeps `ORDER BY` from reading as an `OR`, which would leave
/// every completion splicing itself into the middle of the ordering clause.
fn at_keyword(query: &str, i: usize, kw: &str) -> bool {
    let bytes = query.as_bytes();
    let end = i + kw.len();
    if end > bytes.len() {
        return false;
    }
    // Bytes, not a `str` slice: `i` walks every byte, and slicing inside a multi-byte
    // character panics.
    if !bytes[i..end].eq_ignore_ascii_case(kw.as_bytes()) {
        return false;
    }
    let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
    let after_ok = end == bytes.len() || !is_word_byte(bytes[end]);
    before_ok && after_ok
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The clause being typed at the cursor: what an offer would replace, and what the offers
/// are ranked against.
///
/// Trimmed of the leading space that follows an `AND`, so typing `… AND exe` looks for
/// `exe` rather than for ` exe`.
pub fn fragment(query: &str, cursor: usize) -> &str {
    let cursor = query.floor_char_boundary(cursor.min(query.len()));
    query[clause_start(&query[..cursor])..cursor].trim_start()
}

/// The query with the clause being typed replaced by `clause`, and where the cursor belongs
/// afterwards: at the end of what was put in.
///
/// Everything before the clause is kept exactly as typed, including the `AND` and whatever
/// spacing surrounds it, and so is everything after the cursor: this is a completion, not a
/// reformatting.
pub fn apply(query: &str, cursor: usize, clause: &str) -> (String, usize) {
    let cursor = query.floor_char_boundary(cursor.min(query.len()));
    let (typed, rest) = query.split_at(cursor);
    // One space after the keyword, however many were typed.
    let head = typed[..clause_start(typed)].trim_end();

    let mut out = String::with_capacity(query.len() + clause.len() + 2);
    if !head.is_empty() {
        out.push_str(head);
        out.push(' ');
    }
    out.push_str(clause);
    let cursor = out.len();
    // Text typed into the front of something else is kept apart from it, so the quote
    // that ends the offer is not read as opening whatever follows.
    if rest.starts_with(|c: char| !c.is_whitespace()) {
        out.push(' ');
    }
    out.push_str(rest);
    (out, cursor)
}

/// The offers under the query bar, ranked against what is being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    items: Vec<Clause>,
    cursor: usize,
}

/// As many as fit under the bar without burying the list behind it. A completion nobody can
/// see is worse than none: it moves the rows and answers nothing.
pub const MAX_OFFERS: usize = 6;

impl Completion {
    /// Offers for the clause at `cursor`, or `None` when there is nothing worth showing.
    ///
    /// Nothing is offered for an empty fragment. A popup that appears the moment Insert mode
    /// opens, listing every clause in the catalogue, is in the way of the far more common
    /// case: typing a query you already know.
    pub fn new(query: &str, cursor: usize, clauses: Vec<Clause>) -> Option<Self> {
        let typed = fragment(query, cursor);
        // A trailing space says the clause is finished. Offering against a clause someone
        // has already typed out puts the identical text at the top of a list that is now
        // only in the way.
        if typed.ends_with(char::is_whitespace) {
            return None;
        }
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

    /// Wrapping, because six offers are one screenful: running off the end and stopping
    /// there is a dead key, and wrapping is what every completion list does.
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

    // The cursor at the end of the line, which is where typing leaves it.
    fn typed(q: &str) -> &str {
        fragment(q, q.len())
    }

    fn accept(q: &str, clause: &str) -> String {
        apply(q, q.len(), clause).0
    }

    fn offers(q: &str) -> Option<Completion> {
        Completion::new(q, q.len(), catalogue())
    }

    /// Split a query at the `|` marking the cursor.
    fn at(marked: &str) -> (String, usize) {
        let cursor = marked.find('|').expect("mark the cursor with |");
        (marked.replace('|', ""), cursor)
    }

    /// Accept `clause` at the marked cursor, and draw the result with its cursor.
    fn accept_at(marked: &str, clause: &str) -> String {
        let (q, cursor) = at(marked);
        let (mut out, cursor) = apply(&q, cursor, clause);
        out.insert(cursor, '|');
        out
    }

    #[test]
    fn a_fresh_query_completes_from_the_start() {
        assert_eq!(clause_start("exec"), 0);
        assert_eq!(typed("exec"), "exec");
        assert_eq!(
            accept("exec", "ExecutionStatus = 'Running'"),
            "ExecutionStatus = 'Running'"
        );
    }

    #[test]
    fn only_the_clause_being_typed_is_replaced() {
        let q = "WorkflowType = 'Checkout' AND exec";
        assert_eq!(typed(q), "exec");
        assert_eq!(
            accept(q, "ExecutionStatus = 'Running'"),
            "WorkflowType = 'Checkout' AND ExecutionStatus = 'Running'"
        );
    }

    #[test]
    fn a_partly_typed_clause_is_replaced_whole_not_word_by_word() {
        // The bug this module exists to avoid: completing the last word would leave the
        // field name behind and produce `ExecutionStatus = ExecutionStatus = 'Running'`.
        let q = "ExecutionStatus = 'Run";
        assert_eq!(typed(q), "ExecutionStatus = 'Run");
        assert_eq!(
            accept(q, "ExecutionStatus = 'Running'"),
            "ExecutionStatus = 'Running'"
        );
    }

    #[test]
    fn a_keyword_inside_quotes_is_not_a_split() {
        let q = "WorkflowId = 'send and forget'";
        assert_eq!(clause_start(q), 0, "the `and` is part of the id");
        assert_eq!(typed(q), q);
    }

    #[test]
    fn order_by_is_not_mistaken_for_an_or() {
        let q = "ExecutionStatus = 'Running' ORDER BY Start";
        assert_eq!(typed(q), q, "`ORDER` is not `OR`: {}", typed(q));
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
        assert_eq!(typed(q), "c");
        assert_eq!(accept(q, "C = '3'"), "a = '1' AND b = '2' AND C = '3'");
    }

    #[test]
    fn or_splits_as_readily_as_and() {
        let q = "a = '1' OR b";
        assert_eq!(typed(q), "b");
        assert_eq!(accept(q, "B = '2'"), "a = '1' OR B = '2'");
    }

    #[test]
    fn lower_case_keywords_split_too() {
        let q = "a = '1' and b";
        assert_eq!(typed(q), "b");
        assert_eq!(accept(q, "B = '2'"), "a = '1' and B = '2'");
    }

    #[test]
    fn a_query_ending_in_the_keyword_completes_after_it() {
        let q = "a = '1' AND ";
        assert_eq!(typed(q), "");
        assert_eq!(accept(q, "B = '2'"), "a = '1' AND B = '2'");
    }

    #[test]
    fn an_unbalanced_quote_does_not_run_off_the_end() {
        // Half-typed literals are the normal state of a query bar.
        let q = "WorkflowId = 'half";
        assert_eq!(clause_start(q), 0);
        assert_eq!(typed(q), q);
    }

    #[test]
    fn a_keyword_beside_a_multibyte_character_does_not_panic() {
        // Every byte is tried as the start of a keyword, including the ones in the middle
        // of `é`, and a paste is how a query comes to have one.
        let q = "WorkflowId = 'café' AND éxec";
        assert_eq!(typed(q), "éxec");
        assert_eq!(typed("é AND x"), "x");
    }

    #[test]
    fn mid_line_the_clause_left_of_the_cursor_is_the_one_replaced() {
        assert_eq!(
            accept_at(
                "exec| AND WorkflowType = 'Checkout'",
                "ExecutionStatus = 'Running'"
            ),
            "ExecutionStatus = 'Running'| AND WorkflowType = 'Checkout'"
        );
        assert_eq!(
            accept_at(
                "a = '1' AND  exec|  OR c = '3'",
                "ExecutionStatus = 'Running'"
            ),
            "a = '1' AND ExecutionStatus = 'Running'|  OR c = '3'"
        );
    }

    #[test]
    fn mid_line_what_is_right_of_the_cursor_is_never_deleted() {
        // Typed into the front of an existing clause with no `AND` yet. The grammar cannot
        // tell that from one clause, and guessing "one" would delete the second.
        assert_eq!(
            accept_at(
                "exec|WorkflowType = 'Checkout'",
                "ExecutionStatus = 'Running'"
            ),
            "ExecutionStatus = 'Running'| WorkflowType = 'Checkout'"
        );
        assert_eq!(
            accept_at(
                "ExecutionStatus = 'Run|ning' AND b = '2'",
                "ExecutionStatus = 'Running'"
            ),
            "ExecutionStatus = 'Running'| ning' AND b = '2'",
            "the price of never deleting: what followed the cursor is still there"
        );
    }

    #[test]
    fn mid_line_only_what_is_left_of_the_cursor_ranks_the_offers() {
        let (q, cursor) = at("a = '1' AND orderw|zzz AND c = '3'");
        assert_eq!(fragment(&q, cursor), "orderw");
        let c = Completion::new(&q, cursor, catalogue()).expect("offers");
        assert_eq!(
            c.selected().map(|c| c.text.as_str()),
            Some("WorkflowType = 'OrderWorkflow'")
        );
    }

    #[test]
    fn mid_line_an_ordering_after_the_cursor_survives() {
        assert_eq!(
            accept_at(
                "exec| ORDER BY StartTime DESC",
                "ExecutionStatus = 'Running'"
            ),
            "ExecutionStatus = 'Running'| ORDER BY StartTime DESC"
        );
    }

    #[test]
    fn a_half_typed_literal_at_the_cursor_does_not_reach_past_it() {
        // The quote before the cursor is unbalanced, which is the normal state of a literal
        // being typed. Read on through the line, it would make the `AND` after it a string.
        assert_eq!(
            accept_at(
                "a = '1' AND WorkflowType = 'Ord| AND b = '2'",
                "WorkflowType = 'OrderWorkflow'"
            ),
            "a = '1' AND WorkflowType = 'OrderWorkflow'| AND b = '2'"
        );
    }

    #[test]
    fn mid_line_nothing_is_offered_after_a_space_or_at_a_clause_start() {
        for marked in ["a = '1' AND |b = '2'", "a = '1' |AND b = '2'", "|exec"] {
            let (q, cursor) = at(marked);
            assert!(
                Completion::new(&q, cursor, catalogue()).is_none(),
                "{marked}"
            );
        }
    }

    #[test]
    fn a_cursor_inside_a_character_is_read_as_before_it() {
        assert_eq!(fragment("é", 1), "");
        assert_eq!(apply("é", 1, "X"), ("X é".to_string(), 1));
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
        assert!(offers("").is_none());
        assert!(offers("a = '1' AND ").is_none());
    }

    #[test]
    fn a_finished_clause_is_left_alone() {
        // The space says "done with that one". Without this the top offer is the text the
        // reader just finished typing.
        assert!(offers("ExecutionStatus = 'Running' ").is_none());
        assert!(offers("ExecutionStatus = 'Runn").is_some());
    }

    #[test]
    fn what_is_typed_ranks_the_offers() {
        let c = offers("orderw").expect("offers");
        assert_eq!(
            c.selected().map(|c| c.text.as_str()),
            Some("WorkflowType = 'OrderWorkflow'"),
            "got {:?}",
            c.items().iter().map(|c| &c.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_offer_is_reachable_by_words_its_text_does_not_contain() {
        let c = offers("last hour").expect("offers");
        assert!(
            c.selected()
                .is_some_and(|c| c.text.starts_with("StartTime > ")),
            "got {:?}",
            c.selected()
        );
    }

    #[test]
    fn a_fragment_matching_nothing_offers_nothing() {
        assert!(offers("zzzqqq").is_none());
    }

    #[test]
    fn the_list_is_capped_so_it_cannot_bury_the_rows() {
        // `e` matches most of the catalogue.
        let c = offers("e").expect("offers");
        assert!(c.items().len() <= MAX_OFFERS, "{}", c.items().len());
    }

    #[test]
    fn the_selection_wraps_rather_than_stopping_dead() {
        let mut c = offers("e").expect("offers");
        let n = c.items().len();
        c.move_cursor(-1);
        assert_eq!(c.cursor(), n - 1, "up from the top goes to the bottom");
        c.move_cursor(1);
        assert_eq!(c.cursor(), 0);
    }
}
