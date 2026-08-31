//! Visibility query strings.
//!
//! The raw query is the interface. tmprl never holds a structured filter that it renders
//! down to a string the user cannot see — every filter the interface offers compiles *into*
//! this text, and the text stays editable. A lossy abstraction over the query is the thing
//! that makes the web UI's filter bar frustrating, and it is not worth reproducing.
//!
//! So the only query manipulation here is what the RPCs actually require: adapting a
//! user-authored filter for `CountWorkflowExecutions`, which does not accept `ORDER BY` and
//! needs its own `GROUP BY`.

/// The clause the header counts group on.
const GROUP_BY_STATUS: &str = "GROUP BY ExecutionStatus";

/// Turn a user's list query into the one that produces the header counts.
///
/// `CountWorkflowExecutions` rejects `ORDER BY`, so it is stripped; any `GROUP BY` the user
/// wrote is replaced, because the header renders per-status counts and nothing else.
pub fn count_query(filter: &str) -> String {
    let filter = strip_clause(filter, "group by");
    let filter = strip_clause(&filter, "order by");
    let filter = filter.trim();
    if filter.is_empty() {
        GROUP_BY_STATUS.to_string()
    } else {
        format!("{filter} {GROUP_BY_STATUS}")
    }
}

/// Remove a trailing `<keyword> ...` clause, if the query has one.
///
/// Matching is case-insensitive and skips anything inside single quotes, so a workflow id
/// like `'daily order by region'` is not mistaken for a clause. Only the last occurrence is
/// cut, which is what a trailing clause is.
fn strip_clause(query: &str, keyword: &str) -> String {
    match find_clause(query, keyword) {
        Some(at) => query[..at].trim_end().to_string(),
        None => query.trim_end().to_string(),
    }
}

/// Byte offset of the last unquoted occurrence of `keyword`, which the caller passes in
/// lowercase. Whitespace inside the keyword is matched flexibly so that `ORDER   BY` and
/// `order by` both count.
fn find_clause(query: &str, keyword: &str) -> Option<usize> {
    let words: Vec<&str> = keyword.split_whitespace().collect();
    let bytes = query.as_bytes();
    let mut in_quote = false;
    let mut found = None;
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
        // A clause keyword must start at a word boundary, or `reorder by` would match.
        let at_boundary = i == 0 || !is_word_byte(bytes[i - 1]);
        if at_boundary && let Some(end) = match_words(bytes, i, &words) {
            found = Some(i);
            i = end;
            continue;
        }
        i += 1;
    }
    found
}

/// If `words` match at `start` separated by whitespace, the offset just past them.
fn match_words(bytes: &[u8], start: usize, words: &[&str]) -> Option<usize> {
    let mut i = start;
    for (n, word) in words.iter().enumerate() {
        if n > 0 {
            let ws = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i == ws {
                return None;
            }
        }
        let end = i + word.len();
        if end > bytes.len() || !bytes[i..end].eq_ignore_ascii_case(word.as_bytes()) {
            return None;
        }
        i = end;
    }
    // The keyword must end at a word boundary too.
    if i < bytes.len() && is_word_byte(bytes[i]) {
        return None;
    }
    Some(i)
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_filter_still_groups() {
        assert_eq!(count_query(""), "GROUP BY ExecutionStatus");
        assert_eq!(count_query("   "), "GROUP BY ExecutionStatus");
    }

    #[test]
    fn a_filter_is_kept_and_grouped() {
        assert_eq!(
            count_query("WorkflowType = 'Foo'"),
            "WorkflowType = 'Foo' GROUP BY ExecutionStatus"
        );
    }

    #[test]
    fn order_by_is_stripped_because_count_rejects_it() {
        assert_eq!(
            count_query("WorkflowType = 'Foo' ORDER BY StartTime DESC"),
            "WorkflowType = 'Foo' GROUP BY ExecutionStatus"
        );
        // Case and spacing vary in hand-written queries.
        assert_eq!(
            count_query("WorkflowType = 'Foo' order   by StartTime"),
            "WorkflowType = 'Foo' GROUP BY ExecutionStatus"
        );
    }

    #[test]
    fn an_existing_group_by_is_replaced_not_appended() {
        assert_eq!(
            count_query("WorkflowType = 'Foo' GROUP BY WorkflowType"),
            "WorkflowType = 'Foo' GROUP BY ExecutionStatus"
        );
    }

    #[test]
    fn both_clauses_are_stripped_together() {
        assert_eq!(
            count_query("A = 1 GROUP BY B ORDER BY C"),
            "A = 1 GROUP BY ExecutionStatus"
        );
    }

    #[test]
    fn a_quoted_value_is_not_mistaken_for_a_clause() {
        // This is the bug a naive `find("order by")` would ship: a workflow id that
        // happens to contain the words would truncate the user's filter.
        assert_eq!(
            count_query("WorkflowId = 'daily order by region'"),
            "WorkflowId = 'daily order by region' GROUP BY ExecutionStatus"
        );
    }

    #[test]
    fn a_keyword_inside_an_identifier_is_not_a_clause() {
        assert_eq!(
            count_query("MyOrder By_Field = 1"),
            "MyOrder By_Field = 1 GROUP BY ExecutionStatus"
        );
    }
}
