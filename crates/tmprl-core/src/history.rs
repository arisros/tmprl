//! Turning a flat event log into the thing a human wants to read.

use crate::payload::Payload;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Workflow,
    WorkflowTask,
    Activity,
    Timer,
    ChildWorkflow,
    ExternalWorkflow,
    Update,
    Nexus,
    Marker,
    SearchAttributes,
}

impl Category {
    pub fn is_plumbing(self) -> bool {
        matches!(self, Category::WorkflowTask)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Opens,
    Continues,
    Closes,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroupRef {
    Workflow,
    Opened(i64),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub stack_trace: Option<String>,
    pub non_retryable: bool,
    pub cause: Option<Box<Failure>>,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    pub fn headline(&self) -> String {
        match (&self.kind, self.message.is_empty()) {
            (Some(k), false) => format!("{k}: {}", self.message),
            (Some(k), true) => k.clone(),
            (None, _) => self.message.clone(),
        }
    }

    pub fn chain(&self) -> impl Iterator<Item = &Failure> {
        std::iter::successors(Some(self), |f| f.cause.as_deref())
    }

    pub fn root(&self) -> &Failure {
        self.chain().last().unwrap_or(self)
    }

    pub fn depth(&self) -> usize {
        self.chain().count()
    }
}

impl From<&str> for Failure {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Failure {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedEvent {
    pub id: i64,
    pub time: Option<i64>,
    pub name: &'static str,
    pub category: Category,
    pub group: GroupRef,
    pub role: Role,
    pub outcome: Outcome,
    pub subject: String,
    pub attempt: Option<i32>,
    pub failure: Option<Failure>,
    pub fields: Vec<(&'static str, String)>,
    pub payloads: Vec<(String, Payload)>,
}

impl NormalizedEvent {
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

pub fn close_reason(event: &NormalizedEvent) -> Option<String> {
    let text = match &event.failure {
        Some(failure) => {
            let root = failure.root().headline();
            if root.trim().is_empty() {
                failure.headline()
            } else {
                root
            }
        }
        None => event
            .fields
            .iter()
            .find(|(name, _)| *name == "reason")
            .map(|(_, value)| value.clone())
            .unwrap_or_default(),
    };
    let line = text
        .split(|c: char| c.is_control())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!line.is_empty()).then_some(line)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub key: GroupRef,
    pub category: Category,
    pub subject: String,
    pub events: Vec<i64>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub outcome: Outcome,
    pub attempts: i32,
    pub failure: Option<Failure>,
}

impl Group {
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none() && self.outcome == Outcome::Pending
    }

    pub fn duration_ms(&self) -> Option<i64> {
        Some(self.ended_at? - self.started_at?)
    }

    pub fn payload_ends(&self) -> Vec<i64> {
        let mut ends: Vec<i64> = [self.events.first(), self.events.last()]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        ends.dedup();
        ends
    }

    pub fn ids<'a>(&self, events: &'a [NormalizedEvent]) -> Vec<(&'static str, &'a str)> {
        let mut out: Vec<(&'static str, &'a str)> = Vec::new();
        for id in &self.events {
            let Some(e) = events.iter().find(|e| e.id == *id) else {
                continue;
            };
            for (name, value) in &e.fields {
                if ID_FIELDS.contains(name)
                    && !value.is_empty()
                    && !out.iter().any(|(n, _)| n == name)
                {
                    out.push((name, value.as_str()));
                }
            }
        }
        out.sort_by_key(|(name, _)| ID_FIELDS.iter().position(|f| f == name));
        out
    }

    pub fn field<'a>(&self, events: &'a [NormalizedEvent], name: &str) -> Option<&'a str> {
        self.events.iter().find_map(|id| {
            let e = events.iter().find(|e| e.id == *id)?;
            e.fields
                .iter()
                .find(|(n, v)| *n == name && !v.is_empty())
                .map(|(_, v)| v.as_str())
        })
    }

    pub fn id<'a>(&self, events: &'a [NormalizedEvent]) -> Option<&'a str> {
        self.ids(events)
            .into_iter()
            .find(|(name, _)| PRIMARY_IDS.contains(name))
            .map(|(_, value)| value)
    }

    pub fn first_event(&self) -> Option<i64> {
        self.events.first().copied()
    }
}

const ID_FIELDS: [&str; 5] = ["activityId", "workflowId", "runId", "updateId", "taskQueue"];
const PRIMARY_IDS: [&str; 3] = ["activityId", "workflowId", "updateId"];

pub fn group_events(events: &[NormalizedEvent]) -> Vec<Group> {
    let mut grouping = Grouping::default();
    for ev in events {
        grouping.push(ev);
    }
    grouping.into_groups()
}

#[derive(Debug, Clone, Default)]
pub struct Grouping {
    groups: Vec<Group>,
    index: std::collections::HashMap<GroupRef, usize>,
}

impl Grouping {
    pub fn from_groups(groups: Vec<Group>) -> Self {
        let mut index = std::collections::HashMap::with_capacity(groups.len());
        for (at, g) in groups.iter().enumerate() {
            index.entry(g.key).or_insert(at);
        }
        Self { groups, index }
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn into_groups(self) -> Vec<Group> {
        self.groups
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Events must arrive in history order and once each; filter replays with [`merge_events`].
    pub fn push(&mut self, ev: &NormalizedEvent) -> usize {
        let at = match self.index.get(&ev.group) {
            Some(&at) => at,
            None => {
                self.groups.push(Group {
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
                self.index.insert(ev.group, self.groups.len() - 1);
                self.groups.len() - 1
            }
        };
        let g = &mut self.groups[at];

        g.events.push(ev.id);
        if let Some(n) = ev.attempt {
            g.attempts = g.attempts.max(n);
        }
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
        at
    }
}

pub fn merge_events(existing: &mut Vec<NormalizedEvent>, incoming: Vec<NormalizedEvent>) -> usize {
    let highest = existing.last().map(|e| e.id).unwrap_or(i64::MIN);
    let before = existing.len();
    existing.extend(incoming.into_iter().filter(|e| e.id > highest));
    existing.len() - before
}

// Temporal only resets to a completed workflow task.
pub fn reset_point(events: &[NormalizedEvent], at: i64) -> Option<i64> {
    events
        .iter()
        .filter(|e| e.id <= at)
        .rfind(|e| e.category == Category::WorkflowTask && e.outcome == Outcome::Completed)
        .map(|e| e.id)
}

pub fn failures(groups: &[Group]) -> Vec<&Group> {
    groups.iter().filter(|g| g.outcome.is_failure()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_group_lists_its_single_event_once() {
        let g = Group {
            key: GroupRef::Workflow,
            category: Category::Workflow,
            subject: "PayloadProbe".into(),
            events: vec![1],
            started_at: None,
            ended_at: None,
            outcome: Outcome::Pending,
            attempts: 1,
            failure: None,
        };
        assert_eq!(g.payload_ends(), vec![1]);
    }

    #[test]
    fn a_closed_group_lists_the_event_that_opened_and_the_one_that_closed_it() {
        let g = Group {
            key: GroupRef::Opened(5),
            category: Category::Activity,
            subject: "ChargeCard".into(),
            events: vec![5, 6, 7],
            started_at: None,
            ended_at: None,
            outcome: Outcome::Completed,
            attempts: 1,
            failure: None,
        };
        assert_eq!(g.payload_ends(), vec![5, 7], "the middle carries nothing");
    }

    fn ev(id: i64, name: &'static str, group: GroupRef, role: Role, time: i64) -> NormalizedEvent {
        NormalizedEvent::new(id, name, Category::Activity, group, role).with_time(Some(time))
    }

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
    fn a_closing_event_says_why_in_one_line() {
        let close = |name| ev(9, name, GroupRef::Workflow, Role::Closes, 50);

        let mut failed = close("WorkflowExecutionFailed");
        let mut outer = Failure::new("activity error");
        let mut root = Failure::new("nik not found\n  at validate()\u{1b}[31m");
        root.kind = Some("ValidationError".into());
        outer.cause = Some(Box::new(root));
        failed.failure = Some(outer);
        assert_eq!(
            close_reason(&failed).as_deref(),
            Some("ValidationError: nik not found at validate() [31m")
        );

        let mut terminated = close("WorkflowExecutionTerminated");
        terminated.fields = vec![
            ("reason", "stuck since Monday".into()),
            ("identity", "me".into()),
        ];
        assert_eq!(
            close_reason(&terminated).as_deref(),
            Some("stuck since Monday")
        );

        assert_eq!(close_reason(&close("WorkflowExecutionTimedOut")), None);
        let mut blank = close("WorkflowExecutionTerminated");
        blank.fields = vec![("reason", " ".into())];
        assert_eq!(close_reason(&blank), None);
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
        assert_eq!(
            groups[0].failure.as_ref().map(|f| f.message.as_str()),
            Some("final")
        );
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
        let mut held: Vec<NormalizedEvent> = retried_activity();
        assert_eq!(held.len(), 3);

        let replay = retried_activity();
        assert_eq!(merge_events(&mut held, replay), 0, "nothing was new");
        assert_eq!(held.len(), 3);

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
    fn a_reset_resolves_back_to_the_last_completed_workflow_task() {
        let events = vec![
            NormalizedEvent::new(1, "S", Category::Workflow, GroupRef::Workflow, Role::Opens),
            NormalizedEvent::new(
                2,
                "WTS",
                Category::WorkflowTask,
                GroupRef::Opened(2),
                Role::Opens,
            ),
            NormalizedEvent::new(
                3,
                "WTC",
                Category::WorkflowTask,
                GroupRef::Opened(2),
                Role::Closes,
            )
            .with_outcome(Outcome::Completed),
            NormalizedEvent::new(
                4,
                "ATS",
                Category::Activity,
                GroupRef::Opened(4),
                Role::Opens,
            ),
            NormalizedEvent::new(
                5,
                "ATC",
                Category::Activity,
                GroupRef::Opened(4),
                Role::Closes,
            )
            .with_outcome(Outcome::Completed),
        ];
        assert_eq!(
            reset_point(&events, 5),
            Some(3),
            "back to the workflow task"
        );
        assert_eq!(reset_point(&events, 3), Some(3), "already on one");
        assert_eq!(reset_point(&events, 2), None, "nothing completed yet");
    }

    #[test]
    fn a_failed_workflow_task_is_not_a_reset_point() {
        let events = vec![
            NormalizedEvent::new(
                2,
                "WTS",
                Category::WorkflowTask,
                GroupRef::Opened(2),
                Role::Opens,
            ),
            NormalizedEvent::new(
                3,
                "WTF",
                Category::WorkflowTask,
                GroupRef::Opened(2),
                Role::Closes,
            )
            .with_outcome(Outcome::Failed),
        ];
        assert_eq!(reset_point(&events, 3), None);
    }

    #[test]
    fn a_reset_takes_the_latest_valid_point_not_the_first() {
        let events = vec![
            NormalizedEvent::new(
                3,
                "WTC",
                Category::WorkflowTask,
                GroupRef::Opened(2),
                Role::Closes,
            )
            .with_outcome(Outcome::Completed),
            NormalizedEvent::new(
                9,
                "WTC",
                Category::WorkflowTask,
                GroupRef::Opened(8),
                Role::Closes,
            )
            .with_outcome(Outcome::Completed),
            NormalizedEvent::new(
                12,
                "ATC",
                Category::Activity,
                GroupRef::Opened(10),
                Role::Closes,
            )
            .with_outcome(Outcome::Completed),
        ];
        assert_eq!(reset_point(&events, 12), Some(9));
        assert_eq!(reset_point(&events, 8), Some(3));
    }

    #[test]
    fn an_empty_history_groups_to_nothing() {
        assert!(group_events(&[]).is_empty());
        assert!(failures(&[]).is_empty());
    }

    #[test]
    fn a_headline_names_the_class_when_the_worker_gave_one() {
        let bare = Failure::new("card declined");
        assert_eq!(bare.headline(), "card declined");

        let typed = Failure {
            kind: Some("PaymentDeclined".into()),
            ..Failure::new("card declined")
        };
        assert_eq!(typed.headline(), "PaymentDeclined: card declined");

        let no_message = Failure {
            kind: Some("PaymentDeclined".into()),
            ..Failure::default()
        };
        assert_eq!(no_message.headline(), "PaymentDeclined");
    }

    #[test]
    fn the_root_of_a_chain_is_what_actually_went_wrong() {
        let f = Failure {
            cause: Some(Box::new(Failure {
                cause: Some(Box::new(Failure::new("Read timed out"))),
                ..Failure::new("card declined")
            })),
            ..Failure::new("activity task failed")
        };

        assert_eq!(f.depth(), 3);
        assert_eq!(f.root().message, "Read timed out");
        assert_eq!(
            f.chain().map(|l| l.message.as_str()).collect::<Vec<_>>(),
            ["activity task failed", "card declined", "Read timed out"],
            "the chain reads outermost first, the way it is rendered"
        );
    }

    #[test]
    fn a_failure_with_nothing_under_it_is_its_own_root() {
        let f = Failure::new("plain");
        assert_eq!(f.depth(), 1);
        assert_eq!(f.root().message, "plain");
    }

    #[test]
    fn an_activity_group_is_named_by_its_activity_id() {
        let events = retried_activity();
        let groups = group_events(&events);
        let g = &groups[0];
        assert_eq!(g.id(&events), Some("charge"));
        assert_eq!(g.ids(&events)[0], ("activityId", "charge"));
    }

    #[test]
    fn a_child_group_gathers_its_workflow_and_run_id_from_two_events() {
        let mut initiated = NormalizedEvent::new(
            5,
            "StartChildWorkflowExecutionInitiated",
            Category::ChildWorkflow,
            GroupRef::Opened(5),
            Role::Opens,
        );
        initiated.fields.push(("workflowId", "child-1".into()));
        initiated.fields.push(("namespace", "default".into()));
        let mut started = NormalizedEvent::new(
            6,
            "ChildWorkflowExecutionStarted",
            Category::ChildWorkflow,
            GroupRef::Opened(5),
            Role::Continues,
        );
        started.fields.push(("runId", "r-9".into()));
        let events = vec![initiated, started];
        let groups = group_events(&events);

        assert_eq!(
            groups[0].ids(&events),
            vec![("workflowId", "child-1"), ("runId", "r-9")]
        );
        assert_eq!(groups[0].id(&events), Some("child-1"));
    }

    #[test]
    fn a_group_with_no_id_has_none() {
        let events = vec![ev(5, "TimerStarted", GroupRef::Opened(5), Role::Opens, 1)];
        let groups = group_events(&events);
        assert_eq!(groups[0].id(&events), None);
    }
}
