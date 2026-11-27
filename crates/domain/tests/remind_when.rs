use domain::{Due, RemindPreset, is_when, parse_when};
use jiff::ZonedDateTime;

/// Sunday 2026-11-15 10:00 UTC.
fn now() -> ZonedDateTime {
    "2026-11-15T10:00:00[UTC]".parse().unwrap()
}

fn at(s: &str) -> i64 {
    s.parse::<ZonedDateTime>().unwrap().timestamp().as_second()
}

fn one(phrase: &str) -> Option<Due> {
    parse_when(phrase, &now())
}

fn due(s: &str) -> Option<Due> {
    Some(Due {
        at: at(s),
        recurring: false,
        interval_secs: 0,
    })
}

#[test]
fn relative_times() {
    assert_eq!(one("in 20 minutes"), due("2026-11-15T10:20:00[UTC]"));
    assert_eq!(one("in 2 hours"), due("2026-11-15T12:00:00[UTC]"));
    assert_eq!(one("in 1 day"), due("2026-11-16T10:00:00[UTC]"));
    assert_eq!(one("in 2 weeks"), due("2026-11-29T10:00:00[UTC]"));
}

#[test]
fn clock_times_roll_to_the_next_day_when_passed() {
    assert_eq!(one("at 15:30"), due("2026-11-15T15:30:00[UTC]"));
    assert_eq!(one("at 09:00"), due("2026-11-16T09:00:00[UTC]"));
}

#[test]
fn days_default_to_nine() {
    assert_eq!(one("tomorrow"), due("2026-11-16T09:00:00[UTC]"));
    assert_eq!(one("tomorrow at 17:45"), due("2026-11-16T17:45:00[UTC]"));
    assert_eq!(one("today at 18:00"), due("2026-11-15T18:00:00[UTC]"));
    assert_eq!(one("on monday"), due("2026-11-16T09:00:00[UTC]"));
    // Today is Sunday: "on sunday" means next week.
    assert_eq!(one("on sunday at 08:00"), due("2026-11-22T08:00:00[UTC]"));
    assert_eq!(
        one("on 2026-12-01 at 07:15"),
        due("2026-12-01T07:15:00[UTC]")
    );
}

#[test]
fn past_times_are_refused() {
    assert_eq!(one("today at 09:00"), None);
    assert_eq!(one("today"), None);
    assert_eq!(one("on 2026-11-01"), None);
}

#[test]
fn recurring_phrases() {
    let rec = |s: &str, interval_secs: i64| {
        Some(Due {
            at: at(s),
            recurring: true,
            interval_secs,
        })
    };
    let (day, week) = (86_400, 7 * 86_400);
    assert_eq!(one("every monday"), rec("2026-11-16T09:00:00[UTC]", week));
    assert_eq!(
        one("every friday at 16:00"),
        rec("2026-11-20T16:00:00[UTC]", week)
    );
    assert_eq!(one("every day"), rec("2026-11-16T09:00:00[UTC]", day));
    assert_eq!(one("every week"), rec("2026-11-22T10:00:00[UTC]", week));
    // The grammar accepts it, Parla cannot place it.
    assert!(is_when("every other tuesday"));
    assert_eq!(one("every other tuesday"), None);
}

#[test]
fn garbage_is_not_a_phrase() {
    for p in ["", "soon", "in 0 minutes", "at 25:00", "in 5 parsecs"] {
        assert_eq!(one(p), None, "{p}");
        assert!(!is_when(p), "{p}");
    }
}

#[test]
fn huge_durations_are_refused_not_panics() {
    for p in [
        "in 999999999 hours",
        "in 4294967295 minutes",
        "in 4294967295 days",
        "in 4294967295 weeks",
    ] {
        assert_eq!(one(p), None, "{p}");
    }
}

#[test]
fn snooze_and_remind_presets() {
    let n = now();
    let at_of = |p: RemindPreset| p.at(&n);
    assert_eq!(
        at_of(RemindPreset::In20Minutes),
        Some(at("2026-11-15T10:20:00[UTC]"))
    );
    assert_eq!(
        at_of(RemindPreset::In1Hour),
        Some(at("2026-11-15T11:00:00[UTC]"))
    );
    assert_eq!(
        at_of(RemindPreset::In3Hours),
        Some(at("2026-11-15T13:00:00[UTC]"))
    );
    assert_eq!(
        at_of(RemindPreset::Tomorrow),
        Some(at("2026-11-16T09:00:00[UTC]"))
    );
    // Sunday: next week is tomorrow, Monday at nine.
    assert_eq!(
        at_of(RemindPreset::NextWeek),
        Some(at("2026-11-16T09:00:00[UTC]"))
    );
    assert_eq!(RemindPreset::ALL.len(), 5);
}
