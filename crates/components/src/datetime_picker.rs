//! Custom date and time for Send later: two plain-text fields (`YYYY-MM-DD`,
//! `HH:MM`, 24 hours) read in the schedule zone.

use jiff::ZonedDateTime;
use jiff::civil::{Date, Time};

/// What the custom fields currently say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomCheck {
    /// A field is still empty.
    Incomplete,
    /// A field is not a date or a time.
    Invalid,
    /// Not after now.
    Past,
    /// More than `domain::MAX_AHEAD_DAYS` ahead.
    TooFar,
    /// Unix seconds.
    Ok(i64),
}

pub fn check_custom(date: &str, time: &str, now: &ZonedDateTime) -> CustomCheck {
    let (date, time) = (date.trim(), time.trim());
    if date.is_empty() || time.is_empty() {
        return CustomCheck::Incomplete;
    }
    let Some(at) = parse(date, time, now) else {
        return CustomCheck::Invalid;
    };
    match domain::validate(at, now.timestamp().as_second(), false, true) {
        Ok(()) => CustomCheck::Ok(at),
        Err(domain::ScheduleError::Past) => CustomCheck::Past,
        Err(domain::ScheduleError::TooFar) => CustomCheck::TooFar,
        Err(domain::ScheduleError::FilesNeedLocal) => CustomCheck::Invalid,
    }
}

/// The wall-clock reading in the zone of `now`, as unix seconds. A time
/// that does not exist (a DST gap) moves forward, as the zone database says.
fn parse(date: &str, time: &str, now: &ZonedDateTime) -> Option<i64> {
    let date: Date = date.parse().ok()?;
    let (h, m) = time.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let time = Time::new(h.parse().ok()?, m.parse().ok()?, 0, 0).ok()?;
    let at = date
        .to_datetime(time)
        .to_zoned_datetime(now.time_zone().clone())
        .ok()?;
    Some(at.timestamp().as_second())
}

/// Suggested fields: tomorrow, 09:00.
pub fn default_fields(now: &ZonedDateTime) -> (String, String) {
    let tomorrow = now.date().tomorrow().unwrap_or(now.date());
    (tomorrow.to_string(), "09:00".to_string())
}
