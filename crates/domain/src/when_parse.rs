//! Time-phrase pieces shared by Send-later presets and local reminders. All
//! arithmetic is checked: input typed by the user or read from settings never
//! panics, an unrepresentable time is `None`.

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp, ZonedDateTime};

pub(crate) enum Unit {
    Minutes,
    Hours,
    Days,
    Weeks,
}

pub(crate) fn unit(word: &str) -> Option<Unit> {
    Some(match word.to_ascii_lowercase().as_str() {
        "minute" | "minutes" | "min" => Unit::Minutes,
        "hour" | "hours" => Unit::Hours,
        "day" | "days" => Unit::Days,
        "week" | "weeks" => Unit::Weeks,
        _ => return None,
    })
}

/// The one-letter units of Send-later presets: `m`, `h`, `d`.
pub(crate) fn short_unit(c: char) -> Option<Unit> {
    match c {
        'm' => Some(Unit::Minutes),
        'h' => Some(Unit::Hours),
        'd' => Some(Unit::Days),
        _ => None,
    }
}

/// `now` plus `n` of `unit`. Days and weeks are calendar days (the wall-clock
/// time survives a DST change). `None` on overflow or out-of-range results.
pub(crate) fn offset(now: &ZonedDateTime, n: i64, unit: Unit) -> Option<Timestamp> {
    let secs = |per: i64| -> Option<Timestamp> {
        let d = SignedDuration::from_secs(n.checked_mul(per)?);
        now.timestamp().checked_add(d).ok()
    };
    let days = |n: i64| -> Option<Timestamp> {
        let span = Span::new().try_days(n).ok()?;
        Some(now.checked_add(span).ok()?.timestamp())
    };
    match unit {
        Unit::Minutes => secs(60),
        Unit::Hours => secs(3600),
        Unit::Days => days(n),
        Unit::Weeks => days(n.checked_mul(7)?),
    }
}

/// `H:MM` or `HH:MM`.
pub(crate) fn time(word: &str) -> Option<Time> {
    let (h, m) = word.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    Time::new(h.parse().ok()?, m.parse().ok()?, 0, 0).ok()
}

pub(crate) fn weekday(word: &str) -> Option<Weekday> {
    Some(match word.to_ascii_lowercase().as_str() {
        "monday" | "mon" => Weekday::Monday,
        "tuesday" | "tue" => Weekday::Tuesday,
        "wednesday" | "wed" => Weekday::Wednesday,
        "thursday" | "thu" => Weekday::Thursday,
        "friday" | "fri" => Weekday::Friday,
        "saturday" | "sat" => Weekday::Saturday,
        "sunday" | "sun" => Weekday::Sunday,
        _ => return None,
    })
}

/// The next `day` strictly after `from`.
pub(crate) fn next_weekday(from: Date, day: Weekday) -> Option<Date> {
    let mut d = from.tomorrow().ok()?;
    while d.weekday() != day {
        d = d.tomorrow().ok()?;
    }
    Some(d)
}

/// The instant of `date` at `time` in `tz`.
pub(crate) fn at(date: Date, time: Time, tz: &TimeZone) -> Option<Timestamp> {
    Some(
        date.to_datetime(time)
            .to_zoned_datetime(tz.clone())
            .ok()?
            .timestamp(),
    )
}
