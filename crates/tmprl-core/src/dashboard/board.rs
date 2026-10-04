//! A board: a layout, the requests behind it, what has come back, and the cursor over it.

use super::compose::{Facts, Slot, compose};
use super::histogram::{Bucket, edges};
use super::layout::{Layout, PanelKind, PanelSpec};
use super::source::{
    Item, MAX_QUEUES, MAX_REASONS, MAX_TALLIES, QueueRef, Source, SourceData, items,
};
use crate::fault::Fault;
use crate::loadable::Loadable;
use crate::query;
use crate::workflow::{WorkflowRow, WorkflowStatus, by_start_time_desc};

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

/// A way for the cursor to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    Left,
    Right,
    Up,
    Down,
}

/// The item under the cursor, remembered across a refresh by what it is, not where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    panel: usize,
    spec: PanelSpec,
    index: usize,
    key: String,
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
    /// Whether the layout follows the data, and the panels it has shown with items in them.
    adaptive: bool,
    kept: Vec<Slot>,
    /// How many of the sources are counts of one name, added by `sync_tallies`.
    tallies: usize,
    /// When the board was last told the time. Its histograms are cut against it.
    clock: i64,
    /// The workflows a retrying panel is looking into now. One it has stopped looking
    /// into is not asked about again.
    watching: Vec<Source>,
}

impl Board {
    /// A board with the layout it was given, as `dashboard.toml` asks for.
    pub fn new(layout: Layout, scope: &[String]) -> Self {
        let mut board = Self {
            layout: Layout::default(),
            scope: scope.to_vec(),
            sources: Vec::new(),
            data: Vec::new(),
            faults: Vec::new(),
            panels: Vec::new(),
            adaptive: false,
            kept: Vec::new(),
            tallies: 0,
            clock: 0,
            watching: Vec::new(),
        };
        board.lay_out(layout);
        board
    }

    /// A board that chooses its panels from what its probes find. It starts as the builtin
    /// layout, whose four sources are the probes.
    pub fn adaptive(scope: &[String]) -> Self {
        let mut board = Self::new(Layout::builtin(), scope);
        board.adaptive = true;
        board
    }

    pub fn is_adaptive(&self) -> bool {
        self.adaptive
    }

    /// Let panels that have emptied go, as `R` does: the layout is being asked for afresh.
    pub fn forget(&mut self) {
        self.kept.clear();
        self.recompose();
    }

    fn lay_out(&mut self, layout: Layout) {
        let namespace = self.scope.first().map_or("", String::as_str).to_string();
        let mut panels = Vec::new();
        for spec in layout.panels() {
            let wanted = spec.source(&self.scope);
            let source = match self.sources.iter().position(|s| *s == wanted) {
                Some(at) => at,
                None => {
                    self.sources.push(wanted);
                    self.data.push(Loadable::NotAsked);
                    self.faults.push(None);
                    self.sources.len() - 1
                }
            };
            panels.push(Panel {
                items: items(spec, self.data[source].value(), &namespace),
                spec: spec.clone(),
                source,
            });
        }
        self.panels = panels;
        self.layout = layout;
        self.sync_tallies();
        self.sync_queues();
        self.sync_reasons();
        self.sync_buckets();
        self.sync_pending();
    }

    fn closing(row: &WorkflowRow) -> Option<Source> {
        match row.status {
            WorkflowStatus::Failed | WorkflowStatus::Terminated | WorkflowStatus::Canceled => {
                Some(Source::Close {
                    namespace: row.namespace.clone(),
                    workflow_id: row.workflow_id.clone(),
                    run_id: row.run_id.clone(),
                })
            }
            // A timeout's closing event carries no reason, and the rest did not end badly.
            WorkflowStatus::TimedOut
            | WorkflowStatus::Running
            | WorkflowStatus::Completed
            | WorkflowStatus::ContinuedAsNew
            | WorkflowStatus::Paused
            | WorkflowStatus::Unspecified => None,
        }
    }

    /// Add a request for why each listed workflow that ended badly did.
    fn sync_reasons(&mut self) {
        let asked = self
            .sources
            .iter()
            .filter(|s| matches!(s, Source::Close { .. }))
            .count();
        let mut missing: Vec<Source> = Vec::new();
        for item in self.panels.iter().flat_map(|p| p.items.iter()) {
            let Item::Workflow(row) = item else {
                continue;
            };
            if let Some(wanted) = Self::closing(row)
                && !self.sources.contains(&wanted)
                && !missing.contains(&wanted)
            {
                missing.push(wanted);
            }
        }
        for wanted in missing.into_iter().take(MAX_REASONS.saturating_sub(asked)) {
            self.sources.push(wanted);
            self.data.push(Loadable::NotAsked);
            self.faults.push(None);
        }
    }

    /// Tell the board the time, so its histograms cover the stretches that end now. A
    /// stretch that has just begun gets a request of its own.
    pub fn advance(&mut self, now_ms: i64) {
        self.clock = now_ms;
        self.sync_buckets();
        self.sync_pending();
    }

    fn bucket_sources(&self, panel: &Panel) -> Vec<Source> {
        let PanelKind::Histogram {
            query,
            window,
            bucket_ms,
        } = &panel.spec.kind
        else {
            return Vec::new();
        };
        if self.clock == 0 {
            return Vec::new();
        }
        let namespaces = self.sources[panel.source].namespaces().to_vec();
        edges(window, *bucket_ms, self.clock)
            .into_iter()
            .map(|(from_ms, to_ms)| Source::Bucket {
                namespaces: namespaces.clone(),
                query: query.clone(),
                by: window.by,
                from_ms,
                to_ms,
            })
            .collect()
    }

    fn bucket_of(&self, wanted: &Source) -> Option<Bucket> {
        let Source::Bucket { from_ms, to_ms, .. } = wanted else {
            return None;
        };
        let count = self
            .sources
            .iter()
            .position(|s| s == wanted)
            .and_then(|at| match self.data[at].value() {
                Some(SourceData::Counts(counts)) => Some(counts.total),
                _ => None,
            });
        Some(Bucket {
            from_ms: *from_ms,
            to_ms: *to_ms,
            count,
        })
    }

    /// Add a request for each stretch of a histogram that has none, and make a cursor stop
    /// of each stretch with something in it.
    fn sync_buckets(&mut self) {
        for at in 0..self.panels.len() {
            let wanted = self.bucket_sources(&self.panels[at]);
            if wanted.is_empty() {
                continue;
            }
            let items = wanted
                .iter()
                .filter_map(|source| self.bucket_of(source))
                .filter_map(|bucket| {
                    Some(Item::Bucket {
                        from_ms: bucket.from_ms,
                        to_ms: bucket.to_ms,
                        count: bucket.count.filter(|n| *n > 0)?,
                    })
                })
                .collect();
            self.panels[at].items = items;
            for source in wanted {
                if !self.sources.contains(&source) {
                    self.sources.push(source);
                    self.data.push(Loadable::NotAsked);
                    self.faults.push(None);
                }
            }
        }
    }

    /// Look into the workflows a retrying panel lists: add a request for what each is
    /// waiting on, and make a line of each one with an activity tried often enough.
    fn sync_pending(&mut self) {
        let mut watching = Vec::new();
        for at in 0..self.panels.len() {
            let PanelKind::Retrying { attempts, scan, .. } = self.panels[at].spec.kind else {
                continue;
            };
            let mut rows: Vec<WorkflowRow> = match self.data[self.panels[at].source].value() {
                Some(SourceData::Workflows { rows, .. }) => rows
                    .iter()
                    .filter(|row| row.status == WorkflowStatus::Running)
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            };
            // The longest running first: of one page, they are the likeliest to be stuck.
            rows.sort_by(|a, b| by_start_time_desc(b, a));
            rows.truncate(scan);

            let mut found: Vec<Item> = Vec::new();
            for row in rows {
                let wanted = Source::Pending {
                    namespace: row.namespace.clone(),
                    workflow_id: row.workflow_id.clone(),
                    run_id: row.run_id.clone(),
                };
                match self.sources.iter().position(|s| *s == wanted) {
                    Some(source) => {
                        if let Some(SourceData::Pending(pending)) = self.data[source].value()
                            && let Some(activity) = pending
                                .iter()
                                .filter(|activity| activity.attempt >= attempts)
                                .max_by_key(|activity| activity.attempt)
                        {
                            found.push(Item::Retry {
                                row,
                                activity: Box::new(activity.clone()),
                            });
                        }
                    }
                    None => {
                        self.sources.push(wanted.clone());
                        self.data.push(Loadable::NotAsked);
                        self.faults.push(None);
                    }
                }
                watching.push(wanted);
            }
            found.sort_by(|a, b| match (a, b) {
                (
                    Item::Retry {
                        activity: x,
                        row: p,
                    },
                    Item::Retry {
                        activity: y,
                        row: q,
                    },
                ) => y
                    .attempt
                    .cmp(&x.attempt)
                    .then_with(|| by_start_time_desc(q, p)),
                _ => std::cmp::Ordering::Equal,
            });
            found.truncate(self.panels[at].spec.limit);
            self.panels[at].items = found;
        }
        self.watching = watching;
    }

    /// Whether nothing on the board reads this source any more, so that asking for it
    /// again would be a request wasted. A workflow that has left the page a retrying panel
    /// looks into is the case.
    pub fn dormant(&self, source: usize) -> bool {
        matches!(
            self.sources.get(source),
            Some(wanted @ Source::Pending { .. }) if !self.watching.contains(wanted)
        )
    }

    /// Whether a retrying panel has yet to hear about some of the workflows it looks into,
    /// so that an empty one is not read as "nothing is retrying".
    pub fn looking(&self) -> bool {
        self.watching.iter().any(|wanted| {
            self.sources
                .iter()
                .position(|s| s == wanted)
                .is_none_or(|at| self.data[at].value().is_none() && self.data[at].error().is_none())
        })
    }

    /// A histogram's columns, oldest first, each with its count once that has arrived.
    pub fn buckets(&self, panel: usize) -> Vec<Bucket> {
        self.panels.get(panel).map_or_else(Vec::new, |panel| {
            self.bucket_sources(panel)
                .iter()
                .filter_map(|source| self.bucket_of(source))
                .collect()
        })
    }

    /// Why a listed workflow closed, once that has been asked and answered.
    pub fn reason(&self, row: &WorkflowRow) -> Option<&str> {
        let wanted = Self::closing(row)?;
        let at = self.sources.iter().position(|s| *s == wanted)?;
        match self.data[at].value()? {
            SourceData::Close(reason) => reason.as_deref(),
            SourceData::Counts(_)
            | SourceData::Workflows { .. }
            | SourceData::Schedules(_)
            | SourceData::Pending(_)
            | SourceData::Queue(_) => None,
        }
    }

    /// A tally over one page of a longer list names what is there but miscounts it. Give
    /// each name the count on record for it, and add a request for the ones that have none.
    fn sync_tallies(&mut self) {
        let Self {
            panels,
            sources,
            data,
            faults,
            tallies,
            ..
        } = self;
        let mut missing: Vec<Source> = Vec::new();
        for panel in panels.iter_mut() {
            let field = match panel.spec.kind {
                PanelKind::Types { .. } => "WorkflowType",
                PanelKind::Queues { .. } => "TaskQueue",
                PanelKind::Counts { .. }
                | PanelKind::Workflows { .. }
                | PanelKind::Histogram { .. }
                | PanelKind::Retrying { .. }
                | PanelKind::Schedules { .. } => continue,
            };
            let Source::Workflows {
                namespaces,
                query: filter,
                window,
            } = &sources[panel.source]
            else {
                continue;
            };
            let (namespaces, filter, window) = (namespaces.clone(), filter.clone(), *window);
            let partial = matches!(
                data[panel.source].value(),
                Some(SourceData::Workflows { more: true, .. })
            );
            for item in panel.items.iter_mut() {
                let (name, namespaces, count, exact) = match item {
                    Item::Type { name, count, exact } => (&*name, namespaces.clone(), count, exact),
                    Item::Queue(QueueRef {
                        namespace,
                        name,
                        running,
                        exact,
                        ..
                    }) => (&*name, vec![namespace.clone()], running, exact),
                    Item::Status { .. }
                    | Item::Retry { .. }
                    | Item::Workflow(_)
                    | Item::Schedule(_)
                    | Item::Bucket { .. } => continue,
                };
                if !partial {
                    *exact = true;
                    continue;
                }
                let Some(value) = query::quotable(name) else {
                    continue;
                };
                let wanted = Source::Counts {
                    namespaces,
                    query: query::and(&filter, &format!("{field} = '{value}'")),
                    window,
                };
                match sources.iter().position(|s| *s == wanted) {
                    Some(at) => {
                        if let Some(SourceData::Counts(counts)) = data[at].value() {
                            *count = usize::try_from(counts.total).unwrap_or(0);
                            *exact = true;
                        }
                    }
                    None if !missing.contains(&wanted) => missing.push(wanted),
                    None => {}
                }
            }
            if partial {
                panel.items.sort_by(|a, b| match (a, b) {
                    (
                        Item::Type {
                            name: a, count: x, ..
                        },
                        Item::Type {
                            name: b, count: y, ..
                        },
                    ) => y.cmp(x).then_with(|| a.cmp(b)),
                    (Item::Queue(a), Item::Queue(b)) => b
                        .running
                        .cmp(&a.running)
                        .then_with(|| a.name.cmp(&b.name))
                        .then_with(|| a.namespace.cmp(&b.namespace)),
                    _ => std::cmp::Ordering::Equal,
                });
            }
        }
        for wanted in missing
            .into_iter()
            .take(MAX_TALLIES.saturating_sub(*tallies))
        {
            sources.push(wanted);
            data.push(Loadable::NotAsked);
            faults.push(None);
            *tallies += 1;
        }
    }

    /// Give each listed queue the health on record for it, and add a request for the ones
    /// that have none yet.
    fn sync_queues(&mut self) {
        let Self {
            panels,
            sources,
            data,
            faults,
            ..
        } = self;
        let mut missing: Vec<Source> = Vec::new();
        for item in panels.iter_mut().flat_map(|p| p.items.iter_mut()) {
            let Item::Queue(queue) = item else {
                continue;
            };
            let wanted = Source::Queue {
                namespace: queue.namespace.clone(),
                name: queue.name.clone(),
            };
            match sources.iter().position(|s| *s == wanted) {
                Some(at) => {
                    queue.health = match data[at].value() {
                        Some(SourceData::Queue(health)) => Some(health.clone()),
                        _ => None,
                    }
                }
                None if !missing.contains(&wanted) => missing.push(wanted),
                None => {}
            }
        }
        let described = sources
            .iter()
            .filter(|s| matches!(s, Source::Queue { .. }))
            .count();
        for wanted in missing
            .into_iter()
            .take(MAX_QUEUES.saturating_sub(described))
        {
            sources.push(wanted);
            data.push(Loadable::NotAsked);
            faults.push(None);
        }
    }

    fn probe(&self, slot: Slot) -> Option<&SourceData> {
        let wanted = slot.spec().source(&self.scope);
        let at = self.sources.iter().position(|s| *s == wanted)?;
        self.data[at].value()
    }

    fn recompose(&mut self) {
        if !self.adaptive {
            return;
        }
        let rows = |slot: Slot| match self.probe(slot) {
            Some(SourceData::Workflows { rows, .. }) => Some(rows.as_slice()),
            _ => None,
        };
        let facts = Facts {
            failures: rows(Slot::Failures),
            running: rows(Slot::Queues),
            schedules: match self.probe(Slot::Schedules) {
                Some(SourceData::Schedules(rows)) => Some(rows.as_slice()),
                _ => None,
            },
        };
        let layout = compose(&facts, &self.kept);
        if layout != self.layout {
            self.lay_out(layout);
        }
        for slot in Slot::ALL {
            let spec = slot.spec();
            let shown = self
                .panels
                .iter()
                .any(|p| p.spec == spec && !p.items.is_empty());
            if shown && !self.kept.contains(&slot) {
                self.kept.push(slot);
            }
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
            | SourceData::Schedules(_)
            | SourceData::Queue(_)
            | SourceData::Pending(_)
            | SourceData::Close(_) => None,
        }
    }

    /// How many rows a tally found its names in, once every number beside them is a count.
    /// The counts are then true, but a name the sample missed is still missing.
    pub fn discovered(&self, panel: usize) -> Option<usize> {
        let sampled = self.sampled(panel)?;
        let panel = self.panels.get(panel)?;
        let tally = match panel.spec.kind {
            PanelKind::Types { .. } | PanelKind::Queues { .. } => true,
            PanelKind::Counts { .. }
            | PanelKind::Workflows { .. }
            | PanelKind::Histogram { .. }
            | PanelKind::Retrying { .. }
            | PanelKind::Schedules { .. } => false,
        };
        (tally && !panel.items.is_empty() && !panel.items.iter().any(Item::approximate))
            .then_some(sampled)
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
        self.recompose();
        self.sync_tallies();
        self.sync_queues();
        self.sync_reasons();
        self.sync_buckets();
        self.sync_pending();
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

    /// Whether a panel's items run left to right, as statuses and columns do, not down.
    fn across(&self, panel: usize) -> bool {
        match self.panels[panel].spec.kind {
            PanelKind::Counts { .. } | PanelKind::Histogram { .. } => true,
            PanelKind::Workflows { .. }
            | PanelKind::Types { .. }
            | PanelKind::Queues { .. }
            | PanelKind::Retrying { .. }
            | PanelKind::Schedules { .. } => false,
        }
    }

    /// Each row's first panel and how many it has.
    fn row_spans(&self) -> Vec<(usize, usize)> {
        self.layout
            .rows
            .iter()
            .scan(0, |next, row| {
                let first = *next;
                *next += row.panels.len();
                Some((first, row.panels.len()))
            })
            .collect()
    }

    /// The nearest panel with items in the same row, to the right or to the left.
    fn beside(&self, panel: usize, forward: bool) -> Option<usize> {
        let (first, len) = self
            .row_spans()
            .into_iter()
            .find(|(first, len)| (*first..first + len).contains(&panel))?;
        let has_items = |i: &usize| !self.panels[*i].items.is_empty();
        if forward {
            (panel + 1..first + len).find(has_items)
        } else {
            (first..panel).rev().find(has_items)
        }
    }

    /// The panel with items under or over this one: in the nearest row that has any, the
    /// one standing closest to this panel's middle.
    fn beyond(&self, panel: usize, forward: bool) -> Option<usize> {
        let rows = self.row_spans();
        let row = rows
            .iter()
            .position(|(first, len)| (*first..first + len).contains(&panel))?;
        // Where a panel's middle is across its row, from 0 to 1, by the widths it shares.
        let middle = |(first, len): (usize, usize), of: usize| -> f64 {
            let width = |i: usize| f64::from(self.panels[i].spec.width.max(1));
            let total: f64 = (first..first + len).map(width).sum();
            let before: f64 = (first..of).map(width).sum();
            (before + width(of) / 2.0) / total
        };
        let here = middle(rows[row], panel);
        let candidates: Box<dyn Iterator<Item = &(usize, usize)>> = if forward {
            Box::new(rows[row + 1..].iter())
        } else {
            Box::new(rows[..row].iter().rev())
        };
        for &(first, len) in candidates {
            let nearest = (first..first + len)
                .filter(|i| !self.panels[*i].items.is_empty())
                .min_by(|a, b| {
                    let away = |i: usize| (middle((first, len), i) - here).abs();
                    away(*a).total_cmp(&away(*b))
                });
            if nearest.is_some() {
                return nearest;
            }
        }
        None
    }

    /// Where the cursor goes from `cursor` one step that way, as the panels sit on screen.
    ///
    /// Along a panel it moves through the items: down a list, across a strip of statuses
    /// or columns. Against it, or past its end, it leaves for the panel on that side. It
    /// stays put when there is nothing that way.
    pub fn step(&self, cursor: usize, direction: Heading) -> usize {
        let Some((panel, index)) = self.locate(cursor) else {
            return cursor;
        };
        let len = self.panels[panel].items.len();
        let across = self.across(panel);
        let sideways = matches!(direction, Heading::Left | Heading::Right);
        let forward = matches!(direction, Heading::Right | Heading::Down);
        if sideways == across {
            if forward && index + 1 < len {
                return cursor + 1;
            }
            if !forward && index > 0 {
                return cursor - 1;
            }
        }
        let target = if sideways {
            self.beside(panel, forward)
        } else {
            self.beyond(panel, forward)
        };
        let Some(target) = target else {
            return cursor;
        };
        let last = self.panels[target].items.len() - 1;
        let at = match (sideways, self.across(target)) {
            // Into a strip from its side: the near end.
            (true, true) => {
                if forward {
                    0
                } else {
                    last
                }
            }
            // Into a list from its side: the line the cursor was on, when it came from one.
            (true, false) => {
                if across {
                    0
                } else {
                    index.min(last)
                }
            }
            // Into a strip from above or below: the newest column of a chart, the first
            // status of a count.
            (false, true) => match self.panels[target].spec.kind {
                PanelKind::Histogram { .. } => last,
                _ => 0,
            },
            // Into a list from above or below: the near end.
            (false, false) => {
                if forward {
                    0
                } else {
                    last
                }
            }
        };
        self.first_of(target) + at
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
            spec: self.panels[panel].spec.clone(),
            index,
            key: self.panels[panel].items[index].key(),
        })
    }

    /// Where the anchored item is now. When it is gone the cursor keeps its place in the
    /// panel, so a refresh never throws it across the screen.
    pub fn reanchor(&self, anchor: &Anchor) -> usize {
        // By what the panel is, since an adaptive layout moves panels as they fill.
        let panel = self
            .panels
            .iter()
            .position(|p| p.spec == anchor.spec)
            .unwrap_or(anchor.panel.min(self.panels.len().saturating_sub(1)));
        let items = self.items(panel);
        let index = items
            .iter()
            .position(|i| i.key() == anchor.key)
            .unwrap_or(anchor.index.min(items.len().saturating_sub(1)));
        (self.first_of(panel) + index).min(self.len().saturating_sub(1))
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
            (Item::Workflow(row), _) | (Item::Retry { row, .. }, _) => Drill::Workflow(row.clone()),
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
            (Item::Bucket { from_ms, to_ms, .. }, kind) => {
                let PanelKind::Histogram { query, window, .. } = kind else {
                    return None;
                };
                Drill::Query {
                    query: Source::Bucket {
                        namespaces: namespaces.clone(),
                        query: query.clone(),
                        by: window.by,
                        from_ms: *from_ms,
                        to_ms: *to_ms,
                    }
                    .query(now_ms),
                    namespaces,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::fixtures::*;
    use crate::dashboard::layout::{DAY_MS, MAX_LIMIT, RUNNING, Window};
    use crate::dashboard::parse_dashboard;
    use crate::dashboard::source::QueueRef;
    use crate::pending::PendingActivity;
    use crate::taskqueue::QueueHealth;
    use crate::timerange::to_rfc3339;
    use crate::workflow::{StatusCounts, WorkflowStatus};

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

    /// The builtin board whose failures came back as one page of more.
    fn sampled() -> Board {
        let mut board = loaded();
        board.apply(
            1,
            Ok(SourceData::Workflows {
                rows: vec![
                    wf("a", "Order", "orders", 10),
                    wf("b", "Order", "orders", 30),
                    wf("c", "Refund", "billing", 20),
                ],
                more: true,
            }),
        );
        board
    }

    fn count_of(board: &Board, name: &str) -> usize {
        let wanted = format!("{} AND WorkflowType = '{name}'", query::PROBLEMS);
        board
            .sources()
            .iter()
            .position(|s| matches!(s, Source::Counts { query, .. } if *query == wanted))
            .unwrap_or_else(|| panic!("nothing counts {name}"))
    }

    fn names_counted(board: &Board) -> usize {
        board
            .sources()
            .iter()
            .filter(|s| matches!(s, Source::Counts { query, .. } if query.contains("WorkflowType")))
            .count()
    }

    fn total(n: i64) -> Result<SourceData, Fault> {
        Ok(SourceData::Counts(StatusCounts::new(n, [])))
    }

    fn tallies(board: &Board, panel: usize) -> Vec<(String, usize, bool)> {
        board
            .items(panel)
            .iter()
            .map(|item| match item {
                Item::Type { name, count, exact } => (name.clone(), *count, *exact),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_tally_of_a_whole_list_is_already_a_count() {
        let board = loaded();
        assert_eq!(names_counted(&board), 0);
        assert_eq!(
            tallies(&board, 2),
            [
                ("Order".to_string(), 2, true),
                ("Refund".to_string(), 1, true)
            ]
        );
        assert_eq!(board.discovered(2), None);
    }

    #[test]
    fn a_tally_of_a_sample_asks_for_a_count_of_each_name_it_found() {
        let board = sampled();
        assert_eq!(names_counted(&board), 2);
        assert_eq!(
            board.sources()[count_of(&board, "Order")],
            Source::Counts {
                namespaces: scope(),
                query: format!("{} AND WorkflowType = 'Order'", query::PROBLEMS),
                window: Window::since(DAY_MS),
            }
        );
        assert!(board.items(2).iter().all(Item::approximate));
        assert_eq!(board.discovered(2), None);
    }

    #[test]
    fn the_counts_replace_the_tally_and_put_the_most_first() {
        let mut board = sampled();
        let (order, refund) = (count_of(&board, "Order"), count_of(&board, "Refund"));
        board.apply(refund, total(900));
        assert_eq!(
            tallies(&board, 2),
            [
                ("Refund".to_string(), 900, true),
                ("Order".to_string(), 2, false)
            ]
        );
        assert_eq!(board.discovered(2), None, "one is still a tally");

        board.apply(order, total(40));
        assert_eq!(
            tallies(&board, 2),
            [
                ("Refund".to_string(), 900, true),
                ("Order".to_string(), 40, true)
            ]
        );
        assert_eq!(board.discovered(2), Some(3));
        assert_eq!(
            board.sampled(1),
            Some(3),
            "the list beside it is still a sample"
        );
        assert_eq!(board.discovered(1), None);
    }

    #[test]
    fn a_refreshed_sample_keeps_the_counts_it_already_has() {
        let mut board = sampled();
        let order = count_of(&board, "Order");
        board.apply(order, total(40));
        board.apply(
            1,
            Ok(SourceData::Workflows {
                rows: vec![wf("d", "Order", "orders", 40)],
                more: true,
            }),
        );
        assert_eq!(tallies(&board, 2), [("Order".to_string(), 40, true)]);
        assert_eq!(names_counted(&board), 2, "nothing is asked for twice");
    }

    #[test]
    fn a_count_that_fails_leaves_the_tally_marked_as_one() {
        let mut board = sampled();
        let order = count_of(&board, "Order");
        board.apply(order, Err(fault()));
        assert_eq!(tallies(&board, 2)[0], ("Order".to_string(), 2, false));
    }

    #[test]
    fn a_name_a_query_cannot_hold_and_names_past_the_cap_stay_tallies() {
        let layout =
            parse_dashboard("[[row]]\n[[row.panel]]\nkind = \"types\"\nlimit = 50").unwrap();
        let mut board = Board::new(layout, &scope());
        let mut rows = vec![wf("q", "O'Brien", "q", 1)];
        rows.extend(
            (0..MAX_TALLIES + 5).map(|i| wf(&format!("r{i}"), &format!("T{i:02}"), "q", 2)),
        );
        board.apply(0, Ok(SourceData::Workflows { rows, more: true }));
        assert_eq!(board.items(0).len(), MAX_TALLIES + 6);
        assert_eq!(board.sources().len(), 1 + MAX_TALLIES);
        assert!(
            !board
                .sources()
                .iter()
                .any(|s| matches!(s, Source::Counts { query, .. } if query.contains("Brien")))
        );
    }

    fn closing(board: &Board, run: &str) -> Option<usize> {
        board
            .sources()
            .iter()
            .position(|s| matches!(s, Source::Close { run_id, .. } if run_id == run))
    }

    #[test]
    fn a_listed_failure_is_asked_why_it_closed() {
        let mut board = loaded();
        let a = closing(&board, "a").expect("a failed");
        let row = wf("a", "Order", "orders", 10);
        assert_eq!(board.reason(&row), None, "not answered yet");

        board.apply(a, Ok(SourceData::Close(Some("boom".into()))));
        assert_eq!(board.reason(&row), Some("boom"));

        board.apply(a, Err(fault()));
        assert_eq!(board.reason(&row), Some("boom"), "a failed retry keeps it");
        assert!(board.sources()[a].settles(NOW));
        assert!(!board.sources()[0].settles(NOW));
    }

    #[test]
    fn only_a_workflow_that_ended_badly_with_a_reason_to_give_is_asked() {
        let layout = parse_dashboard("[[row]]\n[[row.panel]]\nkind = \"workflows\"").unwrap();
        let mut board = Board::new(layout, &scope());
        let with = |run: &str, status: WorkflowStatus| WorkflowRow {
            status,
            ..wf(run, "T", "q", 1)
        };
        board.apply(
            0,
            rows(vec![
                with("failed", WorkflowStatus::Failed),
                with("terminated", WorkflowStatus::Terminated),
                with("canceled", WorkflowStatus::Canceled),
                with("timedout", WorkflowStatus::TimedOut),
                with("running", WorkflowStatus::Running),
                with("completed", WorkflowStatus::Completed),
            ]),
        );
        let asked: Vec<bool> = [
            "failed",
            "terminated",
            "canceled",
            "timedout",
            "running",
            "completed",
        ]
        .iter()
        .map(|run| closing(&board, run).is_some())
        .collect();
        assert_eq!(asked, [true, true, true, false, false, false]);

        board.apply(0, rows(vec![with("failed", WorkflowStatus::Failed)]));
        assert_eq!(board.sources().len(), 4, "the same run is not asked twice");
    }

    #[test]
    fn a_board_asks_why_of_a_bounded_number_of_workflows() {
        let layout = parse_dashboard(
            "[[row]]\n[[row.panel]]\nkind = \"workflows\"\nlimit = 50\n\
             [[row.panel]]\nkind = \"workflows\"\nquery = \"A = 'b'\"\nlimit = 50",
        )
        .unwrap();
        let mut board = Board::new(layout, &scope());
        let page = |prefix: &str| {
            rows(
                (0..30)
                    .map(|i| wf(&format!("{prefix}{i}"), "T", "q", i))
                    .collect(),
            )
        };
        board.apply(0, page("x"));
        board.apply(1, page("y"));
        assert_eq!(board.sources().len(), 2 + MAX_REASONS);
    }

    #[test]
    fn the_cursor_goes_the_way_the_key_points() {
        // Status over failures (wide) and types, over queues (empty) and schedules.
        // 0 1: statuses. 2 3 4: failures. 5 6: types. 7 8: schedules.
        let board = loaded();
        let go = |from: usize, path: &[Heading]| path.iter().fold(from, |at, h| board.step(at, *h));
        use Heading::{Down, Left, Right, Up};

        assert_eq!(go(0, &[Right]), 1, "along the strip of statuses");
        assert_eq!(go(0, &[Right, Right]), 1, "and no further than its end");
        assert_eq!(go(1, &[Left]), 0);
        assert_eq!(go(0, &[Left]), 0);

        assert_eq!(
            go(1, &[Down]),
            2,
            "out of the strip, onto the list under it"
        );
        assert_eq!(go(2, &[Down, Down]), 4, "down the list");
        assert_eq!(go(4, &[Down]), 7, "past its end, to the row below");
        assert_eq!(go(8, &[Down]), 8, "nothing under the last row");
        assert_eq!(go(7, &[Up]), 6, "up into the panel over it, at its foot");
        assert_eq!(go(5, &[Up]), 0);
        assert_eq!(go(0, &[Up]), 0);

        assert_eq!(go(3, &[Right]), 6, "sideways keeps the line");
        assert_eq!(go(4, &[Right]), 6, "or the last one, in a shorter list");
        assert_eq!(go(6, &[Left]), 3);
        assert_eq!(go(3, &[Left]), 3, "nothing to the left of the first panel");
        assert_eq!(go(7, &[Left]), 7, "an empty panel is not a place to go");
    }

    fn retrying() -> Board {
        let layout =
            parse_dashboard("[[row]]\n[[row.panel]]\nkind = \"retrying\"\nattempts = 3\nscan = 2")
                .unwrap();
        let mut board = Board::new(layout, &scope());
        let running = |run: &str, start: i64| WorkflowRow {
            status: WorkflowStatus::Running,
            ..wf(run, "T", "q", start)
        };
        board.apply(
            0,
            rows(vec![
                running("new", 30),
                running("old", 10),
                running("mid", 20),
            ]),
        );
        board
    }

    fn pending_of(board: &Board, run: &str) -> Option<usize> {
        board
            .sources()
            .iter()
            .position(|s| matches!(s, Source::Pending { run_id, .. } if run_id == run))
    }

    fn tried(kind: &str, attempt: i32) -> PendingActivity {
        PendingActivity {
            activity_type: kind.into(),
            attempt,
            ..PendingActivity::default()
        }
    }

    #[test]
    fn a_retrying_panel_looks_into_the_longest_running_of_what_it_lists() {
        let board = retrying();
        assert!(pending_of(&board, "old").is_some());
        assert!(pending_of(&board, "mid").is_some());
        assert!(pending_of(&board, "new").is_none(), "past `scan`");
        assert!(board.is_empty(), "nothing is known to be retrying yet");
    }

    #[test]
    fn a_workflow_is_listed_once_an_activity_has_been_tried_often_enough() {
        let mut board = retrying();
        let (old, mid) = (
            pending_of(&board, "old").unwrap(),
            pending_of(&board, "mid").unwrap(),
        );
        board.apply(
            old,
            Ok(SourceData::Pending(vec![
                tried("Charge", 2),
                tried("Notify", 1),
            ])),
        );
        assert!(board.is_empty(), "two tries is under the threshold");

        board.apply(
            mid,
            Ok(SourceData::Pending(vec![
                tried("Charge", 3),
                tried("Score", 9),
            ])),
        );
        board.apply(old, Ok(SourceData::Pending(vec![tried("Charge", 4)])));
        let shown: Vec<(&str, &str, i32)> = board
            .items(0)
            .iter()
            .map(|item| match item {
                Item::Retry { row, activity } => (
                    row.run_id.as_str(),
                    activity.activity_type.as_str(),
                    activity.attempt,
                ),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            shown,
            [("mid", "Score", 9), ("old", "Charge", 4)],
            "the furthest along first, and of one workflow its worst activity"
        );
        assert!(matches!(board.drill(0, NOW), Some(Drill::Workflow(row)) if row.run_id == "mid"));
    }

    #[test]
    fn a_workflow_that_leaves_the_list_is_no_longer_asked_about() {
        let mut board = retrying();
        let old = pending_of(&board, "old").unwrap();
        assert!(!board.dormant(old));
        assert!(
            !board.dormant(0),
            "only what the board added can go dormant"
        );

        let running = WorkflowRow {
            status: WorkflowStatus::Running,
            ..wf("mid", "T", "q", 20)
        };
        board.apply(0, rows(vec![running]));
        assert!(board.dormant(old));
        assert!(!board.dormant(pending_of(&board, "mid").unwrap()));
    }

    const HOUR: i64 = 3_600_000;

    #[test]
    fn a_chart_is_walked_sideways_and_entered_at_its_newest_column() {
        let layout = parse_dashboard(
            "[[row]]\n[[row.panel]]\nkind = \"counts\"\n\
             [[row]]\n[[row.panel]]\nkind = \"histogram\"\nsince = \"3h\"\nbucket = \"1h\"",
        )
        .unwrap();
        let mut board = Board::new(layout, &scope());
        let now = (NOW / HOUR) * HOUR + HOUR / 3;
        board.advance(now);
        board.apply(
            0,
            Ok(SourceData::Counts(StatusCounts::new(
                9,
                [(WorkflowStatus::Running, 9)],
            ))),
        );
        let hour = (now / HOUR) * HOUR;
        for back in [3, 1, 0] {
            board.apply(column(&board, hour - back * HOUR), total(4));
        }
        // 0: the status. 1 2 3: the columns with something in them, oldest first.
        assert_eq!(board.len(), 4);
        assert_eq!(board.step(0, Heading::Down), 3, "the newest column");
        assert_eq!(board.step(3, Heading::Left), 2);
        assert_eq!(board.step(1, Heading::Left), 1);
        assert_eq!(board.step(2, Heading::Down), 2, "a chart has no down");
        assert_eq!(board.step(2, Heading::Up), 0);
    }

    /// One histogram of what closed in the last three hours, told it is twenty past.
    fn charted() -> (Board, i64) {
        let layout = parse_dashboard(
            "[[row]]\n[[row.panel]]\nkind = \"histogram\"\nquery = \"A = 'b'\"\n\
             since = \"3h\"\nbucket = \"1h\"\nby = \"close\"",
        )
        .unwrap();
        let mut board = Board::new(layout, &scope());
        let now = (NOW / HOUR) * HOUR + HOUR / 3;
        board.advance(now);
        (board, now)
    }

    fn column(board: &Board, from_ms: i64) -> usize {
        board
            .sources()
            .iter()
            .position(|s| matches!(s, Source::Bucket { from_ms: at, .. } if *at == from_ms))
            .unwrap()
    }

    #[test]
    fn a_histogram_asks_for_a_count_of_each_stretch_once_it_knows_the_time() {
        let layout =
            parse_dashboard("[[row]]\n[[row.panel]]\nkind = \"histogram\"\nsince = \"3h\"")
                .unwrap();
        let untold = Board::new(layout, &scope());
        assert_eq!(untold.sources().len(), 1, "only the window's own count");
        assert!(untold.buckets(0).is_empty());

        let (board, now) = charted();
        let hour = (now / HOUR) * HOUR;
        assert_eq!(board.sources().len(), 1 + 4);
        assert_eq!(
            board.buckets(0),
            [3, 2, 1, 0].map(|back| Bucket {
                from_ms: hour - back * HOUR,
                to_ms: hour - back * HOUR + HOUR,
                count: None,
            })
        );
        assert!(
            board.is_empty(),
            "nothing counted, nothing to put the cursor on"
        );
        assert_eq!(
            board.sources()[column(&board, hour)].query(now),
            format!(
                "A = 'b' AND CloseTime >= '{}' AND CloseTime < '{}'",
                to_rfc3339(hour),
                to_rfc3339(hour + HOUR)
            )
        );
    }

    #[test]
    fn a_stretch_with_something_in_it_is_a_stop_that_opens_its_workflows() {
        let (mut board, now) = charted();
        let hour = (now / HOUR) * HOUR;
        board.apply(column(&board, hour - 2 * HOUR), total(0));
        board.apply(column(&board, hour - HOUR), total(7));
        assert_eq!(
            board.items(0),
            [Item::Bucket {
                from_ms: hour - HOUR,
                to_ms: hour,
                count: 7,
            }]
        );
        assert_eq!(board.buckets(0)[1].count, Some(0));
        assert_eq!(
            board.drill(0, now),
            Some(Drill::Query {
                namespaces: scope(),
                query: format!(
                    "A = 'b' AND CloseTime >= '{}' AND CloseTime < '{}'",
                    to_rfc3339(hour - HOUR),
                    to_rfc3339(hour)
                ),
            })
        );
    }

    #[test]
    fn a_stretch_that_has_passed_is_settled_and_time_moving_on_adds_only_the_new_one() {
        let (mut board, now) = charted();
        let hour = (now / HOUR) * HOUR;
        assert!(board.sources()[column(&board, hour - HOUR)].settles(now));
        assert!(
            !board.sources()[column(&board, hour)].settles(now),
            "still filling"
        );
        assert!(!board.sources()[0].settles(now));

        board.apply(column(&board, hour - HOUR), total(7));
        board.advance(now + HOUR / 2);
        assert_eq!(board.sources().len(), 5, "the same stretches");

        board.advance(now + HOUR);
        assert_eq!(board.sources().len(), 6, "one new stretch");
        let shown = board.buckets(0);
        assert_eq!(shown.len(), 4);
        assert_eq!(shown[1].count, Some(7), "and what was counted moved along");
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

    fn board_titles(board: &Board) -> Vec<Vec<&str>> {
        titles(board.layout())
    }

    #[test]
    fn an_adaptive_board_reshapes_as_its_probes_answer() {
        let mut board = Board::adaptive(&scope());
        assert!(board.is_adaptive());
        assert_eq!(board.layout(), &Layout::builtin());

        board.apply(1, rows(Vec::new()));
        board.apply(2, rows(vec![wf("a", "Order", "orders", 1)]));
        board.apply(3, Ok(SourceData::Schedules(Vec::new())));
        assert_eq!(
            board_titles(&board),
            [vec!["Status"], vec!["Running"], vec!["Task queues"]]
        );
        assert_eq!(
            board.sources().iter().filter(|s| !s.settles(NOW)).count(),
            5,
            "the probes, and the one queue they found"
        );
        assert_eq!(board.len(), 2);
    }

    #[test]
    fn the_cursor_follows_its_item_when_the_layout_moves_under_it() {
        let mut board = Board::adaptive(&scope());
        board.apply(2, rows(vec![wf("a", "Order", "orders", 1)]));
        let anchor = board.anchor(0).unwrap();
        assert!(matches!(board.item(0), Some(Item::Queue(_))));

        board.apply(
            1,
            rows(vec![wf("x", "Order", "q", 9), wf("y", "Refund", "q", 8)]),
        );
        let at = board.reanchor(&anchor);
        assert!(matches!(board.item(at), Some(Item::Queue(_))), "{at}");
        assert_eq!(at, 4, "two failures and two types are above it now");
    }

    #[test]
    fn a_panel_empties_in_place_until_the_layout_is_asked_for_again() {
        let mut board = Board::adaptive(&scope());
        board.apply(1, rows(vec![wf("x", "Order", "q", 9)]));
        board.apply(2, rows(vec![wf("a", "Order", "orders", 1)]));
        board.apply(3, Ok(SourceData::Schedules(Vec::new())));
        assert_eq!(board_titles(&board)[2], ["Recent failures"]);

        board.apply(1, rows(Vec::new()));
        assert_eq!(
            board_titles(&board)[1],
            ["Recent failures"],
            "kept, though empty"
        );

        board.forget();
        assert_eq!(board_titles(&board)[1], ["Running"]);
    }

    #[test]
    fn a_configured_board_keeps_the_layout_it_was_given() {
        let mut board = Board::new(Layout::builtin(), &scope());
        board.apply(1, rows(Vec::new()));
        board.apply(2, rows(Vec::new()));
        board.apply(3, Ok(SourceData::Schedules(Vec::new())));
        board.forget();
        assert!(!board.is_adaptive());
        assert_eq!(board.layout(), &Layout::builtin());
    }

    fn queue(board: &Board, panel: usize, index: usize) -> &QueueRef {
        match &board.items(panel)[index] {
            Item::Queue(queue) => queue,
            other => panic!("{other:?}"),
        }
    }

    fn health(backlog: i64, pollers: usize) -> QueueHealth {
        QueueHealth {
            backlog: Some(backlog),
            pollers,
            ..QueueHealth::default()
        }
    }

    #[test]
    fn a_queue_a_panel_lists_gets_a_request_and_then_its_health() {
        let mut board = Board::new(Layout::builtin(), &scope());
        board.apply(
            2,
            rows(vec![wf("a", "T", "orders", 1), wf("b", "T", "billing", 2)]),
        );
        assert_eq!(
            board.sources()[4..],
            [
                Source::Queue {
                    namespace: "default".into(),
                    name: "billing".into()
                },
                Source::Queue {
                    namespace: "default".into(),
                    name: "orders".into()
                },
            ]
        );
        assert_eq!(queue(&board, 3, 0).health, None);

        board.apply(4, Ok(SourceData::Queue(health(12, 0))));
        assert_eq!(queue(&board, 3, 0).name, "billing");
        assert!(queue(&board, 3, 0).health.as_ref().unwrap().stuck());
        assert_eq!(queue(&board, 3, 1).health, None);
    }

    #[test]
    fn health_survives_the_list_it_hangs_on_being_refreshed() {
        let mut board = Board::new(Layout::builtin(), &scope());
        board.apply(2, rows(vec![wf("a", "T", "orders", 1)]));
        board.apply(4, Ok(SourceData::Queue(health(0, 3))));
        board.apply(
            2,
            rows(vec![wf("a", "T", "orders", 1), wf("b", "T", "orders", 2)]),
        );
        assert_eq!(queue(&board, 3, 0).running, 2);
        assert_eq!(queue(&board, 3, 0).health, Some(health(0, 3)));
        assert_eq!(
            board.sources().len(),
            5,
            "the same queue is not asked for twice"
        );
    }

    #[test]
    fn only_so_many_queues_are_described() {
        let mut layout = Layout::builtin();
        layout.rows[2].panels[0].limit = MAX_LIMIT;
        let mut board = Board::new(layout, &scope());
        let many: Vec<WorkflowRow> = (0..MAX_QUEUES + 3)
            .map(|i| wf(&format!("r{i}"), "T", &format!("queue-{i:02}"), i as i64))
            .collect();
        board.apply(2, rows(many));
        assert_eq!(board.items(3).len(), MAX_QUEUES + 3, "all are listed");
        assert_eq!(board.sources().len(), 4 + MAX_QUEUES);
    }
}
