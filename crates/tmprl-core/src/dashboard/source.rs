//! What panels read from: the requests a board makes, what comes back, and the items
//! a panel turns that into.

use std::collections::BTreeMap;

use super::layout::{PanelKind, PanelSpec, Show, TimeField, Window};
use crate::pending::PendingActivity;
use crate::query;
use crate::schedule::ScheduleRow;
use crate::taskqueue::QueueHealth;
use crate::timerange::to_rfc3339;
use crate::workflow::{
    StatusCounts, WorkflowRow, WorkflowStatus, by_close_time_desc, by_start_time_desc,
};

/// The most task queues a board describes. Each costs two requests a refresh.
pub const MAX_QUEUES: usize = 8;

/// How long after a stretch of time ends its count is taken as final: visibility trails
/// the event by a little.
const SETTLE_MS: i64 = 60_000;

/// The most closed workflows a board asks the reason of. Each costs one request, once.
pub const MAX_REASONS: usize = 40;

/// The most workflows a retrying panel looks into at a time. Each costs one request a
/// refresh.
pub const MAX_SCAN: usize = 50;

/// The most names a board counts exactly. Each costs one request a refresh.
pub const MAX_TALLIES: usize = 24;

/// One request a board makes, shared by every panel that needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Counts {
        namespaces: Vec<String>,
        query: String,
        window: Window,
    },
    Workflows {
        namespaces: Vec<String>,
        query: String,
        window: Window,
    },
    Schedules {
        namespaces: Vec<String>,
    },
    /// One task queue's health. Not asked for by a panel: a board adds one for each queue
    /// its panels turn out to list.
    Queue {
        namespace: String,
        name: String,
    },
    /// Why one workflow closed. Not asked for by a panel either: a board adds one for each
    /// workflow its lists show that ended badly.
    Close {
        namespace: String,
        workflow_id: String,
        run_id: String,
    },
    /// One column of a histogram: the workflows in a stretch of time. Added by the board,
    /// one for each stretch its histograms show.
    Bucket {
        namespaces: Vec<String>,
        query: String,
        by: TimeField,
        from_ms: i64,
        to_ms: i64,
    },
    /// What one running workflow is waiting on. Added by the board, one for each workflow
    /// a retrying panel looks into.
    Pending {
        namespace: String,
        workflow_id: String,
        run_id: String,
    },
}

impl Source {
    pub fn namespaces(&self) -> &[String] {
        match self {
            Source::Counts { namespaces, .. }
            | Source::Bucket { namespaces, .. }
            | Source::Workflows { namespaces, .. }
            | Source::Schedules { namespaces } => namespaces,
            Source::Queue { namespace, .. }
            | Source::Close { namespace, .. }
            | Source::Pending { namespace, .. } => std::slice::from_ref(namespace),
        }
    }

    /// The visibility query to send, its window worked out against `now_ms`.
    pub fn query(&self, now_ms: i64) -> String {
        match self {
            Source::Counts { query, window, .. } | Source::Workflows { query, window, .. } => {
                window.narrow(query, now_ms)
            }
            Source::Bucket {
                query,
                by,
                from_ms,
                to_ms,
                ..
            } => {
                let field = by.attribute();
                query::and(
                    query,
                    &format!(
                        "{field} >= '{}' AND {field} < '{}'",
                        to_rfc3339(*from_ms),
                        to_rfc3339(*to_ms)
                    ),
                )
            }
            Source::Schedules { .. }
            | Source::Queue { .. }
            | Source::Close { .. }
            | Source::Pending { .. } => String::new(),
        }
    }

    /// Whether an answer given at `now_ms` is the last word, so the source is not asked
    /// again on a timer. A closed workflow's closing event does not change, and nor does
    /// what closed in a stretch of time once visibility has caught up with it.
    pub fn settles(&self, now_ms: i64) -> bool {
        match self {
            Source::Close { .. } => true,
            Source::Bucket { by, to_ms, .. } => match by {
                TimeField::Close => to_ms + SETTLE_MS <= now_ms,
                // What started in a stretch keeps changing status after it.
                TimeField::Start => false,
            },
            Source::Counts { .. }
            | Source::Workflows { .. }
            | Source::Schedules { .. }
            | Source::Queue { .. }
            | Source::Pending { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceData {
    Counts(StatusCounts),
    /// `more` when the server had further pages, so what is here is a sample.
    Workflows {
        rows: Vec<WorkflowRow>,
        more: bool,
    },
    Schedules(Vec<ScheduleRow>),
    Queue(QueueHealth),
    /// `None` when the closing event gives no reason.
    Close(Option<String>),
    Pending(Vec<PendingActivity>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRef {
    pub namespace: String,
    pub name: String,
    pub running: usize,
    /// Whether `running` is a count, or a tally of one page of a longer list.
    pub exact: bool,
    /// `None` until the queue has been described, and for queues past [`MAX_QUEUES`].
    pub health: Option<QueueHealth>,
}

/// One line of a panel, and one stop for the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Status {
        status: WorkflowStatus,
        count: i64,
    },
    Workflow(WorkflowRow),
    /// `exact` when `count` is a count, not a tally of one page of a longer list.
    Type {
        name: String,
        count: usize,
        exact: bool,
    },
    Queue(QueueRef),
    Schedule(ScheduleRow),
    /// A workflow with an activity that keeps being retried, and the one furthest along.
    Retry {
        row: WorkflowRow,
        activity: Box<PendingActivity>,
    },
    /// A column of a histogram with something in it.
    Bucket {
        from_ms: i64,
        to_ms: i64,
        count: i64,
    },
}

impl Item {
    /// What `y` copies: the name you would paste into a query or a command.
    pub fn field(&self) -> &str {
        match self {
            Item::Status { status, .. } => status.query_name(),
            Item::Workflow(row) | Item::Retry { row, .. } => &row.workflow_id,
            Item::Type { name, .. } => name,
            Item::Queue(q) => &q.name,
            Item::Schedule(s) => &s.schedule_id,
            // A stretch of time has no name to paste. `Y` gives it as JSON.
            Item::Bucket { .. } => "",
        }
    }

    /// The text `/` matches against.
    pub fn label(&self) -> String {
        match self {
            Item::Status { status, count } => format!("{} {count}", status.query_name()),
            Item::Workflow(w) => format!(
                "{} {} {} {} {} {}",
                w.workflow_id,
                w.workflow_type,
                w.task_queue,
                w.status.query_name(),
                w.namespace,
                w.run_id,
            ),
            Item::Type { name, count, .. } => format!("{name} {count}"),
            Item::Queue(q) => format!("{} {} {}", q.name, q.namespace, q.running),
            Item::Schedule(s) => format!(
                "{} {} {}",
                s.schedule_id,
                s.workflow_type,
                if s.paused { "paused" } else { "running" },
            ),
            Item::Bucket { from_ms, count, .. } => format!("{} {count}", to_rfc3339(*from_ms)),
            Item::Retry { row, activity } => format!(
                "{} {} {} {} {}",
                row.workflow_id,
                row.workflow_type,
                activity.activity_type,
                row.namespace,
                row.run_id,
            ),
        }
    }

    /// Whether the number on this line came from a sample and may be short of the truth.
    pub fn approximate(&self) -> bool {
        match self {
            Item::Type { exact, .. } => !exact,
            Item::Queue(q) => !q.exact,
            Item::Status { .. }
            | Item::Workflow(_)
            | Item::Schedule(_)
            | Item::Retry { .. }
            | Item::Bucket { .. } => false,
        }
    }

    pub(super) fn key(&self) -> String {
        match self {
            Item::Status { status, .. } => status.query_name().to_string(),
            Item::Workflow(row) | Item::Retry { row, .. } => {
                format!("{}/{}", row.namespace, row.run_id)
            }
            Item::Type { name, .. } => name.clone(),
            Item::Queue(q) => format!("{}/{}", q.namespace, q.name),
            Item::Schedule(s) => format!("{}/{}", s.namespace, s.schedule_id),
            Item::Bucket { from_ms, .. } => from_ms.to_string(),
        }
    }
}

/// Workflow types by how many of `rows` have them, most first.
pub fn tally_types(rows: &[WorkflowRow]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for row in rows {
        *counts.entry(row.workflow_type.as_str()).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(name, n)| (name.to_string(), n))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The task queues `rows` run on, busiest first, then any of `names` that none of them use.
///
/// Temporal has no call that lists task queues, so the ones in use are all that can be found.
pub fn discover_queues(rows: &[WorkflowRow], names: &[String], namespace: &str) -> Vec<QueueRef> {
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for row in rows.iter().filter(|r| !r.task_queue.is_empty()) {
        *counts
            .entry((row.namespace.as_str(), row.task_queue.as_str()))
            .or_default() += 1;
    }
    let mut out: Vec<QueueRef> = counts
        .into_iter()
        .map(|((namespace, name), running)| QueueRef {
            namespace: namespace.to_string(),
            name: name.to_string(),
            running,
            exact: false,
            health: None,
        })
        .collect();
    out.sort_by(|a, b| {
        b.running
            .cmp(&a.running)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.namespace.cmp(&b.namespace))
    });
    for name in names {
        if !out.iter().any(|q| &q.name == name) {
            out.push(QueueRef {
                namespace: namespace.to_string(),
                name: name.clone(),
                running: 0,
                exact: false,
                health: None,
            });
        }
    }
    out
}

pub(super) fn items(spec: &PanelSpec, data: Option<&SourceData>, namespace: &str) -> Vec<Item> {
    let mut out: Vec<Item> = match (&spec.kind, data) {
        (PanelKind::Counts { .. }, Some(SourceData::Counts(counts))) => {
            return counts
                .iter()
                .map(|(status, count)| Item::Status { status, count })
                .collect();
        }
        (PanelKind::Workflows { window, .. }, Some(SourceData::Workflows { rows, .. })) => {
            let mut rows = rows.clone();
            rows.sort_by(match window.by {
                TimeField::Start => by_start_time_desc,
                TimeField::Close => by_close_time_desc,
            });
            rows.into_iter().map(Item::Workflow).collect()
        }
        (PanelKind::Types { .. }, Some(SourceData::Workflows { rows, .. })) => tally_types(rows)
            .into_iter()
            .map(|(name, count)| Item::Type {
                name,
                count,
                exact: false,
            })
            .collect(),
        (PanelKind::Queues { names, .. }, Some(SourceData::Workflows { rows, .. })) => {
            discover_queues(rows, names, namespace)
                .into_iter()
                .map(Item::Queue)
                .collect()
        }
        (PanelKind::Schedules { show }, Some(SourceData::Schedules(rows))) => {
            let mut rows: Vec<ScheduleRow> = rows
                .iter()
                .filter(|r| match show {
                    Show::All => true,
                    Show::Paused => r.paused,
                    Show::Upcoming => !r.paused && r.next_run.is_some(),
                })
                .cloned()
                .collect();
            if *show == Show::Upcoming {
                rows.sort_by(|a, b| {
                    a.next_run
                        .cmp(&b.next_run)
                        .then_with(|| a.key().cmp(&b.key()))
                });
            }
            rows.into_iter().map(Item::Schedule).collect()
        }
        _ => Vec::new(),
    };
    out.truncate(spec.limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::fixtures::*;
    use crate::timerange::to_rfc3339;

    const DAY: i64 = 86_400_000;

    #[test]
    fn a_window_becomes_a_literal_instant() {
        let source = Source::Workflows {
            namespaces: scope(),
            query: "A = 'x' OR B = 'y'".into(),
            window: Window::since(3_600_000),
        };
        assert_eq!(
            source.query(NOW),
            format!(
                "(A = 'x' OR B = 'y') AND StartTime > '{}'",
                to_rfc3339(NOW - 3_600_000)
            )
        );
        let open = Source::Workflows {
            namespaces: scope(),
            query: String::new(),
            window: Window::default(),
        };
        assert_eq!(open.query(NOW), "");
    }

    #[test]
    fn a_counts_panel_takes_a_window_too() {
        let source = Source::Counts {
            namespaces: scope(),
            query: " ".into(),
            window: Window::since(3_600_000),
        };
        assert_eq!(
            source.query(NOW),
            format!("StartTime > '{}'", to_rfc3339(NOW - 3_600_000))
        );
    }

    #[test]
    fn older_looks_past_an_instant_and_with_since_makes_a_band() {
        let query = |window: Window| {
            Source::Workflows {
                namespaces: scope(),
                query: "T = 'x'".into(),
                window,
            }
            .query(NOW)
        };
        let older = Window {
            older_ms: Some(DAY),
            ..Window::default()
        };
        assert_eq!(
            query(older),
            format!("T = 'x' AND StartTime < '{}'", to_rfc3339(NOW - DAY))
        );
        let band = Window {
            since_ms: Some(7 * DAY),
            older_ms: Some(DAY),
            by: TimeField::Close,
        };
        assert_eq!(
            query(band),
            format!(
                "T = 'x' AND CloseTime > '{}' AND CloseTime < '{}'",
                to_rfc3339(NOW - 7 * DAY),
                to_rfc3339(NOW - DAY)
            )
        );
    }

    #[test]
    fn a_window_on_close_time_lists_the_last_to_close_first() {
        let closed = |run: &str, start: i64, close: Option<i64>| WorkflowRow {
            close_time: close,
            ..wf(run, "T", "q", start)
        };
        let data = SourceData::Workflows {
            rows: vec![
                closed("early", 30, Some(40)),
                closed("open", 20, None),
                closed("late", 10, Some(90)),
            ],
            more: false,
        };
        let order = |by: TimeField| -> Vec<String> {
            let spec = PanelSpec::new(PanelKind::Workflows {
                query: String::new(),
                window: Window {
                    since_ms: Some(DAY),
                    older_ms: None,
                    by,
                },
            });
            items(&spec, Some(&data), "default")
                .into_iter()
                .map(|i| i.field().to_string())
                .collect()
        };
        assert_eq!(order(TimeField::Start), ["wf-early", "wf-open", "wf-late"]);
        assert_eq!(order(TimeField::Close), ["wf-late", "wf-early", "wf-open"]);
    }

    #[test]
    fn types_are_tallied_most_first_and_ties_by_name() {
        let rows = vec![
            wf("a", "Refund", "q", 1),
            wf("b", "Order", "q", 2),
            wf("c", "Order", "q", 3),
            wf("d", "Audit", "q", 4),
        ];
        assert_eq!(
            tally_types(&rows),
            vec![
                ("Order".to_string(), 2),
                ("Audit".to_string(), 1),
                ("Refund".to_string(), 1),
            ]
        );
    }

    #[test]
    fn queues_come_from_the_rows_and_then_from_the_names() {
        let rows = vec![
            wf("a", "T", "orders", 1),
            wf("b", "T", "orders", 2),
            wf("c", "T", "billing", 3),
            wf("d", "T", "", 4),
        ];
        let names = vec!["orders".to_string(), "idle".to_string()];
        let queues = discover_queues(&rows, &names, "default");
        let seen: Vec<(&str, usize)> = queues
            .iter()
            .map(|q| (q.name.as_str(), q.running))
            .collect();
        assert_eq!(seen, [("orders", 2), ("billing", 1), ("idle", 0)]);
    }

    #[test]
    fn a_schedules_panel_shows_what_it_was_asked_for() {
        let data = SourceData::Schedules(vec![
            schedule("b", false, Some(90)),
            schedule("a", false, Some(40)),
            schedule("c", true, Some(10)),
            schedule("d", false, None),
        ]);
        let ids = |show: Show| -> Vec<String> {
            let spec = PanelSpec::new(PanelKind::Schedules { show });
            items(&spec, Some(&data), "default")
                .into_iter()
                .map(|i| match i {
                    Item::Schedule(s) => s.schedule_id,
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(ids(Show::All), ["b", "a", "c", "d"]);
        assert_eq!(ids(Show::Paused), ["c"]);
        assert_eq!(ids(Show::Upcoming), ["a", "b"]);
    }
}
