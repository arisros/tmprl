//! The dashboard: panels over a scope, the layout they sit in, and `dashboard.toml`.
//!
//! A panel does not fetch. It reads from a [`Source`], and panels asking for the same thing
//! share one, so "recent failures" and "failing types" are one request and always agree.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use crate::config::ConfigError;
use crate::fault::Fault;
use crate::loadable::Loadable;
use crate::query;
use crate::schedule::ScheduleRow;
use crate::timerange::{parse_offset, to_rfc3339};
use crate::workflow::{StatusCounts, WorkflowRow, WorkflowStatus, by_start_time_desc};

const FILE: &str = "dashboard.toml";

pub const MAX_PANELS: usize = 12;
pub const DEFAULT_LIMIT: usize = 10;
pub const MAX_LIMIT: usize = 50;

const RUNNING: &str = "ExecutionStatus = 'Running'";
const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Weight(u16),
    Lines(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    Paused,
    Upcoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKind {
    Counts {
        query: String,
    },
    Workflows {
        query: String,
        since_ms: Option<i64>,
    },
    Types {
        query: String,
        since_ms: Option<i64>,
    },
    Queues {
        query: String,
        names: Vec<String>,
    },
    Schedules {
        show: Show,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSpec {
    pub kind: PanelKind,
    pub title: Option<String>,
    pub width: u16,
    pub limit: usize,
    /// Empty means the scope of the pane the dashboard is in.
    pub namespaces: Vec<String>,
}

impl PanelSpec {
    fn new(kind: PanelKind) -> Self {
        Self {
            kind,
            title: None,
            width: 1,
            limit: DEFAULT_LIMIT,
            namespaces: Vec::new(),
        }
    }

    fn titled(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    pub fn title(&self) -> &str {
        if let Some(title) = &self.title {
            return title;
        }
        match &self.kind {
            PanelKind::Counts { .. } => "Status",
            PanelKind::Workflows { .. } => "Workflows",
            PanelKind::Types { .. } => "Workflow types",
            PanelKind::Queues { .. } => "Task queues",
            PanelKind::Schedules { show } => match show {
                Show::All => "Schedules",
                Show::Paused => "Paused schedules",
                Show::Upcoming => "Upcoming schedules",
            },
        }
    }

    fn source(&self, scope: &[String]) -> Source {
        let namespaces = if self.namespaces.is_empty() {
            scope.to_vec()
        } else {
            self.namespaces.clone()
        };
        match &self.kind {
            PanelKind::Counts { query } => Source::Counts {
                namespaces,
                query: query.clone(),
            },
            PanelKind::Workflows { query, since_ms } | PanelKind::Types { query, since_ms } => {
                Source::Workflows {
                    namespaces,
                    query: query.clone(),
                    since_ms: *since_ms,
                }
            }
            PanelKind::Queues { query, .. } => Source::Workflows {
                namespaces,
                query: query.clone(),
                since_ms: None,
            },
            PanelKind::Schedules { .. } => Source::Schedules { namespaces },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSpec {
    pub size: Size,
    pub panels: Vec<PanelSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub rows: Vec<RowSpec>,
}

impl Layout {
    /// The layout shown when `dashboard.toml` says nothing.
    pub fn builtin() -> Self {
        let failures = || (query::PROBLEMS.to_string(), Some(DAY_MS));
        let (query, since_ms) = failures();
        let mut recent =
            PanelSpec::new(PanelKind::Workflows { query, since_ms }).titled("Recent failures");
        recent.width = 2;
        let (query, since_ms) = failures();
        let types = PanelSpec::new(PanelKind::Types { query, since_ms }).titled("Failing types");

        Self {
            rows: vec![
                RowSpec {
                    size: Size::Lines(3),
                    panels: vec![PanelSpec::new(PanelKind::Counts {
                        query: String::new(),
                    })],
                },
                RowSpec {
                    size: Size::Weight(3),
                    panels: vec![recent, types],
                },
                RowSpec {
                    size: Size::Weight(2),
                    panels: vec![
                        PanelSpec::new(PanelKind::Queues {
                            query: RUNNING.to_string(),
                            names: Vec::new(),
                        }),
                        PanelSpec::new(PanelKind::Schedules { show: Show::All }),
                    ],
                },
            ],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn panels(&self) -> impl Iterator<Item = &PanelSpec> {
        self.rows.iter().flat_map(|r| r.panels.iter())
    }
}

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
}

impl Source {
    pub fn namespaces(&self) -> &[String] {
        match self {
            Source::Counts { namespaces, .. }
            | Source::Workflows { namespaces, .. }
            | Source::Schedules { namespaces } => namespaces,
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
            Source::Schedules { .. } => String::new(),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRef {
    pub namespace: String,
    pub name: String,
    pub running: usize,
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

    fn key(&self) -> String {
        match self {
            Item::Status { status, .. } => status.query_name().to_string(),
            Item::Workflow(row) => format!("{}/{}", row.namespace, row.run_id),
            Item::Type { name, .. } => name.clone(),
            Item::Queue(q) => format!("{}/{}", q.namespace, q.name),
            Item::Schedule(s) => format!("{}/{}", s.namespace, s.schedule_id),
        }
    }
}

/// Where `<CR>` on an item leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drill {
    Query {
        namespaces: Vec<String>,
        query: String,
    },
    Workflow(WorkflowRow),
    Schedule {
        namespace: String,
        schedule_id: String,
    },
}

/// The item under the cursor, remembered across a refresh by what it is, not where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    panel: usize,
    index: usize,
    key: String,
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
            });
        }
    }
    out
}

fn items(spec: &PanelSpec, data: Option<&SourceData>, namespace: &str) -> Vec<Item> {
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

#[derive(Debug)]
struct Panel {
    spec: PanelSpec,
    source: usize,
    items: Vec<Item>,
}

/// A layout, the requests behind it, and what has come back so far.
#[derive(Debug)]
pub struct Board {
    layout: Layout,
    scope: Vec<String>,
    sources: Vec<Source>,
    data: Vec<Loadable<SourceData>>,
    faults: Vec<Option<Fault>>,
    panels: Vec<Panel>,
}

impl Board {
    pub fn new(layout: Layout, scope: &[String]) -> Self {
        let mut sources: Vec<Source> = Vec::new();
        let mut panels = Vec::new();
        for spec in layout.panels() {
            let wanted = spec.source(scope);
            let source = sources
                .iter()
                .position(|s| *s == wanted)
                .unwrap_or_else(|| {
                    sources.push(wanted);
                    sources.len() - 1
                });
            panels.push(Panel {
                spec: spec.clone(),
                source,
                items: Vec::new(),
            });
        }
        Self {
            layout,
            scope: scope.to_vec(),
            data: sources.iter().map(|_| Loadable::NotAsked).collect(),
            faults: sources.iter().map(|_| None).collect(),
            sources,
            panels,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn scope(&self) -> &[String] {
        &self.scope
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn spec(&self, panel: usize) -> Option<&PanelSpec> {
        self.panels.get(panel).map(|p| &p.spec)
    }

    pub fn items(&self, panel: usize) -> &[Item] {
        self.panels.get(panel).map_or(&[], |p| p.items.as_slice())
    }

    pub fn state(&self, panel: usize) -> Option<&Loadable<SourceData>> {
        self.data.get(self.panels.get(panel)?.source)
    }

    /// Why the panel is not current: the load that failed, or the refresh that failed while
    /// an older answer is still on screen.
    pub fn fault(&self, panel: usize) -> Option<&Fault> {
        let source = self.panels.get(panel)?.source;
        self.faults[source].as_ref().or(self.data[source].error())
    }

    /// How many rows a tally or a list was drawn from, when the server had more than that.
    pub fn sampled(&self, panel: usize) -> Option<usize> {
        match self.state(panel)?.value()? {
            SourceData::Workflows { rows, more: true } => Some(rows.len()),
            SourceData::Workflows { more: false, .. }
            | SourceData::Counts(_)
            | SourceData::Schedules(_) => None,
        }
    }

    pub fn begin_refresh(&mut self) {
        for slot in &mut self.data {
            slot.begin_refresh();
        }
    }

    /// Take a reply. A failure does not blank an answer already on screen.
    pub fn apply(&mut self, source: usize, result: Result<SourceData, Fault>) {
        let Some(slot) = self.data.get_mut(source) else {
            return;
        };
        match result {
            Ok(data) => {
                *slot = Loadable::loaded(data);
                self.faults[source] = None;
            }
            Err(fault) if slot.value().is_some() => self.faults[source] = Some(fault),
            Err(fault) => *slot = Loadable::Failed(fault),
        }
        let data = self.data[source].value();
        let namespace = self.scope.first().map_or("", String::as_str);
        for panel in self.panels.iter_mut().filter(|p| p.source == source) {
            panel.items = items(&panel.spec, data, namespace);
        }
    }

    pub fn len(&self) -> usize {
        self.panels.iter().map(|p| p.items.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The cursor position of a panel's first item.
    pub fn first_of(&self, panel: usize) -> usize {
        self.panels.iter().take(panel).map(|p| p.items.len()).sum()
    }

    /// The panel a cursor position is in, and the item within it.
    pub fn locate(&self, cursor: usize) -> Option<(usize, usize)> {
        let mut before = 0;
        for (i, panel) in self.panels.iter().enumerate() {
            if cursor < before + panel.items.len() {
                return Some((i, cursor - before));
            }
            before += panel.items.len();
        }
        None
    }

    pub fn item(&self, cursor: usize) -> Option<&Item> {
        let (panel, index) = self.locate(cursor)?;
        self.panels[panel].items.get(index)
    }

    /// The first item of the next panel that has any, or of the previous one. The cursor
    /// stays put when there is none that way.
    pub fn panel_step(&self, cursor: usize, forward: bool) -> usize {
        let Some((here, _)) = self.locate(cursor) else {
            return cursor;
        };
        let has_items = |i: &usize| !self.panels[*i].items.is_empty();
        let target = if forward {
            (here + 1..self.panels.len()).find(has_items)
        } else {
            (0..here).rev().find(has_items)
        };
        target.map_or(cursor, |panel| self.first_of(panel))
    }

    pub fn anchor(&self, cursor: usize) -> Option<Anchor> {
        let (panel, index) = self.locate(cursor)?;
        Some(Anchor {
            panel,
            index,
            key: self.panels[panel].items[index].key(),
        })
    }

    /// Where the anchored item is now. When it is gone the cursor keeps its place in the
    /// panel, so a refresh never throws it across the screen.
    pub fn reanchor(&self, anchor: &Anchor) -> usize {
        let items = self.items(anchor.panel);
        let index = items
            .iter()
            .position(|i| i.key() == anchor.key)
            .unwrap_or(anchor.index.min(items.len().saturating_sub(1)));
        (self.first_of(anchor.panel) + index).min(self.len().saturating_sub(1))
    }

    /// `None` when there is nothing under the cursor, or when the item's name cannot go
    /// inside a quoted literal.
    pub fn drill(&self, cursor: usize, now_ms: i64) -> Option<Drill> {
        let (panel, index) = self.locate(cursor)?;
        let panel = &self.panels[panel];
        let source = &self.sources[panel.source];
        let narrowed = |filter: &str, field: &str, value: &str| -> Option<String> {
            let value = query::quotable(value)?;
            Some(query::and(filter, &format!("{field} = '{value}'")))
        };
        let namespaces = source.namespaces().to_vec();

        Some(match (&panel.items[index], &panel.spec.kind) {
            (Item::Workflow(row), _) => Drill::Workflow(row.clone()),
            (Item::Schedule(row), _) => Drill::Schedule {
                namespace: row.namespace.clone(),
                schedule_id: row.schedule_id.clone(),
            },
            (Item::Status { status, .. }, _) => Drill::Query {
                namespaces,
                query: narrowed(
                    &source.query(now_ms),
                    "ExecutionStatus",
                    status.query_name(),
                )?,
            },
            (Item::Type { name, .. }, _) => Drill::Query {
                namespaces,
                query: narrowed(&source.query(now_ms), "WorkflowType", name)?,
            },
            (Item::Queue(queue), _) => Drill::Query {
                namespaces: vec![queue.namespace.clone()],
                query: narrowed(&source.query(now_ms), "TaskQueue", &queue.name)?,
            },
        })
    }
}

const TOP_KEYS: &str = "row";
const ROW_KEYS: &str = "height, lines or panel";

fn allowed(kind: &str) -> Option<(&'static [&'static str], &'static str)> {
    Some(match kind {
        "counts" => (
            &["kind", "title", "width", "namespaces", "query"],
            "kind, title, width, namespaces or query",
        ),
        "workflows" | "types" => (
            &[
                "kind",
                "title",
                "width",
                "namespaces",
                "limit",
                "query",
                "since",
            ],
            "kind, title, width, namespaces, limit, query or since",
        ),
        "queues" => (
            &[
                "kind",
                "title",
                "width",
                "namespaces",
                "limit",
                "query",
                "names",
            ],
            "kind, title, width, namespaces, limit, query or names",
        ),
        "schedules" => (
            &["kind", "title", "width", "namespaces", "limit", "show"],
            "kind, title, width, namespaces, limit or show",
        ),
        _ => return None,
    })
}

fn wrong(path: String, expected: &'static str) -> ConfigError {
    ConfigError::Type {
        file: FILE,
        path,
        expected,
    }
}

fn only(
    table: &toml::Table,
    keys: &[&str],
    place: String,
    expected: &'static str,
) -> Result<(), ConfigError> {
    match table.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(key) => Err(ConfigError::UnknownDashboardKey {
            place,
            key: key.clone(),
            expected,
        }),
        None => Ok(()),
    }
}

fn tables<'a>(
    table: &'a toml::Table,
    key: &str,
    path: &str,
    expected: &'static str,
) -> Result<Vec<&'a toml::Table>, ConfigError> {
    let Some(raw) = table.get(key) else {
        return Ok(Vec::new());
    };
    raw.as_array()
        .and_then(|a| a.iter().map(|v| v.as_table()).collect::<Option<Vec<_>>>())
        .ok_or_else(|| wrong(path.to_string(), expected))
}

fn text(table: &toml::Table, key: &str, path: &str) -> Result<Option<String>, ConfigError> {
    match table.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| wrong(format!("{path}.{key}"), "a string")),
    }
}

fn number(
    table: &toml::Table,
    key: &str,
    path: &str,
    range: RangeInclusive<i64>,
    expected: &'static str,
) -> Result<Option<u16>, ConfigError> {
    match table.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_integer()
            .filter(|n| range.contains(n))
            .and_then(|n| u16::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| wrong(format!("{path}.{key}"), expected)),
    }
}

fn strings(table: &toml::Table, key: &str, path: &str) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = table.get(key) else {
        return Ok(Vec::new());
    };
    raw.as_array()
        .and_then(|a| {
            a.iter()
                .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| wrong(format!("{path}.{key}"), "an array of names"))
}

fn filter(table: &toml::Table, path: &str, default: &str) -> Result<String, ConfigError> {
    let query = text(table, "query", path)?.unwrap_or_else(|| default.to_string());
    if query::orders(&query) {
        return Err(wrong(
            format!("{path}.query"),
            "a query without ORDER BY, a panel sorts its own rows",
        ));
    }
    Ok(query)
}

fn window(table: &toml::Table, path: &str) -> Result<Option<i64>, ConfigError> {
    match text(table, "since", path)? {
        None => Ok(None),
        Some(s) => parse_offset(&s)
            .filter(|ms| *ms > 0)
            .map(Some)
            .ok_or_else(|| wrong(format!("{path}.since"), "a duration such as 30m, 24h or 7d")),
    }
}

fn parse_panel(table: &toml::Table, path: &str) -> Result<PanelSpec, ConfigError> {
    let kind =
        text(table, "kind", path)?.ok_or_else(|| wrong(format!("{path}.kind"), "a panel kind"))?;
    let (keys, expected) = allowed(&kind).ok_or_else(|| ConfigError::BadPanelKind {
        path: format!("{path}.kind"),
        value: kind.clone(),
    })?;
    only(table, keys, format!("{path}, a {kind} panel"), expected)?;

    let kind = match kind.as_str() {
        "counts" => PanelKind::Counts {
            query: filter(table, path, "")?,
        },
        "workflows" => PanelKind::Workflows {
            query: filter(table, path, "")?,
            since_ms: window(table, path)?,
        },
        "types" => PanelKind::Types {
            query: filter(table, path, "")?,
            since_ms: window(table, path)?,
        },
        "queues" => PanelKind::Queues {
            query: filter(table, path, RUNNING)?,
            names: strings(table, "names", path)?,
        },
        _ => PanelKind::Schedules {
            show: match text(table, "show", path)?.as_deref() {
                None | Some("all") => Show::All,
                Some("paused") => Show::Paused,
                Some("upcoming") => Show::Upcoming,
                Some(_) => {
                    return Err(wrong(
                        format!("{path}.show"),
                        "`paused`, `upcoming` or `all`",
                    ));
                }
            },
        },
    };

    Ok(PanelSpec {
        kind,
        title: text(table, "title", path)?,
        width: number(table, "width", path, 1..=100, "an integer from 1 to 100")?.unwrap_or(1),
        limit: number(
            table,
            "limit",
            path,
            1..=MAX_LIMIT as i64,
            "an integer from 1 to 50",
        )?
        .map_or(DEFAULT_LIMIT, usize::from),
        namespaces: strings(table, "namespaces", path)?,
    })
}

/// Parse `dashboard.toml`:
///
/// ```toml
/// [[row]]
/// lines = 3
///
/// [[row.panel]]
/// kind = "counts"
///
/// [[row]]
///
/// [[row.panel]]
/// kind  = "workflows"
/// title = "Recent failures"
/// query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
/// since = "24h"
/// width = 2
/// ```
///
/// Strict: a key that does not exist, or one that belongs to another kind of panel, is an
/// error. A misspelt `query` would otherwise show every workflow under a "failures" title.
/// A file with no rows is an empty layout, which the caller reads as "not configured".
pub fn parse_dashboard(src: &str) -> Result<Layout, ConfigError> {
    let table: toml::Table = toml::from_str(src).map_err(|e| ConfigError::Syntax {
        file: FILE,
        message: e.message().to_string(),
    })?;
    only(&table, &["row"], "the file".to_string(), TOP_KEYS)?;

    let mut rows = Vec::new();
    for (r, row) in tables(&table, "row", "row", "an array of [[row]] tables")?
        .into_iter()
        .enumerate()
    {
        let path = format!("row[{r}]");
        only(row, &["height", "lines", "panel"], path.clone(), ROW_KEYS)?;
        let height = number(row, "height", &path, 1..=100, "an integer from 1 to 100")?;
        let lines = number(row, "lines", &path, 3..=50, "an integer from 3 to 50")?;
        let size = match (height, lines) {
            (Some(_), Some(_)) => return Err(wrong(path, "given `height` or `lines`, not both")),
            (_, Some(n)) => Size::Lines(n),
            (weight, None) => Size::Weight(weight.unwrap_or(1)),
        };

        let panel_path = format!("{path}.panel");
        let specs = tables(
            row,
            "panel",
            &panel_path,
            "an array of [[row.panel]] tables",
        )?;
        if specs.is_empty() {
            return Err(wrong(panel_path, "at least one [[row.panel]] table"));
        }
        let panels = specs
            .into_iter()
            .enumerate()
            .map(|(p, panel)| parse_panel(panel, &format!("{panel_path}[{p}]")))
            .collect::<Result<Vec<_>, _>>()?;
        rows.push(RowSpec { size, panels });
    }

    let layout = Layout { rows };
    if layout.panels().count() > MAX_PANELS {
        return Err(wrong("row".to_string(), "at most 12 panels in all"));
    }
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::Code;

    const NOW: i64 = 1_700_000_000_000;

    fn scope() -> Vec<String> {
        vec!["default".to_string()]
    }

    fn wf(run: &str, kind: &str, queue: &str, start: i64) -> WorkflowRow {
        WorkflowRow {
            namespace: "default".into(),
            workflow_id: format!("wf-{run}"),
            run_id: run.into(),
            workflow_type: kind.into(),
            task_queue: queue.into(),
            status: WorkflowStatus::Failed,
            start_time: Some(start),
            close_time: None,
            history_length: 0,
        }
    }

    fn schedule(id: &str, paused: bool, next_run: Option<i64>) -> ScheduleRow {
        ScheduleRow {
            namespace: "default".into(),
            schedule_id: id.into(),
            workflow_type: "Nightly".into(),
            paused,
            notes: String::new(),
            spec: String::new(),
            next_run,
            recent_runs: 0,
        }
    }

    fn rows(rows: Vec<WorkflowRow>) -> Result<SourceData, Fault> {
        Ok(SourceData::Workflows { rows, more: false })
    }

    fn fault() -> Fault {
        Fault::rpc("ListWorkflowExecutions", Code::Unavailable, "down")
    }

    /// The builtin board with counts, three failures and two schedules loaded.
    fn loaded() -> Board {
        let mut board = Board::new(Layout::builtin(), &scope());
        board.apply(
            0,
            Ok(SourceData::Counts(StatusCounts::new(
                7,
                [(WorkflowStatus::Running, 4), (WorkflowStatus::Failed, 3)],
            ))),
        );
        board.apply(
            1,
            rows(vec![
                wf("a", "Order", "orders", 10),
                wf("b", "Order", "orders", 30),
                wf("c", "Refund", "billing", 20),
            ]),
        );
        board.apply(
            3,
            Ok(SourceData::Schedules(vec![
                schedule("nightly", false, Some(50)),
                schedule("weekly", true, None),
            ])),
        );
        board
    }

    fn panel(src: &str) -> Result<Layout, ConfigError> {
        parse_dashboard(&format!("[[row]]\n[[row.panel]]\n{src}"))
    }

    fn unknown_key(result: Result<Layout, ConfigError>) -> String {
        match result {
            Err(ConfigError::UnknownDashboardKey { key, .. }) => key,
            other => panic!("expected an unknown key, got {other:?}"),
        }
    }

    fn wrong_path(result: Result<Layout, ConfigError>) -> String {
        match result {
            Err(ConfigError::Type { path, .. }) => path,
            other => panic!("expected a type error, got {other:?}"),
        }
    }

    #[test]
    fn a_file_with_no_rows_is_an_empty_layout() {
        assert!(parse_dashboard("").unwrap().is_empty());
        assert!(parse_dashboard("# nothing yet\n").unwrap().is_empty());
    }

    #[test]
    fn the_documented_example_parses() {
        let layout = parse_dashboard(
            r#"
            [[row]]
            lines = 3

            [[row.panel]]
            kind = "counts"

            [[row]]
            height = 3

            [[row.panel]]
            kind  = "workflows"
            title = "Recent failures"
            query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
            since = "24h"
            width = 2

            [[row.panel]]
            kind  = "types"
            query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
            since = "24h"
            "#,
        )
        .unwrap();

        assert_eq!(layout.rows[0].size, Size::Lines(3));
        assert_eq!(layout.rows[1].size, Size::Weight(3));
        let recent = &layout.rows[1].panels[0];
        assert_eq!(recent.title(), "Recent failures");
        assert_eq!(recent.width, 2);
        assert_eq!(
            recent.kind,
            PanelKind::Workflows {
                query: query::PROBLEMS.to_string(),
                since_ms: Some(DAY_MS),
            }
        );
        assert_eq!(layout.rows[1].panels[1].title(), "Workflow types");
    }

    #[test]
    fn what_a_panel_leaves_out_gets_a_default() {
        let layout = panel("kind = \"queues\"").unwrap();
        assert_eq!(layout.rows[0].size, Size::Weight(1));
        let spec = &layout.rows[0].panels[0];
        assert_eq!((spec.width, spec.limit), (1, DEFAULT_LIMIT));
        assert!(spec.namespaces.is_empty());
        assert_eq!(
            spec.kind,
            PanelKind::Queues {
                query: RUNNING.to_string(),
                names: Vec::new(),
            }
        );

        let layout = panel("kind = \"schedules\"\nshow = \"paused\"\nlimit = 5").unwrap();
        let spec = &layout.rows[0].panels[0];
        assert_eq!(spec.kind, PanelKind::Schedules { show: Show::Paused });
        assert_eq!(spec.limit, 5);
        assert_eq!(spec.title(), "Paused schedules");
    }

    #[test]
    fn a_key_that_does_not_exist_is_an_error_at_every_level() {
        assert_eq!(unknown_key(parse_dashboard("rows = 1")), "rows");
        assert_eq!(
            unknown_key(parse_dashboard(
                "[[row]]\nheigth = 2\n[[row.panel]]\nkind = \"counts\""
            )),
            "heigth"
        );
        assert_eq!(
            unknown_key(panel("kind = \"workflows\"\nqeury = \"x\"")),
            "qeury"
        );
    }

    #[test]
    fn a_key_of_another_kind_of_panel_is_an_error() {
        assert_eq!(
            unknown_key(panel("kind = \"counts\"\nsince = \"1h\"")),
            "since"
        );
        assert_eq!(unknown_key(panel("kind = \"counts\"\nlimit = 3")), "limit");
        assert_eq!(
            unknown_key(panel("kind = \"schedules\"\nquery = \"x\"")),
            "query"
        );
        let message = panel("kind = \"workflows\"\nshow = \"all\"")
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("row[0].panel[0], a workflows panel"),
            "{message}"
        );
    }

    #[test]
    fn a_kind_must_be_given_and_must_exist() {
        assert_eq!(wrong_path(panel("title = \"x\"")), "row[0].panel[0].kind");
        assert_eq!(
            panel("kind = \"gauge\""),
            Err(ConfigError::BadPanelKind {
                path: "row[0].panel[0].kind".into(),
                value: "gauge".into(),
            })
        );
    }

    #[test]
    fn a_row_has_one_size_and_at_least_one_panel() {
        assert_eq!(
            wrong_path(parse_dashboard(
                "[[row]]\nheight = 2\nlines = 4\n[[row.panel]]\nkind = \"counts\""
            )),
            "row[0]"
        );
        assert_eq!(
            wrong_path(parse_dashboard("[[row]]\nlines = 4")),
            "row[0].panel"
        );
        assert_eq!(wrong_path(parse_dashboard("row = 3")), "row");
    }

    #[test]
    fn numbers_outside_their_range_are_errors() {
        assert_eq!(
            wrong_path(panel("kind = \"counts\"\nwidth = 0")),
            "row[0].panel[0].width"
        );
        assert_eq!(
            wrong_path(panel("kind = \"workflows\"\nlimit = 51")),
            "row[0].panel[0].limit"
        );
        assert_eq!(
            wrong_path(parse_dashboard(
                "[[row]]\nlines = 2\n[[row.panel]]\nkind = \"counts\""
            )),
            "row[0].lines"
        );
    }

    #[test]
    fn a_query_may_not_order_and_a_window_must_be_a_duration() {
        assert_eq!(
            wrong_path(panel(
                "kind = \"workflows\"\nquery = \"A = 'b' ORDER BY StartTime\""
            )),
            "row[0].panel[0].query"
        );
        assert_eq!(
            wrong_path(panel("kind = \"workflows\"\nsince = \"yesterday\"")),
            "row[0].panel[0].since"
        );
        assert_eq!(
            wrong_path(panel("kind = \"workflows\"\nsince = \"0h\"")),
            "row[0].panel[0].since"
        );
        assert_eq!(
            wrong_path(panel("kind = \"schedules\"\nshow = \"soon\"")),
            "row[0].panel[0].show"
        );
        assert_eq!(
            wrong_path(panel("kind = \"queues\"\nnames = [\"a\", 3]")),
            "row[0].panel[0].names"
        );
    }

    #[test]
    fn a_layout_holds_a_bounded_number_of_panels() {
        let mut src = String::from("[[row]]\n");
        for _ in 0..=MAX_PANELS {
            src.push_str("[[row.panel]]\nkind = \"counts\"\n");
        }
        assert_eq!(wrong_path(parse_dashboard(&src)), "row");
    }

    #[test]
    fn panels_asking_for_the_same_thing_share_a_source() {
        let board = Board::new(Layout::builtin(), &scope());
        assert_eq!(board.panel_count(), 5);
        assert_eq!(board.sources().len(), 4);
        assert_eq!(board.panels[1].source, board.panels[2].source);
        assert!(matches!(board.state(0), Some(Loadable::NotAsked)));
    }

    #[test]
    fn a_panel_with_its_own_namespaces_does_not_share() {
        let layout = parse_dashboard(
            r#"
            [[row]]
            [[row.panel]]
            kind = "counts"
            [[row.panel]]
            kind = "counts"
            namespaces = ["prod"]
            "#,
        )
        .unwrap();
        let board = Board::new(layout, &scope());
        assert_eq!(board.sources().len(), 2);
        assert_eq!(board.sources()[0].namespaces(), ["default"]);
        assert_eq!(board.sources()[1].namespaces(), ["prod"]);
    }

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
    fn a_list_is_newest_first_and_stops_at_its_limit() {
        let mut layout = Layout::builtin();
        layout.rows[1].panels[0].limit = 2;
        let mut board = Board::new(layout, &scope());
        board.apply(
            1,
            rows(vec![
                wf("a", "T", "q", 10),
                wf("b", "T", "q", 30),
                wf("c", "T", "q", 20),
            ]),
        );
        let runs: Vec<&str> = board
            .items(1)
            .iter()
            .map(|i| match i {
                Item::Workflow(row) => row.run_id.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(runs, ["b", "c"]);
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

    #[test]
    fn the_cursor_runs_through_the_panels_in_order() {
        let board = loaded();
        // counts 2, failures 3, types 2, queues 0 (not loaded), schedules 2
        assert_eq!(board.len(), 9);
        assert_eq!(board.locate(0), Some((0, 0)));
        assert_eq!(board.locate(2), Some((1, 0)));
        assert_eq!(board.locate(5), Some((2, 0)));
        assert_eq!(board.locate(7), Some((4, 0)));
        assert_eq!(board.locate(9), None);
        assert_eq!(board.first_of(3), 7);
        assert!(matches!(board.item(8), Some(Item::Schedule(s)) if s.schedule_id == "weekly"));
    }

    #[test]
    fn stepping_by_panel_skips_empty_ones_and_stops_at_the_ends() {
        let board = loaded();
        assert_eq!(board.panel_step(3, true), 5);
        assert_eq!(
            board.panel_step(5, true),
            7,
            "the empty queues panel is skipped"
        );
        assert_eq!(board.panel_step(8, true), 8);
        assert_eq!(board.panel_step(8, false), 5);
        assert_eq!(board.panel_step(1, false), 1);
    }

    #[test]
    fn the_cursor_follows_its_item_through_a_refresh() {
        let mut board = loaded();
        let anchor = board.anchor(3).unwrap();
        board.apply(
            1,
            rows(vec![
                wf("z", "Order", "orders", 99),
                wf("b", "Order", "orders", 30),
                wf("c", "Refund", "billing", 20),
            ]),
        );
        assert_eq!(board.reanchor(&anchor), 4, "run c moved down one");

        board.apply(1, rows(vec![wf("z", "Order", "orders", 99)]));
        assert_eq!(board.reanchor(&anchor), 2, "gone, so the panel's last item");

        board.apply(1, rows(Vec::new()));
        assert!(board.reanchor(&anchor) < board.len());
    }

    #[test]
    fn a_failed_refresh_keeps_the_answer_on_screen() {
        let mut board = loaded();
        board.begin_refresh();
        board.apply(1, Err(fault()));
        assert_eq!(board.items(1).len(), 3);
        assert_eq!(board.fault(1), Some(&fault()));
        assert_eq!(
            board.fault(2),
            Some(&fault()),
            "the panel sharing the source"
        );

        board.apply(1, rows(Vec::new()));
        assert_eq!(board.fault(1), None);

        board.apply(2, Err(fault()));
        assert!(matches!(board.state(3), Some(Loadable::Failed(_))));
        assert_eq!(board.fault(3), Some(&fault()));
    }

    #[test]
    fn begin_refresh_marks_only_what_has_nothing_to_show() {
        let mut board = loaded();
        board.begin_refresh();
        assert!(board.state(0).unwrap().value().is_some());
        assert!(board.state(3).unwrap().is_loading());
    }

    #[test]
    fn a_tally_over_a_partial_list_says_how_many_it_saw() {
        let mut board = loaded();
        assert_eq!(board.sampled(2), None);
        board.apply(
            1,
            Ok(SourceData::Workflows {
                rows: vec![wf("a", "T", "q", 1), wf("b", "T", "q", 2)],
                more: true,
            }),
        );
        assert_eq!(board.sampled(2), Some(2));
        assert_eq!(board.sampled(0), None);
    }

    #[test]
    fn an_item_opens_the_workflows_it_stands_for() {
        let mut board = loaded();
        board.apply(2, rows(vec![wf("r", "Order", "orders", 5)]));
        let window = format!(
            "{} AND StartTime > '{}'",
            query::PROBLEMS,
            to_rfc3339(NOW - DAY_MS)
        );

        assert_eq!(
            board.drill(1, NOW),
            Some(Drill::Query {
                namespaces: scope(),
                query: "ExecutionStatus = 'Failed'".into(),
            })
        );
        assert!(matches!(board.drill(2, NOW), Some(Drill::Workflow(row)) if row.run_id == "b"));
        assert_eq!(
            board.drill(5, NOW),
            Some(Drill::Query {
                namespaces: scope(),
                query: format!("{window} AND WorkflowType = 'Order'"),
            })
        );
        assert_eq!(
            board.drill(7, NOW),
            Some(Drill::Query {
                namespaces: scope(),
                query: format!("{RUNNING} AND TaskQueue = 'orders'"),
            })
        );
        assert_eq!(
            board.drill(8, NOW),
            Some(Drill::Schedule {
                namespace: "default".into(),
                schedule_id: "nightly".into(),
            })
        );
        assert_eq!(board.drill(99, NOW), None);
    }

    #[test]
    fn a_name_with_a_quote_in_it_cannot_be_opened() {
        let mut board = Board::new(Layout::builtin(), &scope());
        board.apply(1, rows(vec![wf("a", "It's", "q", 1)]));
        assert!(matches!(board.item(1), Some(Item::Type { .. })));
        assert_eq!(board.drill(1, NOW), None);
    }
}
