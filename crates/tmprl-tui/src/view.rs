//! What one pane owns.

use std::cell::{Cell, RefCell};

use tmprl_client::{NamespaceInfo, ProfileSummary};
use tmprl_core::dashboard::{Board, Item, Pacer};
use tmprl_core::history::NormalizedEvent;
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::{FlatCache, Payload};
use tmprl_core::pending::PendingActivity;
use tmprl_core::search::Search;
use tmprl_core::{Loadable, ScheduleRow, StatusCounts, WorkflowList, WorkflowRow};

use crate::app::Screen;
use crate::ui::category_label;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowsDrawn {
    pub area: tmprl_ui::Rect,
    pub first: usize,
}

impl RowsDrawn {
    pub fn row_at(&self, column: u16, line: u16) -> Option<usize> {
        let a = self.area;
        (column >= a.x && column < a.right() && line >= a.y && line < a.bottom())
            .then(|| self.first + (line - a.y) as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub from: (u16, u16),
    pub to: (u16, u16),
}

impl TextSelection {
    pub fn lines(&self, area: tmprl_ui::Rect) -> Vec<(u16, u16, u16)> {
        let key = |(x, y): (u16, u16)| (y, x);
        let (start, end) = if key(self.from) <= key(self.to) {
            (self.from, self.to)
        } else {
            (self.to, self.from)
        };
        let last_column = area.right().saturating_sub(1);
        (start.1..=end.1)
            .map(|y| {
                let first = if y == start.1 { start.0 } else { area.x };
                let last = if y == end.1 { end.0 } else { last_column };
                (y, first, last)
            })
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct DrawnText {
    pub area: tmprl_ui::Rect,
    pub cells: Vec<Vec<String>>,
}

impl DrawnText {
    pub fn text(&self, selection: &TextSelection) -> String {
        selection
            .lines(self.area)
            .into_iter()
            .filter_map(|(y, first, last)| {
                let row = self.cells.get(y.checked_sub(self.area.y)? as usize)?;
                let from = first.saturating_sub(self.area.x) as usize;
                let to = (last.saturating_sub(self.area.x) as usize + 1).min(row.len());
                Some(row.get(from..to)?.concat().trim_end().to_string())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct View {
    pub screen: Screen,
    pub profiles: Loadable<Vec<ProfileSummary>>,
    pub namespaces: Loadable<Vec<NamespaceInfo>>,
    pub workflows: Loadable<WorkflowList>,
    pub counts: Loadable<StatusCounts>,
    pub history: Loadable<Outline>,
    pub schedules: Loadable<Vec<ScheduleRow>>,
    pub dashboard: Option<Board>,
    pub dashboard_pacer: Pacer,
    pub dashboard_tasks: Vec<tokio::task::JoinHandle<()>>,
    pub seek_schedule: Option<String>,

    pub viewing: Option<WorkflowRow>,
    pub following: bool,
    pub timeline: bool,
    pub timeline_gaps_open: bool,
    pub show_detail: bool,
    pub piped: Option<Result<String, String>>,
    pub queried: Option<Vec<(String, Payload)>>,
    pub handlers: Option<tmprl_core::interaction::Handlers>,
    pub detail_scroll: usize,
    pub detail_max_scroll: usize,
    pub detail_focus: bool,
    pub detail_beside: bool,
    pub detail_page: usize,
    pub detail_area: tmprl_ui::Rect,
    pub body_area: tmprl_ui::Rect,
    pub rows_drawn: Cell<Option<RowsDrawn>>,
    pub detail_text: RefCell<DrawnText>,

    pub query: String,
    pub scope: Vec<String>,

    pub cursor: usize,
    pub anchor: Option<usize>,
    pub page: usize,

    pub cursor_key: Option<(String, String)>,
    pub namespace_cursor: usize,
    pub workflow_cursor: usize,
    pub generation: u64,
    pub history_events: Vec<NormalizedEvent>,
    pub history_token: Vec<u8>,
    pub history_resume: Vec<u8>,
    pub follow_task: Option<tokio::task::JoinHandle<()>>,
    pub pending: Vec<PendingActivity>,
    pub pending_task: Option<tokio::task::JoinHandle<()>>,
    pub loading_more: bool,
    pub detail_seek: bool,
    flat: RefCell<FlatCache>,
}

impl View {
    pub fn new(namespace: &str) -> Self {
        Self {
            screen: Screen::Namespaces,
            profiles: Loadable::NotAsked,
            namespaces: Loadable::NotAsked,
            workflows: Loadable::NotAsked,
            counts: Loadable::NotAsked,
            history: Loadable::NotAsked,
            schedules: Loadable::NotAsked,
            dashboard: None,
            dashboard_pacer: Pacer::default(),
            dashboard_tasks: Vec::new(),
            seek_schedule: None,
            viewing: None,
            following: false,
            timeline: false,
            timeline_gaps_open: false,
            show_detail: false,
            piped: None,
            queried: None,
            handlers: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            detail_focus: false,
            detail_beside: false,
            detail_page: 0,
            detail_area: tmprl_ui::Rect::default(),
            body_area: tmprl_ui::Rect::default(),
            rows_drawn: Cell::new(None),
            detail_text: RefCell::default(),
            query: String::new(),
            scope: vec![namespace.to_string()],
            cursor: 0,
            anchor: None,
            page: 10,
            cursor_key: None,
            namespace_cursor: 0,
            workflow_cursor: 0,
            generation: 0,
            history_events: Vec::new(),
            history_token: Vec::new(),
            history_resume: Vec::new(),
            follow_task: None,
            pending: Vec::new(),
            pending_task: None,
            loading_more: false,
            detail_seek: false,
            flat: RefCell::default(),
        }
    }

    pub fn fork(&self) -> Self {
        let mut out = View::new(self.scope.first().map(String::as_str).unwrap_or_default());
        out.screen = self.screen;
        out.scope = self.scope.clone();
        out.query = self.query.clone();
        out.viewing = self.viewing.clone();
        out.show_detail = self.show_detail;
        out.timeline = self.timeline;
        out.timeline_gaps_open = self.timeline_gaps_open;
        out.namespace_cursor = self.namespace_cursor;
        out.workflow_cursor = self.workflow_cursor;
        out
    }

    pub fn stop_following(&mut self) {
        self.following = false;
        for task in [self.follow_task.take(), self.pending_task.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
        self.stop_dashboard();
    }

    pub fn stop_dashboard(&mut self) {
        for task in self.dashboard_tasks.drain(..) {
            task.abort();
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.stop_following();
    }
}

impl View {
    pub fn profile_rows(&self) -> &[ProfileSummary] {
        self.profiles.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn namespace_rows(&self) -> &[NamespaceInfo] {
        self.namespaces.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn workflow_rows(&self) -> &[WorkflowRow] {
        self.workflows
            .value()
            .map(WorkflowList::rows)
            .unwrap_or(&[])
    }

    pub fn schedule_rows(&self) -> &[ScheduleRow] {
        self.schedules.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn dashboard_items(&self) -> impl Iterator<Item = &Item> {
        self.dashboard
            .iter()
            .flat_map(|b| (0..b.panel_count()).flat_map(move |p| b.items(p)))
    }

    pub fn search_labels(&self) -> Vec<String> {
        let mut labels = self.row_labels();
        if self.screen == Screen::History {
            self.append_payloads(&mut labels);
        } else {
            self.flat.borrow_mut().sweep();
        }
        labels
    }

    pub fn row_labels(&self) -> Vec<String> {
        match self.screen {
            Screen::Profiles => self
                .profile_rows()
                .iter()
                .map(|p| {
                    format!(
                        "{} {} {}",
                        p.name,
                        p.address.as_deref().unwrap_or_default(),
                        p.namespace.as_deref().unwrap_or_default()
                    )
                })
                .collect(),
            Screen::Namespaces => self
                .namespace_rows()
                .iter()
                .map(|n| format!("{} {} {}", n.name, n.state, n.description))
                .collect(),
            Screen::Workflows => self
                .workflow_rows()
                .iter()
                .map(|w| {
                    format!(
                        "{} {} {} {} {} {}",
                        w.workflow_id,
                        w.workflow_type,
                        w.task_queue,
                        w.status.query_name(),
                        w.namespace,
                        w.run_id,
                    )
                })
                .collect(),
            Screen::Schedules => self
                .schedule_rows()
                .iter()
                .map(|s| {
                    format!(
                        "{} {} {} {} {}",
                        s.schedule_id,
                        s.workflow_type,
                        s.spec,
                        s.notes,
                        if s.paused { "paused" } else { "running" },
                    )
                })
                .collect(),
            Screen::History => self.history_labels(),
            Screen::Dashboard => self.dashboard_items().map(Item::label).collect(),
        }
    }

    fn history_labels(&self) -> Vec<String> {
        let Some(outline) = self.history.value() else {
            return Vec::new();
        };
        (0..outline.len())
            .map(|row| history_label(outline, row))
            .collect()
    }

    fn append_payloads(&self, labels: &mut [String]) {
        let Some(outline) = self.history.value() else {
            return;
        };
        let mut cache = self.flat.borrow_mut();
        for (row, label) in labels.iter_mut().enumerate() {
            for e in payload_events(outline, row) {
                for (name, p) in &e.payloads {
                    for line in cache.lines(name, p) {
                        label.push(' ');
                        label.push_str(line);
                    }
                }
            }
        }
        cache.sweep();
    }

    pub fn payload_match(&self, row: usize, search: &Search) -> Option<String> {
        if self.screen != Screen::History {
            return None;
        }
        let outline = self.history.value()?;
        if search.matches(&history_label(outline, row)) {
            return None;
        }
        let cache = self.flat.borrow();
        for e in payload_events(outline, row) {
            for (name, p) in &e.payloads {
                let lines = match cache.peek(name, p) {
                    Some(lines) => lines.to_vec(),
                    None => tmprl_core::payload::flatten(name, p),
                };
                if let Some(line) = lines.iter().find(|l| search.matches(l)) {
                    let path = line.split_once('=').map_or(line.as_str(), |(path, _)| path);
                    return Some(path.to_string());
                }
            }
        }
        None
    }

    #[cfg(test)]
    pub fn cached_payloads(&self) -> usize {
        self.flat.borrow().len()
    }

    pub fn undecoded_payloads(&self) -> usize {
        self.history_events
            .iter()
            .flat_map(|e| &e.payloads)
            .filter(|(_, p)| p.needs_codec())
            .count()
    }

    pub fn row_count(&self) -> usize {
        match self.screen {
            Screen::Profiles => self.profile_rows().len(),
            Screen::Namespaces => self.namespace_rows().len(),
            Screen::Workflows => self.workflow_rows().len(),
            Screen::History => self.history.value().map(Outline::len).unwrap_or(0),
            Screen::Schedules => self.schedule_rows().len(),
            Screen::Dashboard => self.dashboard.as_ref().map_or(0, Board::len),
        }
    }

    pub fn is_fanned_out(&self) -> bool {
        self.scope.len() > 1
    }

    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.selection().is_some_and(|(lo, hi)| i >= lo && i <= hi)
    }
}

fn history_label(outline: &Outline, row: usize) -> String {
    match outline.row_at(row) {
        Some(Row::Group { group, .. }) => match outline.group(group) {
            Some(g) => {
                let mut s = format!("{} {}", category_label(g.category), g.subject);
                if let Some(f) = &g.failure {
                    s.push(' ');
                    s.push_str(&f.headline());
                }
                s
            }
            None => String::new(),
        },
        Some(Row::Event { event, .. }) => match outline.event(event) {
            Some(e) => {
                let mut s = format!("{} {}", e.id, e.name);
                for (k, v) in &e.fields {
                    s.push_str(&format!(" {k}={v}"));
                }
                if let Some(f) = &e.failure {
                    s.push(' ');
                    s.push_str(&f.headline());
                }
                s
            }
            None => String::new(),
        },
        None => String::new(),
    }
}

fn payload_events(outline: &Outline, row: usize) -> Vec<&NormalizedEvent> {
    match outline.row_at(row) {
        Some(Row::Group { group, .. }) => {
            let events = outline.events();
            outline
                .group(group)
                .map(|g| {
                    g.payload_ends()
                        .into_iter()
                        .filter_map(|id| {
                            events
                                .binary_search_by_key(&id, |e| e.id)
                                .ok()
                                .map(|i| &events[i])
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        Some(Row::Event { event, .. }) => outline.event(event).into_iter().collect(),
        None => Vec::new(),
    }
}
