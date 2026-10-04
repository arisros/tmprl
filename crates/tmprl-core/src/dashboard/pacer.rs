//! When each of a dashboard's requests is next asked for.

/// How a request ended, as far as asking again is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    /// Answered, with something that cannot change: there is nothing to ask again for.
    Settled,
    Failed,
    /// The server will not answer this however often it is asked.
    Refused,
}

#[derive(Debug, Clone, Copy, Default)]
struct Pace {
    in_flight: bool,
    failures: u32,
    answered_ms: Option<i64>,
    /// Not to be asked again until everything is: refused, or settled.
    refused: bool,
}

/// When each of a board's sources is next asked for.
///
/// A source is asked again one interval after its last answer, never while a request for it
/// is still out, and less and less often while it keeps failing.
#[derive(Debug, Clone, Default)]
pub struct Pacer {
    slots: Vec<Pace>,
}

impl Pacer {
    /// The longest a failing source waits between tries.
    pub const MAX_BACKOFF_MS: i64 = 300_000;

    /// Everything was just asked for, as on opening the dashboard or `R`.
    pub fn restart(&mut self, sources: usize) {
        self.slots = vec![
            Pace {
                in_flight: true,
                ..Pace::default()
            };
            sources
        ];
    }

    fn delay(interval_ms: i64, failures: u32) -> i64 {
        interval_ms
            .saturating_mul(1 << failures.min(16))
            .min(Self::MAX_BACKOFF_MS.max(interval_ms))
    }

    /// The sources to ask for now, marked as asked.
    pub fn take_due(&mut self, sources: usize, now_ms: i64, interval_ms: i64) -> Vec<usize> {
        self.slots.resize(sources, Pace::default());
        let mut due = Vec::new();
        for (i, pace) in self.slots.iter_mut().enumerate() {
            let waited = pace
                .answered_ms
                .is_none_or(|at| now_ms - at >= Self::delay(interval_ms, pace.failures));
            if !pace.in_flight && !pace.refused && waited {
                pace.in_flight = true;
                due.push(i);
            }
        }
        due
    }

    /// Record an answer. `true` when this is the first failure after things were working,
    /// which is the one worth telling someone about.
    pub fn answered(&mut self, source: usize, outcome: Outcome, now_ms: i64) -> bool {
        let Some(pace) = self.slots.get_mut(source) else {
            return false;
        };
        pace.in_flight = false;
        pace.answered_ms = Some(now_ms);
        match outcome {
            Outcome::Answered | Outcome::Settled => {
                pace.refused = outcome == Outcome::Settled;
                pace.failures = 0;
                false
            }
            Outcome::Failed | Outcome::Refused => {
                pace.refused = outcome == Outcome::Refused;
                pace.failures += 1;
                pace.failures == 1
            }
        }
    }

    /// Sources that appeared since the last look, marked as asked: the queues a board
    /// found on the rows it just received.
    pub fn take_new(&mut self, sources: usize) -> Vec<usize> {
        let from = self.slots.len();
        if sources <= from {
            return Vec::new();
        }
        self.slots.resize(
            sources,
            Pace {
                in_flight: true,
                ..Pace::default()
            },
        );
        (from..sources).collect()
    }

    pub fn in_flight(&self, source: usize) -> bool {
        self.slots.get(source).is_some_and(|p| p.in_flight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::fixtures::*;

    const EVERY: i64 = 30_000;

    #[test]
    fn a_settled_source_is_not_asked_again_until_a_restart() {
        let mut pacer = Pacer::default();
        pacer.restart(2);
        assert!(!pacer.answered(0, Outcome::Settled, NOW));
        pacer.answered(1, Outcome::Answered, NOW);
        assert_eq!(pacer.take_due(2, NOW + EVERY * 9, EVERY), [1]);

        pacer.restart(2);
        assert!(pacer.in_flight(0));
    }

    #[test]
    fn nothing_is_due_while_its_request_is_out_or_its_answer_is_fresh() {
        let mut pacer = Pacer::default();
        pacer.restart(2);
        assert!(pacer.in_flight(0) && pacer.in_flight(1));
        assert!(pacer.take_due(2, NOW + EVERY * 9, EVERY).is_empty());

        pacer.answered(0, Outcome::Answered, NOW);
        assert!(pacer.take_due(2, NOW + EVERY - 1, EVERY).is_empty());
        assert_eq!(pacer.take_due(2, NOW + EVERY, EVERY), [0]);
        assert!(
            pacer.take_due(2, NOW + EVERY, EVERY).is_empty(),
            "now it is out"
        );
    }

    #[test]
    fn a_failing_source_is_asked_less_and_less_often() {
        let mut pacer = Pacer::default();
        pacer.restart(1);
        assert!(
            pacer.answered(0, Outcome::Failed, NOW),
            "the first failure is news"
        );
        assert!(pacer.take_due(1, NOW + EVERY * 2 - 1, EVERY).is_empty());
        assert_eq!(pacer.take_due(1, NOW + EVERY * 2, EVERY), [0]);

        assert!(
            !pacer.answered(0, Outcome::Failed, NOW),
            "the second is not"
        );
        assert!(pacer.take_due(1, NOW + EVERY * 4 - 1, EVERY).is_empty());
        assert_eq!(pacer.take_due(1, NOW + EVERY * 4, EVERY), [0]);

        for _ in 0..20 {
            pacer.answered(0, Outcome::Failed, NOW);
            pacer.take_due(1, NOW + Pacer::MAX_BACKOFF_MS, EVERY);
        }
        pacer.answered(0, Outcome::Failed, NOW);
        assert_eq!(
            pacer.take_due(1, NOW + Pacer::MAX_BACKOFF_MS, EVERY),
            [0],
            "the wait is capped"
        );
    }

    #[test]
    fn an_answer_ends_the_backoff() {
        let mut pacer = Pacer::default();
        pacer.restart(1);
        pacer.answered(0, Outcome::Failed, NOW);
        pacer.take_due(1, NOW + EVERY * 2, EVERY);
        pacer.answered(0, Outcome::Answered, NOW);
        assert_eq!(pacer.take_due(1, NOW + EVERY, EVERY), [0]);
        assert!(
            pacer.answered(0, Outcome::Failed, NOW),
            "failing again is news again"
        );
    }

    #[test]
    fn a_refused_source_is_not_asked_again_until_a_restart() {
        let mut pacer = Pacer::default();
        pacer.restart(1);
        pacer.answered(0, Outcome::Refused, NOW);
        assert!(pacer.take_due(1, NOW + EVERY * 1000, EVERY).is_empty());
        pacer.restart(1);
        pacer.answered(0, Outcome::Answered, NOW);
        assert_eq!(pacer.take_due(1, NOW + EVERY, EVERY), [0]);
    }

    #[test]
    fn a_source_never_asked_for_is_due_at_once() {
        let mut pacer = Pacer::default();
        assert_eq!(pacer.take_due(2, NOW, EVERY), [0, 1]);
    }

    #[test]
    fn sources_that_appear_are_asked_for_once() {
        let mut pacer = Pacer::default();
        pacer.restart(4);
        assert_eq!(pacer.take_new(6), [4, 5]);
        assert!(pacer.in_flight(5));
        assert!(pacer.take_new(6).is_empty());
    }
}
