//! the pause menu, the conversation level menu and their marks.
mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::dnd_menu::{MAX_PAUSE_MINUTES, minutes_until_tomorrow, parse_custom_minutes};
use components::{
    MenuExtra, MenuKind, NotifyControls, NotifyMenu, SendLaterConfig, pause_rows, paused_text,
};
use domain::test_support::{cid, conv};
use domain::{
    ConversationId, ConversationPrefs, DndDay, DndStatus, FixedNow, NotifyLevel, WorkspaceState,
    dnd_ends,
};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use settings::{Clock, WorkspaceKey};

/// 2026-11-15 10:00:00 UTC (a Sunday); 11:00 in Madrid.
const NOW: i64 = 1_794_736_800;

fn madrid() -> TimeZone {
    TimeZone::get("Europe/Madrid").expect("tz")
}

fn zoned(secs: i64) -> jiff::ZonedDateTime {
    Timestamp::from_second(secs)
        .unwrap()
        .to_zoned_datetime(madrid())
}

// Pure parts.

#[test]
fn pause_rows_are_30m_1h_2h_tomorrow_custom() {
    let rows = pause_rows(&zoned(NOW), Clock::H24, &common::en());
    let minutes: Vec<_> = rows.iter().map(|r| r.minutes).collect();
    // Tomorrow 09:00 CET is 22 hours from 11:00 CET.
    assert_eq!(
        minutes,
        [Some(30), Some(60), Some(120), Some(22 * 60), None]
    );
    assert_eq!(rows[0].label, "30 minutes");
    assert_eq!(rows[3].label, "Until tomorrow 09:00");
    assert_eq!(rows[4].label, "Custom...");
}

#[test]
fn until_tomorrow_uses_schedule_zone() {
    // The same instant is 10:00 UTC and 11:00 in Madrid; 09:00 tomorrow is a
    // different distance in each zone.
    assert_eq!(minutes_until_tomorrow(&zoned(NOW)), 22 * 60);
    let utc = Timestamp::from_second(NOW)
        .unwrap()
        .to_zoned_datetime(TimeZone::UTC);
    assert_eq!(minutes_until_tomorrow(&utc), 23 * 60);
    // Tokyo is 19:00: 14 hours to 09:00 tomorrow.
    let tokyo = Timestamp::from_second(NOW)
        .unwrap()
        .to_zoned_datetime(TimeZone::get("Asia/Tokyo").unwrap());
    assert_eq!(minutes_until_tomorrow(&tokyo), 14 * 60);
}

#[test]
fn until_tomorrow_rounds_partial_minutes_up() {
    // 10:59:30 in Madrid: 22h 0m 30s to go is 22h 1m.
    assert_eq!(minutes_until_tomorrow(&zoned(NOW + 3570)), 21 * 60 + 1);
}

#[test]
fn custom_minutes_grammar() {
    for (text, want) in [
        ("45", Some(45)),
        (" 45 ", Some(45)),
        ("90m", Some(90)),
        ("2h", Some(120)),
        ("1h30m", Some(90)),
        ("1H 30M", Some(90)),
        ("0", None),
        ("", None),
        ("abc", None),
        ("1m1h", None),
        ("1h1h", None),
        ("h", None),
        ("5x", None),
        ("-5", None),
        ("1h30", None),
    ] {
        assert_eq!(parse_custom_minutes(text), want, "{text:?}");
    }
    assert_eq!(parse_custom_minutes("168h"), Some(MAX_PAUSE_MINUTES));
    assert_eq!(parse_custom_minutes("169h"), None);
    assert_eq!(parse_custom_minutes("99999999999"), None);
}

#[test]
fn paused_text_names_the_end() {
    let cfg = SendLaterConfig {
        presets: vec![],
        zone: madrid(),
        clock: Clock::H24,
        server: false,
    };
    let l = common::en();
    let snooze = |until| DndStatus {
        snooze_until: Some(until),
        ..Default::default()
    };
    let today = dnd_ends(&snooze(NOW + 1800), NOW, &madrid()).unwrap();
    assert_eq!(today.day, DndDay::Today);
    assert_eq!(
        paused_text(&today, &cfg, &l),
        "Notifications paused until 11:30"
    );
    let tomorrow = dnd_ends(&snooze(NOW + 22 * 3600), NOW, &madrid()).unwrap();
    assert_eq!(
        paused_text(&tomorrow, &cfg, &l),
        "Notifications paused until tomorrow 09:00"
    );
    let later = dnd_ends(&snooze(NOW + 3 * 86_400), NOW, &madrid()).unwrap();
    assert!(
        paused_text(&later, &cfg, &l).starts_with("Notifications paused until "),
        "{}",
        paused_text(&later, &cfg, &l)
    );
}

const PAUSE: [&str; 5] = [
    "notify-pause-0",
    "notify-pause-1",
    "notify-pause-2",
    "notify-pause-3",
    "notify-pause-4",
];
const LEVEL: [&str; 4] = [
    "notify-level-0",
    "notify-level-1",
    "notify-level-2",
    "notify-level-3",
];

// The menu view over a fake session.

#[derive(Default)]
struct Calls {
    pauses: Vec<u32>,
    resumes: usize,
    prefs: Vec<(ConversationId, ConversationPrefs)>,
    extras: Vec<(ConversationId, domain::NotifyExtras)>,
}

struct Src {
    can_pause: bool,
    paused_until: Option<i64>,
    prefs: ConversationPrefs,
    extras: domain::NotifyExtras,
    calls: Rc<RefCell<Calls>>,
}

impl NotifyControls for Src {
    fn can_pause(&self) -> bool {
        self.can_pause
    }
    fn paused(&self, now: i64, tz: &TimeZone) -> Option<domain::DndEnds> {
        let s = DndStatus {
            snooze_until: self.paused_until,
            ..Default::default()
        };
        dnd_ends(&s, now, tz)
    }
    fn pause(&mut self, minutes: u32, _: &mut gpui::App) {
        self.calls.borrow_mut().pauses.push(minutes);
    }
    fn resume(&mut self, _: &mut gpui::App) {
        self.calls.borrow_mut().resumes += 1;
    }
    fn conversation_prefs(&self, _: &ConversationId) -> ConversationPrefs {
        self.prefs
    }
    fn set_conversation_prefs(
        &mut self,
        c: ConversationId,
        p: ConversationPrefs,
        _: &mut gpui::App,
    ) {
        self.calls.borrow_mut().prefs.push((c, p));
    }
    fn notify_extras(&self, _: &ConversationId) -> domain::NotifyExtras {
        self.extras
    }
    fn set_notify_extras(&mut self, c: ConversationId, e: domain::NotifyExtras, _: &mut gpui::App) {
        self.calls.borrow_mut().extras.push((c, e));
    }
    fn notify_config(&self, _: &gpui::App) -> SendLaterConfig {
        SendLaterConfig {
            presets: vec![],
            zone: madrid(),
            clock: Clock::H24,
            server: false,
        }
    }
}

type Menu = Entity<NotifyMenu<Src>>;

fn setup(
    cx: &mut TestAppContext,
    src: Src,
    kind: MenuKind,
    extra: Vec<MenuExtra>,
) -> (Menu, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let src = cx.new(|_| src);
    let (menu, vcx) = cx.add_window_view(move |w, cx| NotifyMenu::new(src, kind, extra, w, cx));
    (menu, vcx)
}

fn src(calls: &Rc<RefCell<Calls>>) -> Src {
    Src {
        can_pause: true,
        paused_until: None,
        prefs: ConversationPrefs::default(),
        extras: domain::NotifyExtras::default(),
        calls: calls.clone(),
    }
}

#[gpui::test]
fn pause_menu_snoozes_for_choice(cx: &mut TestAppContext) {
    for (ix, want) in [(0, 30), (1, 60), (2, 120), (3, 22 * 60)] {
        let calls = Rc::<RefCell<Calls>>::default();
        let (m, vcx) = setup(cx, src(&calls), MenuKind::Workspace, vec![]);
        m.update_in(vcx, |m, w, cx| m.choose_pause(ix, w, cx));
        assert_eq!(calls.borrow().pauses, [want], "row {ix}");
    }
}

#[gpui::test]
fn pause_menu_rows_paint_and_click(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let (_m, vcx) = setup(cx, src(&calls), MenuKind::Workspace, vec![]);
    for sel in PAUSE {
        assert!(vcx.debug_bounds(sel).is_some(), "{sel}");
    }
    assert!(vcx.debug_bounds("notify-resume").is_none());
    let b = vcx.debug_bounds("notify-pause-1").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(calls.borrow().pauses, [60]);
}

#[gpui::test]
fn custom_row_opens_a_field_and_pauses_for_a_valid_duration(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let (m, vcx) = setup(cx, src(&calls), MenuKind::Workspace, vec![]);
    m.update_in(vcx, |m, w, cx| m.choose_pause(4, w, cx));
    assert!(calls.borrow().pauses.is_empty(), "opening is not a pause");
    assert!(vcx.debug_bounds("notify-custom-field").is_some());
    m.update(vcx, |m, cx| m.set_custom("bogus", cx));
    m.update(vcx, |m, cx| m.submit_custom(cx));
    assert!(calls.borrow().pauses.is_empty(), "invalid does not pause");
    m.update(vcx, |m, cx| m.set_custom("1h30m", cx));
    m.update(vcx, |m, cx| m.submit_custom(cx));
    assert_eq!(calls.borrow().pauses, [90]);
}

#[gpui::test]
fn resume_ends_snooze(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let mut s = src(&calls);
    s.paused_until = Some(NOW + 1800);
    let (m, vcx) = setup(cx, s, MenuKind::Workspace, vec![]);
    // Paused: the pause choices give way to the paused line and Resume.
    assert!(vcx.debug_bounds("notify-pause-0").is_none());
    assert!(vcx.debug_bounds("notify-resume").is_some());
    assert!(m.read_with(vcx, |m, cx| m.paused(cx)).is_some());
    m.update(vcx, |m, cx| m.resume(cx));
    assert_eq!(calls.borrow().resumes, 1);
}

#[gpui::test]
fn controls_hidden_without_capability(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let mut s = src(&calls);
    s.can_pause = false;
    let (_m, vcx) = setup(cx, s, MenuKind::Workspace, vec![]);
    for sel in PAUSE {
        assert!(vcx.debug_bounds(sel).is_none(), "{sel}");
    }
    assert!(vcx.debug_bounds("notify-resume").is_none());
}

#[gpui::test]
fn workspace_menu_lists_the_hosts_extra_entries(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let ran = Rc::new(RefCell::new(0));
    let r = ran.clone();
    let extra = vec![MenuExtra {
        danger: false,
        divider_before: false,
        icon: None,
        after: false,
        shortcut: None,
        label: "Remove workspace".into(),
        action: Rc::new(move |_, _| *r.borrow_mut() += 1),
    }];
    let mut s = src(&calls);
    s.can_pause = false;
    let (_m, vcx) = setup(cx, s, MenuKind::Workspace, extra);
    let b = vcx.debug_bounds("notify-extra-0").expect("extra entry");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(*ran.borrow(), 1);
}

#[gpui::test]
fn level_menu_sets_prefs(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let mut s = src(&calls);
    s.prefs = ConversationPrefs {
        level: NotifyLevel::All,
        muted: true,
    };
    let (m, vcx) = setup(cx, s, MenuKind::Conversation(cid("C1")), vec![]);
    // Choosing a level keeps the mute flag.
    m.update(vcx, |m, cx| m.choose_level(NotifyLevel::Nothing, cx));
    // Muting flips the flag and keeps the level.
    m.update(vcx, |m, cx| m.toggle_mute(cx));
    assert_eq!(
        calls.borrow().prefs,
        [
            (
                cid("C1"),
                ConversationPrefs {
                    level: NotifyLevel::Nothing,
                    muted: true
                }
            ),
            (
                cid("C1"),
                ConversationPrefs {
                    level: NotifyLevel::All,
                    muted: false
                }
            ),
        ]
    );
    for sel in LEVEL {
        assert!(vcx.debug_bounds(sel).is_some(), "{sel}");
    }
    assert!(vcx.debug_bounds("notify-mute").is_some());
    assert!(vcx.debug_bounds("notify-pause-0").is_none());
}

// The sidebar row menu: host entries around Notifications and Mute.

fn row_extras(ran: &Rc<RefCell<Vec<&'static str>>>) -> Vec<MenuExtra> {
    let entry = |name: &'static str| -> MenuExtra {
        let r = ran.clone();
        MenuExtra::new(name, Rc::new(move |_, _| r.borrow_mut().push(name)))
    };
    vec![
        entry("details").icon(components::IconName::Info),
        entry("star").icon(components::IconName::Star),
        entry("read")
            .icon(components::IconName::Check)
            .after_notifications(),
        entry("popout")
            .icon(components::IconName::External)
            .after_notifications(),
    ]
}

#[gpui::test]
fn row_menu_orders_entries_around_the_notification_rows(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let ran = Rc::default();
    let (_m, vcx) = setup(cx, src(&calls), MenuKind::Row(cid("C1")), row_extras(&ran));
    let mut y = |sel: &'static str| {
        vcx.debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel}"))
            .origin
            .y
    };
    let order = [
        y("notify-extra-0"),
        y("notify-extra-1"),
        y("notify-levels"),
        y("notify-mute"),
        y("notify-extra-2"),
        y("notify-extra-3"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    // The levels are one step in; the pause controls never show.
    assert!(vcx.debug_bounds("notify-level-0").is_none());
    assert!(vcx.debug_bounds("notify-pause-0").is_none());
}

#[gpui::test]
fn row_menu_entries_run_their_actions(cx: &mut TestAppContext) {
    for (sel, want) in [
        ("notify-extra-0", "details"),
        ("notify-extra-1", "star"),
        ("notify-extra-2", "read"),
        ("notify-extra-3", "popout"),
    ] {
        let calls = Rc::<RefCell<Calls>>::default();
        let ran = Rc::default();
        let (_m, vcx) = setup(cx, src(&calls), MenuKind::Row(cid("C1")), row_extras(&ran));
        let b = vcx.debug_bounds(sel).unwrap();
        vcx.simulate_click(b.center(), gpui::Modifiers::none());
        assert_eq!(*ran.borrow(), [want]);
    }
}

#[gpui::test]
fn row_menu_notifications_drill_into_levels_and_mute_toggles(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let ran = Rc::default();
    let (m, vcx) = setup(cx, src(&calls), MenuKind::Row(cid("C1")), row_extras(&ran));
    let b = vcx.debug_bounds("notify-mute").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(calls.borrow().prefs.len(), 1);
    assert!(calls.borrow().prefs[0].1.muted);
    let (m2, vcx) = setup(cx, src(&calls), MenuKind::Row(cid("C1")), row_extras(&ran));
    let _ = m;
    let b = vcx.debug_bounds("notify-levels").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert!(m2.read_with(vcx, |m, _| m.levels_open()));
    let b = vcx.debug_bounds("notify-level-3").expect("levels shown");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    let got = calls.borrow().prefs.last().unwrap().1;
    assert_eq!(got.level, NotifyLevel::Nothing);
}

#[gpui::test]
fn escape_in_the_levels_goes_back_then_closes(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let ran = Rc::default();
    let (m, vcx) = setup(cx, src(&calls), MenuKind::Row(cid("C1")), row_extras(&ran));
    let dismissed = Rc::new(RefCell::new(0));
    let d = dismissed.clone();
    let _sub = vcx
        .update(|_, cx| cx.subscribe(&m, move |_, _: &gpui::DismissEvent, _| *d.borrow_mut() += 1));
    let b = vcx.debug_bounds("notify-levels").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert!(m.read_with(vcx, |m, _| m.levels_open()));
    vcx.dispatch_action(components::PickerDismiss);
    assert!(!m.read_with(vcx, |m, _| m.levels_open()));
    assert_eq!(*dismissed.borrow(), 0, "first Esc only steps back");
    vcx.dispatch_action(components::PickerDismiss);
    assert_eq!(*dismissed.borrow(), 1, "second Esc closes");
}

#[test]
fn bindings_show_as_glyphs_on_mac_and_words_elsewhere() {
    assert_eq!(
        components::display_binding("cmd-shift-o", true),
        "\u{2318}\u{21E7}O"
    );
    assert_eq!(
        components::display_binding("ctrl-shift-o", false),
        "Ctrl+Shift+O"
    );
    assert_eq!(components::display_binding("escape", true), "escape");
}

// Marks on the sidebar rows.

#[test]
fn sidebar_rows_show_level_and_mute_from_the_prefs() {
    use components::{SidebarRow, build_rows};
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100);
    for id in ["c1", "c2", "c3"] {
        let mut c = conv(id, id);
        c.unread = true;
        st.conversations.insert(c.id.clone(), c);
    }
    st.local_notify_prefs.insert(
        cid("c1"),
        ConversationPrefs {
            level: NotifyLevel::Mentions,
            muted: false,
        },
    );
    st.local_notify_prefs.insert(
        cid("c2"),
        ConversationPrefs {
            level: NotifyLevel::Default,
            muted: true,
        },
    );
    let rows = build_rows(
        &st,
        None,
        &settings::SidebarSettings::default(),
        &Default::default(),
        &common::en(),
    );
    let find = |name: &str| {
        rows.iter()
            .find_map(|r| match r {
                SidebarRow::Conversation {
                    id, level, muted, ..
                } if id.0.as_ref() == name => Some((*level, *muted)),
                _ => None,
            })
            .expect(name)
    };
    assert_eq!(find("c1"), (NotifyLevel::Mentions, false));
    assert_eq!(find("c2"), (NotifyLevel::Default, true));
    assert_eq!(find("c3"), (NotifyLevel::Default, false));
    // The server's prefs count instead once the provider keeps them.
    st.caps.notify_prefs = true;
    let rows = build_rows(
        &st,
        None,
        &settings::SidebarSettings::default(),
        &Default::default(),
        &common::en(),
    );
    assert!(rows.iter().all(|r| !matches!(
        r,
        SidebarRow::Conversation { level, .. } if *level != NotifyLevel::Default
    )));
}

#[test]
fn level_marks() {
    use components::level_mark;
    assert_eq!(level_mark(NotifyLevel::Default), None);
    assert!(level_mark(NotifyLevel::All).is_some());
    assert_eq!(level_mark(NotifyLevel::Mentions), Some("@"));
    assert!(level_mark(NotifyLevel::Nothing).is_some());
}

// The rail's paused mark.

#[test]
fn rail_marks_a_paused_workspace() {
    use components::rail::{RailItem, rail_items};
    let key = WorkspaceKey::new("fake", "w1");
    let account = domain::AccountInfo {
        key: key.clone(),
        name: Arc::from("One"),
        org: None,
        icon: None,
    };
    let me = domain::test_support::uid("u1");
    let mut st = WorkspaceState::new(key.clone(), 100);
    st.self_user = Some(me.clone());
    st.dnd.insert(
        me,
        DndStatus {
            snooze_until: Some(NOW + 60),
            ..Default::default()
        },
    );
    let paused = |now| {
        let items = rail_items(
            std::slice::from_ref(&account),
            &[(key.clone(), &st)],
            None,
            &|_| None,
            now,
        );
        match &items[0] {
            RailItem::Workspace(w) => w.paused,
            RailItem::Org { .. } => unreachable!(),
        }
    };
    assert!(paused(NOW));
    assert!(!paused(NOW + 60), "the mark ends with the pause");
}

#[gpui::test]
fn conversation_menu_lists_the_board_extras(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let (_m, vcx) = setup(cx, src(&calls), MenuKind::Conversation(cid("C1")), vec![]);
    for sel in [
        "notify-threads",
        "notify-sound",
        "notify-mute-hour",
        "notify-mute",
    ] {
        assert!(vcx.debug_bounds(sel).is_some(), "{sel}");
    }
}

#[gpui::test]
fn thread_replies_and_sound_rows_flip_the_extras(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let (_m, vcx) = setup(cx, src(&calls), MenuKind::Conversation(cid("C1")), vec![]);
    for sel in ["notify-threads", "notify-sound"] {
        let b = vcx.debug_bounds(sel).expect(sel);
        vcx.simulate_click(b.center(), gpui::Modifiers::none());
    }
    assert_eq!(
        calls.borrow().extras,
        [
            (
                cid("C1"),
                domain::NotifyExtras {
                    threads_off: true,
                    ..Default::default()
                }
            ),
            (
                cid("C1"),
                domain::NotifyExtras {
                    sound_off: true,
                    ..Default::default()
                }
            ),
        ]
    );
}

#[gpui::test]
fn mute_for_one_hour_stores_a_deadline(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let mut s = src(&calls);
    s.extras.sound_off = true;
    let (_m, vcx) = setup(cx, s, MenuKind::Conversation(cid("C1")), vec![]);
    let b = vcx.debug_bounds("notify-mute-hour").expect("row");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(
        calls.borrow().extras,
        [(
            cid("C1"),
            domain::NotifyExtras {
                sound_off: true,
                mute_until: Some(NOW + 3600),
                ..Default::default()
            }
        )]
    );
    // The flag is untouched: the deadline alone mutes.
    assert!(calls.borrow().prefs.is_empty());
}

#[gpui::test]
fn timed_mute_offers_unmute_and_clears_the_deadline(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let mut s = src(&calls);
    s.extras.mute_until = Some(NOW + 600);
    let (m, vcx) = setup(cx, s, MenuKind::Conversation(cid("C1")), vec![]);
    // Muted by the deadline: no second "Mute for 1 hour" row.
    assert!(vcx.debug_bounds("notify-mute-hour").is_none());
    m.update(vcx, |m, cx| m.toggle_mute(cx));
    assert_eq!(
        calls.borrow().extras,
        [(cid("C1"), domain::NotifyExtras::default())]
    );
    // The flag was never set: it is not written.
    assert!(calls.borrow().prefs.is_empty());
}

#[gpui::test]
fn an_expired_deadline_does_not_count_as_muted(cx: &mut TestAppContext) {
    let calls = Rc::<RefCell<Calls>>::default();
    let mut s = src(&calls);
    s.extras.mute_until = Some(NOW - 1);
    let (m, vcx) = setup(cx, s, MenuKind::Conversation(cid("C1")), vec![]);
    assert!(!m.read_with(vcx, |m, cx| m.muted_now(cx)));
    assert!(vcx.debug_bounds("notify-mute-hour").is_some());
}
