//! What a dashboard is made of: rows of panels, and the builtin arrangement of them.

use super::compose::Slot;
use super::source::Source;
use crate::query;
use crate::timerange::to_rfc3339;

pub const MAX_PANELS: usize = 12;

pub const DEFAULT_LIMIT: usize = 10;

pub const MAX_LIMIT: usize = 50;

pub(super) const RUNNING: &str = "ExecutionStatus = 'Running'";

pub(super) const DAY_MS: i64 = 86_400_000;

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

/// The timestamp a [`Window`] measures against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeField {
    #[default]
    Start,
    Close,
}

impl TimeField {
    fn attribute(self) -> &'static str {
        match self {
            TimeField::Start => "StartTime",
            TimeField::Close => "CloseTime",
        }
    }
}

/// How far back a panel looks: no further than `since`, no nearer than `older`, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Window {
    pub since_ms: Option<i64>,
    pub older_ms: Option<i64>,
    pub by: TimeField,
}

impl Window {
    pub fn since(ms: i64) -> Self {
        Self {
            since_ms: Some(ms),
            ..Self::default()
        }
    }

    /// `filter` narrowed to the window. The grammar has no `now()`, so each bound becomes a
    /// literal instant every time this is asked.
    pub(super) fn narrow(&self, filter: &str, now_ms: i64) -> String {
        let field = self.by.attribute();
        let mut out = filter.trim().to_string();
        if let Some(ms) = self.since_ms {
            out = query::and(&out, &format!("{field} > '{}'", to_rfc3339(now_ms - ms)));
        }
        if let Some(ms) = self.older_ms {
            out = query::and(&out, &format!("{field} < '{}'", to_rfc3339(now_ms - ms)));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKind {
    Counts { query: String, window: Window },
    Workflows { query: String, window: Window },
    Types { query: String, window: Window },
    Queues { query: String, names: Vec<String> },
    Schedules { show: Show },
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
    pub(super) fn new(kind: PanelKind) -> Self {
        Self {
            kind,
            title: None,
            width: 1,
            limit: DEFAULT_LIMIT,
            namespaces: Vec::new(),
        }
    }

    pub(super) fn titled(mut self, title: &str) -> Self {
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

    pub(super) fn source(&self, scope: &[String]) -> Source {
        let namespaces = if self.namespaces.is_empty() {
            scope.to_vec()
        } else {
            self.namespaces.clone()
        };
        match &self.kind {
            PanelKind::Counts { query, window } => Source::Counts {
                namespaces,
                query: query.clone(),
                window: *window,
            },
            PanelKind::Workflows { query, window } | PanelKind::Types { query, window } => {
                Source::Workflows {
                    namespaces,
                    query: query.clone(),
                    window: *window,
                }
            }
            PanelKind::Queues { query, .. } => Source::Workflows {
                namespaces,
                query: query.clone(),
                window: Window::default(),
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
    /// The layout shown before anything is known about a namespace, and for one with
    /// nothing in it.
    pub fn builtin() -> Self {
        Self {
            rows: vec![
                RowSpec {
                    size: Size::Lines(3),
                    panels: vec![PanelSpec::new(PanelKind::Counts {
                        query: String::new(),
                        window: Window::default(),
                    })],
                },
                RowSpec {
                    size: Size::Weight(3),
                    panels: vec![Slot::Failures.spec(), Slot::Types.spec()],
                },
                RowSpec {
                    size: Size::Weight(2),
                    panels: vec![Slot::Queues.spec(), Slot::Schedules.spec()],
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
