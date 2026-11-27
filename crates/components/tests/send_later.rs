mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::{Composer, ComposerEvent, ComposerSource};
use components::{
    CustomCheck, ScheduledSource, ScheduledView, SendLaterConfig, check_custom, needs_local,
    preset_choices,
};
use domain::test_support::cid;
use domain::{
    Capabilities, ComposedText, ConversationId, FixedNow, LocalSchedule, LocalScheduleState,
    MessageWindow, ScheduleId, ScheduledMessage, UserId,
};
use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use settings::Clock;

/// 2026-11-15 10:00:00 UTC (a Sunday); 11:00 in Madrid.
const NOW: i64 = 1_794_736_800;

fn madrid() -> TimeZone {
    TimeZone::get("Europe/Madrid").expect("tz")
}

fn zoned_now() -> jiff::ZonedDateTime {
    Timestamp::from_second(NOW)
        .unwrap()
        .to_zoned_datetime(madrid())
}

fn config(server: bool) -> SendLaterConfig {
    SendLaterConfig {
        presets: vec!["tomorrow 09:00".into(), "in 2h".into(), "bogus".into()],
        zone: madrid(),
        clock: Clock::H24,
        server,
    }
}

// Presets and the custom check, pure.

#[test]
fn preset_choices_compute_in_zone_and_skip_unparsable() {
    let presets = ["tomorrow 09:00", "monday 09:00", "bogus"].map(String::from);
    let got = preset_choices(&presets, &zoned_now());
    // Tomorrow 09:00 CET is 08:00 UTC. Today is Sunday, so "monday 09:00" is the
    // same moment and is listed once.
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].preset, "tomorrow 09:00");
    assert_eq!(got[0].at, NOW - 2 * 3600 + 86_400);
    let later = vec!["tomorrow 09:00".to_string(), "in 2h".to_string()];
    assert_eq!(preset_choices(&later, &zoned_now()).len(), 2);
}

#[test]
fn custom_check_rules() {
    let now = zoned_now();
    assert_eq!(check_custom("", "09:00", &now), CustomCheck::Incomplete);
    assert_eq!(
        check_custom("2026-11-16", "", &now),
        CustomCheck::Incomplete
    );
    assert_eq!(
        check_custom("tomorrow", "09:00", &now),
        CustomCheck::Invalid
    );
    assert_eq!(
        check_custom("2026-11-16", "25:00", &now),
        CustomCheck::Invalid
    );
    assert_eq!(check_custom("2026-11-15", "10:59", &now), CustomCheck::Past);
    assert_eq!(check_custom("2026-11-15", "11:00", &now), CustomCheck::Past);
    assert_eq!(
        check_custom("2026-11-15", "11:01", &now),
        CustomCheck::Ok(NOW + 60)
    );
    assert_eq!(
        check_custom("2027-06-01", "09:00", &now),
        CustomCheck::TooFar
    );
    // Exactly 120 days ahead is the last allowed day.
    assert!(matches!(
        check_custom("2027-03-15", "10:00", &now),
        CustomCheck::Ok(_)
    ));
}

// Composer.

struct Src {
    caps: Capabilities,
    cfg: SendLaterConfig,
    windows: std::collections::HashMap<ConversationId, MessageWindow>,
}

impl ComposerSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.windows.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        None
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn send_later(&self, _: &App) -> SendLaterConfig {
        self.cfg.clone()
    }
}

type Events = Rc<RefCell<Vec<ComposerEvent>>>;

fn setup(
    cx: &mut TestAppContext,
    server: bool,
) -> (Entity<Composer<Src>>, Events, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let src = cx.new(|_| Src {
        caps: Capabilities {
            scheduled_send: server,
            ..Default::default()
        },
        cfg: config(server),
        windows: Default::default(),
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|w, cx| {
        cx.subscribe(&view, move |_, e: &ComposerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    (view, events, vcx)
}

#[gpui::test]
fn presets_show_times_in_zone(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, true);
    vcx.simulate_input("later");
    v.update(vcx, |c, cx| c.open_send_later(cx));
    let rows = v
        .read_with(vcx, |c, cx| c.send_later_rows(cx))
        .expect("menu open");
    // Two presets and Custom.
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].at, Some(NOW - 2 * 3600 + 86_400));
    assert!(rows[0].label.contains("09:00"), "{}", rows[0].label);
    assert_eq!(rows[1].at, Some(NOW + 2 * 3600));
    assert!(rows[1].label.contains("13:00"), "{}", rows[1].label);
    assert_eq!(rows[2].at, None);
    assert_eq!(v.read_with(vcx, |c, cx| c.send_later_local_label(cx)), None);
    v.update_in(vcx, |c, w, cx| c.choose_send_later(0, w, cx));
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Schedule {
            body: ComposedText::plain("later"),
            post_at: NOW - 2 * 3600 + 86_400,
        }]
    );
    assert!(!v.read_with(vcx, |c, _| c.send_later_open()));
    assert_eq!(v.read_with(vcx, |c, cx| c.text(cx).to_string()), "");
}

#[gpui::test]
fn empty_text_does_not_open_the_menu(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, true);
    v.update(vcx, |c, cx| c.open_send_later(cx));
    assert!(!v.read_with(vcx, |c, _| c.send_later_open()));
}

#[gpui::test]
fn custom_picker_disables_past_and_far(cx: &mut TestAppContext) {
    let (v, ev, vcx) = setup(cx, true);
    vcx.simulate_input("later");
    v.update(vcx, |c, cx| c.open_send_later(cx));
    v.update_in(vcx, |c, w, cx| c.choose_send_later(2, w, cx));
    let check = |v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext| {
        v.read_with(vcx, |c, cx| c.custom_check(cx))
            .expect("custom open")
    };
    v.update(vcx, |c, cx| c.set_custom("2026-11-15", "09:00", cx));
    assert_eq!(check(&v, vcx), CustomCheck::Past);
    v.update(vcx, |c, cx| c.submit_custom(cx));
    assert!(ev.borrow().is_empty(), "a past time is not submitted");
    v.update(vcx, |c, cx| c.set_custom("2028-01-01", "09:00", cx));
    assert_eq!(check(&v, vcx), CustomCheck::TooFar);
    v.update(vcx, |c, cx| c.submit_custom(cx));
    assert!(ev.borrow().is_empty());
    v.update(vcx, |c, cx| c.set_custom("2026-12-01", "09:30", cx));
    let CustomCheck::Ok(at) = check(&v, vcx) else {
        panic!("valid time");
    };
    v.update(vcx, |c, cx| c.submit_custom(cx));
    assert_eq!(
        *ev.borrow(),
        [ComposerEvent::Schedule {
            body: ComposedText::plain("later"),
            post_at: at,
        }]
    );
    // 09:30 in Madrid (CET, UTC+1) on 2026-12-01.
    assert_eq!(at, 1_796_113_800);
}

#[test]
fn files_force_local_with_label() {
    assert!(needs_local(true, true));
    assert!(needs_local(false, false));
    assert!(!needs_local(false, true));
}

#[gpui::test]
fn no_capability_shows_local_label(cx: &mut TestAppContext) {
    let (v, _, vcx) = setup(cx, false);
    vcx.simulate_input("later");
    v.update(vcx, |c, cx| c.open_send_later(cx));
    let label = v
        .read_with(vcx, |c, cx| c.send_later_local_label(cx))
        .expect("local label");
    assert!(label.contains("Parla must be open"), "{label}");
}

// Scheduled view.

#[derive(Default)]
struct Log {
    rescheduled: Vec<(String, String, i64)>,
    deleted: Vec<String>,
    sent_now: Vec<String>,
}

struct Sched {
    server: Vec<ScheduledMessage>,
    local: Vec<LocalSchedule>,
    log: Log,
}

impl ScheduledSource for Sched {
    fn scheduled(&self) -> &[ScheduledMessage] {
        &self.server
    }
    fn local(&self) -> &[LocalSchedule] {
        &self.local
    }
    fn reschedule(&mut self, item: ScheduledMessage, text: String, post_at: i64, _: &mut App) {
        self.log
            .rescheduled
            .push((item.id.0.to_string(), text, post_at));
    }
    fn delete(&mut self, id: ScheduleId, _: &mut App) {
        self.log.deleted.push(id.0.to_string());
    }
    fn send_now(&mut self, id: ScheduleId, _: &mut App) {
        self.log.sent_now.push(id.0.to_string());
    }
    fn conversation_label(&self, c: &ConversationId, _: &App) -> String {
        format!("#{}", c.0)
    }
    fn send_later_config(&self, _: &App) -> SendLaterConfig {
        config(true)
    }
}

fn sid(s: &str) -> ScheduleId {
    ScheduleId(Arc::from(s))
}

fn server_item(id: &str, text: &str, at: i64) -> ScheduledMessage {
    ScheduledMessage {
        id: sid(id),
        conversation: cid("C1"),
        thread: None,
        text: Arc::from(text),
        post_at: at,
    }
}

fn local_item(id: &str, text: &str, at: i64, st: LocalScheduleState) -> LocalSchedule {
    LocalSchedule {
        id: sid(id),
        conversation: cid("C2"),
        thread: None,
        text: Arc::from(text),
        files: vec![],
        post_at: at,
        created_ms: 0,
        state: st,
    }
}

fn setup_view(
    cx: &mut TestAppContext,
    src: Sched,
) -> (
    Entity<Sched>,
    Entity<ScheduledView<Sched>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let src = cx.new(|_| src);
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| ScheduledView::new(s, cx));
    (src, view, vcx)
}

#[gpui::test]
fn local_items_tagged(cx: &mut TestAppContext) {
    let day = 86_400;
    let (_s, v, vcx) = setup_view(
        cx,
        Sched {
            server: vec![
                server_item("s1", "from server", NOW + day + 3600),
                server_item("s2", "also server", NOW + 3 * day),
            ],
            local: vec![
                local_item(
                    "l1",
                    "kept here",
                    NOW + day + 7200,
                    LocalScheduleState::Pending,
                ),
                local_item(
                    "l2",
                    "did not go",
                    NOW + 100,
                    LocalScheduleState::Failed("boom".into()),
                ),
            ],
            log: Log::default(),
        },
    );
    let groups = v.read_with(vcx, |v, cx| v.groups(cx));
    // Three days: the failed one today, two tomorrow, one later.
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].rows.len(), 1);
    assert_eq!(groups[1].rows.len(), 2);
    assert_eq!(groups[1].rows[0].text, "from server");
    assert_eq!(groups[1].rows[1].text, "kept here");
    assert!(groups[1].rows[0].tag.is_none());
    assert!(!groups[1].rows[0].local);
    let tagged = &groups[1].rows[1];
    assert!(tagged.local);
    assert_eq!(tagged.tag.as_deref(), Some("local, needs app open"));
    assert_eq!(tagged.conversation, "#C2");
    let failed = &groups[0].rows[0];
    assert!(failed.failed);
    assert!(failed.tag.as_deref().unwrap().contains("boom"));
}

#[gpui::test]
fn edit_is_one_reschedule(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup_view(
        cx,
        Sched {
            server: vec![server_item("s1", "old text", NOW + 86_400)],
            local: vec![],
            log: Log::default(),
        },
    );
    v.update_in(vcx, |v, w, cx| v.begin_edit(sid("s1"), w, cx));
    v.update(vcx, |v, cx| {
        v.set_edit("new text", "2026-12-01", "09:30", cx)
    });
    v.update(vcx, |v, cx| v.save_edit(cx));
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.log.rescheduled,
            [("s1".to_string(), "new text".to_string(), 1_796_113_800)]
        );
        assert!(s.log.deleted.is_empty() && s.log.sent_now.is_empty());
    });
    // The edit closed; saving again does nothing.
    v.update(vcx, |v, cx| v.save_edit(cx));
    s.read_with(vcx, |s, _| assert_eq!(s.log.rescheduled.len(), 1));
}

#[gpui::test]
fn edit_with_a_past_time_is_refused(cx: &mut TestAppContext) {
    let (s, v, vcx) = setup_view(
        cx,
        Sched {
            server: vec![server_item("s1", "old text", NOW + 86_400)],
            local: vec![],
            log: Log::default(),
        },
    );
    v.update_in(vcx, |v, w, cx| v.begin_edit(sid("s1"), w, cx));
    v.update(vcx, |v, cx| v.set_edit("x", "2026-11-14", "09:30", cx));
    v.update(vcx, |v, cx| v.save_edit(cx));
    s.read_with(vcx, |s, _| assert!(s.log.rescheduled.is_empty()));
}

fn edit_open(cx: &mut TestAppContext) -> (Entity<Sched>, &mut VisualTestContext) {
    let (s, v, vcx) = setup_view(
        cx,
        Sched {
            server: vec![server_item("s1", "old text", NOW + 86_400)],
            local: vec![],
            log: Log::default(),
        },
    );
    v.update_in(vcx, |v, w, cx| v.begin_edit(sid("s1"), w, cx));
    vcx.run_until_parked();
    (s, vcx)
}

#[gpui::test]
fn edit_tab_order_ends_on_cancel_then_save(cx: &mut TestAppContext) {
    // text, date, time, Cancel: Enter on Cancel closes without saving.
    let (s, vcx) = edit_open(cx);
    vcx.simulate_keystrokes("tab tab tab enter");
    s.read_with(vcx, |s, _| assert!(s.log.rescheduled.is_empty()));
    // text, date, time, Cancel, Save: Enter on Save reschedules.
    let (s, vcx) = edit_open(cx);
    vcx.simulate_keystrokes("tab tab tab tab enter");
    s.read_with(vcx, |s, _| {
        assert_eq!(s.log.rescheduled.len(), 1);
        assert_eq!(s.log.rescheduled[0].1, "old text");
    });
    // Shift-Tab from the text wraps to Save.
    let (s, vcx) = edit_open(cx);
    vcx.simulate_keystrokes("shift-tab enter");
    s.read_with(vcx, |s, _| assert_eq!(s.log.rescheduled.len(), 1));
}

#[gpui::test]
fn edit_enter_saves_and_escape_closes_the_editor(cx: &mut TestAppContext) {
    let (s, vcx) = edit_open(cx);
    vcx.simulate_keystrokes("enter");
    s.read_with(vcx, |s, _| assert_eq!(s.log.rescheduled.len(), 1));
    let (s, vcx) = edit_open(cx);
    vcx.simulate_keystrokes("escape");
    s.read_with(vcx, |s, _| assert!(s.log.rescheduled.is_empty()));
    // The editor is gone: Enter no longer saves.
    vcx.simulate_keystrokes("enter");
    s.read_with(vcx, |s, _| assert!(s.log.rescheduled.is_empty()));
}

#[gpui::test]
fn shift_enter_on_cancel_does_not_save(cx: &mut TestAppContext) {
    // text, date, time, Cancel: Shift+Enter must neither cancel nor save.
    let (s, vcx) = edit_open(cx);
    vcx.simulate_keystrokes("tab tab tab shift-enter");
    s.read_with(vcx, |s, _| assert!(s.log.rescheduled.is_empty()));
}
