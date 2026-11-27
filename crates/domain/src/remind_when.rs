//! Reminder time phrases, read locally for providers that keep no reminders.

use jiff::civil::{Date, Time};
use jiff::{Timestamp, ToSpan, ZonedDateTime};

use crate::when_parse::{at, next_weekday, offset, time, unit, weekday};

/// When a local reminder is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    /// Unix seconds.
    pub at: i64,
    pub recurring: bool,
    /// Seconds between occurrences; `0` when not recurring.
    pub interval_secs: i64,
}

const DAY_SECS: i64 = 86_400;
const WEEK_SECS: i64 = 7 * DAY_SECS;

/// Local reminders without a time of day fire at this hour.
const DEFAULT_HOUR: i8 = 9;

/// The whole phrase is one reminder time: `in 2 hours`, `at 09:00`,
/// `tomorrow at 09:00`, `on monday at 09:00`, `on 2026-12-01`, `every monday`.
pub fn is_when(phrase: &str) -> bool {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    let Some(first) = words.first() else {
        return false;
    };
    if first.eq_ignore_ascii_case("in") {
        return words.len() == 3 && is_count(words[1]) && unit(words[2]).is_some();
    }
    if first.eq_ignore_ascii_case("at") {
        return words.len() == 2 && time(words[1]).is_some();
    }
    if first.eq_ignore_ascii_case("today") || first.eq_ignore_ascii_case("tomorrow") {
        return match words.len() {
            1 => true,
            3 => words[1].eq_ignore_ascii_case("at") && time(words[2]).is_some(),
            _ => false,
        };
    }
    if first.eq_ignore_ascii_case("on") {
        let day = words
            .get(1)
            .is_some_and(|d| weekday(d).is_some() || d.parse::<Date>().is_ok());
        return match words.len() {
            2 => day,
            4 => day && words[2].eq_ignore_ascii_case("at") && time(words[3]).is_some(),
            _ => false,
        };
    }
    first.eq_ignore_ascii_case("every") && words.len() >= 2
}

/// Whether `phrase` repeats ("every monday"); an editor whose own control
/// sets the repeat refuses such a time.
pub fn is_repeating_when(phrase: &str) -> bool {
    is_when(phrase)
        && phrase
            .split_whitespace()
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("every"))
}

/// The next time `phrase` names, after `now`; `None` when the phrase is not
/// one Parla can keep (a time in the past, or an `every` it cannot place).
pub fn parse_when(phrase: &str, now: &ZonedDateTime) -> Option<Due> {
    if !is_when(phrase) {
        return None;
    }
    let lower = phrase.to_ascii_lowercase();
    let w: Vec<&str> = lower.split_whitespace().collect();
    let tz = now.time_zone();
    let today = now.date();
    let ahead = |date: Date, t: Time| -> Option<Timestamp> {
        let at = at(date, t, tz)?;
        (at > now.timestamp()).then_some(at)
    };
    let nine = Time::constant(DEFAULT_HOUR, 0, 0, 0);
    let once = |at: Option<Timestamp>| {
        at.map(|a| Due {
            at: a.as_second(),
            recurring: false,
            interval_secs: 0,
        })
    };
    match w[0] {
        "in" => {
            let n: i64 = w[1].parse().ok()?;
            once(Some(offset(now, n, unit(w[2])?)?))
        }
        "at" => {
            let t = time(w[1])?;
            once(ahead(today, t).or_else(|| ahead(today.tomorrow().ok()?, t)))
        }
        "today" => {
            let t = if w.len() == 3 { time(w[2])? } else { nine };
            once(ahead(today, t))
        }
        "tomorrow" => {
            let t = if w.len() == 3 { time(w[2])? } else { nine };
            once(ahead(today.tomorrow().ok()?, t))
        }
        "on" => {
            let t = if w.len() == 4 { time(w[3])? } else { nine };
            let date = match weekday(w[1]) {
                Some(d) => next_weekday(today, d)?,
                None => w[1].parse::<Date>().ok()?,
            };
            once(ahead(date, t))
        }
        "every" => {
            let t = if w.len() >= 4 && w[w.len() - 2] == "at" {
                time(w[w.len() - 1])?
            } else {
                nine
            };
            let head = &w[1..if w.len() >= 4 && w[w.len() - 2] == "at" {
                w.len() - 2
            } else {
                w.len()
            }];
            let (at, interval_secs) = match head {
                [d] if weekday(d).is_some() => {
                    (ahead(next_weekday(today, weekday(d)?)?, t)?, WEEK_SECS)
                }
                ["day"] => (
                    ahead(today, t).or_else(|| ahead(today.tomorrow().ok()?, t))?,
                    DAY_SECS,
                ),
                ["week"] => (now.checked_add(7.days()).ok()?.timestamp(), WEEK_SECS),
                _ => return None,
            };
            Some(Due {
                at: at.as_second(),
                recurring: true,
                interval_secs,
            })
        }
        _ => None,
    }
}

fn is_count(word: &str) -> bool {
    word.parse::<u32>().is_ok_and(|n| n > 0)
}

/// The quick times a reminder can be set or snoozed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RemindPreset {
    In20Minutes,
    In1Hour,
    In3Hours,
    /// Tomorrow at nine.
    Tomorrow,
    /// Next Monday at nine.
    NextWeek,
}

impl RemindPreset {
    pub const ALL: [RemindPreset; 5] = [
        Self::In20Minutes,
        Self::In1Hour,
        Self::In3Hours,
        Self::Tomorrow,
        Self::NextWeek,
    ];

    fn phrase(self) -> &'static str {
        match self {
            Self::In20Minutes => "in 20 minutes",
            Self::In1Hour => "in 1 hour",
            Self::In3Hours => "in 3 hours",
            Self::Tomorrow => "tomorrow",
            Self::NextWeek => "on monday",
        }
    }

    /// Unix seconds of the preset after `now`.
    pub fn at(self, now: &ZonedDateTime) -> Option<i64> {
        parse_when(self.phrase(), now).map(|d| d.at)
    }
}
