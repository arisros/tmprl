//! The workflow domain model.
//!
//! These types live here rather than in `tmprl-client` because logic hangs off them —
//! status ordering, relative ages, merge-sorting a multi-namespace fan-out, re-finding the
//! cursor after a refresh. All of that is computable without a server or a terminal, so it
//! belongs in the crate that needs neither to be tested. `tmprl-client` maps protobuf into
//! these; nothing above it ever sees a generated type.

use std::cmp::Ordering;

/// Execution status, mirroring `temporal.api.enums.v1.WorkflowExecutionStatus`.
///
/// This is a hand-written mirror rather than a re-export so that `tmprl-core` stays free of
/// the generated protos. The conversion in `tmprl-client` matches on the proto enum
/// exhaustively, so a status added by Temporal is a compile error there, not a blank cell
/// here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum WorkflowStatus {
    #[default]
    Unspecified,
    Running,
    Completed,
    Failed,
    Canceled,
    Terminated,
    ContinuedAsNew,
    TimedOut,
    Paused,
}

impl WorkflowStatus {
    /// Every status, in the order the header renders them: the ones an operator is looking
    /// for first. "How many are broken" is the question people open this screen to answer.
    pub const DISPLAY_ORDER: [WorkflowStatus; 9] = [
        WorkflowStatus::Running,
        WorkflowStatus::Failed,
        WorkflowStatus::TimedOut,
        WorkflowStatus::Terminated,
        WorkflowStatus::Canceled,
        WorkflowStatus::Completed,
        WorkflowStatus::ContinuedAsNew,
        WorkflowStatus::Paused,
        WorkflowStatus::Unspecified,
    ];

    /// The name Temporal uses in a visibility query, and in the `GROUP BY` payloads that
    /// `CountWorkflowExecutions` returns. Round-trips with [`WorkflowStatus::parse`].
    pub fn query_name(self) -> &'static str {
        match self {
            WorkflowStatus::Unspecified => "Unspecified",
            WorkflowStatus::Running => "Running",
            WorkflowStatus::Completed => "Completed",
            WorkflowStatus::Failed => "Failed",
            WorkflowStatus::Canceled => "Canceled",
            WorkflowStatus::Terminated => "Terminated",
            WorkflowStatus::ContinuedAsNew => "ContinuedAsNew",
            WorkflowStatus::TimedOut => "TimedOut",
            WorkflowStatus::Paused => "Paused",
        }
    }

    /// A glyph, so status is legible without colour. `NO_COLOR`, a 16-colour terminal and a
    /// colour-blind reader all get the same information as everyone else.
    pub fn glyph(self) -> char {
        match self {
            WorkflowStatus::Unspecified => '?',
            WorkflowStatus::Running => '●',
            WorkflowStatus::Completed => '✓',
            WorkflowStatus::Failed => '✗',
            WorkflowStatus::Canceled => '⊘',
            WorkflowStatus::Terminated => '■',
            WorkflowStatus::ContinuedAsNew => '↻',
            WorkflowStatus::TimedOut => '◔',
            WorkflowStatus::Paused => '‖',
        }
    }

    /// Parse the name Temporal returns. Accepts the query spelling (`ContinuedAsNew`) and
    /// the proto spelling (`WORKFLOW_EXECUTION_STATUS_CONTINUED_AS_NEW`), because the two
    /// arrive from different RPCs and it is not worth making callers care which.
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim().trim_matches('"');
        let squashed: String = t
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let squashed = squashed
            .strip_prefix("workflowexecutionstatus")
            .unwrap_or(&squashed);
        WorkflowStatus::DISPLAY_ORDER
            .iter()
            .copied()
            .find(|s| {
                let name: String = s
                    .query_name()
                    .chars()
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                name == squashed
            })
            .filter(|_| !squashed.is_empty())
    }

    /// Whether the execution is still open. Closed workflows never change again, which is
    /// what lets the list cache them across a refresh.
    pub fn is_running(self) -> bool {
        matches!(self, WorkflowStatus::Running | WorkflowStatus::Paused)
    }
}

/// One row of the workflow table.
///
/// `namespace` is carried on the row rather than held once for the whole list because a
/// multi-namespace fan-out merges rows from several namespaces into one table, and a row
/// that cannot say where it came from is not actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRow {
    pub namespace: String,
    pub workflow_id: String,
    pub run_id: String,
    pub workflow_type: String,
    pub task_queue: String,
    pub status: WorkflowStatus,
    /// Epoch milliseconds. `None` when the server did not set it, which is rare but legal.
    pub start_time: Option<i64>,
    pub close_time: Option<i64>,
    pub history_length: i64,
}

impl WorkflowRow {
    /// Identity of a row across refreshes and across namespaces.
    ///
    /// A run id is unique within a namespace but not across a fan-out, so the key is the
    /// pair. This is what the cursor is anchored to — see [`find_by_key`].
    pub fn key(&self) -> (&str, &str) {
        (self.namespace.as_str(), self.run_id.as_str())
    }
}

/// Newest first, which is the order the workflow list is read in.
///
/// Ties break on the row key so the order is total: a merge of several namespaces that
/// started workflows in the same millisecond must not shuffle between refreshes.
pub fn by_start_time_desc(a: &WorkflowRow, b: &WorkflowRow) -> Ordering {
    b.start_time
        .cmp(&a.start_time)
        .then_with(|| a.namespace.cmp(&b.namespace))
        .then_with(|| a.run_id.cmp(&b.run_id))
}

/// Merge per-namespace pages into one table, newest first.
///
/// This sorts rather than merges pre-sorted runs, because there is nothing to merge: the
/// server does not order `ListWorkflowExecutions`, and standard visibility rejects an
/// `ORDER BY` clause. Ordering is entirely tmprl's job — see [`WorkflowList`].
pub fn merge_by_start_time(pages: Vec<Vec<WorkflowRow>>) -> Vec<WorkflowRow> {
    let mut all: Vec<WorkflowRow> = pages.into_iter().flatten().collect();
    all.sort_by(by_start_time_desc);
    all
}

/// Where a row with this key ended up after a refresh.
///
/// The workflow list is live: rows appear above the cursor while you are reading, so a
/// cursor stored as a row index silently points at a different workflow a second later.
/// Anchoring to the key and re-finding it is the whole fix.
pub fn find_by_key(rows: &[WorkflowRow], key: (&str, &str)) -> Option<usize> {
    rows.iter().position(|r| r.key() == key)
}

/// Counts per status for the list header, from `CountWorkflowExecutions ... GROUP BY`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusCounts {
    /// The server's total. Grouped counts are approximate and may sum to less than this,
    /// which is documented Temporal behaviour, so the total is kept separately rather than
    /// derived.
    pub total: i64,
    counts: Vec<(WorkflowStatus, i64)>,
}

impl StatusCounts {
    pub fn new(total: i64, counts: impl IntoIterator<Item = (WorkflowStatus, i64)>) -> Self {
        let mut counts: Vec<(WorkflowStatus, i64)> = counts.into_iter().collect();
        counts.sort_by_key(|(s, _)| {
            WorkflowStatus::DISPLAY_ORDER
                .iter()
                .position(|d| d == s)
                .unwrap_or(usize::MAX)
        });
        Self { total, counts }
    }

    /// Non-zero counts, in display order.
    pub fn iter(&self) -> impl Iterator<Item = (WorkflowStatus, i64)> + '_ {
        self.counts.iter().copied().filter(|(_, n)| *n > 0)
    }

    pub fn get(&self, status: WorkflowStatus) -> i64 {
        self.counts
            .iter()
            .find(|(s, _)| *s == status)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }
}

/// A short, fixed-width age like `4s`, `12m`, `3h`, `9d`.
///
/// The workflow list is a dense table on a terminal that may be 80 columns wide, so this
/// trades precision for a column that never wraps. Exact timestamps belong in the detail
/// view, where there is room for them.
pub fn humanize_age_ms(millis: i64) -> String {
    if millis < 0 {
        // Clock skew between the server and this machine. Better to show `0s` than a
        // negative age that looks like a bug in the table.
        return "0s".into();
    }
    let secs = millis / 1000;
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// The workflow table as it accumulates pages.
///
/// Infinite scroll appends pages; a refresh or a new query replaces them. Two properties
/// have to hold no matter which happened:
///
/// * **Sorted newest-first.** The server does not order `ListWorkflowExecutions`, and the
///   dev server's standard visibility store rejects `ORDER BY` outright, so the ordering is
///   this type's job rather than the query's.
/// * **No duplicates.** Pages are snapshots of a set that is changing underneath them, so
///   the same execution can legitimately arrive on two pages. A table that shows a workflow
///   twice makes the operator doubt the whole screen.
#[derive(Debug, Clone, Default)]
pub struct WorkflowList {
    rows: Vec<WorkflowRow>,
    tokens: Vec<(String, Vec<u8>)>,
}

impl WorkflowList {
    pub fn rows(&self) -> &[WorkflowRow] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Per-namespace continuation tokens to send with the next page request.
    pub fn tokens(&self) -> &[(String, Vec<u8>)] {
        &self.tokens
    }

    /// Whether any namespace still has pages left.
    pub fn has_more(&self) -> bool {
        !self.tokens.is_empty()
    }

    /// Start over: a new query, or a refresh of the current one.
    pub fn reset(&mut self, rows: Vec<WorkflowRow>, tokens: Vec<(String, Vec<u8>)>) {
        self.rows.clear();
        self.tokens = tokens;
        self.insert_sorted(rows);
    }

    /// Add the next page.
    pub fn append(&mut self, rows: Vec<WorkflowRow>, tokens: Vec<(String, Vec<u8>)>) {
        self.tokens = tokens;
        self.insert_sorted(rows);
    }

    /// Where the row with this key sits now, if it is still listed.
    pub fn position_of(&self, key: (&str, &str)) -> Option<usize> {
        find_by_key(&self.rows, key)
    }

    fn insert_sorted(&mut self, rows: Vec<WorkflowRow>) {
        self.rows.extend(rows);
        self.rows.sort_by(by_start_time_desc);
        // `by_start_time_desc` breaks ties on the key, so duplicates are adjacent.
        self.rows.dedup_by(|a, b| a.key() == b.key());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ns: &str, run: &str, start: Option<i64>) -> WorkflowRow {
        WorkflowRow {
            namespace: ns.into(),
            workflow_id: format!("wf-{run}"),
            run_id: run.into(),
            workflow_type: "T".into(),
            task_queue: "q".into(),
            status: WorkflowStatus::Running,
            start_time: start,
            close_time: None,
            history_length: 3,
        }
    }

    #[test]
    fn status_names_round_trip() {
        for s in WorkflowStatus::DISPLAY_ORDER {
            assert_eq!(WorkflowStatus::parse(s.query_name()), Some(s));
        }
    }

    #[test]
    fn status_parses_both_spellings_temporal_uses() {
        // `GROUP BY` payloads arrive quoted, and the proto spelling turns up in errors.
        assert_eq!(
            WorkflowStatus::parse("\"ContinuedAsNew\""),
            Some(WorkflowStatus::ContinuedAsNew)
        );
        assert_eq!(
            WorkflowStatus::parse("WORKFLOW_EXECUTION_STATUS_CONTINUED_AS_NEW"),
            Some(WorkflowStatus::ContinuedAsNew)
        );
        assert_eq!(
            WorkflowStatus::parse("  running  "),
            Some(WorkflowStatus::Running)
        );
        assert_eq!(WorkflowStatus::parse("nonsense"), None);
        assert_eq!(WorkflowStatus::parse(""), None);
    }

    #[test]
    fn every_status_has_a_distinct_glyph() {
        // Two statuses sharing a glyph would make the column ambiguous for exactly the
        // readers the glyph column exists for.
        let mut g: Vec<char> = WorkflowStatus::DISPLAY_ORDER
            .iter()
            .map(|s| s.glyph())
            .collect();
        g.sort_unstable();
        let before = g.len();
        g.dedup();
        assert_eq!(before, g.len(), "duplicate status glyph");
    }

    #[test]
    fn display_order_covers_every_status() {
        // A status missing here would be silently dropped from the header counts.
        assert_eq!(
            WorkflowStatus::DISPLAY_ORDER.len(),
            9,
            "DISPLAY_ORDER must list every WorkflowStatus variant"
        );
        let mut seen = WorkflowStatus::DISPLAY_ORDER.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 9, "DISPLAY_ORDER repeats a status");
    }

    #[test]
    fn merge_orders_newest_first_across_namespaces() {
        let merged = merge_by_start_time(vec![
            vec![row("a", "a2", Some(200)), row("a", "a1", Some(100))],
            vec![row("b", "b3", Some(300)), row("b", "b0", Some(50))],
        ]);
        let ids: Vec<&str> = merged.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(ids, ["b3", "a2", "a1", "b0"]);
    }

    #[test]
    fn merge_is_stable_when_start_times_collide() {
        // Same millisecond, different namespaces: the order must not depend on which page
        // happened to arrive first, or rows shuffle under the cursor on every refresh.
        let one = merge_by_start_time(vec![
            vec![row("b", "x", Some(100))],
            vec![row("a", "y", Some(100))],
        ]);
        let two = merge_by_start_time(vec![
            vec![row("a", "y", Some(100))],
            vec![row("b", "x", Some(100))],
        ]);
        assert_eq!(one, two);
    }

    #[test]
    fn rows_without_a_start_time_sort_last() {
        let merged = merge_by_start_time(vec![
            [row("a", "none", None), row("a", "has", Some(10))].into(),
        ]);
        assert_eq!(merged[0].run_id, "has");
    }

    #[test]
    fn the_cursor_follows_its_run_id_when_rows_shift() {
        let before = [row("a", "r1", Some(100)), row("a", "r2", Some(90))];
        let key = before[0].key();
        let key = (key.0.to_string(), key.1.to_string());

        // A newer workflow arrives at the top, pushing everything down one row.
        let after = vec![
            row("a", "r0", Some(110)),
            row("a", "r1", Some(100)),
            row("a", "r2", Some(90)),
        ];
        assert_eq!(
            find_by_key(&after, (&key.0, &key.1)),
            Some(1),
            "the cursor must follow the run id, not stay on index 0"
        );
    }

    #[test]
    fn a_vanished_row_reports_no_position() {
        let rows = vec![row("a", "r1", Some(100))];
        assert_eq!(find_by_key(&rows, ("a", "gone")), None);
        // A run id from another namespace must not match.
        assert_eq!(find_by_key(&rows, ("other", "r1")), None);
    }

    #[test]
    fn counts_render_in_display_order_and_skip_zeroes() {
        let c = StatusCounts::new(
            10,
            [
                (WorkflowStatus::Completed, 6),
                (WorkflowStatus::Running, 3),
                (WorkflowStatus::Failed, 1),
                (WorkflowStatus::Canceled, 0),
            ],
        );
        let got: Vec<_> = c.iter().map(|(s, n)| (s.query_name(), n)).collect();
        assert_eq!(got, [("Running", 3), ("Failed", 1), ("Completed", 6)]);
        assert_eq!(c.total, 10);
        assert_eq!(c.get(WorkflowStatus::Failed), 1);
        assert_eq!(c.get(WorkflowStatus::TimedOut), 0);
    }

    #[test]
    fn ages_are_short_enough_for_a_narrow_column() {
        assert_eq!(humanize_age_ms(4_000), "4s");
        assert_eq!(humanize_age_ms(59_999), "59s");
        assert_eq!(humanize_age_ms(60_000), "1m");
        assert_eq!(humanize_age_ms(3_600_000), "1h");
        assert_eq!(humanize_age_ms(86_400_000), "1d");
        // Server clock ahead of ours must not render as a negative age.
        assert_eq!(humanize_age_ms(-5_000), "0s");
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;

    fn row(ns: &str, run: &str, start: i64) -> WorkflowRow {
        WorkflowRow {
            namespace: ns.into(),
            workflow_id: format!("wf-{run}"),
            run_id: run.into(),
            workflow_type: "T".into(),
            task_queue: "q".into(),
            status: WorkflowStatus::Running,
            start_time: Some(start),
            close_time: None,
            history_length: 1,
        }
    }

    fn ids(list: &WorkflowList) -> Vec<&str> {
        list.rows().iter().map(|r| r.run_id.as_str()).collect()
    }

    #[test]
    fn an_empty_list_has_nothing_more_to_fetch() {
        let list = WorkflowList::default();
        assert!(list.is_empty() && !list.has_more() && list.rows().is_empty());
    }

    #[test]
    fn appended_pages_stay_sorted_newest_first() {
        // The server returns pages in no particular order, so a later page routinely
        // contains rows that belong above rows already on screen.
        let mut list = WorkflowList::default();
        list.reset(vec![row("a", "r2", 200)], vec![("a".into(), vec![1])]);
        list.append(vec![row("a", "r3", 300), row("a", "r1", 100)], vec![]);

        assert_eq!(ids(&list), ["r3", "r2", "r1"]);
        assert!(!list.has_more(), "an empty token list ends the scroll");
    }

    #[test]
    fn a_row_arriving_on_two_pages_is_listed_once() {
        // Pages are snapshots of a set that changes underneath them, so overlap is normal.
        let mut list = WorkflowList::default();
        list.reset(vec![row("a", "r1", 100)], vec![("a".into(), vec![1])]);
        list.append(vec![row("a", "r1", 100), row("a", "r0", 50)], vec![]);
        assert_eq!(ids(&list), ["r1", "r0"]);
    }

    #[test]
    fn the_same_run_id_in_two_namespaces_is_two_rows() {
        // Run ids are unique per namespace, not globally: deduplicating on the run id
        // alone would silently hide a row in a fan-out.
        let mut list = WorkflowList::default();
        list.reset(
            vec![row("a", "shared", 100), row("b", "shared", 90)],
            vec![],
        );
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn reset_drops_the_previous_query_s_rows() {
        let mut list = WorkflowList::default();
        list.reset(vec![row("a", "old", 100)], vec![("a".into(), vec![1])]);
        list.reset(vec![row("a", "new", 200)], vec![]);
        assert_eq!(ids(&list), ["new"]);
        assert!(
            !list.has_more(),
            "reset must clear the old continuation token"
        );
    }

    #[test]
    fn the_cursor_key_survives_a_page_landing_above_it() {
        let mut list = WorkflowList::default();
        list.reset(vec![row("a", "r1", 100)], vec![("a".into(), vec![1])]);
        assert_eq!(list.position_of(("a", "r1")), Some(0));

        list.append(vec![row("a", "r9", 900)], vec![]);
        assert_eq!(
            list.position_of(("a", "r1")),
            Some(1),
            "the anchored row moved down; its key must still find it"
        );
        assert_eq!(list.position_of(("a", "gone")), None);
    }

    #[test]
    fn tokens_track_which_namespaces_still_have_pages() {
        let mut list = WorkflowList::default();
        list.reset(
            vec![row("a", "r1", 100)],
            vec![("a".into(), vec![1]), ("b".into(), vec![2])],
        );
        assert!(list.has_more());
        assert_eq!(list.tokens().len(), 2);

        // Namespace `a` exhausts; `b` still has pages.
        list.append(vec![row("b", "r2", 90)], vec![("b".into(), vec![3])]);
        assert_eq!(list.tokens(), &[("b".to_string(), vec![3])]);
        assert!(list.has_more());
    }
}
