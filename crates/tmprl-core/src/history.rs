//! Turning a flat event log into the thing a human wants to read.
//!
//! Temporal sends a workflow's history as an ordered list of events linked only by integer
//! back-references. An activity that was scheduled, started and completed is three rows on
//! the wire and *one thing* to a reader. Reconstructing that is, per
//! `docs/ARCHITECTURE.md`, the hardest part of the port.
//!
//! The split of labour:
//!
//! * `tmprl-client` maps each protobuf event onto a [`NormalizedEvent`] through one
//!   exhaustive match. The generated types stop there.
//! * This module folds those into [`Group`]s. It is pure, so the grouping rules — the part
//!   that is actually easy to get wrong — are tested with hand-built events and no server.

use crate::payload::Payload;

/// What kind of thing an event is about. Drives icons, filtering and the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// The workflow execution itself: started, completed, signalled, terminated.
    Workflow,
    /// Workflow task — the worker polling and responding. Noise most of the time, which is
    /// why the compact view can fold it away.
    WorkflowTask,
    Activity,
    Timer,
    ChildWorkflow,
    /// Signals and cancellation aimed at *another* workflow.
    ExternalWorkflow,
    Update,
    Nexus,
    Marker,
    SearchAttributes,
}

impl Category {
    /// Whether this is machinery rather than something the workflow author wrote. The
    /// compact view hides these until asked.
    pub fn is_plumbing(self) -> bool {
        matches!(self, Category::WorkflowTask)
    }
}

/// Where an event sits in the life of the thing it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Opens a group: scheduled, initiated, started-by-us.
    Opens,
    /// Neither opens nor closes — a worker picked the task up, a cancel was requested.
    Continues,
    /// Closes a group: completed, failed, timed out, cancelled.
    Closes,
}

/// How something ended. `Pending` means it has not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
    #[default]
    Pending,
    Completed,
    Failed,
    Canceled,
    TimedOut,
    Terminated,
    ContinuedAsNew,
    Rejected,
}

impl Outcome {
    /// Whether this outcome is one a reader is hunting for. The minimap and the problem
    /// list are built from this.
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Outcome::Failed | Outcome::TimedOut | Outcome::Terminated | Outcome::Rejected
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Outcome::Pending => "Pending",
            Outcome::Completed => "Completed",
            Outcome::Failed => "Failed",
            Outcome::Canceled => "Canceled",
            Outcome::TimedOut => "TimedOut",
            Outcome::Terminated => "Terminated",
            Outcome::ContinuedAsNew => "ContinuedAsNew",
            Outcome::Rejected => "Rejected",
        }
    }
}

/// Which group an event belongs to.
///
/// Groups are keyed by the id of the event that opened them, because that is the one
/// identifier every back-reference in the protocol actually points at. Keying by a
/// user-facing name instead (an activity id, say) would need a lookup that can fail, and
/// would merge two genuinely separate schedulings of the same name into one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroupRef {
    /// The workflow execution itself.
    Workflow,
    /// The group opened by this event id.
    Opened(i64),
}

/// One protobuf history event, flattened.
///
/// Deliberately not a 60-variant mirror of the protobuf `oneof`. Everything downstream
/// needs — what it is about, which group it joins, whether it opens or closes that group,
/// how it ended — is extracted by the mapping in `tmprl-client`, so this module and the
/// views never touch a generated type or re-derive the same facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedEvent {
    pub id: i64,
    /// Epoch milliseconds.
    pub time: Option<i64>,
    /// The protobuf event name, e.g. `ActivityTaskScheduled`. Kept verbatim because it is
    /// what Temporal's own docs, the CLI and the web UI all call it.
    pub name: &'static str,
    pub category: Category,
    pub group: GroupRef,
    pub role: Role,
    pub outcome: Outcome,
    /// What the event is about: an activity type, a timer id, a signal name.
    pub subject: String,
    /// Attempt number, where the protocol reports one. A retry does *not* produce a second
    /// scheduling event — the count lives here.
    pub attempt: Option<i32>,
    /// Failure message, when the event carries one.
    pub failure: Option<String>,
    /// Detail rows for the expanded view, in protocol order.
    pub fields: Vec<(&'static str, String)>,
    /// Payloads this event carries, labelled — `input`, `result`, `details[1]`. Labels are
    /// owned because an argument list needs an index in them.
    pub payloads: Vec<(String, Payload)>,
}

impl NormalizedEvent {
    /// A minimal event, for tests and for the arms of the mapping that carry nothing else.
    pub fn new(
        id: i64,
        name: &'static str,
        category: Category,
        group: GroupRef,
        role: Role,
    ) -> Self {
        Self {
            id,
            time: None,
            name,
            category,
            group,
            role,
            outcome: Outcome::Pending,
            subject: String::new(),
            attempt: None,
            failure: None,
            fields: Vec::new(),
            payloads: Vec::new(),
        }
    }

    pub fn with_time(mut self, time: Option<i64>) -> Self {
        self.time = time;
        self
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    pub fn with_outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }
}

/// Several events that are one thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub key: GroupRef,
    pub category: Category,
    /// From the opening event — the activity type, timer id, child workflow id.
    pub subject: String,
    /// Every member event id, in history order.
    pub events: Vec<i64>,
    pub started_at: Option<i64>,
    /// `None` while the group is still open.
    pub ended_at: Option<i64>,
    pub outcome: Outcome,
    /// Highest attempt seen. 1 unless something was retried.
    pub attempts: i32,
    pub failure: Option<String>,
}

impl Group {
    /// Still running: nothing has closed it.
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none() && self.outcome == Outcome::Pending
    }

    /// Wall-clock duration, once it has ended.
    pub fn duration_ms(&self) -> Option<i64> {
        Some(self.ended_at? - self.started_at?)
    }

    /// The id of the event that opened this group, for jumping to it.
    pub fn first_event(&self) -> Option<i64> {
        self.events.first().copied()
    }
}

/// Fold normalised events into groups, in the order the groups were opened.
///
/// A single forward pass: every event names its own group, so nothing here needs to look
/// ahead or resolve a name to an id. Events are expected in history order, which is the
/// order the server sends them.
///
/// Events whose group was never opened — the first page of a history that starts mid-run,
/// or a back-reference to an event Temporal has since archived — are not dropped. They open
/// a group of their own, so a truncated history renders as a partial group rather than as
/// nothing at all.
pub fn group_events(events: &[NormalizedEvent]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    // Parallel to `groups`, so a lookup is by key without hashing a small collection.
    let mut index: Vec<GroupRef> = Vec::new();

    for ev in events {
        let at = match index.iter().position(|k| *k == ev.group) {
            Some(at) => at,
            None => {
                groups.push(Group {
                    key: ev.group,
                    category: ev.category,
                    subject: ev.subject.clone(),
                    events: Vec::new(),
                    started_at: ev.time,
                    ended_at: None,
                    outcome: Outcome::Pending,
                    attempts: 1,
                    failure: None,
                });
                index.push(ev.group);
                groups.len() - 1
            }
        };
        let g = &mut groups[at];

        g.events.push(ev.id);
        if let Some(n) = ev.attempt {
            g.attempts = g.attempts.max(n);
        }
        // The opening event is the one that names the group. A later event may carry a
        // subject too (a child workflow's run id, say) but must not rename it.
        if ev.role == Role::Opens && !ev.subject.is_empty() && g.subject.is_empty() {
            g.subject = ev.subject.clone();
        }
        if ev.failure.is_some() {
            g.failure = ev.failure.clone();
        }
        if ev.role == Role::Closes {
            g.ended_at = ev.time;
            g.outcome = ev.outcome;
        }
    }

    groups
}

/// Append only the events we do not already hold.
///
/// Returns how many were actually new.
///
/// Follow mode re-reads from the last continuation token it saw, which replays the events
/// after that point, and a resumed follow replays whatever page the token sat in. History is
/// append-only with strictly ascending ids, so "new" is exactly "id greater than the highest
/// we hold" — no set, no scan of what we already have.
pub fn merge_events(existing: &mut Vec<NormalizedEvent>, incoming: Vec<NormalizedEvent>) -> usize {
    let highest = existing.last().map(|e| e.id).unwrap_or(i64::MIN);
    let before = existing.len();
    existing.extend(incoming.into_iter().filter(|e| e.id > highest));
    existing.len() - before
}

/// Groups that failed, timed out or were terminated, in history order.
///
/// This is what the problem list and the minimap read: on a long history the interesting
/// question is "where did it go wrong", and scrolling to find out does not scale.
pub fn failures(groups: &[Group]) -> Vec<&Group> {
    groups.iter().filter(|g| g.outcome.is_failure()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: i64, name: &'static str, group: GroupRef, role: Role, time: i64) -> NormalizedEvent {
        NormalizedEvent::new(id, name, Category::Activity, group, role).with_time(Some(time))
    }

    /// The worked example from `docs/ARCHITECTURE.md`: an activity that was retried once
    /// and then succeeded. Three events on the wire, one thing to a reader.
    fn retried_activity() -> Vec<NormalizedEvent> {
        let mut scheduled = ev(
            5,
            "ActivityTaskScheduled",
            GroupRef::Opened(5),
            Role::Opens,
            1_000,
        )
        .with_subject("ChargeCard");
        scheduled.fields.push(("activityId", "charge".into()));

        let mut started = ev(
            6,
            "ActivityTaskStarted",
            GroupRef::Opened(5),
            Role::Continues,
            2_000,
        );
        // A retry does not schedule again: the attempt count rides on the started event.
        started.attempt = Some(2);
        started.failure = Some("card declined".into());

        let completed = ev(
            7,
            "ActivityTaskCompleted",
            GroupRef::Opened(5),
            Role::Closes,
            41_000,
        )
        .with_outcome(Outcome::Completed);

        vec![scheduled, started, completed]
    }

    #[test]
    fn three_events_become_one_group() {
        let groups = group_events(&retried_activity());
        assert_eq!(groups.len(), 1);

        let g = &groups[0];
        assert_eq!(g.key, GroupRef::Opened(5));
        assert_eq!(g.subject, "ChargeCard");
        assert_eq!(g.events, [5, 6, 7]);
        assert_eq!(g.outcome, Outcome::Completed);
        assert_eq!(g.attempts, 2, "the retry must be visible on the group");
        assert_eq!(g.started_at, Some(1_000));
        assert_eq!(g.ended_at, Some(41_000));
        assert_eq!(g.duration_ms(), Some(40_000));
        assert!(!g.is_open());
    }

    #[test]
    fn a_group_with_no_closing_event_is_still_running() {
        let events = &retried_activity()[..2];
        let groups = group_events(events);
        assert!(groups[0].is_open());
        assert_eq!(groups[0].outcome, Outcome::Pending);
        assert_eq!(
            groups[0].duration_ms(),
            None,
            "a running group has no duration"
        );
    }

    #[test]
    fn interleaved_groups_do_not_bleed_into_each_other() {
        // Two activities in flight at once is the normal case, and the events arrive
        // interleaved. Grouping by arrival order rather than by back-reference would
        // scramble them.
        let events = vec![
            ev(
                5,
                "ActivityTaskScheduled",
                GroupRef::Opened(5),
                Role::Opens,
                100,
            )
            .with_subject("A"),
            ev(
                6,
                "ActivityTaskScheduled",
                GroupRef::Opened(6),
                Role::Opens,
                110,
            )
            .with_subject("B"),
            ev(
                7,
                "ActivityTaskStarted",
                GroupRef::Opened(6),
                Role::Continues,
                120,
            ),
            ev(
                8,
                "ActivityTaskStarted",
                GroupRef::Opened(5),
                Role::Continues,
                130,
            ),
            ev(
                9,
                "ActivityTaskFailed",
                GroupRef::Opened(6),
                Role::Closes,
                140,
            )
            .with_outcome(Outcome::Failed),
            ev(
                10,
                "ActivityTaskCompleted",
                GroupRef::Opened(5),
                Role::Closes,
                150,
            )
            .with_outcome(Outcome::Completed),
        ];
        let groups = group_events(&events);

        assert_eq!(groups.len(), 2);
        // Ordered by when each group opened, not by when it closed.
        assert_eq!(groups[0].subject, "A");
        assert_eq!(groups[0].events, [5, 8, 10]);
        assert_eq!(groups[0].outcome, Outcome::Completed);
        assert_eq!(groups[1].subject, "B");
        assert_eq!(groups[1].events, [6, 7, 9]);
        assert_eq!(groups[1].outcome, Outcome::Failed);
    }

    #[test]
    fn workflow_level_events_share_one_group() {
        let events = vec![
            NormalizedEvent::new(
                1,
                "WorkflowExecutionStarted",
                Category::Workflow,
                GroupRef::Workflow,
                Role::Opens,
            )
            .with_time(Some(10))
            .with_subject("OrderWorkflow"),
            NormalizedEvent::new(
                2,
                "WorkflowExecutionSignaled",
                Category::Workflow,
                GroupRef::Workflow,
                Role::Continues,
            )
            .with_time(Some(20)),
            NormalizedEvent::new(
                3,
                "WorkflowExecutionCompleted",
                Category::Workflow,
                GroupRef::Workflow,
                Role::Closes,
            )
            .with_time(Some(30))
            .with_outcome(Outcome::Completed),
        ];
        let groups = group_events(&events);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, GroupRef::Workflow);
        assert_eq!(groups[0].subject, "OrderWorkflow");
        assert_eq!(groups[0].outcome, Outcome::Completed);
    }

    #[test]
    fn an_orphaned_event_opens_its_own_group_rather_than_vanishing() {
        // A history page that starts mid-run refers back to events it does not contain.
        // Dropping those would render a page as empty and look like a bug in tmprl.
        let events = vec![
            ev(
                42,
                "ActivityTaskCompleted",
                GroupRef::Opened(5),
                Role::Closes,
                900,
            )
            .with_outcome(Outcome::Completed),
        ];
        let groups = group_events(&events);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].events, [42]);
        assert_eq!(groups[0].outcome, Outcome::Completed);
    }

    #[test]
    fn a_later_event_does_not_rename_its_group() {
        let events = vec![
            ev(
                5,
                "ActivityTaskScheduled",
                GroupRef::Opened(5),
                Role::Opens,
                10,
            )
            .with_subject("real"),
            ev(
                6,
                "ActivityTaskStarted",
                GroupRef::Opened(5),
                Role::Continues,
                20,
            )
            .with_subject("other"),
        ];
        assert_eq!(group_events(&events)[0].subject, "real");
    }

    #[test]
    fn the_last_failure_on_a_group_is_the_one_kept() {
        let mut first = ev(
            6,
            "ActivityTaskStarted",
            GroupRef::Opened(5),
            Role::Continues,
            20,
        );
        first.failure = Some("first".into());
        let mut last = ev(
            7,
            "ActivityTaskFailed",
            GroupRef::Opened(5),
            Role::Closes,
            30,
        );
        last.failure = Some("final".into());
        last.outcome = Outcome::Failed;

        let groups = group_events(&[
            ev(
                5,
                "ActivityTaskScheduled",
                GroupRef::Opened(5),
                Role::Opens,
                10,
            ),
            first,
            last,
        ]);
        assert_eq!(groups[0].failure.as_deref(), Some("final"));
    }

    #[test]
    fn failures_are_findable_without_scrolling() {
        let events = vec![
            ev(
                1,
                "ActivityTaskScheduled",
                GroupRef::Opened(1),
                Role::Opens,
                10,
            )
            .with_subject("ok"),
            ev(
                2,
                "ActivityTaskCompleted",
                GroupRef::Opened(1),
                Role::Closes,
                20,
            )
            .with_outcome(Outcome::Completed),
            ev(
                3,
                "ActivityTaskScheduled",
                GroupRef::Opened(3),
                Role::Opens,
                30,
            )
            .with_subject("bad"),
            ev(
                4,
                "ActivityTaskTimedOut",
                GroupRef::Opened(3),
                Role::Closes,
                40,
            )
            .with_outcome(Outcome::TimedOut),
        ];
        let groups = group_events(&events);
        let bad = failures(&groups);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].subject, "bad");
    }

    #[test]
    fn every_outcome_agrees_with_itself_about_being_a_failure() {
        for (o, fail) in [
            (Outcome::Pending, false),
            (Outcome::Completed, false),
            (Outcome::Canceled, false),
            (Outcome::ContinuedAsNew, false),
            (Outcome::Failed, true),
            (Outcome::TimedOut, true),
            (Outcome::Terminated, true),
            (Outcome::Rejected, true),
        ] {
            assert_eq!(o.is_failure(), fail, "{} classified wrongly", o.label());
        }
    }

    #[test]
    fn replayed_events_are_not_appended_twice() {
        // Follow mode resumes from a continuation token, which replays the page that token
        // sat in. Appending blindly would list the same events twice and inflate every
        // group's event count.
        let mut held: Vec<NormalizedEvent> = retried_activity();
        assert_eq!(held.len(), 3);

        let replay = retried_activity();
        assert_eq!(merge_events(&mut held, replay), 0, "nothing was new");
        assert_eq!(held.len(), 3);

        // A genuinely new event lands.
        let fresh = vec![ev(
            8,
            "TimerStarted",
            GroupRef::Opened(8),
            Role::Opens,
            50_000,
        )];
        assert_eq!(merge_events(&mut held, fresh), 1);
        assert_eq!(held.len(), 4);
    }

    #[test]
    fn a_partial_replay_keeps_only_the_tail() {
        let mut held: Vec<NormalizedEvent> = retried_activity();
        // The server replays from event 6 and adds 8 and 9.
        let mut incoming = retried_activity()[1..].to_vec();
        incoming.push(ev(
            8,
            "TimerStarted",
            GroupRef::Opened(8),
            Role::Opens,
            50_000,
        ));
        incoming.push(ev(
            9,
            "TimerFired",
            GroupRef::Opened(8),
            Role::Closes,
            60_000,
        ));

        assert_eq!(merge_events(&mut held, incoming), 2);
        let ids: Vec<i64> = held.iter().map(|e| e.id).collect();
        assert_eq!(ids, [5, 6, 7, 8, 9]);
    }

    #[test]
    fn merging_into_an_empty_history_keeps_everything() {
        let mut held = Vec::new();
        assert_eq!(merge_events(&mut held, retried_activity()), 3);
        assert_eq!(held.len(), 3);
    }

    #[test]
    fn an_empty_history_groups_to_nothing() {
        assert!(group_events(&[]).is_empty());
        assert!(failures(&[]).is_empty());
    }
}
