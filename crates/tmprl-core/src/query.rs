//! Visibility query strings.

const GROUP_BY_STATUS: &str = "GROUP BY ExecutionStatus";

// CountWorkflowExecutions rejects ORDER BY.
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

pub fn by_id(text: &str) -> Option<String> {
    let text = quotable(text)?;
    Some(format!("WorkflowId = '{text}' OR RunId = '{text}'"))
}

pub fn by_execution(workflow_id: &str, run_id: Option<&str>) -> Option<String> {
    let workflow_id = quotable(workflow_id)?;
    Some(match run_id {
        Some(run) => format!(
            "WorkflowId = '{workflow_id}' AND RunId = '{}'",
            quotable(run)?
        ),
        None => format!("WorkflowId = '{workflow_id}'"),
    })
}

// STARTS_WITH needs advanced visibility, so it is kept out of the exact lookup.
pub fn by_prefix(text: &str) -> Option<String> {
    let text = quotable(text)?;
    Some(format!("WorkflowId STARTS_WITH '{text}'"))
}

pub(crate) fn quotable(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty() && !text.contains('\'')).then_some(text)
}

pub const PROBLEMS: &str = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')";

pub fn orders(query: &str) -> bool {
    find_clause(query, "order by").is_some()
}

pub fn and(filter: &str, clause: &str) -> String {
    let filter = filter.trim();
    if filter.is_empty() {
        clause.to_string()
    } else if find_clause(filter, "or").is_some() {
        format!("({filter}) AND {clause}")
    } else {
        format!("{filter} AND {clause}")
    }
}

fn strip_clause(query: &str, keyword: &str) -> String {
    match find_clause(query, keyword) {
        Some(at) => query[..at].trim_end().to_string(),
        None => query.trim_end().to_string(),
    }
}

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
    fn an_order_by_is_seen_unless_it_is_quoted() {
        assert!(orders("ExecutionStatus = 'Running' order  by StartTime"));
        assert!(!orders("WorkflowId = 'daily order by region'"));
    }

    #[test]
    fn and_parenthesises_a_filter_that_has_an_or() {
        assert_eq!(and("", "A = 'x'"), "A = 'x'");
        assert_eq!(and(" B = 'y' ", "A = 'x'"), "B = 'y' AND A = 'x'");
        assert_eq!(
            and("B = 'y' OR C = 'z'", "A = 'x'"),
            "(B = 'y' OR C = 'z') AND A = 'x'"
        );
        assert_eq!(
            and("B = 'this or that'", "A = 'x'"),
            "B = 'this or that' AND A = 'x'"
        );
    }

    #[test]
    fn an_id_is_looked_up_as_either_kind_of_id() {
        let q = by_id("20e3bca5-b066").expect("plain text is quotable");
        assert!(q.contains("WorkflowId = '20e3bca5-b066'"), "{q}");
        assert!(q.contains("RunId = '20e3bca5-b066'"), "{q}");
    }

    #[test]
    fn an_execution_is_one_run_when_its_run_id_is_known() {
        assert_eq!(
            by_execution("child-1", Some("r-9")).as_deref(),
            Some("WorkflowId = 'child-1' AND RunId = 'r-9'")
        );
        assert_eq!(
            by_execution("child-1", None).as_deref(),
            Some("WorkflowId = 'child-1'")
        );
        assert_eq!(by_execution("o'brien", None), None);
        assert_eq!(by_execution("child-1", Some("r'9")), None);
    }

    #[test]
    fn a_quote_in_the_prompt_builds_no_query() {
        assert_eq!(by_id("o'brien"), None);
        assert_eq!(by_prefix("o'brien"), None);
        assert_eq!(by_id("   "), None);
    }

    #[test]
    fn a_prefix_query_is_kept_apart_from_the_exact_one() {
        let q = by_prefix("order-").expect("quotable");
        assert_eq!(q, "WorkflowId STARTS_WITH 'order-'");
    }

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
