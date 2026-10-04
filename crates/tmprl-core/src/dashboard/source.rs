//! What panels read from: the requests a board makes, what comes back, and the items
//! a panel turns that into.

use std::collections::BTreeMap;

use super::layout::{PanelKind, PanelSpec, Show};
use crate::query;
use crate::schedule::ScheduleRow;
use crate::taskqueue::QueueHealth;
use crate::timerange::to_rfc3339;
use crate::workflow::{StatusCounts, WorkflowRow, WorkflowStatus, by_start_time_desc};

/// The most task queues a board describes. Each costs two requests a refresh.
pub const MAX_QUEUES: usize = 8;

/// One request a board makes, shared by every panel that needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Counts {
        namespaces: Vec<String>,
        query: String,
    },
    Workflows {
        namespaces: Vec<String>,
        query: String,
        since_ms: Option<i64>,
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
}

impl Source {
    pub fn namespaces(&self) -> &[String] {
        match self {
            Source::Counts { namespaces, .. }
            | Source::Workflows { namespaces, .. }
            | Source::Schedules { namespaces } => namespaces,
            Source::Queue { namespace, .. } => std::slice::from_ref(namespace),
        }
    }

    /// The visibility query to send. The grammar has no `now()`, so a `since` window becomes
    /// a literal instant each time this is asked.
    pub fn query(&self, now_ms: i64) -> String {
        match self {
            Source::Counts { query, .. } => query.clone(),
            Source::Workflows {
                query, since_ms, ..
            } => since(query, *since_ms, now_ms),
            Source::Schedules { .. } | Source::Queue { .. } => String::new(),
        }
    }
}

fn since(filter: &str, since_ms: Option<i64>, now_ms: i64) -> String {
    match since_ms {
        Some(ms) => query::and(
            filter,
            &format!("StartTime > '{}'", to_rfc3339(now_ms - ms)),
        ),
        None => filter.trim().to_string(),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRef {
    pub namespace: String,
    pub name: String,
    pub running: usize,
    /// `None` until the queue has been described, and for queues past [`MAX_QUEUES`].
    pub health: Option<QueueHealth>,
}

/// One line of a panel, and one stop for the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Status { status: WorkflowStatus, count: i64 },
    Workflow(WorkflowRow),
    Type { name: String, count: usize },
    Queue(QueueRef),
    Schedule(ScheduleRow),
}

impl Item {
    /// What `y` copies: the name you would paste into a query or a command.
    pub fn field(&self) -> &str {
        match self {
            Item::Status { status, .. } => status.query_name(),
            Item::Workflow(row) => &row.workflow_id,
            Item::Type { name, .. } => name,
            Item::Queue(q) => &q.name,
            Item::Schedule(s) => &s.schedule_id,
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
            Item::Type { name, count } => format!("{name} {count}"),
            Item::Queue(q) => format!("{} {} {}", q.name, q.namespace, q.running),
            Item::Schedule(s) => format!(
                "{} {} {}",
                s.schedule_id,
                s.workflow_type,
                if s.paused { "paused" } else { "running" },
            ),
        }
    }

    pub(super) fn key(&self) -> String {
        match self {
            Item::Status { status, .. } => status.query_name().to_string(),
            Item::Workflow(row) => format!("{}/{}", row.namespace, row.run_id),
            Item::Type { name, .. } => name.clone(),
            Item::Queue(q) => format!("{}/{}", q.namespace, q.name),
            Item::Schedule(s) => format!("{}/{}", s.namespace, s.schedule_id),
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
        (PanelKind::Workflows { .. }, Some(SourceData::Workflows { rows, .. })) => {
            let mut rows = rows.clone();
            rows.sort_by(by_start_time_desc);
            rows.into_iter().map(Item::Workflow).collect()
        }
        (PanelKind::Types { .. }, Some(SourceData::Workflows { rows, .. })) => tally_types(rows)
            .into_iter()
            .map(|(name, count)| Item::Type { name, count })
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

    #[test]
    fn a_window_becomes_a_literal_instant() {
        let source = Source::Workflows {
            namespaces: scope(),
            query: "A = 'x' OR B = 'y'".into(),
            since_ms: Some(3_600_000),
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
            since_ms: None,
        };
        assert_eq!(open.query(NOW), "");
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
