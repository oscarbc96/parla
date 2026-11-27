//! Reminders for a provider that keeps none: held and persisted by Parla.

use std::sync::Arc;

use domain::test_support::uid;
use domain::{
    Bootstrap, Changed, Effect, Input, PersistOp, Reminder, ReminderId, ReminderLink,
    ReminderRepeat, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![],
            members: vec![],
        }),
    );
    assert!(!s.caps.reminders);
    s
}

fn id(s: &str) -> ReminderId {
    ReminderId(Arc::from(s))
}

fn add(s: &mut WorkspaceState, i: &str, due: i64) -> Vec<Effect> {
    reduce(
        s,
        Input::AddLocalReminder {
            id: id(i),
            text: Arc::from(i),
            due,
            recurring: false,
            interval_secs: 0,
            user: uid("me"),
            link: None,
        },
    )
}

fn no_call(fx: &[Effect]) -> bool {
    !fx.iter()
        .any(|e| matches!(e, Effect::Call(_) | Effect::Load(_)))
}

#[test]
fn add_holds_and_persists_without_a_call() {
    let mut s = state();
    let fx = add(&mut s, "a", 100);
    assert!(no_call(&fx));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Reminders)))
    );
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.id == id("a") && r.due == Some(100) && !r.completed)
    ));
    assert_eq!(s.reminders.as_ref().map(Vec::len), Some(1));
}

#[test]
fn complete_marks_and_persists_without_a_call() {
    let mut s = state();
    add(&mut s, "a", 100);
    let fx = reduce(&mut s, Input::CompleteReminder(id("a")));
    assert!(no_call(&fx));
    assert!(s.reminders.as_ref().unwrap()[0].completed);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.completed))
    );
}

#[test]
fn delete_removes_and_persists_without_a_call() {
    let mut s = state();
    add(&mut s, "a", 100);
    add(&mut s, "b", 200);
    let fx = reduce(&mut s, Input::DeleteReminder(id("a")));
    assert!(no_call(&fx));
    assert_eq!(s.reminders.as_ref().unwrap().len(), 1);
    assert!(
        fx.iter().any(
            |e| matches!(e, Effect::Persist(PersistOp::LocalReminderRemove(r)) if *r == id("a"))
        )
    );
}

#[test]
fn open_never_loads_from_the_provider() {
    let mut s = state();
    let fx = reduce(&mut s, Input::OpenReminders);
    assert!(no_call(&fx));
}

#[test]
fn loaded_replaces_the_list() {
    let mut s = state();
    let r = Reminder {
        id: id("x"),
        text: Arc::from("x"),
        due: Some(5),
        recurring: true,
        completed: false,
        fired: false,
        interval_secs: 0,
        for_user: uid("me"),
        link: None,
        snoozed_until: None,
    };
    let fx = reduce(&mut s, Input::LocalRemindersLoaded(vec![r.clone()]));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Reminders)))
    );
    assert_eq!(s.reminders, Some(vec![r]));
    // Loading an empty store still ends the loading state.
    let mut s = state();
    reduce(&mut s, Input::LocalRemindersLoaded(vec![]));
    assert_eq!(s.reminders, Some(vec![]));
}

fn due_ids(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::ReminderDue(r) => Some(r.id.0.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn tick_alerts_each_due_reminder_once_and_marks_it_fired() {
    let mut s = state();
    add(&mut s, "late", 50);
    add(&mut s, "now", 100);
    add(&mut s, "later", 500);
    let fx = reduce(&mut s, Input::ReminderTick { now: 100 });
    assert_eq!(due_ids(&fx), ["late", "now"]);
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.id == id("now") && r.fired)
    ));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Reminders)))
    );
    // Nothing alerts twice.
    let fx = reduce(&mut s, Input::ReminderTick { now: 400 });
    assert!(due_ids(&fx).is_empty());
    assert!(!fx.iter().any(|e| matches!(e, Effect::Persist(_))));
    let fx = reduce(&mut s, Input::ReminderTick { now: 500 });
    assert_eq!(due_ids(&fx), ["later"]);
}

#[test]
fn tick_skips_completed_fired_and_undated_reminders() {
    let mut s = state();
    let mk = |i: &str, due: Option<i64>, completed: bool, fired: bool| Reminder {
        id: id(i),
        text: Arc::from(i),
        due,
        recurring: false,
        completed,
        fired,
        interval_secs: 0,
        for_user: uid("me"),
        link: None,
        snoozed_until: None,
    };
    reduce(
        &mut s,
        Input::LocalRemindersLoaded(vec![
            mk("done", Some(1), true, false),
            mk("fired", Some(1), false, true),
            mk("undated", None, false, false),
            mk("due", Some(1), false, false),
        ]),
    );
    let fx = reduce(&mut s, Input::ReminderTick { now: 10 });
    assert_eq!(due_ids(&fx), ["due"]);
}

#[test]
fn next_reminder_due_is_the_earliest_unfired_open_one() {
    let mut s = state();
    assert_eq!(domain::next_reminder_due(&s), None);
    add(&mut s, "b", 300);
    add(&mut s, "a", 200);
    assert_eq!(domain::next_reminder_due(&s), Some(200));
    reduce(&mut s, Input::ReminderTick { now: 200 });
    assert_eq!(domain::next_reminder_due(&s), Some(300));
    reduce(&mut s, Input::CompleteReminder(id("b")));
    assert_eq!(domain::next_reminder_due(&s), None);
}

#[test]
fn tick_holds_alerts_while_notifications_are_paused() {
    let mut s = state();
    add(&mut s, "a", 100);
    reduce(
        &mut s,
        Input::Snooze {
            minutes: 30,
            now_unix: 0,
        },
    );
    // Paused until 1800: a due reminder stays unfired, nothing is persisted.
    let fx = reduce(&mut s, Input::ReminderTick { now: 100 });
    assert!(due_ids(&fx).is_empty());
    assert!(!fx.iter().any(|e| matches!(e, Effect::Persist(_))));
    assert_eq!(domain::next_reminder_due(&s), Some(100));
    let fx = reduce(&mut s, Input::ReminderTick { now: 1799 });
    assert!(due_ids(&fx).is_empty());
    // The pause ended: it alerts once.
    let fx = reduce(&mut s, Input::ReminderTick { now: 1800 });
    assert_eq!(due_ids(&fx), ["a"]);
    let fx = reduce(&mut s, Input::ReminderTick { now: 1900 });
    assert!(due_ids(&fx).is_empty());
}

#[test]
fn tick_alerts_again_when_the_pause_is_cut_short() {
    let mut s = state();
    add(&mut s, "a", 100);
    reduce(
        &mut s,
        Input::Snooze {
            minutes: 30,
            now_unix: 0,
        },
    );
    assert!(due_ids(&reduce(&mut s, Input::ReminderTick { now: 200 })).is_empty());
    reduce(&mut s, Input::EndSnooze);
    assert_eq!(
        due_ids(&reduce(&mut s, Input::ReminderTick { now: 300 })),
        ["a"]
    );
}

#[test]
fn recurring_reminder_alerts_then_moves_to_its_next_due() {
    let mut s = state();
    reduce(
        &mut s,
        Input::AddLocalReminder {
            id: id("weekly"),
            text: Arc::from("weekly"),
            due: 100,
            recurring: true,
            interval_secs: 7 * 86_400,
            user: uid("me"),
            link: None,
        },
    );
    let fx = reduce(&mut s, Input::ReminderTick { now: 100 });
    assert_eq!(due_ids(&fx), ["weekly"]);
    let week = 7 * 86_400;
    let r = &s.reminders.as_ref().unwrap()[0];
    assert!(!r.fired);
    assert_eq!(r.due, Some(100 + week));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::LocalReminderPut(r)) if r.due == Some(100 + week) && !r.fired
    )));
    // Not again before the next due, and a long gap skips missed weeks.
    assert!(due_ids(&reduce(&mut s, Input::ReminderTick { now: 200 })).is_empty());
    let fx = reduce(
        &mut s,
        Input::ReminderTick {
            now: 100 + 3 * week + 5,
        },
    );
    assert_eq!(due_ids(&fx), ["weekly"]);
    assert_eq!(s.reminders.as_ref().unwrap()[0].due, Some(100 + 4 * week));
}

#[test]
fn daily_reminder_repeats_every_day() {
    let mut s = state();
    reduce(
        &mut s,
        Input::AddLocalReminder {
            id: id("daily"),
            text: Arc::from("daily"),
            due: 100,
            recurring: true,
            interval_secs: 86_400,
            user: uid("me"),
            link: None,
        },
    );
    let fx = reduce(&mut s, Input::ReminderTick { now: 100 });
    assert_eq!(due_ids(&fx), ["daily"]);
    assert_eq!(s.reminders.as_ref().unwrap()[0].due, Some(100 + 86_400));
    reduce(
        &mut s,
        Input::ReminderTick {
            now: 100 + 86_400 * 2 + 5,
        },
    );
    assert_eq!(s.reminders.as_ref().unwrap()[0].due, Some(100 + 86_400 * 3));
}

fn link() -> ReminderLink {
    ReminderLink {
        message: domain::test_support::mid("c1", "7.000000"),
        author: Some(uid("u2")),
        excerpt: Arc::from("can you check the contrast?"),
    }
}

#[test]
fn a_reminder_about_a_message_keeps_its_link() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::AddLocalReminder {
            id: id("m"),
            text: Arc::from("Reply"),
            due: 100,
            recurring: false,
            interval_secs: 0,
            user: uid("me"),
            link: Some(link()),
        },
    );
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.link == Some(link()))
    ));
}

#[test]
fn snooze_moves_the_due_time_and_alerts_again() {
    let mut s = state();
    add(&mut s, "a", 100);
    reduce(&mut s, Input::ReminderTick { now: 150 });
    assert!(s.reminders.as_ref().unwrap()[0].fired);
    let fx = reduce(
        &mut s,
        Input::SnoozeReminder {
            id: id("a"),
            until: 1_300,
        },
    );
    assert!(no_call(&fx));
    let r = &s.reminders.as_ref().unwrap()[0];
    assert_eq!(r.due, Some(1_300));
    assert!(!r.fired && !r.completed);
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.due == Some(1_300))
    ));
    assert_eq!(domain::next_reminder_due(&s), Some(1_300));
}

#[test]
fn edit_changes_text_time_and_repeat() {
    let mut s = state();
    add(&mut s, "a", 100);
    let fx = reduce(
        &mut s,
        Input::EditReminder {
            id: id("a"),
            text: Arc::from("Check contrast"),
            due: Some(5_000),
            repeat: ReminderRepeat::Daily,
            now: 0,
        },
    );
    assert!(no_call(&fx));
    let r = &s.reminders.as_ref().unwrap()[0];
    assert_eq!(&*r.text, "Check contrast");
    assert_eq!(r.due, Some(5_000));
    assert!(r.recurring);
    assert_eq!(r.interval_secs, 86_400);
    assert_eq!(ReminderRepeat::of(r), ReminderRepeat::Daily);
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.recurring && &*r.text == "Check contrast")
    ));
    // No new time keeps the old one; Never stops the repeat.
    reduce(
        &mut s,
        Input::EditReminder {
            id: id("a"),
            text: Arc::from("Check contrast"),
            due: None,
            repeat: ReminderRepeat::Never,
            now: 0,
        },
    );
    let r = &s.reminders.as_ref().unwrap()[0];
    assert_eq!(r.due, Some(5_000));
    assert!(!r.recurring && r.interval_secs == 0);
}

#[test]
fn a_provider_reminder_is_not_snoozed_or_edited_locally() {
    let mut s = state();
    s.caps.reminders = true;
    reduce(
        &mut s,
        Input::Loaded(Ok(domain::Loaded::Reminders(vec![Reminder {
            id: id("p"),
            text: Arc::from("p"),
            due: Some(10),
            recurring: false,
            completed: false,
            fired: false,
            interval_secs: 0,
            for_user: uid("me"),
            link: None,
            snoozed_until: None,
        }]))),
    );
    let a = reduce(
        &mut s,
        Input::SnoozeReminder {
            id: id("p"),
            until: 99,
        },
    );
    let b = reduce(
        &mut s,
        Input::EditReminder {
            id: id("p"),
            text: Arc::from("q"),
            due: Some(99),
            repeat: ReminderRepeat::Never,
            now: 0,
        },
    );
    assert!(a.is_empty() && b.is_empty());
    assert_eq!(s.reminders.as_ref().unwrap()[0].due, Some(10));
}

fn add_repeating(s: &mut WorkspaceState, i: &str, due: i64, interval_secs: i64) {
    reduce(
        s,
        Input::AddLocalReminder {
            id: id(i),
            text: Arc::from(i),
            due,
            recurring: true,
            interval_secs,
            user: uid("me"),
            link: None,
        },
    );
}

fn one(s: &WorkspaceState) -> &Reminder {
    &s.reminders.as_ref().unwrap()[0]
}

#[test]
fn snoozing_a_daily_reminder_moves_only_that_occurrence() {
    let mut s = state();
    let day = 86_400;
    add_repeating(&mut s, "d", 100, day);
    reduce(&mut s, Input::ReminderTick { now: 100 });
    assert_eq!(one(&s).due, Some(100 + day));
    // Snoozed 20 minutes: the series keeps its time.
    reduce(
        &mut s,
        Input::SnoozeReminder {
            id: id("d"),
            until: 100 + 1_200,
        },
    );
    assert_eq!(one(&s).due, Some(100 + day), "the series is not moved");
    assert_eq!(one(&s).snoozed_until, Some(1_300));
    assert_eq!(domain::next_reminder_due(&s), Some(1_300));
    let fx = reduce(&mut s, Input::ReminderTick { now: 1_300 });
    assert_eq!(due_ids(&fx), ["d"]);
    assert_eq!(one(&s).snoozed_until, None);
    assert_eq!(one(&s).due, Some(100 + day));
    let fx = reduce(&mut s, Input::ReminderTick { now: 100 + day });
    assert_eq!(due_ids(&fx), ["d"]);
    assert_eq!(one(&s).due, Some(100 + 2 * day));
}

#[test]
fn snoozing_a_daily_reminder_to_next_week_keeps_the_days_between() {
    let mut s = state();
    let day = 86_400;
    add_repeating(&mut s, "d", 100, day);
    reduce(&mut s, Input::ReminderTick { now: 100 });
    reduce(
        &mut s,
        Input::SnoozeReminder {
            id: id("d"),
            until: 100 + 7 * day,
        },
    );
    assert_eq!(one(&s).due, Some(100 + day));
    assert_eq!(domain::next_reminder_due(&s), Some(100 + day));
    assert!(fx_persists_snooze(&mut s));
}

fn fx_persists_snooze(s: &mut WorkspaceState) -> bool {
    let fx = reduce(
        s,
        Input::SnoozeReminder {
            id: id("d"),
            until: 9_999_999,
        },
    );
    fx.iter().any(|e| {
        matches!(e, Effect::Persist(PersistOp::LocalReminderPut(r)) if r.snoozed_until == Some(9_999_999))
    })
}

#[test]
fn a_fired_one_off_set_to_repeat_gets_a_future_due() {
    let mut s = state();
    add(&mut s, "a", 100);
    reduce(&mut s, Input::ReminderTick { now: 150 });
    assert!(one(&s).fired);
    reduce(
        &mut s,
        Input::EditReminder {
            id: id("a"),
            text: Arc::from("a"),
            due: None,
            repeat: ReminderRepeat::Daily,
            now: 200_000,
        },
    );
    let r = one(&s);
    assert!(!r.fired);
    assert!(r.recurring);
    assert_eq!(r.due, Some(100 + 3 * 86_400), "next occurrence after now");
    assert_eq!(domain::next_reminder_due(&s), Some(100 + 3 * 86_400));
}

#[test]
fn a_new_time_clears_a_snooze() {
    let mut s = state();
    add_repeating(&mut s, "d", 100, 86_400);
    reduce(&mut s, Input::ReminderTick { now: 100 });
    reduce(
        &mut s,
        Input::SnoozeReminder {
            id: id("d"),
            until: 1_300,
        },
    );
    reduce(
        &mut s,
        Input::EditReminder {
            id: id("d"),
            text: Arc::from("d"),
            due: Some(50_000),
            repeat: ReminderRepeat::Daily,
            now: 200,
        },
    );
    assert_eq!(one(&s).snoozed_until, None);
    assert_eq!(one(&s).due, Some(50_000));
}

#[test]
fn an_excerpt_is_cut_on_a_word_and_marked() {
    let long = "word ".repeat(60);
    let l = ReminderLink::new(domain::test_support::mid("c1", "7.000000"), None, &long);
    assert!(l.excerpt.chars().count() <= domain::REMINDER_EXCERPT_CHARS + 1);
    assert!(l.excerpt.ends_with("word\u{2026}"), "{}", l.excerpt);
    let short = ReminderLink::new(
        domain::test_support::mid("c1", "7.000000"),
        None,
        "hi there",
    );
    assert_eq!(&*short.excerpt, "hi there");
    // Multi-byte text is cut on a char.
    let wide = "é".repeat(300);
    let w = ReminderLink::new(domain::test_support::mid("c1", "7.000000"), None, &wide);
    assert_eq!(
        w.excerpt.chars().count(),
        domain::REMINDER_EXCERPT_CHARS + 1
    );
}

fn add_linked(s: &mut WorkspaceState) {
    reduce(
        s,
        Input::AddLocalReminder {
            id: id("m"),
            text: Arc::from(""),
            due: 100,
            recurring: false,
            interval_secs: 0,
            user: uid("me"),
            link: Some(link()),
        },
    );
}

#[test]
fn editing_the_message_updates_the_excerpt() {
    let mut s = state();
    add_linked(&mut s);
    let fx = reduce(
        &mut s,
        Input::Provider(domain::ProviderEvent::MessageEdited {
            id: domain::test_support::mid("c1", "7.000000"),
            text: Arc::from("*bold* new text"),
            plain: Arc::from("bold new text"),
            edited: domain::test_support::ts("8.000000"),
        }),
    );
    assert_eq!(&*one(&s).link.as_ref().unwrap().excerpt, "bold new text");
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::LocalReminderPut(r)) if r.link.as_ref().is_some_and(|l| &*l.excerpt == "bold new text")
    )));
}

#[test]
fn deleting_the_message_blanks_the_excerpt() {
    let mut s = state();
    add_linked(&mut s);
    let fx = reduce(
        &mut s,
        Input::Provider(domain::ProviderEvent::MessageDeleted(
            domain::test_support::mid("c1", "7.000000"),
        )),
    );
    let l = one(&s).link.as_ref().unwrap();
    assert_eq!(&*l.excerpt, "");
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::LocalReminderPut(r)) if r.link.as_ref().is_some_and(|l| l.excerpt.is_empty())
    )));
}

#[test]
fn a_reminder_about_a_message_keeps_no_label_text() {
    let mut s = state();
    add_linked(&mut s);
    assert!(one(&s).about_message());
    add(&mut s, "plain", 200);
    let p = s
        .reminders
        .as_ref()
        .unwrap()
        .iter()
        .find(|r| r.id == id("plain"))
        .unwrap();
    assert!(!p.about_message());
}
