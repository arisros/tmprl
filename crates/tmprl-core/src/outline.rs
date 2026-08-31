//! The collapsible view over a grouped history, and the virtualisation that makes it
//! survive a large one.
//!
//! Histories routinely reach tens of thousands of events and pathological ones reach
//! millions, so nothing here ever materialises a list of rendered rows. The outline knows
//! how many rows it *would* have and can answer "what is row 84,102" without building rows
//! 0 to 84,101. Scrolling moves an index.
//!
//! The trick is one cumulative-offset table, rebuilt only when the shape changes — a group
//! expanded, plumbing toggled — and never per frame. Looking a row up is then a binary
//! search over that table.

use crate::history::{Category, Group, NormalizedEvent, Outcome};

/// One line on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A group's own line: the summary of several events.
    Group {
        /// Index into [`Outline::groups`].
        group: usize,
        expanded: bool,
    },
    /// One event inside an expanded group.
    Event {
        group: usize,
        /// Index into [`Outline::events`].
        event: usize,
    },
}

/// A grouped history, with expansion and filtering, addressable by row.
pub struct Outline {
    events: Vec<NormalizedEvent>,
    groups: Vec<Group>,
    /// Parallel to `groups`.
    expanded: Vec<bool>,
    /// Indices into `groups`, in display order, after filtering.
    visible: Vec<usize>,
    /// Rows before `visible[i]`. One longer than `visible`, so the last entry is the total
    /// row count — which is what makes `len()` free.
    offsets: Vec<usize>,
    /// Whether workflow-task groups are shown. They are the worker polling, and on a real
    /// history they are the majority of events and almost never what you came to read.
    show_plumbing: bool,
}

impl Outline {
    pub fn new(events: Vec<NormalizedEvent>, groups: Vec<Group>) -> Self {
        let expanded = vec![false; groups.len()];
        let mut o = Self {
            events,
            groups,
            expanded,
            visible: Vec::new(),
            offsets: Vec::new(),
            show_plumbing: false,
        };
        o.reindex();
        o
    }

    /// Swap in a re-grouped history after another page arrived, keeping what the reader
    /// has set up.
    ///
    /// History is append-only, so re-grouping the accumulated events yields the same groups
    /// in the same order plus new ones on the end — which is why expansion can be carried
    /// over by index. Rebuilding the outline from scratch on every page would silently fold
    /// shut whatever the reader had just opened.
    pub fn replace(&mut self, events: Vec<NormalizedEvent>, groups: Vec<Group>) {
        self.expanded.resize(groups.len(), false);
        self.expanded.truncate(groups.len());
        self.events = events;
        self.groups = groups;
        self.reindex();
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn events(&self) -> &[NormalizedEvent] {
        &self.events
    }

    pub fn show_plumbing(&self) -> bool {
        self.show_plumbing
    }

    /// Total rows. Free: it is the last cumulative offset, not a count of anything.
    pub fn len(&self) -> usize {
        self.offsets.last().copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// What is on row `row`, without building the rows before it.
    pub fn row_at(&self, row: usize) -> Option<Row> {
        if row >= self.len() {
            return None;
        }
        // The last offset is the total, so a hit is always in `visible`.
        let slot = match self.offsets.binary_search(&row) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let group = self.visible[slot];
        let local = row - self.offsets[slot];
        Some(if local == 0 {
            Row::Group {
                group,
                expanded: self.expanded[group],
            }
        } else {
            let event_id = self.groups[group].events[local - 1];
            Row::Event {
                group,
                event: self.event_index(event_id).unwrap_or(0),
            }
        })
    }

    /// The rows in `[first, first + count)`. Only these are built.
    pub fn slice(&self, first: usize, count: usize) -> Vec<Row> {
        (first..first.saturating_add(count))
            .map_while(|r| self.row_at(r))
            .collect()
    }

    pub fn group(&self, index: usize) -> Option<&Group> {
        self.groups.get(index)
    }

    pub fn event(&self, index: usize) -> Option<&NormalizedEvent> {
        self.events.get(index)
    }

    /// Fold a group open or shut. Returns the row the group's own line now sits on, so a
    /// caller can keep the cursor on it.
    pub fn toggle(&mut self, group: usize) -> Option<usize> {
        *self.expanded.get_mut(group)? = !self.expanded[group];
        self.reindex();
        self.row_of_group(group)
    }

    pub fn is_expanded(&self, group: usize) -> bool {
        self.expanded.get(group).copied().unwrap_or(false)
    }

    pub fn expand_all(&mut self) {
        self.expanded.iter_mut().for_each(|e| *e = true);
        self.reindex();
    }

    pub fn collapse_all(&mut self) {
        self.expanded.iter_mut().for_each(|e| *e = false);
        self.reindex();
    }

    /// Show or hide workflow-task groups.
    pub fn set_show_plumbing(&mut self, show: bool) {
        self.show_plumbing = show;
        self.reindex();
    }

    /// Which row a group's own line is on, if it is visible.
    pub fn row_of_group(&self, group: usize) -> Option<usize> {
        let slot = self.visible.iter().position(|g| *g == group)?;
        Some(self.offsets[slot])
    }

    /// The next group at or after `from` whose outcome is a failure — `]f`, and what the
    /// minimap points at. On a long history "where did it go wrong" is the whole question,
    /// and scrolling to find out does not scale.
    pub fn next_failure(&self, from: usize) -> Option<usize> {
        self.visible
            .iter()
            .copied()
            .filter(|g| self.groups[*g].outcome.is_failure())
            .find(|g| self.row_of_group(*g).is_some_and(|r| r > from))
            .and_then(|g| self.row_of_group(g))
    }

    /// The previous failing group before `from`.
    pub fn prev_failure(&self, from: usize) -> Option<usize> {
        self.visible
            .iter()
            .copied()
            .filter(|g| self.groups[*g].outcome.is_failure())
            .filter_map(|g| self.row_of_group(g))
            .rfind(|r| *r < from)
    }

    /// Rebuild the visibility and offset tables. O(groups), and only on a shape change —
    /// never while scrolling.
    fn reindex(&mut self) {
        self.visible.clear();
        self.offsets.clear();

        let mut total = 0usize;
        for (i, g) in self.groups.iter().enumerate() {
            if !self.show_plumbing && g.category.is_plumbing() {
                continue;
            }
            self.visible.push(i);
            self.offsets.push(total);
            total += 1 + if self.expanded[i] { g.events.len() } else { 0 };
        }
        // The sentinel is what makes `len()` free and the binary search total.
        self.offsets.push(total);
    }

    /// Event ids are ascending, so this is a binary search rather than a map.
    fn event_index(&self, id: i64) -> Option<usize> {
        self.events.binary_search_by_key(&id, |e| e.id).ok()
    }
}

/// A one-line summary of the whole run, for the detail header.
pub fn summarize(groups: &[Group]) -> Summary {
    let mut s = Summary::default();
    for g in groups {
        match g.category {
            Category::Activity => s.activities += 1,
            Category::Timer => s.timers += 1,
            Category::ChildWorkflow => s.children += 1,
            Category::WorkflowTask
            | Category::Workflow
            | Category::ExternalWorkflow
            | Category::Update
            | Category::Nexus
            | Category::Marker
            | Category::SearchAttributes => {}
        }
        if g.outcome.is_failure() {
            s.failures += 1;
        }
        // Plumbing is hidden from the outline, so counting it here would advertise a
        // running thing the reader cannot find on screen.
        if g.is_open() && g.category != Category::Workflow && !g.category.is_plumbing() {
            s.in_flight += 1;
        }
        if g.category == Category::Workflow {
            s.outcome = g.outcome;
        }
    }
    s
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub activities: usize,
    pub timers: usize,
    pub children: usize,
    pub failures: usize,
    /// Groups still running.
    pub in_flight: usize,
    /// How the workflow itself ended.
    pub outcome: Outcome,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{GroupRef, Role, group_events};

    fn ev(id: i64, group: GroupRef, role: Role, cat: Category) -> NormalizedEvent {
        NormalizedEvent::new(id, "E", cat, group, role).with_time(Some(id * 10))
    }

    /// A workflow with one workflow task and two activities, one of which failed.
    fn outline() -> Outline {
        let events = vec![
            ev(1, GroupRef::Workflow, Role::Opens, Category::Workflow).with_subject("Order"),
            ev(2, GroupRef::Opened(2), Role::Opens, Category::WorkflowTask),
            ev(3, GroupRef::Opened(2), Role::Closes, Category::WorkflowTask),
            ev(4, GroupRef::Opened(4), Role::Opens, Category::Activity).with_subject("Charge"),
            ev(5, GroupRef::Opened(4), Role::Continues, Category::Activity),
            ev(6, GroupRef::Opened(4), Role::Closes, Category::Activity)
                .with_outcome(Outcome::Completed),
            ev(7, GroupRef::Opened(7), Role::Opens, Category::Activity).with_subject("Ship"),
            ev(8, GroupRef::Opened(7), Role::Closes, Category::Activity)
                .with_outcome(Outcome::Failed),
        ];
        let groups = group_events(&events);
        Outline::new(events, groups)
    }

    #[test]
    fn collapsed_rows_are_one_per_group_with_plumbing_hidden() {
        let o = outline();
        // Workflow, Charge, Ship. The workflow-task group is plumbing.
        assert_eq!(o.len(), 3);
        assert_eq!(
            o.row_at(0),
            Some(Row::Group {
                group: 0,
                expanded: false
            })
        );
        assert_eq!(o.row_at(3), None, "past the end");
    }

    #[test]
    fn showing_plumbing_adds_the_workflow_task_group() {
        let mut o = outline();
        o.set_show_plumbing(true);
        assert_eq!(o.len(), 4);
        assert!(o.show_plumbing());
    }

    #[test]
    fn expanding_a_group_inserts_exactly_its_events() {
        let mut o = outline();
        let before = o.len();

        // The "Charge" activity is group 2 (workflow, workflow-task, charge, ship).
        let row = o.toggle(2).expect("the group is visible");
        assert_eq!(row, 1, "its own line stays where it was");
        assert_eq!(o.len(), before + 3, "three events joined the outline");

        assert_eq!(
            o.row_at(1),
            Some(Row::Group {
                group: 2,
                expanded: true
            })
        );
        // Rows 2..4 are its events, in history order.
        for (offset, id) in [(2usize, 4i64), (3, 5), (4, 6)] {
            let Some(Row::Event { group, event }) = o.row_at(offset) else {
                panic!(
                    "row {offset} should be an event, got {:?}",
                    o.row_at(offset)
                );
            };
            assert_eq!(group, 2);
            assert_eq!(o.event(event).unwrap().id, id);
        }
        // The next group follows immediately after them.
        assert!(matches!(o.row_at(5), Some(Row::Group { group: 3, .. })));
    }

    #[test]
    fn collapsing_restores_the_previous_shape() {
        let mut o = outline();
        let before = o.len();
        o.toggle(2);
        o.toggle(2);
        assert_eq!(o.len(), before);
        assert!(!o.is_expanded(2));
    }

    #[test]
    fn a_row_lookup_does_not_depend_on_reading_earlier_rows() {
        // The virtualisation property: row_at is a binary search, so asking for a row deep
        // in a large history costs the same as asking for the first.
        let events: Vec<NormalizedEvent> = (1..=30_000)
            .map(|i| {
                let g = GroupRef::Opened(i - (i % 3));
                let role = match i % 3 {
                    0 => Role::Opens,
                    1 => Role::Continues,
                    _ => Role::Closes,
                };
                ev(i, g, role, Category::Activity)
            })
            .collect();
        let groups = group_events(&events);
        let mut o = Outline::new(events, groups);
        o.expand_all();

        let last = o.len() - 1;
        assert!(o.row_at(last).is_some());
        assert!(o.row_at(last / 2).is_some());
        assert_eq!(o.row_at(o.len()), None);
    }

    #[test]
    fn expand_and_collapse_all_move_together() {
        let mut o = outline();
        let collapsed = o.len();
        o.expand_all();
        assert!(o.len() > collapsed);
        assert!(o.is_expanded(2) && o.is_expanded(3));
        o.collapse_all();
        assert_eq!(o.len(), collapsed);
    }

    #[test]
    fn failures_are_reachable_without_scrolling_to_them() {
        let o = outline();
        // "Ship" failed and is the last row.
        let at = o.next_failure(0).expect("there is a failure below row 0");
        assert!(matches!(o.row_at(at), Some(Row::Group { group: 3, .. })));
        assert_eq!(o.next_failure(at), None, "nothing after the last failure");
        assert_eq!(o.prev_failure(at), None, "nothing before the first");
        assert_eq!(o.prev_failure(o.len()), Some(at));
    }

    #[test]
    fn hidden_groups_are_not_reachable_by_row() {
        // Plumbing is filtered out, so no row can resolve to it — otherwise a cursor could
        // land on something the screen is not showing.
        let o = outline();
        for r in 0..o.len() {
            let group = match o.row_at(r).unwrap() {
                Row::Group { group, .. } | Row::Event { group, .. } => group,
            };
            assert!(
                !o.group(group).unwrap().category.is_plumbing(),
                "row {r} resolved to a hidden group"
            );
        }
    }

    #[test]
    fn a_new_page_does_not_fold_shut_what_the_reader_opened() {
        let mut o = outline();
        o.toggle(2);
        assert!(o.is_expanded(2));
        let rows_before = o.len();

        // A second page arrives: the same events plus two more, re-grouped from scratch.
        let mut events: Vec<NormalizedEvent> = o.events().to_vec();
        events.push(ev(9, GroupRef::Opened(9), Role::Opens, Category::Timer).with_subject("wait"));
        events.push(
            ev(10, GroupRef::Opened(9), Role::Closes, Category::Timer)
                .with_outcome(Outcome::Completed),
        );
        let groups = group_events(&events);
        o.replace(events, groups);

        assert!(o.is_expanded(2), "expansion must survive a new page");
        assert_eq!(o.len(), rows_before + 1, "one new collapsed group");
        assert_eq!(o.groups().len(), 5);
    }

    #[test]
    fn replacing_with_fewer_groups_does_not_panic() {
        // Defensive: a refresh against a different run could return a shorter history, and
        // an expansion vector left longer than the groups would index out of bounds.
        let mut o = outline();
        o.expand_all();
        o.replace(Vec::new(), Vec::new());
        assert!(o.is_empty());
        assert_eq!(o.row_at(0), None);
    }

    #[test]
    fn an_empty_history_has_no_rows() {
        let o = Outline::new(Vec::new(), Vec::new());
        assert!(o.is_empty());
        assert_eq!(o.row_at(0), None);
        assert_eq!(o.slice(0, 10), Vec::new());
        assert_eq!(o.next_failure(0), None);
    }

    #[test]
    fn a_slice_builds_only_what_was_asked_for() {
        let mut o = outline();
        o.expand_all();
        let rows = o.slice(1, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], o.row_at(1).unwrap());
        // Asking past the end returns what exists rather than panicking.
        assert_eq!(o.slice(o.len() - 1, 50).len(), 1);
    }

    #[test]
    fn the_summary_counts_what_the_header_shows() {
        let s = summarize(outline().groups());
        assert_eq!(s.activities, 2);
        assert_eq!(s.failures, 1);
        assert_eq!(s.timers, 0);
        assert_eq!(
            s.outcome,
            Outcome::Pending,
            "the workflow group never closed"
        );
    }

    #[test]
    fn the_summary_never_counts_something_the_outline_hides() {
        // An unfinished workflow task is "running", but it is plumbing and the outline
        // does not show it. Reporting it would send the reader hunting for a row that is
        // not there.
        let events = vec![
            ev(1, GroupRef::Workflow, Role::Opens, Category::Workflow),
            ev(2, GroupRef::Opened(2), Role::Opens, Category::WorkflowTask),
        ];
        let groups = group_events(&events);
        assert_eq!(summarize(&groups).in_flight, 0);

        let o = Outline::new(events, groups);
        assert_eq!(o.len(), 1, "only the workflow group is visible");
    }
}
