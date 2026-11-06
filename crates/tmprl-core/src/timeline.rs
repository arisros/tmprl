//! The time axis behind the timeline view.

use crate::history::{Category, Group};

pub const GAP_COLUMNS: usize = 3;

const COLLAPSE_RATIO: f64 = 0.1;

const TICK_SPACING: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: i64,
    pub end: i64,
    pub active: bool,
    pub collapsed: bool,
}

impl Segment {
    pub fn duration(&self) -> i64 {
        self.end - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    start: i64,
    end: i64,
    segments: Vec<Segment>,
}

impl Timeline {
    pub fn new(groups: &[Group], now: i64) -> Option<Self> {
        let workflow = groups.iter().find(|g| g.category == Category::Workflow);
        let start = workflow
            .and_then(|w| w.started_at)
            .or_else(|| groups.iter().filter_map(|g| g.started_at).min())?;
        let latest = groups
            .iter()
            .filter_map(|g| g.ended_at.or(g.started_at))
            .max()
            .unwrap_or(start);
        let running = match workflow {
            Some(w) => w.is_open(),
            None => groups.iter().any(Group::is_open),
        };
        let end = match workflow.and_then(|w| w.ended_at) {
            Some(closed) if !running => closed.max(latest),
            _ if running => now.max(latest),
            _ => latest,
        }
        .max(start + 1);

        let mut t = Self {
            start,
            end,
            segments: segments(groups, start, end),
        };
        t.fold_gaps(true);
        Some(t)
    }

    pub fn start(&self) -> i64 {
        self.start
    }

    pub fn end(&self) -> i64 {
        self.end
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn fold_gaps(&mut self, fold: bool) {
        if !fold {
            self.segments.iter_mut().for_each(|s| s.collapsed = false);
            return;
        }
        if self.segments.len() <= 1 {
            return;
        }
        loop {
            let scaled: i64 = self
                .segments
                .iter()
                .filter(|s| !s.collapsed)
                .map(Segment::duration)
                .sum();
            if scaled <= 0 {
                return;
            }
            let next = self.segments.iter_mut().find(|s| {
                !s.active && !s.collapsed && s.duration() as f64 / scaled as f64 >= COLLAPSE_RATIO
            });
            match next {
                Some(s) => s.collapsed = true,
                None => return,
            }
        }
    }

    pub fn has_folds(&self) -> bool {
        self.segments.iter().any(|s| s.collapsed)
    }

    pub fn scale(&self, width: usize) -> Scale {
        Scale::new(self, width)
    }
}

fn segments(groups: &[Group], start: i64, end: i64) -> Vec<Segment> {
    let mut spans: Vec<(i64, i64)> = groups
        .iter()
        .filter(|g| g.category != Category::Workflow && !g.category.is_plumbing())
        .filter_map(|g| {
            let from = g.started_at?.clamp(start, end);
            let to = match g.ended_at {
                Some(e) => e,
                None if g.is_open() => end,
                None => from,
            }
            .clamp(start, end);
            (to > from).then_some((from, to))
        })
        .collect();
    if !spans.is_sorted_by_key(|s| s.0) {
        spans.sort_by_key(|s| s.0);
    }

    let mut out: Vec<Segment> = Vec::new();
    let mut cursor = start;
    for (from, to) in spans {
        if let Some(last) = out.last_mut()
            && last.active
            && from <= last.end
        {
            if to > last.end {
                last.end = to;
                cursor = to;
            }
            continue;
        }
        if cursor < from {
            out.push(idle(cursor, from));
        }
        out.push(Segment {
            start: from,
            end: to,
            active: true,
            collapsed: false,
        });
        cursor = to;
    }
    if cursor < end {
        out.push(idle(cursor, end));
    }
    out
}

fn idle(start: i64, end: i64) -> Segment {
    Segment {
        start,
        end,
        active: false,
        collapsed: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    start: i64,
    end: i64,
    from: f64,
    to: f64,
    collapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    pub column: usize,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    width: usize,
    placed: Vec<Placed>,
}

impl Scale {
    fn new(t: &Timeline, width: usize) -> Self {
        let span = width.saturating_sub(1) as f64;
        let folds = t.segments.iter().filter(|s| s.collapsed).count();
        let gap = if folds * GAP_COLUMNS * 2 <= width {
            GAP_COLUMNS as f64
        } else {
            1.0
        };
        let scaled_ms: i64 = t
            .segments
            .iter()
            .filter(|s| !s.collapsed)
            .map(Segment::duration)
            .sum();
        let scaled_cols = (span - gap * folds as f64).max(0.0);
        let expanded = t.segments.len() - folds;

        let mut x = 0.0;
        let placed = t
            .segments
            .iter()
            .map(|s| {
                let w = if s.collapsed {
                    gap.min(span)
                } else if scaled_ms > 0 {
                    scaled_cols * s.duration() as f64 / scaled_ms as f64
                } else {
                    scaled_cols / expanded.max(1) as f64
                };
                let p = Placed {
                    start: s.start,
                    end: s.end,
                    from: x,
                    to: x + w,
                    collapsed: s.collapsed,
                };
                x += w;
                p
            })
            .collect();
        Self { width, placed }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn project(&self, ms: i64) -> f64 {
        let (Some(first), Some(last)) = (self.placed.first(), self.placed.last()) else {
            return 0.0;
        };
        if ms <= first.start {
            return first.from;
        }
        if ms >= last.end {
            return last.to;
        }
        let i = self.placed.partition_point(|p| p.end < ms);
        let p = &self.placed[i.min(self.placed.len() - 1)];
        let dur = (p.end - p.start).max(1) as f64;
        p.from + (ms - p.start) as f64 / dur * (p.to - p.from)
    }

    pub fn column(&self, ms: i64) -> usize {
        (self.project(ms).round().max(0.0) as usize).min(self.width.saturating_sub(1))
    }

    pub fn unproject(&self, column: f64) -> i64 {
        let (Some(first), Some(last)) = (self.placed.first(), self.placed.last()) else {
            return 0;
        };
        if column <= first.from {
            return first.start;
        }
        if column >= last.to {
            return last.end;
        }
        let i = self.placed.partition_point(|p| p.to < column);
        let p = &self.placed[i.min(self.placed.len() - 1)];
        let w = (p.to - p.from).max(f64::EPSILON);
        p.start + ((column - p.from) / w * (p.end - p.start) as f64).round() as i64
    }

    pub fn folds(&self) -> Vec<std::ops::RangeInclusive<usize>> {
        self.placed
            .iter()
            .filter(|p| p.collapsed)
            .map(|p| {
                let from = p.from.round() as usize;
                let to = ((p.to.round() as usize).saturating_sub(1)).max(from);
                from..=to.min(self.width.saturating_sub(1))
            })
            .collect()
    }

    fn in_fold(&self, column: usize) -> bool {
        self.folds().iter().any(|r| r.contains(&column))
    }

    pub fn ticks(&self) -> Vec<Tick> {
        if self.width < 2 {
            return Vec::new();
        }
        let count = (self.width / TICK_SPACING).clamp(2, 40);
        let step = self.width as f64 / count as f64;
        (1..count)
            .map(|i| (i as f64 * step).round() as usize)
            .filter(|c| *c < self.width && !self.in_fold(*c))
            .map(|column| Tick {
                column,
                at: self.unproject(column as f64),
            })
            .collect()
    }
}

pub fn format_offset(ms: i64, with_millis: bool) -> String {
    let ms = ms.max(0);
    let mut parts = Vec::new();
    let mut rest = ms / 1000;
    for (unit, size) in [("d", 86_400), ("h", 3_600), ("m", 60)] {
        if rest >= size {
            parts.push(format!("{}{unit}", rest / size));
            rest %= size;
        }
    }
    if rest > 0 {
        parts.push(format!("{rest}s"));
    }
    let millis = ms % 1000;
    if millis > 0 && (with_millis || parts.is_empty()) {
        parts.push(format!("{millis}ms"));
    }
    if parts.is_empty() {
        return "0s".into();
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{GroupRef, Outcome};

    fn group(category: Category, start: i64, end: Option<i64>, outcome: Outcome) -> Group {
        Group {
            key: GroupRef::Opened(start),
            category,
            subject: String::new(),
            events: vec![start],
            started_at: Some(start),
            ended_at: end,
            outcome,
            attempts: 1,
            failure: None,
        }
    }

    fn workflow(start: i64, end: Option<i64>) -> Group {
        let outcome = if end.is_some() {
            Outcome::Completed
        } else {
            Outcome::Pending
        };
        Group {
            key: GroupRef::Workflow,
            ..group(Category::Workflow, start, end, outcome)
        }
    }

    fn activity(start: i64, end: i64) -> Group {
        group(Category::Activity, start, Some(end), Outcome::Completed)
    }

    #[test]
    fn a_closed_run_spans_start_to_close() {
        let t = Timeline::new(
            &[workflow(1_000, Some(5_000)), activity(1_000, 5_000)],
            99_999,
        )
        .unwrap();
        assert_eq!((t.start(), t.end()), (1_000, 5_000));
    }

    #[test]
    fn a_running_run_reaches_now() {
        let t = Timeline::new(&[workflow(1_000, None), activity(1_000, 2_000)], 9_000).unwrap();
        assert_eq!(t.end(), 9_000, "the axis grows while the workflow runs");
    }

    #[test]
    fn a_history_with_no_times_has_no_axis() {
        let mut w = workflow(0, None);
        w.started_at = None;
        assert!(Timeline::new(&[w], 10).is_none());
    }

    #[test]
    fn overlapping_groups_make_one_active_segment_and_the_rest_is_idle() {
        let t = Timeline::new(
            &[
                workflow(0, Some(100)),
                activity(10, 30),
                activity(20, 40),
                activity(60, 70),
            ],
            0,
        )
        .unwrap();
        let shape: Vec<_> = t
            .segments()
            .iter()
            .map(|s| (s.start, s.end, s.active))
            .collect();
        assert_eq!(
            shape,
            [
                (0, 10, false),
                (10, 40, true),
                (40, 60, false),
                (60, 70, true),
                (70, 100, false),
            ]
        );
    }

    #[test]
    fn the_workflow_and_its_tasks_never_count_as_activity() {
        let t = Timeline::new(
            &[
                workflow(0, Some(100)),
                group(Category::WorkflowTask, 0, Some(100), Outcome::Completed),
            ],
            0,
        )
        .unwrap();
        assert_eq!(t.segments().len(), 1);
        assert!(!t.segments()[0].active);
    }

    #[test]
    fn a_long_idle_stretch_is_folded_and_a_short_one_is_not() {
        let hour = 3_600_000;
        let t = Timeline::new(
            &[
                workflow(0, Some(hour + 4_001)),
                activity(0, 1_000),
                activity(1_001, 2_000),
                activity(hour + 2_000, hour + 4_001),
            ],
            0,
        )
        .unwrap();
        let folded: Vec<_> = t
            .segments()
            .iter()
            .filter(|s| s.collapsed)
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(folded, [(2_000, hour + 2_000)]);
    }

    #[test]
    fn folding_repeats_until_no_gap_qualifies() {
        let t = Timeline::new(
            &[
                workflow(0, Some(100)),
                activity(0, 26),
                activity(66, 86),
                activity(94, 100),
            ],
            0,
        )
        .unwrap();
        let folded: Vec<_> = t
            .segments()
            .iter()
            .filter(|s| s.collapsed)
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(folded, [(26, 66), (86, 94)]);
    }

    #[test]
    fn unfolding_restores_every_gap() {
        let hour = 3_600_000;
        let mut t = Timeline::new(
            &[
                workflow(0, Some(hour)),
                activity(0, 1_000),
                activity(hour - 1_000, hour),
            ],
            0,
        )
        .unwrap();
        assert!(t.has_folds());
        t.fold_gaps(false);
        assert!(!t.has_folds());
    }

    #[test]
    fn the_run_fills_the_width_end_to_end() {
        let t = Timeline::new(&[workflow(0, Some(1_000)), activity(0, 1_000)], 0).unwrap();
        let s = t.scale(101);
        assert_eq!(s.project(0), 0.0);
        assert_eq!(s.project(1_000), 100.0);
        assert_eq!(s.column(500), 50);
        assert_eq!(s.unproject(50.0), 500);
    }

    #[test]
    fn times_outside_the_run_are_clamped_to_its_edges() {
        let t = Timeline::new(&[workflow(0, Some(1_000)), activity(0, 1_000)], 0).unwrap();
        let s = t.scale(11);
        assert_eq!(s.column(-5_000), 0);
        assert_eq!(s.column(i64::MAX), 10);
    }

    #[test]
    fn a_folded_gap_takes_a_fixed_width_and_the_work_gets_the_rest() {
        let hour = 3_600_000;
        let t = Timeline::new(
            &[
                workflow(0, Some(hour + 2_000)),
                activity(0, 1_000),
                activity(hour + 1_000, hour + 2_000),
            ],
            0,
        )
        .unwrap();
        let s = t.scale(63);
        let folds = s.folds();
        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].clone().count(), GAP_COLUMNS);
        assert!(
            (s.project(1_000) - 29.5).abs() < 1e-9,
            "{}",
            s.project(1_000)
        );
        assert!((s.project(hour + 1_000) - 32.5).abs() < 1e-9);
        assert_eq!(s.project(hour + 2_000), 62.0);
    }

    #[test]
    fn a_narrow_pane_shrinks_each_fold_to_one_column() {
        let t = Timeline::new(
            &[
                workflow(0, Some(100_000)),
                activity(0, 10),
                activity(50_000, 50_010),
                activity(99_990, 100_000),
            ],
            0,
        )
        .unwrap();
        let s = t.scale(8);
        assert!(
            s.folds().iter().all(|r| r.clone().count() == 1),
            "{:?}",
            s.folds()
        );
    }

    #[test]
    fn ticks_skip_the_origin_and_the_folds() {
        let hour = 3_600_000;
        let t = Timeline::new(
            &[
                workflow(0, Some(hour + 2_000)),
                activity(0, 1_000),
                activity(hour + 1_000, hour + 2_000),
            ],
            0,
        )
        .unwrap();
        let s = t.scale(120);
        let ticks = s.ticks();
        assert!(!ticks.is_empty());
        assert!(ticks.iter().all(|k| k.column > 0));
        let folds = s.folds();
        assert!(
            ticks.iter().all(|k| !folds[0].contains(&k.column)),
            "{ticks:?} {folds:?}"
        );
        assert!(ticks.windows(2).all(|w| w[0].at < w[1].at));
    }

    #[test]
    fn offsets_read_like_the_web_axis() {
        assert_eq!(format_offset(0, false), "0s");
        assert_eq!(format_offset(90_000, false), "1m 30s");
        assert_eq!(format_offset(2 * 3_600_000 + 5 * 60_000, false), "2h 5m");
        assert_eq!(format_offset(1_250, false), "1s");
        assert_eq!(format_offset(1_250, true), "1s 250ms");
        assert_eq!(format_offset(250, false), "250ms");
        assert_eq!(format_offset(86_400_000 + 1_000, false), "1d 1s");
    }
}
