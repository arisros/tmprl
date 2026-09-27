//! The clauses the filter picker offers.
//!
//! Every entry here is a *clause*, not a query: accepting one appends it to the bar and
//! leaves the text editable. The query is the interface, so this catalogue only ever writes
//! text you could have typed yourself, and nothing it produces is a form tmprl understands
//! better than you do.
//!
//! What the catalogue knows comes from three places, and the second and third are the point.
//! Statuses are fixed by the protocol. Types and task queues are read off the rows already
//! loaded, so the values offered are ones that exist in this namespace. Search attributes
//! come from the cluster, so a custom attribute is offered with a clause shaped to its type
//! rather than left for you to remember.

use crate::clock::{Clock, rfc3339_utc};
use crate::workflow::WorkflowStatus;

/// One offer in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    /// The text appended to the query bar, and what the picker shows.
    pub text: String,
    /// The group, shown beside the clause. Display only.
    pub note: &'static str,
    /// Words the picker matches on without showing them, so "last hour" finds the timestamp
    /// it compiles to. A literal `2026-09-21T06:00:00Z` is unsearchable otherwise: nobody
    /// knows what the hour was.
    pub keywords: String,
}

impl Clause {
    fn new(text: impl Into<String>, note: &'static str) -> Self {
        Self {
            text: text.into(),
            note,
            keywords: String::new(),
        }
    }

    fn searchable_as(mut self, keywords: impl Into<String>) -> Self {
        self.keywords = keywords.into();
        self
    }
}

/// What a search attribute holds, which is what decides the clause offered for it.
///
/// Mirrors Temporal's `IndexedValueType`. `Unspecified` covers a type this build does not
/// know: the attribute is still offered, because a name you can edit beats no offer at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeType {
    Text,
    Keyword,
    Int,
    Double,
    Bool,
    Datetime,
    KeywordList,
    Unspecified,
}

/// One search attribute the cluster has registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAttribute {
    pub name: String,
    pub kind: AttributeType,
    /// Predefined by Temporal rather than registered by whoever runs this cluster. System
    /// attributes are offered last: the custom ones are why anyone opens this list.
    pub system: bool,
}

/// What the catalogue is built from.
pub struct Facts<'a> {
    /// Workflow types seen in the loaded rows, deduplicated.
    pub types: &'a [&'a str],
    /// Task queues seen in the loaded rows, deduplicated.
    pub queues: &'a [&'a str],
    /// What the cluster registered. Empty until it has been fetched, which is not an error:
    /// the rest of the catalogue stands on its own.
    pub attributes: &'a [SearchAttribute],
    /// Epoch millis, for the relative time clauses.
    pub now_ms: i64,
    /// The viewing zone, which decides when "today" started.
    pub clock: &'a Clock,
}

/// Every clause worth offering, in the order the picker shows them.
pub fn clauses(f: &Facts) -> Vec<Clause> {
    let mut out = Vec::new();
    out.extend(status_clauses());
    out.extend(time_clauses(f.now_ms, f.clock));
    out.extend(value_clauses("WorkflowType", f.types, "type"));
    out.extend(value_clauses("TaskQueue", f.queues, "task queue"));
    out.extend(attribute_clauses(f.attributes, f.now_ms));
    out.extend(order_clauses());
    out
}

/// Status is first because it is what most filters start with.
fn status_clauses() -> Vec<Clause> {
    let mut out: Vec<Clause> = WorkflowStatus::DISPLAY_ORDER
        .iter()
        .map(|s| Clause::new(format!("ExecutionStatus = '{}'", s.query_name()), "status"))
        .collect();

    // The two that are tedious to type and are asked for constantly. `IN` rather than a
    // chain of `OR`s: the grammar takes it and the result still reads in one bar.
    out.push(
        Clause::new(
            "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')",
            "status",
        )
        .searchable_as("problems broken failures anything wrong"),
    );
    out.push(
        Clause::new("ExecutionStatus != 'Completed'", "status")
            .searchable_as("not completed unfinished"),
    );
    out
}

/// Relative windows, compiled to the absolute instant they mean.
///
/// Temporal's visibility grammar has no `now()`, so "the last hour" has to become a literal.
/// It is computed once, when the picker opens: the clause is a fixed point in time, not a
/// window that slides while you read it, which is also what makes it safe to leave in the
/// bar and edit.
fn time_clauses(now_ms: i64, clock: &Clock) -> Vec<Clause> {
    const MINUTE: i64 = 60 * 1_000;
    const HOUR: i64 = 60 * MINUTE;

    let mut out = Vec::new();
    for (label, ago) in [
        ("15 minutes", 15 * MINUTE),
        ("hour", HOUR),
        ("6 hours", 6 * HOUR),
        ("24 hours", 24 * HOUR),
        ("7 days", 7 * 24 * HOUR),
    ] {
        out.push(
            Clause::new(
                format!("StartTime > '{}'", rfc3339_utc(now_ms - ago)),
                "time",
            )
            .searchable_as(format!("last {label} recent since started")),
        );
    }

    if let Some(midnight) = clock.start_of_day(now_ms) {
        out.push(
            Clause::new(format!("StartTime > '{}'", rfc3339_utc(midnight)), "time")
                .searchable_as(format!("today since midnight {}", clock.name())),
        );
    }

    // Closing rather than starting: "what finished overnight" is a different question from
    // "what started overnight", and only one of them is answerable with StartTime.
    out.push(
        Clause::new(
            format!("CloseTime > '{}'", rfc3339_utc(now_ms - 24 * HOUR)),
            "time",
        )
        .searchable_as("closed finished ended last 24 hours"),
    );
    out
}

/// `Field = 'value'` for each value actually seen.
fn value_clauses(field: &str, values: &[&str], note: &'static str) -> Vec<Clause> {
    values
        .iter()
        .map(|v| Clause::new(format!("{field} = '{v}'"), note))
        .collect()
}

/// One clause per registered attribute, shaped to what the attribute holds.
///
/// A `Keyword` takes a quoted literal, an `Int` does not, and a `Datetime` compared against
/// a bare number is rejected by the server. Offering the wrong shape would be worse than
/// offering nothing: it looks right and fails on send.
fn attribute_clauses(attributes: &[SearchAttribute], now_ms: i64) -> Vec<Clause> {
    let mut sorted: Vec<&SearchAttribute> = attributes.iter().collect();
    // Custom first: they are the ones nobody can be expected to remember. Stable within
    // each group so the cluster's own ordering survives.
    sorted.sort_by_key(|a| a.system);

    sorted
        .iter()
        .filter(|a| !is_already_offered(&a.name))
        .map(|a| {
            let note = if a.system { "attribute" } else { "custom" };
            Clause::new(attribute_template(a, now_ms), note)
                .searchable_as(format!("{} search attribute", a.name))
        })
        .collect()
}

/// The clause skeleton for one attribute.
fn attribute_template(a: &SearchAttribute, now_ms: i64) -> String {
    let name = &a.name;
    match a.kind {
        AttributeType::Int | AttributeType::Double => format!("{name} > 0"),
        AttributeType::Bool => format!("{name} = true"),
        AttributeType::Datetime => format!("{name} > '{}'", rfc3339_utc(now_ms - 24 * 3_600_000)),
        // A list is matched with `IN`, and an empty literal is left for you to fill: the
        // values are the cluster's, and guessing one would be inventing data.
        AttributeType::KeywordList => format!("{name} IN ('')"),
        AttributeType::Text | AttributeType::Keyword | AttributeType::Unspecified => {
            format!("{name} = ''")
        }
    }
}

/// Fields the catalogue already offers with real values off the loaded rows. Offering
/// `WorkflowType = ''` beside twenty actual workflow types is noise.
fn is_already_offered(name: &str) -> bool {
    matches!(
        name,
        "ExecutionStatus" | "WorkflowType" | "TaskQueue" | "StartTime" | "CloseTime"
    )
}

/// Trailing clauses. Last, because a query is read left to right and this is the end of it.
fn order_clauses() -> Vec<Clause> {
    ["ORDER BY StartTime DESC", "ORDER BY StartTime ASC"]
        .into_iter()
        .map(|c| Clause::new(c, "order"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-09-21T07:03:22.431Z
    const NOW: i64 = 1_789_974_202_431;

    fn jakarta() -> Clock {
        Clock::named("Asia/Jakarta").unwrap()
    }

    fn facts<'a>(
        types: &'a [&'a str],
        queues: &'a [&'a str],
        attributes: &'a [SearchAttribute],
        clock: &'a Clock,
    ) -> Facts<'a> {
        Facts {
            types,
            queues,
            attributes,
            now_ms: NOW,
            clock,
        }
    }

    fn texts(cs: &[Clause]) -> Vec<&str> {
        cs.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn every_status_is_offered_and_so_is_the_broken_set() {
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &[], &clock));
        let t = texts(&cs);
        assert!(t.contains(&"ExecutionStatus = 'Running'"), "{t:?}");
        assert!(
            t.contains(&"ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"),
            "{t:?}"
        );
    }

    #[test]
    fn a_relative_window_compiles_to_the_instant_it_means() {
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &[], &clock));
        // 07:03:22Z minus an hour, to the second, and quoted the way the grammar wants.
        assert!(
            texts(&cs).contains(&"StartTime > '2026-09-21T06:03:22Z'"),
            "{:?}",
            texts(&cs)
        );
    }

    #[test]
    fn a_time_clause_is_findable_by_the_words_nobody_can_read_off_it() {
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &[], &clock));
        let hour = cs
            .iter()
            .find(|c| c.text == "StartTime > '2026-09-21T06:03:22Z'")
            .expect("the last-hour clause");
        assert!(
            hour.keywords.contains("last hour"),
            "a timestamp is unsearchable without them: {:?}",
            hour.keywords
        );
    }

    #[test]
    fn today_starts_at_midnight_where_the_reader_is_not_at_midnight_utc() {
        // 07:03Z on the 21st is 14:03 in Jakarta, so "today" began at 17:00Z on the 20th.
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &[], &clock));
        assert!(
            texts(&cs).contains(&"StartTime > '2026-09-20T17:00:00Z'"),
            "{:?}",
            texts(&cs)
        );

        let utc = Clock::named("UTC").unwrap();
        let cs = clauses(&facts(&[], &[], &[], &utc));
        assert!(
            texts(&cs).contains(&"StartTime > '2026-09-21T00:00:00Z'"),
            "{:?}",
            texts(&cs)
        );
    }

    #[test]
    fn values_come_from_the_rows_that_are_loaded() {
        let clock = jakarta();
        let cs = clauses(&facts(&["OrderWorkflow"], &["orders"], &[], &clock));
        let t = texts(&cs);
        assert!(t.contains(&"WorkflowType = 'OrderWorkflow'"), "{t:?}");
        assert!(t.contains(&"TaskQueue = 'orders'"), "{t:?}");
    }

    #[test]
    fn an_attribute_is_offered_in_the_shape_its_type_demands() {
        let attrs = [
            SearchAttribute {
                name: "CustomerId".into(),
                kind: AttributeType::Keyword,
                system: false,
            },
            SearchAttribute {
                name: "Amount".into(),
                kind: AttributeType::Int,
                system: false,
            },
            SearchAttribute {
                name: "Approved".into(),
                kind: AttributeType::Bool,
                system: false,
            },
            SearchAttribute {
                name: "Tags".into(),
                kind: AttributeType::KeywordList,
                system: false,
            },
        ];
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &attrs, &clock));
        let t = texts(&cs);
        assert!(t.contains(&"CustomerId = ''"), "{t:?}");
        assert!(t.contains(&"Amount > 0"), "a quoted int is rejected: {t:?}");
        assert!(t.contains(&"Approved = true"), "{t:?}");
        assert!(t.contains(&"Tags IN ('')"), "{t:?}");
    }

    #[test]
    fn a_custom_attribute_is_offered_before_a_system_one() {
        let attrs = [
            SearchAttribute {
                name: "BinaryChecksums".into(),
                kind: AttributeType::KeywordList,
                system: true,
            },
            SearchAttribute {
                name: "CustomerId".into(),
                kind: AttributeType::Keyword,
                system: false,
            },
        ];
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &attrs, &clock));
        let custom = cs.iter().position(|c| c.text.starts_with("CustomerId"));
        let system = cs
            .iter()
            .position(|c| c.text.starts_with("BinaryChecksums"));
        assert!(custom < system, "custom attributes are the point: {cs:?}");
    }

    #[test]
    fn an_attribute_the_catalogue_already_covers_is_not_offered_twice() {
        // The server lists ExecutionStatus and WorkflowType as system attributes. Offering
        // `WorkflowType = ''` beside the types actually loaded is noise.
        let attrs = [
            SearchAttribute {
                name: "WorkflowType".into(),
                kind: AttributeType::Keyword,
                system: true,
            },
            SearchAttribute {
                name: "ExecutionStatus".into(),
                kind: AttributeType::Keyword,
                system: true,
            },
        ];
        let clock = jakarta();
        let cs = clauses(&facts(&["OrderWorkflow"], &[], &attrs, &clock));
        assert!(
            !texts(&cs).contains(&"WorkflowType = ''"),
            "{:?}",
            texts(&cs)
        );
        assert!(
            !texts(&cs).contains(&"ExecutionStatus = ''"),
            "{:?}",
            texts(&cs)
        );
    }

    #[test]
    fn an_unknown_attribute_type_is_still_offered() {
        // A cluster on a newer server than this build is not a reason to hide a filter.
        let attrs = [SearchAttribute {
            name: "Whatever".into(),
            kind: AttributeType::Unspecified,
            system: false,
        }];
        let clock = jakarta();
        let cs = clauses(&facts(&[], &[], &attrs, &clock));
        assert!(texts(&cs).contains(&"Whatever = ''"), "{:?}", texts(&cs));
    }

    #[test]
    fn ordering_comes_last_because_a_query_ends_with_it() {
        let clock = jakarta();
        let cs = clauses(&facts(&["OrderWorkflow"], &["orders"], &[], &clock));
        let last = cs.last().expect("clauses");
        assert_eq!(last.note, "order", "{cs:?}");
    }
}
