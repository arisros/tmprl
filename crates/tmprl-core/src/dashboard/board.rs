//! A board: a layout, the requests behind it, what has come back, and the cursor over it.

use super::compose::{Facts, Slot, compose};
use super::layout::{Layout, PanelSpec};
use super::source::{Item, MAX_QUEUES, Source, SourceData, items};
use crate::fault::Fault;
use crate::loadable::Loadable;
use crate::query;
use crate::workflow::WorkflowRow;

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
        self.sync_queues();
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
            | SourceData::Queue(_) => None,
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
        self.recompose();
        self.sync_queues();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::fixtures::*;
    use crate::dashboard::layout::{DAY_MS, MAX_LIMIT, RUNNING};
    use crate::dashboard::parse_dashboard;
    use crate::dashboard::source::QueueRef;
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
            board.sources().len(),
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
        assert_eq!(board_titles(&board)[1], ["Recent failures"]);

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
