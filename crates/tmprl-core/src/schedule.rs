//! Schedules: the things that start workflows on a timetable.

use crate::workflow::humanize_age_ms;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRow {
    pub namespace: String,
    pub schedule_id: String,
    pub workflow_type: String,
    pub paused: bool,
    pub notes: String,
    pub spec: String,
    pub next_run: Option<i64>,
    pub recent_runs: usize,
}

impl ScheduleRow {
    pub fn key(&self) -> (&str, &str) {
        (self.namespace.as_str(), self.schedule_id.as_str())
    }

    pub fn glyph(&self) -> char {
        if self.paused { '‖' } else { '●' }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    pub start: i32,
    pub end: i32,
    pub step: i32,
}

// A schedule created with --cron is stored here, with cron_string empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calendar {
    pub second: Vec<Range>,
    pub minute: Vec<Range>,
    pub hour: Vec<Range>,
    pub day_of_month: Vec<Range>,
    pub month: Vec<Range>,
    pub day_of_week: Vec<Range>,
}

pub fn describe_calendar(c: &Calendar) -> String {
    if [
        &c.second,
        &c.minute,
        &c.hour,
        &c.day_of_month,
        &c.month,
        &c.day_of_week,
    ]
    .iter()
    .any(|f| f.is_empty())
    {
        return "never".to_string();
    }

    let minute = field(&c.minute, 0, 59);
    let hour = field(&c.hour, 0, 23);
    let dom = field(&c.day_of_month, 1, 31);
    let month = field(&c.month, 1, 12);
    let dow = field(&c.day_of_week, 0, 6);
    let five = format!("{minute} {hour} {dom} {month} {dow}");

    let second = field(&c.second, 0, 59);
    if second == "0" {
        five
    } else {
        format!("{second} {five}")
    }
}

fn field(ranges: &[Range], min: i32, max: i32) -> String {
    let parts: Vec<String> = ranges
        .iter()
        .map(|r| {
            let step = r.step.max(1);
            let covers_all = r.start <= min && r.end >= max;
            match (covers_all, step) {
                (true, 1) => "*".to_string(),
                (true, s) => format!("*/{s}"),
                (false, 1) if r.start == r.end => r.start.to_string(),
                (false, 1) => format!("{}-{}", r.start, r.end),
                (false, s) => format!("{}-{}/{s}", r.start, r.end),
            }
        })
        .collect();
    parts.join(",")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub every_secs: i64,
    pub offset_secs: i64,
}

pub fn describe_spec(cron: &[String], calendars: &[Calendar], intervals: &[Interval]) -> String {
    let mut parts: Vec<String> = cron.iter().filter(|c| !c.is_empty()).cloned().collect();
    parts.extend(calendars.iter().map(describe_calendar));

    for i in intervals {
        let every = humanize_duration(i.every_secs);
        parts.push(if i.offset_secs == 0 {
            format!("every {every}")
        } else {
            format!("every {every} at +{}", humanize_duration(i.offset_secs))
        });
    }

    if parts.is_empty() {
        "manual".to_string()
    } else {
        parts.join(", ")
    }
}

fn humanize_duration(secs: i64) -> String {
    match secs {
        s if s <= 0 => "0s".into(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3_600 == 0 => format!("{}h", s / 3_600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

pub fn time_until(next_run: Option<i64>, now: i64) -> Option<String> {
    let at = next_run?;
    Some(if at <= now {
        "due".to_string()
    } else {
        humanize_age_ms(at - now)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every(secs: i64) -> Interval {
        Interval {
            every_secs: secs,
            offset_secs: 0,
        }
    }

    #[test]
    fn a_cron_string_is_shown_verbatim() {
        assert_eq!(
            describe_spec(&["0 9 * * 1-5".into()], &[], &[]),
            "0 9 * * 1-5"
        );
    }

    #[test]
    fn an_interval_reads_as_a_period() {
        assert_eq!(describe_spec(&[], &[], &[every(3_600)]), "every 1h");
        assert_eq!(describe_spec(&[], &[], &[every(86_400)]), "every 1d");
        assert_eq!(describe_spec(&[], &[], &[every(300)]), "every 5m");
        assert_eq!(describe_spec(&[], &[], &[every(45)]), "every 45s");
    }

    #[test]
    fn an_offset_interval_says_where_it_lands() {
        let i = Interval {
            every_secs: 86_400,
            offset_secs: 32_400,
        };
        assert_eq!(describe_spec(&[], &[], &[i]), "every 1d at +9h");
    }

    #[test]
    fn several_rules_are_joined_rather_than_one_being_picked() {
        let out = describe_spec(&["0 9 * * 1-5".into()], &[], &[every(3_600)]);
        assert_eq!(out, "0 9 * * 1-5, every 1h");
    }

    fn at(field: &[(i32, i32)]) -> Vec<Range> {
        field
            .iter()
            .map(|(a, b)| Range {
                start: *a,
                end: *b,
                step: 1,
            })
            .collect()
    }

    #[test]
    fn a_cron_schedule_is_stored_as_a_calendar_and_reads_back_as_cron() {
        let c = Calendar {
            second: at(&[(0, 0)]),
            minute: at(&[(0, 0)]),
            hour: at(&[(2, 2)]),
            day_of_month: at(&[(1, 31)]),
            month: at(&[(1, 12)]),
            day_of_week: at(&[(0, 6)]),
        };
        assert_eq!(describe_calendar(&c), "0 2 * * *");
        assert_eq!(describe_spec(&[], &[c], &[]), "0 2 * * *");
    }

    fn full() -> Calendar {
        Calendar {
            second: at(&[(0, 0)]),
            minute: at(&[(0, 0)]),
            hour: at(&[(0, 23)]),
            day_of_month: at(&[(1, 31)]),
            month: at(&[(1, 12)]),
            day_of_week: at(&[(0, 6)]),
        }
    }

    #[test]
    fn a_full_range_is_a_star_and_a_step_keeps_its_slash() {
        let mut c = full();
        c.minute = vec![Range {
            start: 0,
            end: 59,
            step: 15,
        }];
        c.hour = at(&[(9, 17)]);
        assert_eq!(describe_calendar(&c), "*/15 9-17 * * *");
    }

    #[test]
    fn a_seconds_field_shows_only_when_it_is_not_zero() {
        let mut c = full();
        c.second = at(&[(30, 30)]);
        assert_eq!(describe_calendar(&c), "30 0 * * * *", "six fields");
        assert_eq!(describe_calendar(&full()), "0 * * * *", "five fields");
    }

    #[test]
    fn a_calendar_missing_a_field_never_fires() {
        let mut c = full();
        c.hour = Vec::new();
        assert_eq!(describe_calendar(&c), "never");
    }

    #[test]
    fn a_spec_with_no_rules_is_manual() {
        assert_eq!(describe_spec(&[], &[], &[]), "manual");
        assert_eq!(describe_spec(&[String::new()], &[], &[]), "manual");
    }

    #[test]
    fn the_next_run_reads_as_a_countdown() {
        let now = 1_000_000;
        assert_eq!(
            time_until(Some(now + 3_600_000), now).as_deref(),
            Some("1h")
        );
        assert_eq!(time_until(Some(now + 45_000), now).as_deref(), Some("45s"));
        assert_eq!(time_until(None, now), None);
    }

    #[test]
    fn a_run_that_is_already_due_says_so_rather_than_showing_zero() {
        let now = 1_000_000;
        assert_eq!(time_until(Some(now), now).as_deref(), Some("due"));
        assert_eq!(time_until(Some(now - 5_000), now).as_deref(), Some("due"));
    }

    #[test]
    fn paused_shows_in_the_glyph() {
        let row = |paused| ScheduleRow {
            namespace: "d".into(),
            schedule_id: "s".into(),
            workflow_type: "W".into(),
            paused,
            notes: String::new(),
            spec: "every 1h".into(),
            next_run: None,
            recent_runs: 0,
        };
        assert_ne!(row(true).glyph(), row(false).glyph());
        assert_eq!(row(false).key(), ("d", "s"));
    }
}
