//! Parsing the time window a backfill runs over.

use std::fmt;

/// How a backfill's runs behave when they collide with each other.
///
/// A schedule's own policy is usually `Skip`, which discards a run whenever the previous one
/// is still going. A backfill replays many scheduled times at once, so under `Skip` almost
/// every one is dropped and the backfill silently does nothing. Temporal's own documentation
/// says to override it, which is why a backfill carries a policy of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlap {
    /// Run them one after another, none discarded. The safe default for a backfill.
    #[default]
    BufferAll,
    /// Run them all at once. Fastest, and only safe when the workflow tolerates it.
    AllowAll,
    Skip,
    BufferOne,
    CancelOther,
    TerminateOther,
}

impl Overlap {
    /// The proto enum value. Kept beside the names so the two cannot drift.
    pub fn code(self) -> i32 {
        match self {
            Overlap::Skip => 1,
            Overlap::BufferOne => 2,
            Overlap::BufferAll => 3,
            Overlap::CancelOther => 4,
            Overlap::TerminateOther => 5,
            Overlap::AllowAll => 6,
        }
    }

    /// The spelling `temporal schedule backfill --overlap-policy` takes.
    pub fn name(self) -> &'static str {
        match self {
            Overlap::Skip => "Skip",
            Overlap::BufferOne => "BufferOne",
            Overlap::BufferAll => "BufferAll",
            Overlap::CancelOther => "CancelOther",
            Overlap::TerminateOther => "TerminateOther",
            Overlap::AllowAll => "AllowAll",
        }
    }

    /// Every policy, for parsing a name and for listing them in an error.
    pub const ALL: [Overlap; 6] = [
        Overlap::Skip,
        Overlap::BufferOne,
        Overlap::BufferAll,
        Overlap::CancelOther,
        Overlap::TerminateOther,
        Overlap::AllowAll,
    ];

    pub fn parse(s: &str) -> Option<Overlap> {
        Overlap::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s))
    }
}

/// A half-open window, epoch millis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    NoSeparator,
    BadInstant(String),
    NotBefore,
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RangeError::NoSeparator => write!(f, "expected START..END"),
            RangeError::BadInstant(s) => {
                write!(f, "cannot read `{s}` as a time: try 2026-09-01, -7d or now")
            }
            RangeError::NotBefore => write!(f, "the start must be before the end"),
        }
    }
}

impl std::error::Error for RangeError {}

/// Read `START..END`, the form a backfill is asked for.
///
/// An empty end means now, so `-7d..` is the last week. Both sides accept an absolute
/// `YYYY-MM-DD` or RFC 3339 instant, a negative offset like `-7d`, or `now`.
pub fn parse_range(input: &str, now_ms: i64) -> Result<TimeRange, RangeError> {
    let (a, b) = input.split_once("..").ok_or(RangeError::NoSeparator)?;
    let start_ms = parse_instant(a.trim(), now_ms)?;
    let end = b.trim();
    let end_ms = if end.is_empty() {
        now_ms
    } else {
        parse_instant(end, now_ms)?
    };
    if start_ms >= end_ms {
        return Err(RangeError::NotBefore);
    }
    Ok(TimeRange { start_ms, end_ms })
}

/// One end of a range: `now`, an offset such as `-36h`, or an absolute instant.
pub fn parse_instant(s: &str, now_ms: i64) -> Result<i64, RangeError> {
    if s.eq_ignore_ascii_case("now") {
        return Ok(now_ms);
    }
    if let Some(rest) = s.strip_prefix('-') {
        return parse_offset(rest)
            .map(|ms| now_ms - ms)
            .ok_or_else(|| RangeError::BadInstant(s.to_string()));
    }
    parse_absolute(s).ok_or_else(|| RangeError::BadInstant(s.to_string()))
}

/// `90s`, `30m`, `36h`, `7d`, `2w`, as milliseconds.
fn parse_offset(s: &str) -> Option<i64> {
    let (digits, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = digits.parse().ok()?;
    let scale = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return None,
    };
    n.checked_mul(scale)
}

/// `YYYY-MM-DD`, optionally `THH:MM[:SS]` and a trailing `Z`.
///
/// Always read as UTC. A backfill window is compared against scheduled times the server
/// holds in UTC, so guessing a local zone would move the window by hours without saying so.
fn parse_absolute(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, t),
        None => (s, ""),
    };

    let mut d = date.split('-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }

    let (mut h, mut min, mut sec) = (0, 0, 0);
    if !time.is_empty() {
        let mut t = time.split(':');
        h = num(t.next()?)?;
        min = num(t.next()?)?;
        if let Some(v) = t.next() {
            // Drop a fractional part rather than refusing the whole instant over it.
            sec = num(v.split('.').next()?)?;
        }
        if t.next().is_some() || h > 23 || min > 59 || sec > 60 {
            return None;
        }
    }

    let days = days_from_civil(y, m, day)?;
    Some((days * 86_400 + h * 3_600 + min * 60 + sec) * 1_000)
}

fn num(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// Howard Hinnant's `days_from_civil`, which is exact for every year in range and avoids
/// taking a date library as a dependency for the one conversion tmprl needs.
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if d > days_in_month(y, m)? {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn days_in_month(y: i64, m: i64) -> Option<i64> {
    Some(match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        _ => return None,
    })
}

/// An instant as RFC 3339, which is what `temporal schedule backfill` takes.
pub fn to_rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1_000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    let (h, min, s) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Read what the backfill prompt collects: a range, and optionally a policy after it.
///
/// One line rather than a form, because a backfill is two values and a prompt already exists.
/// Leaving the policy off gets [`Overlap::BufferAll`], which is what Temporal's own
/// documentation tells you to use.
pub fn parse_backfill(input: &str, now_ms: i64) -> Result<(TimeRange, Overlap), String> {
    let input = input.trim();
    let (range, policy) = match input.rsplit_once(char::is_whitespace) {
        // Only a trailing word that names a policy is one; anything else is part of the
        // range, so a typo is reported as a bad range rather than silently ignored.
        Some((head, tail)) => match Overlap::parse(tail) {
            Some(p) => (head.trim(), p),
            None => (input, Overlap::default()),
        },
        None => (input, Overlap::default()),
    };
    parse_range(range, now_ms)
        .map(|r| (r, policy))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-06T00:00:00Z.
    const NOW: i64 = 1_788_652_800_000;

    #[test]
    fn a_date_reads_as_utc_midnight() {
        assert_eq!(parse_instant("2026-09-06", NOW), Ok(NOW));
        assert_eq!(to_rfc3339(NOW), "2026-09-06T00:00:00Z");
    }

    #[test]
    fn an_instant_round_trips_through_rfc3339() {
        for s in [
            "1970-01-01T00:00:00Z",
            "2000-02-29T12:34:56Z",
            "2026-12-31T23:59:59Z",
        ] {
            let ms = parse_instant(s, NOW).expect(s);
            assert_eq!(to_rfc3339(ms), s);
        }
    }

    #[test]
    fn an_offset_counts_back_from_now() {
        assert_eq!(parse_instant("-1d", NOW), Ok(NOW - 86_400_000));
        assert_eq!(parse_instant("-2w", NOW), Ok(NOW - 2 * 604_800_000));
        assert_eq!(parse_instant("-90m", NOW), Ok(NOW - 90 * 60_000));
        assert_eq!(parse_instant("now", NOW), Ok(NOW));
    }

    #[test]
    fn an_omitted_end_means_now() {
        // `-7d..` is the last week, which is the shape most backfills are asked for.
        let r = parse_range("-7d..", NOW).unwrap();
        assert_eq!(r.end_ms, NOW);
        assert_eq!(r.start_ms, NOW - 7 * 86_400_000);
    }

    #[test]
    fn a_range_needs_both_ends_in_order() {
        assert_eq!(parse_range("-7d", NOW), Err(RangeError::NoSeparator));
        assert_eq!(parse_range("now..-7d", NOW), Err(RangeError::NotBefore));
        assert_eq!(parse_range("now..now", NOW), Err(RangeError::NotBefore));
    }

    #[test]
    fn an_impossible_date_is_refused_rather_than_rolled_over() {
        // Rolling 2026-02-30 into March would backfill a window nobody asked for.
        for s in ["2026-02-30", "2026-13-01", "2026-09-32", "2026-09", "hello"] {
            assert!(
                matches!(parse_instant(s, NOW), Err(RangeError::BadInstant(_))),
                "{s} should not parse"
            );
        }
    }

    #[test]
    fn a_leap_day_is_accepted_only_in_a_leap_year() {
        assert!(parse_instant("2024-02-29", NOW).is_ok());
        assert!(parse_instant("2000-02-29", NOW).is_ok(), "divisible by 400");
        assert!(
            parse_instant("1900-02-29", NOW).is_err(),
            "divisible by 100"
        );
        assert!(parse_instant("2026-02-29", NOW).is_err());
    }

    #[test]
    fn a_time_of_day_is_optional_and_seconds_within_it_are_too() {
        let day = parse_instant("2026-09-06", NOW).unwrap();
        assert_eq!(parse_instant("2026-09-06T09:30", NOW), Ok(day + 34_200_000));
        assert_eq!(
            parse_instant("2026-09-06T09:30:15Z", NOW),
            Ok(day + 34_215_000)
        );
        assert_eq!(
            parse_instant("2026-09-06T09:30:15.500Z", NOW),
            Ok(day + 34_215_000),
            "a fractional part is dropped rather than refusing the instant"
        );
    }

    #[test]
    fn a_backfill_defaults_to_running_its_actions_in_order() {
        // A schedule's own policy is usually Skip, under which a backfill discards almost
        // every run it replays and appears to do nothing.
        assert_eq!(Overlap::default(), Overlap::BufferAll);
        assert_eq!(Overlap::default().code(), 3);
    }

    #[test]
    fn a_policy_name_matches_the_cli_spelling_in_both_directions() {
        for p in Overlap::ALL {
            assert_eq!(Overlap::parse(p.name()), Some(p));
        }
        assert_eq!(Overlap::parse("bufferall"), Some(Overlap::BufferAll));
        assert_eq!(Overlap::parse("nonsense"), None);
    }
    #[test]
    fn a_backfill_line_takes_a_range_and_an_optional_policy() {
        let (r, p) = parse_backfill("-1d..now", NOW).unwrap();
        assert_eq!(r.start_ms, NOW - 86_400_000);
        assert_eq!(p, Overlap::BufferAll, "the default when none is named");

        let (_, p) = parse_backfill("-1d..now AllowAll", NOW).unwrap();
        assert_eq!(p, Overlap::AllowAll);
    }

    #[test]
    fn a_trailing_word_that_is_not_a_policy_is_not_silently_dropped() {
        // Treating it as a policy typo and ignoring it would run the backfill under a
        // policy the reader did not ask for.
        let e = parse_backfill("-1d..now BuffrAll", NOW).unwrap_err();
        assert!(e.contains("cannot read"), "{e}");
    }
}
