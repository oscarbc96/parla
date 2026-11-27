mod common;

use components::timeline_dividers::{DayLabel, Lead, compute, day_text};
use domain::test_support::{msg, ts, uid};
use domain::{DeliveryState, Message};
use jiff::tz::TimeZone;

fn m(t: &str, author: &str) -> Message {
    let mut x = msg("C1", t, "hi");
    x.author = Some(uid(author));
    x
}

// 2023-11-14 22:13:20 UTC
const NOW: i64 = 1_700_000_000;
const DAY: i64 = 86_400;

fn at(secs: i64) -> String {
    format!("{secs}.000001")
}

#[test]
fn day_dividers_label_today_yesterday_and_older_days() {
    let tz = TimeZone::UTC;
    let msgs = vec![
        m(&at(NOW - 3 * DAY), "u2"),
        m(&at(NOW - DAY), "u2"),
        m(&at(NOW - DAY + 60), "u2"),
        m(&at(NOW - 60), "u2"),
    ];
    let leads = compute(&msgs, &tz, NOW, None, None);
    assert_eq!(leads[0].day, Some(DayLabel::On(NOW - 3 * DAY)));
    assert_eq!(leads[1].day, Some(DayLabel::Yesterday));
    assert_eq!(leads[2].day, None);
    assert_eq!(leads[3].day, Some(DayLabel::Today));
    assert!(leads.iter().all(|l| !l.new));
}

#[test]
fn day_boundary_follows_the_time_zone() {
    // 23:30 and 00:30 UTC are the same day in Madrid (UTC+1 in winter): 00:30 and 01:30.
    let a = 1_700_004_600; // 2023-11-14 23:30:00 UTC
    let b = a + 3600; // 2023-11-15 00:30:00 UTC
    let msgs = vec![m(&at(a), "u2"), m(&at(b), "u2")];
    let utc = compute(&msgs, &TimeZone::UTC, b, None, None);
    assert!(utc[0].day.is_some() && utc[1].day.is_some());
    let madrid = TimeZone::get("Europe/Madrid").expect("tzdb");
    let es = compute(&msgs, &madrid, b, None, None);
    assert_eq!(es[0].day, Some(DayLabel::Today));
    assert_eq!(es[1].day, None);
}

#[test]
fn dst_change_days_stay_whole_days() {
    // Europe/Madrid left DST on 2023-10-29: that day has 25 hours.
    let tz = TimeZone::get("Europe/Madrid").expect("tzdb");
    let start = 1_698_530_400; // 2023-10-29 00:00 +02:00
    let msgs = vec![
        m(&at(start + 60), "u2"),
        m(&at(start + 24 * 3600 + 1800), "u2"), // still 29th (25h day), 23:30 +01:00
        m(&at(start + 25 * 3600 + 60), "u2"),   // 30th 00:01
    ];
    let leads = compute(&msgs, &tz, start + 40 * 3600, None, None);
    assert!(leads[0].day.is_some());
    assert_eq!(leads[1].day, None);
    assert!(leads[2].day.is_some());
    // Yesterday is the civil day before, even across the shift.
    let after = compute(&msgs, &tz, start + 25 * 3600 + 3600, None, None);
    assert_eq!(after[0].day, Some(DayLabel::Yesterday));
    assert_eq!(after[2].day, Some(DayLabel::Today));
}

#[test]
fn new_divider_sits_above_the_first_unread_message_only() {
    let tz = TimeZone::UTC;
    let msgs = vec![
        m(&at(NOW - 300), "u2"),
        m(&at(NOW - 200), "u2"),
        m(&at(NOW - 100), "u2"),
    ];
    let read = ts(&at(NOW - 250));
    let leads = compute(&msgs, &tz, NOW, Some(&read), Some(&uid("u1")));
    assert_eq!(
        leads.iter().map(|l| l.new).collect::<Vec<_>>(),
        [false, true, false]
    );
}

#[test]
fn no_new_divider_when_all_read_or_without_a_marker() {
    let tz = TimeZone::UTC;
    let msgs = vec![m(&at(NOW - 300), "u2"), m(&at(NOW - 200), "u2")];
    let read = ts(&at(NOW - 200));
    let all_read = compute(&msgs, &tz, NOW, Some(&read), Some(&uid("u1")));
    assert!(all_read.iter().all(|l| !l.new));
    let none = compute(&msgs, &tz, NOW, None, Some(&uid("u1")));
    assert!(none.iter().all(|l| !l.new));
}

#[test]
fn own_and_undelivered_messages_do_not_start_the_unread_run() {
    let tz = TimeZone::UTC;
    let mut pending = m(&at(NOW - 150), "u2");
    pending.state = DeliveryState::Pending;
    let msgs = vec![
        m(&at(NOW - 300), "u2"),
        m(&at(NOW - 200), "u1"),
        pending,
        m(&at(NOW - 100), "u2"),
    ];
    let read = ts(&at(NOW - 300));
    let leads = compute(&msgs, &tz, NOW, Some(&read), Some(&uid("u1")));
    assert_eq!(
        leads.iter().map(|l| l.new).collect::<Vec<_>>(),
        [false, false, false, true]
    );
    assert_eq!(compute(&[], &tz, NOW, None, None), Vec::<Lead>::new());
}

#[test]
fn day_text_is_localised() {
    let tz = TimeZone::UTC;
    assert_eq!(day_text(DayLabel::Today, &tz, &common::en()), "Today");
    assert_eq!(
        day_text(DayLabel::Yesterday, &tz, &common::en()),
        "Yesterday"
    );
    assert_eq!(
        day_text(DayLabel::Yesterday, &tz, &common::localizer("es")),
        "Ayer"
    );
    assert_eq!(
        day_text(DayLabel::On(NOW), &tz, &common::en()),
        "Nov 14, 2023"
    );
}

/// Unconfirmed rows sit at the bottom with their local ts, which may be from
/// an earlier day than the confirmed rows above them: no older day divider
/// is drawn out of order, and the days of the confirmed rows still read right.
#[test]
fn local_rows_at_the_bottom_never_draw_an_older_day() {
    let tz = TimeZone::UTC;
    let mut failed = m(&at(NOW - DAY), "u1");
    failed.state = DeliveryState::Failed("no".into());
    let mut pending = m(&at(NOW - 30), "u1");
    pending.state = DeliveryState::Pending;
    let msgs = vec![
        m(&at(NOW - 3 * DAY), "u2"),
        m(&at(NOW - 60), "u2"),
        failed,
        pending,
    ];
    let leads = compute(&msgs, &tz, NOW, None, None);
    assert_eq!(leads[0].day, Some(DayLabel::On(NOW - 3 * DAY)));
    assert_eq!(leads[1].day, Some(DayLabel::Today));
    assert_eq!(leads[2].day, None, "no Yesterday divider below Today");
    assert_eq!(leads[3].day, None);
}
