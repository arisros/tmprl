//! Schedules: the things that start workflows on a timetable.

use crate::workflow::humanize_age_ms;

/// One row of the schedule list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRow {
    pub namespace: String,
    pub schedule_id: String,
    pub workflow_type: String,
    pub paused: bool,
    pub notes: String,
    /// A readable form of the timetable, from [`describe_spec`].
    pub spec: String,
    /// Epoch millis of the next run, when the server offered one.
    pub next_run: Option<i64>,
    pub recent_runs: usize,
}

impl ScheduleRow {
    /// Identity across refreshes. Schedule ids are unique within a namespace, so the pair is
    /// the key, as it is for workflows.
    pub fn key(&self) -> (&str, &str) {
        (self.namespace.as_str(), self.schedule_id.as_str())
    }

    /// Shown in the list. Paused is the state worth spotting: a schedule that is not running
    /// looks identical to one that is until you read it.
    pub fn glyph(&self) -> char {
        if self.paused { '‖' } else { '●' }
    }
}

/// One `start..=end` step in a calendar field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    pub start: i32,
    pub end: i32,
    pub step: i32,
}

/// A structured calendar, one list of ranges per field.
///
/// This is what the server actually stores. Creating a schedule with `--cron "0 2 * * *"`
/// leaves `cron_string` empty and fills this in instead, so a list that reads only the cron
/// string reports every cron schedule as having no timetable at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calendar {
    pub second: Vec<Range>,
    pub minute: Vec<Range>,
    pub hour: Vec<Range>,
    pub day_of_month: Vec<Range>,
    pub month: Vec<Range>,
    pub day_of_week: Vec<Range>,
}

/// Render a structured calendar back to a cron expression.
///
/// The seconds field is included only when it is not plain zero, so an ordinary
/// five-field cron reads as one.
pub fn describe_calendar(c: &Calendar) -> String {
    // "For all fields besides year, at least one Range must be present to match anything."
    // The server populates all six on create, so this only catches a hand-built spec, but
    // rendering an empty field as `*` would claim it fires when it never does.
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

/// One cron field. A range covering the whole domain with step 1 is `*`.
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

/// An interval in a schedule spec: run every `every`, offset by `offset`, both in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub every_secs: i64,
    pub offset_secs: i64,
}

/// Turn a spec into something readable in one column.
///
/// A schedule can carry several cron strings and several intervals at once, and the protobuf
/// keeps them in separate lists. Joining them is the only way one row can say what the
/// timetable actually is.
///
/// Cron strings are shown verbatim. Anyone reading a schedule list already reads cron, and
/// rewriting `0 9 * * 1-5` as prose makes it longer and no clearer.
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
        // A spec with neither is legal: the schedule only runs when triggered by hand.
        "manual".to_string()
    } else {
        parts.join(", ")
    }
}

/// Seconds as something short: `30s`, `5m`, `1h`, `7d`.
fn humanize_duration(secs: i64) -> String {
    match secs {
        s if s <= 0 => "0s".into(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3_600 == 0 => format!("{}h", s / 3_600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// How long until the next run, or `None` when nothing is scheduled.
///
/// A paused schedule can still carry future action times, because the server computes them
/// from the spec rather than from whether it will act on them. Callers show the pause state
/// separately rather than reading it out of this.
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
        // Anyone reading a schedule list already reads cron; prose would be longer and no
        // clearer.
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
        // The protobuf keeps crons and intervals in separate lists and a schedule can carry
        // both. Showing only one would misdescribe the timetable.
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
        // Creating a schedule with --cron leaves cron_string empty and fills in the
        // structured calendar, so reading only the string reports "manual" for every one.
        // These are the exact ranges a dev server stores for `0 2 * * *`.
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

    /// Every field populated, as the server always sends them.
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
        // The proto says every field besides year needs a range to match anything, so
        // rendering the gap as `*` would claim it fires when it never does.
        let mut c = full();
        c.hour = Vec::new();
        assert_eq!(describe_calendar(&c), "never");
    }

    #[test]
    fn a_spec_with_no_rules_is_manual() {
        // Legal, and it means the schedule only runs when triggered by hand.
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
