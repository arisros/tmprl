//! Wall-clock times.

use jiff::Timestamp;
use jiff::tz::TimeZone;

pub const STAMP_WIDTH: usize = 11;

const NO_TIME: &str = "—";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFormat {
    #[default]
    Relative,
    Absolute,
}

impl TimeFormat {
    pub fn toggled(self) -> Self {
        match self {
            Self::Relative => Self::Absolute,
            Self::Absolute => Self::Relative,
        }
    }

    pub fn is_absolute(self) -> bool {
        self == Self::Absolute
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Relative => "relative",
            Self::Absolute => "absolute",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Clock {
    tz: TimeZone,
    name: String,
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

impl Clock {
    pub fn system() -> Self {
        let tz = TimeZone::system();
        let name = tz.iana_name().unwrap_or("local").to_string();
        Self { tz, name }
    }

    pub fn named(name: &str) -> Result<Self, String> {
        match TimeZone::get(name) {
            Ok(tz) => Ok(Self {
                tz,
                name: name.to_string(),
            }),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn from_config(name: Option<&str>) -> Result<Self, String> {
        match name {
            None => Ok(Self::system()),
            Some(n) => Self::named(n),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn stamp(&self, ms: Option<i64>) -> String {
        match self.zoned(ms) {
            None => format!("{NO_TIME:>STAMP_WIDTH$}"),
            Some(z) => z.strftime("%m-%d %H:%M").to_string(),
        }
    }

    pub fn time_of_day(&self, ms: i64) -> String {
        match self.zoned(Some(ms)) {
            None => NO_TIME.to_string(),
            Some(z) => z.strftime("%H:%M:%S").to_string(),
        }
    }

    pub fn full(&self, ms: Option<i64>) -> String {
        match self.zoned(ms) {
            None => NO_TIME.to_string(),
            Some(z) => z.strftime("%Y-%m-%d %H:%M:%S%.3f %:z").to_string(),
        }
    }

    pub fn start_of_day(&self, ms: i64) -> Option<i64> {
        let z = self.zoned(Some(ms))?;
        z.start_of_day().ok()?.timestamp().as_millisecond().into()
    }

    fn zoned(&self, ms: Option<i64>) -> Option<jiff::Zoned> {
        let ms = ms?;
        Timestamp::from_millisecond(ms)
            .ok()
            .map(|t| t.to_zoned(self.tz.clone()))
    }
}

pub fn rfc3339_utc(ms: i64) -> String {
    match Timestamp::from_millisecond(ms) {
        Ok(t) => t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jakarta() -> Clock {
        Clock::named("Asia/Jakarta").unwrap()
    }

    const SAMPLE: i64 = 1_789_628_602_431;

    #[test]
    fn a_stamp_is_rendered_in_the_configured_zone_not_utc() {
        assert_eq!(jakarta().stamp(Some(SAMPLE)), "09-17 14:03");
        assert_eq!(
            Clock::named("UTC").unwrap().stamp(Some(SAMPLE)),
            "09-17 07:03"
        );
    }

    #[test]
    fn the_full_form_carries_the_offset_that_produced_it() {
        assert_eq!(
            jakarta().full(Some(SAMPLE)),
            "2026-09-17 14:03:22.431 +07:00"
        );
    }

    #[test]
    fn a_missing_time_still_fills_its_column() {
        assert_eq!(jakarta().stamp(None).chars().count(), STAMP_WIDTH);
        assert_eq!(jakarta().full(None), NO_TIME);
    }

    #[test]
    fn a_stamp_is_exactly_as_wide_as_the_column_reserved_for_it() {
        assert_eq!(jakarta().stamp(Some(SAMPLE)).chars().count(), STAMP_WIDTH);
    }

    #[test]
    fn a_timestamp_out_of_range_reads_as_absent_rather_than_panicking() {
        assert_eq!(jakarta().stamp(Some(i64::MAX)), jakarta().stamp(None));
    }

    #[test]
    fn an_unknown_zone_is_reported_rather_than_falling_back_to_utc() {
        assert!(Clock::named("Mars/Olympus").is_err());
    }

    #[test]
    fn toggling_twice_is_where_it_started() {
        let f = TimeFormat::default();
        assert_eq!(f, TimeFormat::Relative);
        assert_eq!(f.toggled(), TimeFormat::Absolute);
        assert_eq!(f.toggled().toggled(), f);
    }
}
