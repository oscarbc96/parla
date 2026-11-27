//! Pause notifications: the choices of the menu, their times, and the
//! custom duration. Pure; `notify_menu` draws it.

use i18n::Localizer;
use jiff::ZonedDateTime;
use jiff::civil::Time;

/// The hour "Until tomorrow" resumes at.
pub const RESUME_HOUR: i8 = 9;

/// The longest custom pause: one week, in minutes.
pub const MAX_PAUSE_MINUTES: u32 = 7 * 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseChoice {
    Minutes(u32),
    /// Tomorrow at [`RESUME_HOUR`], in the schedule zone.
    UntilTomorrow,
    /// Opens the duration field.
    Custom,
}

/// One row of the pause menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PauseRow {
    pub choice: PauseChoice,
    pub label: String,
    /// What the row pauses for; `None` for "Custom...".
    pub minutes: Option<u32>,
}

/// Tomorrow at [`RESUME_HOUR`] in the zone of `now` (today when tomorrow is
/// out of range). A wall time that does not exist (a DST gap) moves forward,
/// as the zone database says.
fn next_resume(now: &ZonedDateTime) -> Result<ZonedDateTime, jiff::Error> {
    now.date()
        .tomorrow()
        .unwrap_or(now.date())
        .to_datetime(Time::constant(RESUME_HOUR, 0, 0, 0))
        .to_zoned_datetime(now.time_zone().clone())
}

/// Whole minutes from `now` to tomorrow at `RESUME_HOUR` in the zone of
/// `now` (at least one).
pub fn minutes_until_tomorrow(now: &ZonedDateTime) -> u32 {
    let secs = match next_resume(now) {
        Ok(at) => at.timestamp().as_second() - now.timestamp().as_second(),
        Err(_) => 24 * 3600,
    };
    u32::try_from((secs + 59) / 60).unwrap_or(u32::MAX).max(1)
}

/// The rows in menu order, with their durations computed from `now`.
pub fn pause_rows(now: &ZonedDateTime, clock: settings::Clock, l: &Localizer) -> Vec<PauseRow> {
    let tomorrow = next_resume(now)
        .map(|z| {
            l.time(
                z.timestamp(),
                now.time_zone(),
                crate::message_row::hour_cycle(clock),
            )
        })
        .unwrap_or_default();
    let fixed = |m: u32, label: String| PauseRow {
        choice: PauseChoice::Minutes(m),
        label,
        minutes: Some(m),
    };
    vec![
        fixed(30, t!(l, "notify-pause-30m").into_owned()),
        fixed(60, t!(l, "notify-pause-1h").into_owned()),
        fixed(120, t!(l, "notify-pause-2h").into_owned()),
        PauseRow {
            choice: PauseChoice::UntilTomorrow,
            label: t!(l, "notify-pause-tomorrow", time = tomorrow).into_owned(),
            minutes: Some(minutes_until_tomorrow(now)),
        },
        PauseRow {
            choice: PauseChoice::Custom,
            label: t!(l, "notify-pause-custom").into_owned(),
            minutes: None,
        },
    ]
}

/// A custom duration: bare minutes (`45`), or `90m`, `2h`, `1h30m`.
/// `None` for anything else, zero, or more than [`MAX_PAUSE_MINUTES`].
pub fn parse_custom_minutes(text: &str) -> Option<u32> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let total: u32 = if t.bytes().all(|b| b.is_ascii_digit()) {
        t.parse().ok()?
    } else {
        let mut total = 0u32;
        let mut digits = String::new();
        let mut seen_h = false;
        let mut seen_m = false;
        for c in t.chars() {
            match c {
                '0'..='9' => digits.push(c),
                'h' | 'm' => {
                    let n: u32 = digits.parse().ok()?;
                    digits.clear();
                    let (seen, mult) = if c == 'h' {
                        (&mut seen_h, 60)
                    } else {
                        (&mut seen_m, 1)
                    };
                    if std::mem::replace(seen, true) || (c == 'h' && seen_m) {
                        return None;
                    }
                    total = total.checked_add(n.checked_mul(mult)?)?;
                }
                ' ' => {}
                _ => return None,
            }
        }
        if !digits.is_empty() {
            return None;
        }
        total
    };
    (1..=MAX_PAUSE_MINUTES).contains(&total).then_some(total)
}
