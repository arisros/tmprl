//! The adaptive layout: the panels worth showing, chosen from what the namespace holds.

use super::layout::{DAY_MS, Layout, PanelKind, PanelSpec, RUNNING, RowSpec, Show, Size};
use super::source::tally_types;
use crate::query;
use crate::schedule::ScheduleRow;
use crate::workflow::WorkflowRow;

/// A panel the adaptive layout may or may not show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Failures,
    Types,
    Running,
    Queues,
    Paused,
    Upcoming,
    Schedules,
}

impl Slot {
    pub(super) const ALL: [Slot; 7] = [
        Slot::Failures,
        Slot::Types,
        Slot::Running,
        Slot::Queues,
        Slot::Paused,
        Slot::Upcoming,
        Slot::Schedules,
    ];

    pub(super) fn spec(self) -> PanelSpec {
        let failures = || (query::PROBLEMS.to_string(), Some(DAY_MS));
        let running = || RUNNING.to_string();
        match self {
            Slot::Failures => {
                let (query, since_ms) = failures();
                let mut spec = PanelSpec::new(PanelKind::Workflows { query, since_ms })
                    .titled("Recent failures");
                spec.width = 2;
                spec
            }
            Slot::Types => {
                let (query, since_ms) = failures();
                PanelSpec::new(PanelKind::Types { query, since_ms }).titled("Failing types")
            }
            Slot::Running => {
                let mut spec = PanelSpec::new(PanelKind::Workflows {
                    query: running(),
                    since_ms: None,
                })
                .titled("Running");
                spec.width = 2;
                spec
            }
            Slot::Queues => PanelSpec::new(PanelKind::Queues {
                query: running(),
                names: Vec::new(),
            }),
            Slot::Paused => PanelSpec::new(PanelKind::Schedules { show: Show::Paused }),
            Slot::Upcoming => PanelSpec::new(PanelKind::Schedules {
                show: Show::Upcoming,
            }),
            Slot::Schedules => PanelSpec::new(PanelKind::Schedules { show: Show::All }),
        }
    }
}

/// What the probes have said so far. `None` is "not known yet", which is not "none".
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts<'a> {
    pub failures: Option<&'a [WorkflowRow]>,
    pub running: Option<&'a [WorkflowRow]>,
    pub schedules: Option<&'a [ScheduleRow]>,
}

/// The layout for a namespace nobody wrote a `dashboard.toml` for: the builtin one, less
/// the panels known to have nothing in them.
///
/// A panel still waiting, or whose request failed, is shown, so the screen says what it is
/// waiting for and what went wrong. A panel in `kept` is shown even when empty: it had
/// items a moment ago, and a layout that reshuffles while it is being read is worse than
/// an empty box.
pub fn compose(facts: &Facts, kept: &[Slot]) -> Layout {
    let distinct_types = |rows: &[WorkflowRow]| tally_types(rows).len();
    let wanted = |slot: Slot| -> bool {
        if kept.contains(&slot) {
            return true;
        }
        match slot {
            Slot::Failures => facts.failures.is_none_or(|rows| !rows.is_empty()),
            Slot::Types => facts.failures.is_none_or(|rows| distinct_types(rows) > 1),
            Slot::Running => false,
            Slot::Queues => facts.running.is_none_or(|rows| !rows.is_empty()),
            Slot::Paused => facts
                .schedules
                .is_some_and(|rows| rows.iter().any(|s| s.paused)),
            Slot::Upcoming => facts
                .schedules
                .is_some_and(|rows| rows.iter().any(|s| !s.paused && s.next_run.is_some())),
            Slot::Schedules => facts.schedules.is_none(),
        }
    };

    let mut middle: Vec<Slot> = [Slot::Failures, Slot::Types]
        .into_iter()
        .filter(|s| wanted(*s))
        .collect();
    // Nothing failed, so the room goes to what is running: a dashboard that is only a
    // header says less than the list it is one key from.
    if !middle.contains(&Slot::Failures)
        && (kept.contains(&Slot::Running) || facts.running.is_some_and(|rows| !rows.is_empty()))
    {
        middle.insert(0, Slot::Running);
    }
    let mut bottom: Vec<Slot> = [Slot::Queues, Slot::Paused, Slot::Upcoming, Slot::Schedules]
        .into_iter()
        .filter(|s| wanted(*s))
        .collect();
    // Schedules that are neither paused nor due still exist, and are worth one panel.
    if facts.schedules.is_some_and(|rows| !rows.is_empty())
        && !bottom
            .iter()
            .any(|s| matches!(s, Slot::Paused | Slot::Upcoming | Slot::Schedules))
    {
        bottom.push(Slot::Schedules);
    }

    if middle.is_empty() && bottom.is_empty() {
        return Layout::builtin();
    }
    let row = |size: Size, slots: Vec<Slot>| RowSpec {
        size,
        panels: slots.into_iter().map(Slot::spec).collect(),
    };
    let mut rows = vec![RowSpec {
        size: Size::Lines(3),
        panels: vec![PanelSpec::new(PanelKind::Counts {
            query: String::new(),
        })],
    }];
    if !middle.is_empty() {
        rows.push(row(Size::Weight(3), middle));
    }
    if !bottom.is_empty() {
        rows.push(row(Size::Weight(2), bottom));
    }
    Layout { rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::fixtures::*;

    #[test]
    fn before_anything_is_known_the_layout_is_the_builtin_one() {
        assert_eq!(compose(&Facts::default(), &[]), Layout::builtin());
    }

    #[test]
    fn a_namespace_with_nothing_in_it_keeps_the_builtin_layout() {
        let facts = Facts {
            failures: Some(&[]),
            running: Some(&[]),
            schedules: Some(&[]),
        };
        assert_eq!(compose(&facts, &[]), Layout::builtin());
    }

    #[test]
    fn panels_known_to_be_empty_collapse_and_running_takes_the_room() {
        let running = [wf("a", "Order", "orders", 1)];
        let facts = Facts {
            failures: Some(&[]),
            running: Some(&running),
            schedules: Some(&[]),
        };
        assert_eq!(
            titles(&compose(&facts, &[])),
            [vec!["Status"], vec!["Running"], vec!["Task queues"]]
        );
    }

    #[test]
    fn a_panel_still_waiting_is_shown() {
        let failures = [wf("a", "Order", "orders", 1)];
        let facts = Facts {
            failures: Some(&failures),
            running: None,
            schedules: None,
        };
        assert_eq!(
            titles(&compose(&facts, &[])),
            [
                vec!["Status"],
                vec!["Recent failures"],
                vec!["Task queues", "Schedules"]
            ]
        );
    }

    #[test]
    fn failing_types_are_worth_a_panel_from_two_types_up() {
        let one = [wf("a", "Order", "q", 1), wf("b", "Order", "q", 2)];
        let two = [wf("a", "Order", "q", 1), wf("b", "Refund", "q", 2)];
        let middle = |failures: &[WorkflowRow]| {
            let facts = Facts {
                failures: Some(failures),
                running: Some(&[]),
                schedules: Some(&[]),
            };
            titles(&compose(&facts, &[]))[1]
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(middle(&one), ["Recent failures"]);
        assert_eq!(middle(&two), ["Recent failures", "Failing types"]);
    }

    #[test]
    fn schedules_are_shown_as_what_needs_looking_at() {
        let bottom = |schedules: &[ScheduleRow]| {
            let facts = Facts {
                failures: Some(&[]),
                running: Some(&[]),
                schedules: Some(schedules),
            };
            titles(&compose(&facts, &[]))
                .last()
                .unwrap()
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bottom(&[schedule("a", true, None), schedule("b", false, Some(9))]),
            ["Paused schedules", "Upcoming schedules"]
        );
        assert_eq!(bottom(&[schedule("a", true, None)]), ["Paused schedules"]);
        assert_eq!(bottom(&[schedule("a", false, None)]), ["Schedules"]);
    }

    #[test]
    fn a_panel_that_had_items_stays_when_they_go() {
        let facts = Facts {
            failures: Some(&[]),
            running: Some(&[]),
            schedules: Some(&[]),
        };
        assert_eq!(
            titles(&compose(&facts, &[Slot::Failures, Slot::Paused])),
            [
                vec!["Status"],
                vec!["Recent failures"],
                vec!["Paused schedules"]
            ]
        );
    }
}
