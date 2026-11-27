mod common;

use std::sync::Arc;

use components::{RemindersSource, RemindersView, SendLaterConfig};
use domain::test_support::uid;
use domain::{FixedNow, RemindPreset, Reminder, ReminderId, ReminderLink, ReminderRepeat};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use settings::Clock;

/// 2026-11-15 10:00:00 UTC (a Sunday).
const NOW: i64 = 1_794_736_800;
const DAY: i64 = 86_400;

#[derive(Default)]
struct Log {
    completed: Vec<String>,
    deleted: Vec<String>,
    added: Vec<(String, String)>,
    snoozed: Vec<(String, i64)>,
    edited: Vec<(String, String, String, ReminderRepeat)>,
}

struct Src {
    list: Option<Vec<Reminder>>,
    local: bool,
    log: Log,
}

impl RemindersSource for Src {
    fn reminders(&self) -> Option<&[Reminder]> {
        self.list.as_deref()
    }
    fn complete(&mut self, id: ReminderId, _: &mut App) {
        self.log.completed.push(id.0.to_string());
    }
    fn delete(&mut self, id: ReminderId, _: &mut App) {
        self.log.deleted.push(id.0.to_string());
    }
    fn add(&mut self, text: String, when: String, _: &mut App) {
        self.log.added.push((text, when));
    }
    fn local(&self) -> bool {
        self.local
    }
    fn snooze(&mut self, id: ReminderId, until: i64, _: &mut App) {
        self.log.snoozed.push((id.0.to_string(), until));
    }
    fn edit(
        &mut self,
        id: ReminderId,
        text: String,
        when: String,
        repeat: ReminderRepeat,
        _: &mut App,
    ) {
        self.log.edited.push((id.0.to_string(), text, when, repeat));
    }
    fn conversation_label(&self, _: &domain::ConversationId, _: &App) -> String {
        "#design".into()
    }
    fn time_config(&self, _: &App) -> SendLaterConfig {
        SendLaterConfig {
            presets: vec![],
            zone: TimeZone::UTC,
            clock: Clock::H24,
            server: false,
        }
    }
}

fn rem(id: &str, due: Option<i64>, recurring: bool, completed: bool) -> Reminder {
    Reminder {
        id: ReminderId(Arc::from(id)),
        text: Arc::from(format!("text {id}")),
        due,
        recurring,
        completed,
        fired: false,
        interval_secs: 0,
        for_user: uid("me"),
        link: None,
        snoozed_until: None,
    }
}

fn setup(
    cx: &mut TestAppContext,
    list: Option<Vec<Reminder>>,
    local: bool,
) -> (
    Entity<Src>,
    Entity<RemindersView<Src>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let src = cx.new(|_| Src {
        list,
        local,
        log: Log::default(),
    });
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| RemindersView::new(s, cx));
    (src, view, vcx)
}

fn ids(rows: &[components::ReminderRow]) -> Vec<&str> {
    rows.iter().map(|r| &*r.id.0).collect()
}

#[gpui::test]
fn groups_and_orders(cx: &mut TestAppContext) {
    let (_s, v, vcx) = setup(
        cx,
        Some(vec![
            rem("late", Some(NOW + 3 * DAY), false, false),
            rem("soon", Some(NOW + 3600), false, false),
            rem("nodue", None, false, false),
            rem("weekly", Some(NOW + 2 * DAY), true, false),
            rem("daily", Some(NOW + 600), true, false),
            rem("done2", Some(NOW - DAY), false, true),
            rem("done1", Some(NOW - 2 * DAY), true, true),
        ]),
        false,
    );
    let g = vcx.update(|_, cx| v.read(cx).groups(cx)).expect("loaded");
    // Upcoming by due time, no due time last; completed ones leave both lists.
    assert_eq!(ids(&g.upcoming), ["soon", "late", "nodue"]);
    assert_eq!(ids(&g.recurring), ["daily", "weekly"]);
    assert_eq!(ids(&g.completed), ["done1", "done2"]);
    assert_eq!(g.upcoming[0].time, "Nov 15, 2026, 11:00");
    assert_eq!(g.upcoming[2].time, "");
    // Completed is collapsed until asked for.
    assert!(!v.read_with(vcx, |v, _| v.completed_open()));
    v.update(vcx, |v, cx| v.toggle_completed(cx));
    assert!(v.read_with(vcx, |v, _| v.completed_open()));
}

#[gpui::test]
fn loading_state_while_none(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, None, true);
    assert!(vcx.update(|_, cx| v.read(cx).groups(cx)).is_none());
    s.update(vcx, |s, _| s.list = Some(vec![]));
    let g = vcx.update(|_, cx| v.read(cx).groups(cx)).expect("loaded");
    assert!(g.upcoming.is_empty() && g.recurring.is_empty() && g.completed.is_empty());
}

#[gpui::test]
fn complete_and_delete_dispatch(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(
        cx,
        Some(vec![rem("a", Some(NOW + 60), false, false)]),
        false,
    );
    v.update(vcx, |v, cx| v.complete(ReminderId(Arc::from("a")), cx));
    v.update(vcx, |v, cx| v.delete(ReminderId(Arc::from("b")), cx));
    s.read_with(vcx, |s, _| {
        assert_eq!(s.log.completed, ["a"]);
        assert_eq!(s.log.deleted, ["b"]);
    });
}

#[gpui::test]
fn add_row_validates_when(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![]), false);
    // Blank text, or a time that is not one, keeps the row and shows the error.
    for (what, when) in [
        ("", "in 5 minutes"),
        ("stretch", "whenever"),
        ("stretch", ""),
    ] {
        v.update(vcx, |v, cx| v.set_add(what, when, cx));
        assert!(!v.update(vcx, |v, cx| v.submit(cx)), "{what:?} {when:?}");
        assert!(v.read_with(vcx, |v, _| v.add_invalid()));
    }
    s.read_with(vcx, |s, _| assert!(s.log.added.is_empty()));
    // A valid one is handed to the source verbatim, trimmed, and clears the row.
    v.update(vcx, |v, cx| v.set_add("  stretch  ", " in 20 minutes ", cx));
    assert!(v.update(vcx, |v, cx| v.submit(cx)));
    assert!(!v.read_with(vcx, |v, _| v.add_invalid()));
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.log.added,
            [("stretch".to_string(), "in 20 minutes".to_string())]
        );
    });
    assert!(!v.update(vcx, |v, cx| v.submit(cx)), "the row was cleared");
}

#[gpui::test]
fn overdue_is_open_non_recurring_and_past(cx: &mut TestAppContext) {
    let (_s, v, vcx) = setup(
        cx,
        Some(vec![
            rem("late", Some(NOW - 60), false, false),
            rem("soon", Some(NOW + 60), false, false),
            rem("nodue", None, false, false),
            rem("weekly", Some(NOW - 60), true, false),
            rem("done", Some(NOW - 60), false, true),
        ]),
        false,
    );
    let g = vcx.update(|_, cx| v.read(cx).groups(cx)).expect("loaded");
    let over = |rows: &[components::ReminderRow]| {
        rows.iter()
            .filter(|r| r.overdue)
            .map(|r| r.id.0.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(over(&g.upcoming), ["late"]);
    assert!(over(&g.recurring).is_empty() && over(&g.completed).is_empty());
}

fn focus_view(v: &Entity<RemindersView<Src>>, vcx: &mut VisualTestContext) {
    v.update_in(vcx, |v, w, cx| {
        let f = gpui::Focusable::focus_handle(v, cx);
        w.focus(&f, cx);
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn keyboard_tab_order_fills_the_form_and_enter_adds(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![]), false);
    focus_view(&v, vcx);
    // The first Tab lands on the text, the second on the time.
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("stretch");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("in 20 minutes");
    vcx.simulate_keystrokes("enter");
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.log.added,
            [("stretch".to_string(), "in 20 minutes".to_string())]
        );
    });
}

#[gpui::test]
fn add_button_is_the_third_stop_and_enter_activates_it(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![]), false);
    v.update(vcx, |v, cx| v.set_add("stretch", "in 5 minutes", cx));
    focus_view(&v, vcx);
    vcx.simulate_keystrokes("tab tab tab enter");
    s.read_with(vcx, |s, _| assert_eq!(s.log.added.len(), 1));
    // Shift-Tab from the first field leaves it backwards (the window's
    // previous stop, not a wrap inside the view): typing then goes nowhere.
    v.update(vcx, |v, cx| v.set_add("again", "in 5 minutes", cx));
    focus_view(&v, vcx);
    vcx.simulate_keystrokes("tab shift-tab");
    vcx.simulate_input("zz");
    v.update(vcx, |v, cx| assert!(v.submit(cx)));
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.log.added[1],
            ("again".to_string(), "in 5 minutes".to_string())
        )
    });
}

#[gpui::test]
fn shift_enter_on_a_button_or_check_box_does_not_add(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(
        cx,
        Some(vec![rem("a", Some(NOW + 60), false, false)]),
        false,
    );
    v.update(vcx, |v, cx| v.set_add("stretch", "in 5 minutes", cx));
    focus_view(&v, vcx);
    // Add button, then the check box.
    vcx.simulate_keystrokes("tab tab tab shift-enter");
    s.read_with(vcx, |s, _| assert!(s.log.added.is_empty()));
    vcx.simulate_keystrokes("tab shift-enter");
    s.read_with(vcx, |s, _| {
        assert!(s.log.added.is_empty() && s.log.completed.is_empty())
    });
    // Delete.
    vcx.simulate_keystrokes("tab shift-enter");
    s.read_with(vcx, |s, _| {
        assert!(s.log.added.is_empty() && s.log.deleted.is_empty())
    });
}

#[gpui::test]
fn check_box_completes_and_escape_closes(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(
        cx,
        Some(vec![rem("a", Some(NOW + 60), false, false)]),
        false,
    );
    let closed = std::rc::Rc::new(std::cell::Cell::new(false));
    let c = closed.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&v, move |_, _: &components::RemindersEvent, _| c.set(true))
            .detach();
    });
    focus_view(&v, vcx);
    // text, time, Add, then the row's check box.
    vcx.simulate_keystrokes("tab tab tab tab enter");
    s.read_with(vcx, |s, _| assert_eq!(s.log.completed, ["a"]));
    vcx.simulate_keystrokes("escape");
    assert!(closed.get());
}

fn rid(s: &str) -> ReminderId {
    ReminderId(Arc::from(s))
}

#[gpui::test]
fn snooze_menu_offers_presets_and_snoozes(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![rem("a", Some(NOW - 60), false, false)]), true);
    assert!(v.read_with(vcx, |v, _| v.snooze_open().is_none()));
    v.update(vcx, |v, cx| v.toggle_snooze(rid("a"), cx));
    assert_eq!(
        v.read_with(vcx, |v, _| v.snooze_open().cloned()),
        Some(rid("a"))
    );
    v.update(vcx, |v, cx| v.snooze(rid("a"), RemindPreset::In1Hour, cx));
    s.read_with(vcx, |s, _| {
        assert_eq!(s.log.snoozed, [("a".to_string(), NOW + 3_600)])
    });
    assert!(v.read_with(vcx, |v, _| v.snooze_open().is_none()));
}

#[gpui::test]
fn provider_reminders_offer_no_snooze_or_edit(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(
        cx,
        Some(vec![rem("a", Some(NOW + 60), false, false)]),
        false,
    );
    v.update(vcx, |v, cx| v.toggle_snooze(rid("a"), cx));
    assert!(v.read_with(vcx, |v, _| v.snooze_open().is_none()));
    v.update_in(vcx, |v, w, cx| v.begin_edit(rid("a"), w, cx));
    assert!(v.read_with(vcx, |v, _| v.editing().is_none()));
    v.update(vcx, |v, cx| v.snooze(rid("a"), RemindPreset::Tomorrow, cx));
    s.read_with(vcx, |s, _| assert!(s.log.snoozed.is_empty()));
}

#[gpui::test]
fn edit_saves_text_time_and_repeat(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![rem("a", Some(NOW + 60), false, false)]), true);
    v.update_in(vcx, |v, w, cx| v.begin_edit(rid("a"), w, cx));
    assert_eq!(
        v.read_with(vcx, |v, _| v.editing().cloned()),
        Some(rid("a"))
    );
    // The text starts as the reminder's; a bad time keeps the editor open.
    assert_eq!(v.read_with(vcx, |v, cx| v.edit_text(cx)), "text a");
    v.update(vcx, |v, cx| v.set_edit("Check contrast", "whenever", cx));
    assert!(!v.update(vcx, |v, cx| v.save_edit(cx)));
    assert!(v.read_with(vcx, |v, _| v.edit_invalid()));
    v.update(vcx, |v, cx| {
        v.set_edit("Check contrast", "tomorrow", cx);
        v.set_edit_repeat(ReminderRepeat::Daily, cx);
    });
    assert!(v.update(vcx, |v, cx| v.save_edit(cx)));
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.log.edited,
            [(
                "a".to_string(),
                "Check contrast".to_string(),
                "tomorrow".to_string(),
                ReminderRepeat::Daily
            )]
        )
    });
    assert!(v.read_with(vcx, |v, _| v.editing().is_none()));
    // A blank time keeps the reminder's; Cancel closes without saving.
    v.update_in(vcx, |v, w, cx| v.begin_edit(rid("a"), w, cx));
    v.update(vcx, |v, cx| v.set_edit("Other", "", cx));
    assert!(v.update(vcx, |v, cx| v.save_edit(cx)));
    s.read_with(vcx, |s, _| assert_eq!(s.log.edited[1].2, ""));
    v.update_in(vcx, |v, w, cx| v.begin_edit(rid("a"), w, cx));
    v.update(vcx, |v, cx| v.cancel_edit(cx));
    assert!(v.read_with(vcx, |v, _| v.editing().is_none()));
    s.read_with(vcx, |s, _| assert_eq!(s.log.edited.len(), 2));
}

#[gpui::test]
fn a_linked_message_shows_and_opens(cx: &mut TestAppContext) {
    let mut r = rem("a", Some(NOW + 60), false, false);
    let target = domain::test_support::mid("c1", "7.000000");
    r.link = Some(ReminderLink::new(
        target.clone(),
        None,
        "can you check the contrast?",
    ));
    let (_s, v, vcx) = setup(cx, Some(vec![r]), true);
    let g = vcx.update(|_, cx| v.read(cx).groups(cx)).expect("loaded");
    let ctx = g.upcoming[0].context.clone().expect("context");
    assert_eq!(ctx.place, "#design");
    assert_eq!(ctx.excerpt, "can you check the contrast?");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(None));
    let o = opened.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&v, move |_, ev: &components::RemindersEvent, _| {
            if let components::RemindersEvent::OpenMessage(id) = ev {
                *o.borrow_mut() = Some(id.clone());
            }
        })
        .detach();
    });
    v.update(vcx, |v, cx| v.open_link(rid("a"), cx));
    assert_eq!(opened.borrow().clone(), Some(target));
}

#[gpui::test]
fn snooze_and_edit_are_tab_stops_after_the_check_box(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![rem("a", Some(NOW + 60), false, false)]), true);
    focus_view(&v, vcx);
    // text, time, Add, check box, then Snooze: Enter opens its menu with the
    // focus on the first preset; Escape closes only the menu.
    vcx.simulate_keystrokes("tab tab tab tab tab enter");
    assert_eq!(
        v.read_with(vcx, |v, _| v.snooze_open().cloned()),
        Some(rid("a"))
    );
    vcx.simulate_keystrokes("escape");
    assert!(v.read_with(vcx, |v, _| v.snooze_open().is_none()));
    focus_view(&v, vcx);
    vcx.simulate_keystrokes("tab tab tab tab tab enter enter");
    s.read_with(vcx, |s, _| {
        assert_eq!(s.log.snoozed, [("a".to_string(), NOW + 1_200)])
    });
    // Edit follows Snooze; Space opens the editor.
    focus_view(&v, vcx);
    vcx.simulate_keystrokes("tab tab tab tab tab tab space");
    assert_eq!(
        v.read_with(vcx, |v, _| v.editing().cloned()),
        Some(rid("a"))
    );
}

#[gpui::test]
fn escape_in_the_snooze_menu_returns_to_the_snooze_button(cx: &mut TestAppContext) {
    let (_s, v, vcx) = setup(cx, Some(vec![rem("a", Some(NOW + 60), false, false)]), true);
    focus_view(&v, vcx);
    vcx.simulate_keystrokes("tab tab tab tab tab enter");
    assert!(v.read_with(vcx, |v, _| v.snooze_open().is_some()));
    vcx.simulate_keystrokes("escape");
    assert!(v.read_with(vcx, |v, _| v.snooze_open().is_none()));
    // The focus is back on Snooze: Enter opens the menu again.
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        v.read_with(vcx, |v, _| v.snooze_open().cloned()),
        Some(rid("a"))
    );
}

#[gpui::test]
fn escape_with_the_repeat_menu_open_closes_only_the_menu(cx: &mut TestAppContext) {
    let (_s, v, vcx) = setup(cx, Some(vec![rem("a", Some(NOW + 60), false, false)]), true);
    focus_view(&v, vcx);
    v.update_in(vcx, |v, w, cx| v.begin_edit(rid("a"), w, cx));
    vcx.run_until_parked();
    // Text, time, then Repeat: Enter opens its menu.
    vcx.simulate_keystrokes("tab tab enter");
    assert!(v.read_with(vcx, |v, _| v.repeat_open()));
    vcx.simulate_keystrokes("escape");
    assert!(v.read_with(vcx, |v, _| !v.repeat_open()));
    assert_eq!(
        v.read_with(vcx, |v, _| v.editing().cloned()),
        Some(rid("a"))
    );
    vcx.simulate_keystrokes("escape");
    assert!(v.read_with(vcx, |v, _| v.editing().is_none()));
}

#[gpui::test]
fn a_repeating_when_is_left_to_the_repeat_control(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup(cx, Some(vec![rem("a", Some(NOW + 60), false, false)]), true);
    v.update_in(vcx, |v, w, cx| v.begin_edit(rid("a"), w, cx));
    v.update(vcx, |v, cx| v.set_edit("Standup", "every monday", cx));
    assert!(!v.update(vcx, |v, cx| v.save_edit(cx)));
    assert!(v.read_with(vcx, |v, _| v.edit_repeat_hint()));
    s.read_with(vcx, |s, _| assert!(s.log.edited.is_empty()));
    v.update(vcx, |v, cx| v.set_edit("Standup", "on monday", cx));
    assert!(v.update(vcx, |v, cx| v.save_edit(cx)));
}

#[gpui::test]
fn day_presets_show_their_time(cx: &mut TestAppContext) {
    let (s, _v, vcx) = setup(cx, Some(vec![]), true);
    let (tomorrow, next) = vcx.update(|_, cx| {
        let cfg = s.read(cx).time_config(cx);
        (
            components::preset_label(RemindPreset::Tomorrow, &cfg.zone, cfg.clock, cx),
            components::preset_label(RemindPreset::NextWeek, &cfg.zone, cfg.clock, cx),
        )
    });
    assert_eq!(tomorrow, "Tomorrow 09:00");
    assert_eq!(next, "Next Monday 09:00");
}

#[gpui::test]
fn a_reminder_about_a_message_shows_the_label_and_its_snoozed_time(cx: &mut TestAppContext) {
    let mut r = rem("a", Some(NOW + 86_400), true, false);
    r.text = Arc::from("");
    r.interval_secs = 86_400;
    r.snoozed_until = Some(NOW + 1_200);
    r.link = Some(ReminderLink::new(
        domain::test_support::mid("c1", "7.000000"),
        None,
        "hi",
    ));
    let (_s, v, vcx) = setup(cx, Some(vec![r]), true);
    let g = vcx.update(|_, cx| v.read(cx).groups(cx)).expect("loaded");
    assert_eq!(g.recurring[0].text, "Follow up on this message");
    assert_eq!(g.recurring[0].due, Some(NOW + 1_200));
}
