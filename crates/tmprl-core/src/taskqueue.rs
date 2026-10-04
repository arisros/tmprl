//! Task queues: whether anything is polling one, and how far behind it is.

/// The two kinds of task a queue carries. The server describes them separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskQueueKind {
    Workflow,
    Activity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueHealth {
    /// Tasks waiting. Approximate, and `None` when the server reports no stats at all,
    /// which must not read as an empty queue.
    pub backlog: Option<i64>,
    /// How long the oldest waiting task has waited.
    pub backlog_age_ms: Option<i64>,
    /// Workers that polled recently. The server forgets one after a few minutes.
    pub pollers: usize,
    pub last_poll_ms: Option<i64>,
}

impl QueueHealth {
    /// A queue's workflow tasks and its activity tasks as one answer: the backlogs add up,
    /// and the oldest task and the latest poll are the ones worth knowing.
    pub fn merge(self, other: QueueHealth) -> QueueHealth {
        let sum = |a: Option<i64>, b: Option<i64>| match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        QueueHealth {
            backlog: sum(self.backlog, other.backlog),
            backlog_age_ms: self.backlog_age_ms.max(other.backlog_age_ms),
            pollers: self.pollers + other.pollers,
            last_poll_ms: self.last_poll_ms.max(other.last_poll_ms),
        }
    }

    /// Work is waiting and nothing is taking it: the state this exists to make visible.
    pub fn stuck(&self) -> bool {
        self.pollers == 0 && self.backlog.is_some_and(|n| n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_kinds_of_task_merge_into_one_answer() {
        let workflow = QueueHealth {
            backlog: Some(3),
            backlog_age_ms: Some(9_000),
            pollers: 1,
            last_poll_ms: Some(50),
        };
        let activity = QueueHealth {
            backlog: Some(4),
            backlog_age_ms: Some(2_000),
            pollers: 2,
            last_poll_ms: Some(80),
        };
        assert_eq!(
            workflow.merge(activity),
            QueueHealth {
                backlog: Some(7),
                backlog_age_ms: Some(9_000),
                pollers: 3,
                last_poll_ms: Some(80),
            }
        );
    }

    #[test]
    fn stats_one_side_lacks_do_not_hide_the_other_sides() {
        let known = QueueHealth {
            backlog: Some(5),
            ..QueueHealth::default()
        };
        assert_eq!(known.clone().merge(QueueHealth::default()).backlog, Some(5));
        assert_eq!(
            QueueHealth::default().merge(QueueHealth::default()).backlog,
            None
        );
    }

    #[test]
    fn a_queue_is_stuck_when_work_waits_and_nothing_polls() {
        let waiting = |backlog, pollers| QueueHealth {
            backlog,
            pollers,
            ..QueueHealth::default()
        };
        assert!(waiting(Some(2), 0).stuck());
        assert!(!waiting(Some(2), 1).stuck());
        assert!(!waiting(Some(0), 0).stuck());
        assert!(!waiting(None, 0).stuck());
    }
}
