//! The histogram panel's time axis: the stretches of time its columns stand for.

use super::layout::Window;

/// The most columns a histogram has. Each is one count.
pub const MAX_BUCKETS: usize = 48;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

const STEPS: [i64; 11] = [
    MINUTE,
    5 * MINUTE,
    15 * MINUTE,
    30 * MINUTE,
    HOUR,
    2 * HOUR,
    3 * HOUR,
    6 * HOUR,
    12 * HOUR,
    DAY,
    7 * DAY,
];

/// One column: a stretch of time, and how many workflows fell in it once that is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub from_ms: i64,
    pub to_ms: i64,
    pub count: Option<i64>,
}

/// The finest round step that covers `span_ms` in no more than [`MAX_BUCKETS`] columns.
pub fn bucket_for(span_ms: i64) -> i64 {
    let most = MAX_BUCKETS as i64;
    let columns = |step: i64| (span_ms + step - 1) / step;
    STEPS
        .into_iter()
        .find(|step| columns(*step) <= most)
        .unwrap_or_else(|| columns(most).max(1))
}

/// How far back a window reaches, and how wide it is.
pub fn span_ms(window: &Window) -> i64 {
    window.since_ms.unwrap_or(0) - window.older_ms.unwrap_or(0)
}

/// The stretches a window is cut into at `now_ms`, oldest first.
///
/// Edges sit on multiples of the step, not on `now_ms`, so a stretch that has passed is the
/// same stretch at every refresh and its count is asked for once. The price is that the
/// first and last columns are part of a step.
pub fn edges(window: &Window, bucket_ms: i64, now_ms: i64) -> Vec<(i64, i64)> {
    let bucket_ms = bucket_ms.max(1);
    let end = now_ms - window.older_ms.unwrap_or(0);
    let start = end - span_ms(window).max(0);
    let mut out = Vec::new();
    let mut from = start.div_euclid(bucket_ms) * bucket_ms;
    while from < end {
        out.push((from, from + bucket_ms));
        from += bucket_ms;
    }
    let extra = out.len().saturating_sub(MAX_BUCKETS);
    out.split_off(extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_is_the_finest_round_one_that_fits() {
        assert_eq!(bucket_for(HOUR), 5 * MINUTE);
        assert_eq!(bucket_for(DAY), 30 * MINUTE);
        assert_eq!(bucket_for(2 * DAY), HOUR);
        assert_eq!(bucket_for(7 * DAY), 6 * HOUR);
        assert_eq!(bucket_for(30 * DAY), DAY);
        assert_eq!(bucket_for(48 * 7 * DAY), 7 * DAY);
        assert_eq!(bucket_for(480 * 7 * DAY), 10 * 7 * DAY, "past the ladder");
    }

    #[test]
    fn edges_sit_on_the_step_so_a_past_stretch_does_not_move() {
        let window = Window::since(3 * HOUR);
        let now = 100 * HOUR + 20 * MINUTE;
        let first = edges(&window, HOUR, now);
        assert_eq!(
            first,
            [
                (97 * HOUR, 98 * HOUR),
                (98 * HOUR, 99 * HOUR),
                (99 * HOUR, 100 * HOUR),
                (100 * HOUR, 101 * HOUR),
            ]
        );
        let later = edges(&window, HOUR, now + 30 * MINUTE);
        assert_eq!(later, first, "half an hour on, the same stretches");
        let next = edges(&window, HOUR, now + HOUR);
        assert_eq!(
            next[..3],
            first[1..],
            "an hour on, one has gone and one is new"
        );
    }

    #[test]
    fn a_window_that_ends_in_the_past_is_cut_up_to_there() {
        let window = Window {
            since_ms: Some(4 * HOUR),
            older_ms: Some(2 * HOUR),
            ..Window::default()
        };
        assert_eq!(
            edges(&window, HOUR, 100 * HOUR),
            [(96 * HOUR, 97 * HOUR), (97 * HOUR, 98 * HOUR)]
        );
    }

    #[test]
    fn there_are_never_more_columns_than_the_most_allowed() {
        let window = Window::since(DAY);
        let all = edges(&window, MINUTE, 1_000 * DAY + 30_000);
        assert_eq!(all.len(), MAX_BUCKETS);
        assert_eq!(
            all.last().unwrap().0,
            1_000 * DAY,
            "the newest are the ones kept"
        );
    }
}
