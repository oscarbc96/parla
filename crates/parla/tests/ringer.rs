//! Ringing: notification with Join and Decline, ringtone and in-app banner,
//! against `FakeProvider` events, the real `GpuiNotifier` on GPUI's test
//! platform and a recording ringtone player. Nothing is audible.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use domain::test_support::{Call, FakeProvider, cid, conv, member, uid};
use domain::{
    Capabilities, ChatProvider, ConversationKind, DndStatus, Huddle, HuddleId, ProviderEvent,
    RING_TIMEOUT,
};
use gpui::{AppContext as _, Entity, SystemNotificationResponse, TestAppContext};
use parla::notifications::{GpuiNotifier, Notifications};
use parla::ringer::{Ringer, RingtoneHandle, RingtonePlayer};
use parla::session::WorkspaceSession;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

#[derive(Clone, Default)]
struct FakePlayer(Rc<RefCell<Vec<&'static str>>>);

impl RingtonePlayer for FakePlayer {
    fn play(&self, looped: bool) -> Option<RingtoneHandle> {
        self.0
            .borrow_mut()
            .push(if looped { "loop" } else { "once" });
        let log = self.0.clone();
        Some(RingtoneHandle::new(move || log.borrow_mut().push("stop")))
    }
}

impl FakePlayer {
    fn log(&self) -> Vec<&'static str> {
        self.0.borrow().clone()
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    player: FakePlayer,
    provider: Arc<FakeProvider>,
    session: Entity<WorkspaceSession>,
}

fn init(cx: &mut TestAppContext, settings_json: &str) -> (tempfile::TempDir, FakePlayer) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(&paths.settings, settings_json).unwrap();
    let player = FakePlayer::default();
    let p = player.clone();
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_app_identity("com.justparla.parla", "Parla");
        Notifications::new(Rc::new(GpuiNotifier::new())).install(cx);
        Ringer::new(Rc::new(p), cx).install(cx);
    });
    (dir, player)
}

fn session(
    cx: &mut TestAppContext,
    ws: &str,
    caps: Capabilities,
) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let provider = Arc::new(FakeProvider::new(ws, uid("u1")).with_capabilities(caps));
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    provider.add_conversation(dm);
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Ada"));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    (provider, s)
}

fn huddle_caps() -> Capabilities {
    Capabilities {
        huddles: true,
        dnd: true,
        ..Default::default()
    }
}

fn setup(cx: &mut TestAppContext, settings_json: &str) -> Fixture {
    let (dir, player) = init(cx, settings_json);
    let (provider, session) = session(cx, "w1", huddle_caps());
    Fixture {
        _dir: dir,
        player,
        provider,
        session,
    }
}

const EN: &str = r#"{ "locale": "en" }"#;

fn ring_event(p: &FakeProvider, id: &str) {
    p.push(ProviderEvent::HuddleRing {
        conversation: cid("D1"),
        huddle: HuddleId(Arc::from(id)),
        from: uid("u2"),
    });
}

fn ring(f: &Fixture, cx: &mut TestAppContext) {
    ring_event(&f.provider, "H1");
    cx.run_until_parked();
}

fn ring_notifications(cx: &TestAppContext) -> Vec<gpui::SystemNotification> {
    cx.delivered_system_notifications()
        .into_iter()
        .filter(|n| n.tag.ends_with("/ring"))
        .collect()
}

fn active(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<Ringer>().active().is_some())
}

fn banner(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<Ringer>().banner(cx).is_some())
}

#[gpui::test]
async fn ring_shows_notification_banner_and_sound(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    let n = ring_notifications(cx);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].tag.as_ref(), "fake/w1/D1/ring");
    assert!(n[0].title.contains("Ada"), "{}", n[0].title);
    let ids: Vec<_> = n[0].actions.iter().map(|a| a.id.to_string()).collect();
    assert_eq!(ids, ["join", "decline"]);
    assert!(banner(cx));
    assert_eq!(f.player.log(), ["loop"]);
}

#[gpui::test]
async fn decline_from_notification_sends_call(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "fake/w1/D1/ring".into(),
        action_id: Some("decline".into()),
    });
    cx.run_until_parked();
    assert!(f.provider.calls().iter().any(|c| matches!(
        c,
        Call::DeclineHuddle { id, .. } if id.0.as_ref() == "H1"
    )));
    assert!(ring_notifications(cx).is_empty());
    assert!(!banner(cx));
    assert_eq!(f.player.log(), ["loop", "stop"]);
}

#[gpui::test]
async fn join_from_banner_opens_huddle(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    cx.update(Ringer::join);
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://fake.example.test/huddle/D1")
    );
    assert!(!active(cx));
    assert!(f.session.read_with(cx, |s, _| s.state().ring.is_none()));
    assert!(ring_notifications(cx).is_empty());
    assert_eq!(f.player.log(), ["loop", "stop"]);
}

#[gpui::test]
async fn join_from_notification_action_opens_huddle(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "fake/w1/D1/ring".into(),
        action_id: Some("join".into()),
    });
    cx.run_until_parked();
    assert!(cx.opened_url().is_some());
    assert!(!active(cx));
}

#[gpui::test]
async fn timeout_leaves_missed_notification(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    cx.executor()
        .advance_clock(RING_TIMEOUT + Duration::from_secs(1));
    cx.run_until_parked();
    assert!(!active(cx));
    assert!(!banner(cx));
    assert_eq!(f.player.log(), ["loop", "stop"]);
    let all = cx.delivered_system_notifications();
    assert!(ring_notifications(cx).is_empty());
    let missed: Vec<_> = all.iter().filter(|n| n.title.contains("Missed")).collect();
    assert_eq!(missed.len(), 1, "{all:?}");
    assert!(missed[0].title.contains("Ada"));
    assert!(missed[0].actions.is_empty());
}

#[gpui::test]
async fn answered_elsewhere_stops_everything(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    f.provider.push(ProviderEvent::HuddleUpdated(Huddle {
        id: HuddleId(Arc::from("H1")),
        conversation: cid("D1"),
        participants: vec![uid("u1"), uid("u2")].into(),
        started: 0,
        created_by: Some(uid("u2")),
        is_dm: true,
    }));
    cx.run_until_parked();
    assert!(!active(cx));
    assert!(!banner(cx));
    assert!(cx.delivered_system_notifications().is_empty());
    assert_eq!(f.player.log(), ["loop", "stop"]);
}

#[gpui::test]
async fn caller_cancelling_stops_without_missed(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    ring(&f, cx);
    f.provider.push(ProviderEvent::HuddleRingCancelled {
        conversation: cid("D1"),
    });
    cx.run_until_parked();
    assert!(!active(cx));
    assert_eq!(f.player.log(), ["loop", "stop"]);
}

#[gpui::test]
async fn dnd_suppresses_ring(cx: &mut TestAppContext) {
    let (_d, player) = init(cx, EN);
    cx.update(|cx| {
        components::set_now(
            cx,
            Arc::new(domain::FixedNow(
                jiff::Timestamp::from_second(1_000).unwrap(),
            )),
        )
    });
    let (provider, _s) = {
        let provider =
            Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(huddle_caps()));
        provider.set_dnd(DndStatus {
            enabled: false,
            next_start: None,
            next_end: None,
            snooze_until: Some(5_000),
        });
        let mut dm = conv("D1", "dm");
        dm.kind = ConversationKind::Direct;
        provider.add_conversation(dm);
        provider.add_member(member("u2", "Ada"));
        let p: Arc<dyn ChatProvider> = provider.clone();
        let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
        let s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
        cx.run_until_parked();
        (provider, s)
    };
    ring_event(&provider, "H1");
    cx.run_until_parked();
    assert!(!active(cx));
    assert!(cx.shown_system_notifications().is_empty());
    assert!(player.log().is_empty());
}

#[gpui::test]
async fn calls_ring_off_never_rings(cx: &mut TestAppContext) {
    let f = setup(cx, r#"{ "locale": "en", "calls": { "ring": false } }"#);
    ring(&f, cx);
    assert!(!active(cx));
    assert!(cx.shown_system_notifications().is_empty());
    assert!(f.player.log().is_empty());
}

#[gpui::test]
async fn quiet_hours_keep_a_ring_silent(cx: &mut TestAppContext) {
    // 00:00 to 23:59 covers every moment the test can run.
    let f = setup(
        cx,
        r#"{ "locale": "en", "notifications": { "quiet_hours": { "from": "00:00", "to": "23:59" } } }"#,
    );
    cx.update(|cx| {
        components::set_now(
            cx,
            Arc::new(domain::FixedNow(
                jiff::Timestamp::from_second(43_200).unwrap(),
            )),
        )
    });
    ring(&f, cx);
    assert!(!active(cx));
    assert!(cx.shown_system_notifications().is_empty());
    assert!(f.player.log().is_empty());
}

#[gpui::test]
async fn sound_none_rings_without_chime(cx: &mut TestAppContext) {
    let f = setup(
        cx,
        r#"{ "locale": "en", "notifications": { "sound": "none" } }"#,
    );
    ring(&f, cx);
    assert!(active(cx));
    assert!(banner(cx));
    assert!(f.player.log().is_empty());
}

#[gpui::test]
async fn ringtone_none_rings_without_chime(cx: &mut TestAppContext) {
    let f = setup(cx, r#"{ "locale": "en", "calls": { "ringtone": "none" } }"#);
    ring(&f, cx);
    assert!(active(cx));
    assert!(f.player.log().is_empty());
}

#[gpui::test]
async fn focused_conversation_plays_once(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    let w = cx.add_empty_window();
    w.update(|window, _| window.activate_window());
    f.session.update(cx, |s, cx| s.open(cid("D1"), cx));
    cx.run_until_parked();
    ring(&f, cx);
    assert_eq!(f.player.log(), ["once"]);
    assert!(active(cx));
}

#[gpui::test]
async fn second_ring_is_missed_only(cx: &mut TestAppContext) {
    let (_d, player) = init(cx, EN);
    let (p1, _s1) = session(cx, "w1", huddle_caps());
    let (p2, _s2) = session(cx, "w2", huddle_caps());
    ring_event(&p1, "H1");
    cx.run_until_parked();
    ring_event(&p2, "H2");
    cx.run_until_parked();
    assert_eq!(ring_notifications(cx).len(), 1);
    assert_eq!(player.log(), ["loop"]);
    let missed = cx
        .delivered_system_notifications()
        .into_iter()
        .filter(|n| n.title.contains("Missed"))
        .count();
    assert_eq!(missed, 1);
    // Ending the first leaves the second as it was: no ring, no sound.
    cx.update(Ringer::decline);
    cx.run_until_parked();
    assert!(!active(cx));
    assert_eq!(player.log(), ["loop", "stop"]);
}

#[gpui::test]
async fn ring_in_lean_workspace(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    f.session.update(cx, |s, cx| s.set_lean(true, cx));
    cx.run_until_parked();
    assert!(f.session.read_with(cx, |s, _| s.is_lean()));
    ring(&f, cx);
    assert!(active(cx));
    assert_eq!(ring_notifications(cx).len(), 1);
    // Joining works from a lean workspace too.
    cx.update(Ringer::join);
    cx.run_until_parked();
    assert!(cx.opened_url().is_some());
}

#[gpui::test]
async fn one_shot_timer_only_while_ringing(cx: &mut TestAppContext) {
    let f = setup(cx, EN);
    assert!(!cx.update(|cx| cx.global::<Ringer>().timer_armed()));
    ring(&f, cx);
    assert!(cx.update(|cx| cx.global::<Ringer>().timer_armed()));
    cx.update(Ringer::decline);
    cx.run_until_parked();
    assert!(!cx.update(|cx| cx.global::<Ringer>().timer_armed()));
}

#[gpui::test]
async fn dnd_starting_mid_ring_stops_banner_and_chime(cx: &mut TestAppContext) {
    let (_d, player) = init(cx, EN);
    let set_now = |cx: &mut TestAppContext, s: i64| {
        cx.update(|cx| {
            components::set_now(
                cx,
                Arc::new(domain::FixedNow(jiff::Timestamp::from_second(s).unwrap())),
            )
        })
    };
    set_now(cx, 1_000);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(huddle_caps()));
    provider.set_dnd(DndStatus {
        enabled: true,
        next_start: Some(2_000),
        next_end: Some(9_000),
        snooze_until: None,
    });
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    provider.add_conversation(dm);
    provider.add_member(member("u2", "Ada"));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let _s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    ring_event(&provider, "H1");
    cx.run_until_parked();
    assert!(active(cx));
    // The DND window opens while the phone rings; the next event reads the clock.
    set_now(cx, 3_000);
    provider.push(ProviderEvent::Disconnected);
    cx.run_until_parked();
    assert!(!active(cx));
    assert!(!banner(cx));
    assert_eq!(player.log(), ["loop", "stop"]);
}

#[gpui::test]
async fn quiet_hours_do_not_block_later_rings(cx: &mut TestAppContext) {
    let f = setup(
        cx,
        r#"{ "locale": "en", "notifications": { "quiet_hours": { "from": "00:00", "to": "23:59" } } }"#,
    );
    cx.update(|cx| {
        components::set_now(
            cx,
            Arc::new(domain::FixedNow(
                jiff::Timestamp::from_second(43_200).unwrap(),
            )),
        )
    });
    ring(&f, cx);
    assert!(!active(cx));
    // The held ring is ended in the reducer, so it cannot refuse later rings.
    assert!(f.session.read_with(cx, |s, _| s.state().ring.is_none()));
}
