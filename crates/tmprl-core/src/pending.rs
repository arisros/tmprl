//! Activities the server is still working on, from `DescribeWorkflowExecution`.

use crate::history::{Category, Failure, Group, NormalizedEvent};
use crate::workflow::humanize_age_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PendingState {
    #[default]
    Unspecified,
    Scheduled,
    Started,
    CancelRequested,
    Paused,
    PauseRequested,
}

impl PendingState {
    pub fn label(self) -> &'static str {
        match self {
            PendingState::Unspecified => "unspecified",
            PendingState::Scheduled => "scheduled",
            PendingState::Started => "started",
            PendingState::CancelRequested => "cancel requested",
            PendingState::Paused => "paused",
            PendingState::PauseRequested => "pause requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingActivity {
    pub activity_id: String,
    pub activity_type: String,
    pub state: PendingState,
    pub attempt: i32,
    pub maximum_attempts: i32,
    pub last_failure: Option<Failure>,
    pub next_attempt_at: Option<i64>,
    pub last_started_at: Option<i64>,
    pub last_heartbeat_at: Option<i64>,
    pub last_worker: Option<String>,
}

impl PendingActivity {
    pub fn is_retrying(&self) -> bool {
        self.attempt > 1 || self.last_failure.is_some()
    }

    pub fn attempts_label(&self) -> String {
        if self.maximum_attempts > 0 {
            format!("{}/{}", self.attempt, self.maximum_attempts)
        } else {
            format!("{}/∞", self.attempt)
        }
    }

    pub fn status(&self, now_ms: i64) -> Option<String> {
        match self.state {
            PendingState::Paused | PendingState::PauseRequested | PendingState::CancelRequested => {
                Some(self.state.label().to_string())
            }
            PendingState::Scheduled => match self.next_attempt_at {
                Some(at) if at > now_ms => {
                    Some(format!("retry in {}", humanize_age_ms(at - now_ms)))
                }
                _ if self.attempt > 1 => Some("retry due".to_string()),
                _ => None,
            },
            PendingState::Started | PendingState::Unspecified => None,
        }
    }
}

pub fn for_group<'a>(
    pending: &'a [PendingActivity],
    group: &Group,
    events: &[NormalizedEvent],
) -> Option<&'a PendingActivity> {
    if pending.is_empty() || group.category != Category::Activity || !group.is_open() {
        return None;
    }
    let opened = group.first_event()?;
    let at = events.binary_search_by_key(&opened, |e| e.id).ok()?;
    let (_, id) = events[at].fields.iter().find(|(k, _)| *k == "activityId")?;
    pending.iter().find(|p| &p.activity_id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{GroupRef, Outcome, Role, group_events};

    fn scheduled(id: i64, activity_id: &str) -> NormalizedEvent {
        let mut e = NormalizedEvent::new(
            id,
            "ActivityTaskScheduled",
            Category::Activity,
            GroupRef::Opened(id),
            Role::Opens,
        )
        .with_subject("ChargeCard");
        e.fields.push(("activityId", activity_id.to_string()));
        e
    }

    fn pending(activity_id: &str, attempt: i32) -> PendingActivity {
        PendingActivity {
            activity_id: activity_id.into(),
            attempt,
            ..Default::default()
        }
    }

    #[test]
    fn an_open_activity_finds_its_pending_entry_by_activity_id() {
        let events = vec![scheduled(5, "1"), scheduled(9, "2")];
        let groups = group_events(&events);
        let live = [pending("2", 4), pending("1", 7)];

        assert_eq!(
            for_group(&live, &groups[0], &events).map(|p| p.attempt),
            Some(7)
        );
        assert_eq!(
            for_group(&live, &groups[1], &events).map(|p| p.attempt),
            Some(4)
        );
    }

    #[test]
    fn a_closed_group_ignores_a_stale_pending_entry() {
        let mut done = NormalizedEvent::new(
            6,
            "ActivityTaskCompleted",
            Category::Activity,
            GroupRef::Opened(5),
            Role::Closes,
        )
        .with_outcome(Outcome::Completed);
        done.time = Some(1);
        let events = vec![scheduled(5, "1"), done];
        let groups = group_events(&events);

        assert_eq!(for_group(&[pending("1", 3)], &groups[0], &events), None);
    }

    #[test]
    fn a_backing_off_activity_says_when_it_tries_again() {
        let mut p = pending("1", 3);
        p.state = PendingState::Scheduled;
        p.next_attempt_at = Some(20_000);
        assert_eq!(p.status(8_000).as_deref(), Some("retry in 12s"));
        assert_eq!(p.status(25_000).as_deref(), Some("retry due"));

        p.state = PendingState::Started;
        assert_eq!(p.status(25_000), None, "×3 already says it is retrying");

        p.state = PendingState::Paused;
        assert_eq!(p.status(25_000).as_deref(), Some("paused"));
    }

    #[test]
    fn a_first_attempt_waiting_for_a_worker_says_nothing() {
        let mut p = pending("1", 1);
        p.state = PendingState::Scheduled;
        assert_eq!(p.status(0), None);
    }

    #[test]
    fn unlimited_attempts_read_as_infinite() {
        let mut p = pending("1", 3);
        assert_eq!(p.attempts_label(), "3/∞");
        p.maximum_attempts = 5;
        assert_eq!(p.attempts_label(), "3/5");
    }

    #[test]
    fn retrying_means_failed_at_least_once() {
        let first = PendingActivity {
            attempt: 1,
            ..Default::default()
        };
        assert!(!first.is_retrying());
        let second = PendingActivity {
            attempt: 2,
            ..Default::default()
        };
        assert!(second.is_retrying());
        let failed_once = PendingActivity {
            attempt: 1,
            last_failure: Some(Failure::default()),
            ..Default::default()
        };
        assert!(
            failed_once.is_retrying(),
            "a failure on record counts whatever the attempt says"
        );
    }
}
