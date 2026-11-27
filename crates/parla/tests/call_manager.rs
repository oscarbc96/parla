//! `CallManager`: Join through the ladder, against `FakeProvider` and a fake
//! helper spawner (in-memory pipes). No real process, no real window.

mod support;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use components::CallBarPhase;
use domain::test_support::{FakeProvider, cid, conv, member, uid};
use domain::{
    CallSurface, Capabilities, ChatProvider, ConversationKind, Huddle, HuddleId, ProviderEvent,
    SecretText,
};
use futures::channel::mpsc;
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::call_manager::{CallManager, CallManagerHandle};
use parla::notifications::{GpuiNotifier, Notifications};
use parla::ringer::Ringer;
use parla::session::WorkspaceSession;
use settings::ConfigPaths;
use store::{MemoryStore, Store};
use support::call_fake::{Fake, Silent};

fn caps() -> Capabilities {
    Capabilities {
        huddles: true,
        call_surface: true,
        ..Default::default()
    }
}

fn init(cx: &mut TestAppContext, join_with: &str, window_ok: bool) -> (tempfile::TempDir, Fake) {
    init_calls(cx, &format!(r#""join_with": "{join_with}""#), window_ok)
}

/// `calls` is the body of the settings file's `calls` object.
fn init_calls(cx: &mut TestAppContext, calls: &str, window_ok: bool) -> (tempfile::TempDir, Fake) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(
        &paths.settings,
        format!(r#"{{ "locale": "en", "calls": {{ {calls} }} }}"#),
    )
    .unwrap();
    let fake = Fake::default();
    let f = fake.clone();
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_app_identity("com.justparla.parla", "Parla");
        Notifications::new(Rc::new(GpuiNotifier::new())).install(cx);
        Ringer::new(Rc::new(Silent), cx).install(cx);
        CallManager::install(Box::new(f), cx).update(cx, |m, _| m.set_window_supported(window_ok));
    });
    (dir, fake)
}

fn surface() -> Result<CallSurface, domain::ProviderError> {
    Ok(CallSurface {
        url: Arc::from("https://app.example.test/h"),
        title: Arc::from("Huddle"),
        cookies: vec![domain::SurfaceCookie {
            name: Arc::from("d"),
            value: SecretText::new("sekrit-value"),
            domain: Arc::from(".example.test"),
            path: Arc::from("/"),
            secure: true,
            http_only: true,
        }],
        allowed_host_suffixes: Arc::from([Arc::from("example.test")]),
        signed_out_paths: Arc::from([Arc::from("/signin")]),
        user_agent: None,
    })
}

fn session(
    cx: &mut TestAppContext,
    ws: &str,
    caps: Capabilities,
) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let provider = Arc::new(FakeProvider::new(ws, uid("u1")).with_capabilities(caps));
    provider.set_call_surface(surface);
    provider.add_conversation(conv("C1", "general"));
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

fn huddle(p: &FakeProvider, c: &str, with_me: bool) {
    let mut who = vec![uid("u2")];
    if with_me {
        who.push(uid("u1"));
    }
    p.push(ProviderEvent::HuddleUpdated(Huddle {
        id: HuddleId(Arc::from(format!("H-{c}"))),
        conversation: cid(c),
        participants: who.into(),
        started: 0,
        created_by: Some(uid("u2")),
        is_dm: false,
    }));
}

fn join(s: &Entity<WorkspaceSession>, c: &str, cx: &mut TestAppContext) {
    s.update(cx, |s, cx| {
        components::HuddleSource::join_huddle(s, cid(c), cx)
    });
    cx.run_until_parked();
}

fn setup(cx: &mut TestAppContext, join_with: &str, window_ok: bool) -> Rig {
    let (dir, fake) = init(cx, join_with, window_ok);
    let (provider, session) = session(cx, "w1", caps());
    huddle(&provider, "C1", false);
    cx.run_until_parked();
    Rig {
        _dir: dir,
        fake,
        provider,
        session,
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    fake: Fake,
    provider: Arc<FakeProvider>,
    session: Entity<WorkspaceSession>,
}

fn phase(cx: &mut TestAppContext) -> Option<CallBarPhase> {
    cx.update(|cx| {
        cx.global::<CallManagerHandle>()
            .0
            .read(cx)
            .bar_model(cx)
            .map(|m| m.phase)
    })
}

#[gpui::test]
async fn join_with_window_opens_call_window(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    join(&r.session, "C1", cx);
    assert_eq!(r.fake.spawns(), 1);
    assert_eq!(cx.opened_url(), None);
    let lines = r.fake.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
}

#[gpui::test]
async fn join_with_browser_never_spawns(cx: &mut TestAppContext) {
    let r = setup(cx, "browser", true);
    join(&r.session, "C1", cx);
    assert_eq!(r.fake.spawns(), 0);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://fake.example.test/huddle/C1")
    );
}

#[gpui::test]
async fn unsupported_platform_uses_browser(cx: &mut TestAppContext) {
    let r = setup(cx, "window", false);
    join(&r.session, "C1", cx);
    assert_eq!(r.fake.spawns(), 0);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://fake.example.test/huddle/C1")
    );
}

#[gpui::test]
async fn no_capability_uses_browser(cx: &mut TestAppContext) {
    let (_d, fake) = init(cx, "window", true);
    let (p, s) = session(
        cx,
        "w1",
        Capabilities {
            huddles: true,
            ..Default::default()
        },
    );
    huddle(&p, "C1", false);
    cx.run_until_parked();
    join(&s, "C1", cx);
    assert_eq!(fake.spawns(), 0);
    assert!(cx.opened_url().is_some());
}

#[gpui::test]
async fn window_failure_opens_browser_once_with_toast(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    r.fake.fail_next_spawn();
    join(&r.session, "C1", cx);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://fake.example.test/huddle/C1")
    );
    let notice = r.session.read_with(cx, |s, _| s.file_notice());
    assert!(matches!(notice, Some(components::FileNotice::Error(_))));
    assert_eq!(phase(cx), None, "the failed call is over");
}

#[gpui::test]
async fn ring_join_opens_call_window(cx: &mut TestAppContext) {
    let (_d, fake) = init(cx, "window", true);
    let (p, s) = session(cx, "w1", caps());
    p.push(ProviderEvent::HuddleRing {
        conversation: cid("D1"),
        huddle: HuddleId(Arc::from("H1")),
        from: uid("u2"),
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.global::<Ringer>().active().is_some()));
    cx.update(Ringer::join);
    cx.run_until_parked();
    assert_eq!(fake.spawns(), 1);
    assert_eq!(cx.opened_url(), None);
    assert!(cx.update(|cx| cx.global::<Ringer>().active().is_none()));
    assert!(s.read_with(cx, |s, _| s.state().ring.is_none()));
}

#[gpui::test]
async fn ring_during_call_is_missed_only(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    join(&r.session, "C1", cx);
    assert_eq!(phase(cx), Some(CallBarPhase::Loading));
    r.provider.push(ProviderEvent::HuddleRing {
        conversation: cid("D1"),
        huddle: HuddleId(Arc::from("H9")),
        from: uid("u2"),
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.global::<Ringer>().active().is_none()));
    assert!(!cx.update(|cx| cx.global::<Ringer>().banner(cx).is_some()));
    let missed = cx
        .delivered_system_notifications()
        .into_iter()
        .filter(|n| n.title.contains("Missed"))
        .count();
    assert_eq!(missed, 1);
}

#[gpui::test]
async fn second_workspace_join_asks(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    let (p2, s2) = session(cx, "w2", caps());
    huddle(&p2, "C1", false);
    cx.run_until_parked();
    cx.add_empty_window();
    join(&r.session, "C1", cx);
    r.fake.live();
    cx.run_until_parked();
    assert_eq!(phase(cx), Some(CallBarPhase::Live));

    join(&s2, "C1", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    assert_eq!(r.fake.spawns(), 1);
    assert_eq!(phase(cx), Some(CallBarPhase::Live));

    join(&s2, "C1", cx);
    cx.simulate_prompt_answer("Leave and join");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(r.fake.spawns(), 2, "the second workspace's call started");
    assert!(r.fake.kills() >= 1);
}

#[gpui::test]
async fn provider_end_closes_window(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    join(&r.session, "C1", cx);
    r.fake.live();
    cx.run_until_parked();
    assert_eq!(phase(cx), Some(CallBarPhase::Live));
    r.provider.push(ProviderEvent::HuddleEnded {
        conversation: cid("C1"),
        id: HuddleId(Arc::from("H-C1")),
        ended: 0,
    });
    cx.run_until_parked();
    assert!(
        r.fake.lines().iter().any(|l| l.contains("leave")),
        "{:?}",
        r.fake.lines()
    );
}

#[gpui::test]
async fn quit_ends_call(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    join(&r.session, "C1", cx);
    r.fake.live();
    cx.run_until_parked();
    cx.update(|cx| cx.global::<CallManagerHandle>().0.clone())
        .update(cx, |m, _| m.shutdown());
    cx.run_until_parked();
    assert_eq!(r.fake.kills(), 1);
    assert_eq!(phase(cx), None);
}

#[gpui::test]
async fn bar_model_follows_the_call(cx: &mut TestAppContext) {
    let r = setup(cx, "window", true);
    assert_eq!(phase(cx), None);
    join(&r.session, "C1", cx);
    r.fake.live();
    cx.run_until_parked();
    let m = cx.update(|cx| {
        cx.global::<CallManagerHandle>()
            .0
            .read(cx)
            .bar_model(cx)
            .unwrap()
    });
    assert_eq!(m.phase, CallBarPhase::Live);
    assert_eq!(m.title.as_ref(), "general");
    assert_eq!(m.participants, 1);
    assert!(!m.load_slow);
    assert!(!m.screen_share_unavailable);
    // The conversation of the running call offers no Start or Join.
    let in_call = |c: &str, cx: &mut TestAppContext| {
        cx.update(|cx| {
            cx.global::<CallManagerHandle>().0.read(cx).in_call_for(
                &settings::WorkspaceKey::new("fake", "w1"),
                &cid(c),
                cx,
            )
        })
    };
    assert!(in_call("C1", cx));
    assert!(!in_call("C2", cx));
}

// Native audio rung: a fake engine, so no devices and no network.

use calls::native::{CallEvent, CallEvents, DeviceChange, Participant};
use domain::NativeCallJoin;
use domain::test_support::Op;
use parla::call_manager::{CallDevices, NativeEngine, NativeHandle};

#[derive(Default)]
struct EngineState {
    tx: Mutex<Option<mpsc::Sender<CallEvent>>>,
    mutes: Mutex<Vec<bool>>,
    joins: AtomicUsize,
    leaves: AtomicUsize,
    /// The devices each join asked for.
    joined_with: Mutex<Vec<CallDevices>>,
    /// Microphones the fake reports as connected; `None` is the default list.
    inputs: Mutex<Option<Vec<String>>>,
    /// Each reopen: the devices, and whether input and output reopened.
    reopens: Mutex<Vec<(CallDevices, bool, bool)>>,
}

#[derive(Clone, Default)]
struct FakeEngine(Arc<EngineState>);

struct FakeHandle(Arc<EngineState>);

impl NativeHandle for FakeHandle {
    fn mute(&self, muted: bool) {
        self.0.mutes.lock().unwrap().push(muted);
    }
    fn reopen(&self, devices: &CallDevices, input: bool, output: bool) {
        self.0
            .reopens
            .lock()
            .unwrap()
            .push((devices.clone(), input, output));
    }
    fn leave(self: Box<Self>) {
        self.0.leaves.fetch_add(1, Ordering::SeqCst);
    }
}

impl NativeEngine for FakeEngine {
    fn join(
        &self,
        _: NativeCallJoin,
        devices: &CallDevices,
    ) -> (Box<dyn NativeHandle>, CallEvents) {
        self.0.joins.fetch_add(1, Ordering::SeqCst);
        self.0.joined_with.lock().unwrap().push(devices.clone());
        let (tx, rx) = mpsc::channel(16);
        *self.0.tx.lock().unwrap() = Some(tx);
        (Box::new(FakeHandle(self.0.clone())), rx)
    }
    fn device_names(&self) -> (Vec<String>, Vec<String>) {
        let inputs = self.0.inputs.lock().unwrap().clone();
        (
            inputs.unwrap_or_else(|| vec!["Built-in Microphone".into(), "USB Mic".into()]),
            vec!["Built-in Speakers".into()],
        )
    }
}

impl FakeEngine {
    fn reopens(&self) -> Vec<(CallDevices, bool, bool)> {
        self.0.reopens.lock().unwrap().clone()
    }

    fn say(&self, e: CallEvent) {
        self.0
            .tx
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .try_send(e)
            .unwrap();
    }
}

fn native_caps() -> Capabilities {
    Capabilities {
        huddles: true,
        call_surface: true,
        native_calls: true,
        ..Default::default()
    }
}

fn native_setup(cx: &mut TestAppContext, join_with: &str, window_ok: bool) -> (Rig, FakeEngine) {
    let (dir, fake) = init(cx, join_with, window_ok);
    let engine = FakeEngine::default();
    let e = engine.clone();
    cx.update(|cx| cx.global::<CallManagerHandle>().0.clone())
        .update(cx, |m, _| m.set_native_engine(Some(Arc::new(e))));
    let (provider, session) = session(cx, "w1", native_caps());
    huddle(&provider, "C1", false);
    cx.run_until_parked();
    (
        Rig {
            _dir: dir,
            fake,
            provider,
            session,
        },
        engine,
    )
}

fn bar(cx: &mut TestAppContext) -> Option<components::CallBarModel> {
    cx.update(|cx| cx.global::<CallManagerHandle>().0.read(cx).bar_model(cx))
}

fn attendee(id: &str, dropped: bool) -> Participant {
    Participant {
        attendee_id: id.into(),
        dropped,
        ..Default::default()
    }
}

#[gpui::test]
async fn native_join_is_connecting_then_live_and_muted_on_join(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    assert_eq!(r.fake.spawns(), 0, "no call window");
    assert_eq!(cx.opened_url(), None);
    assert_eq!(e.0.joins.load(Ordering::SeqCst), 1);
    let m = bar(cx).unwrap();
    assert_eq!(m.phase, CallBarPhase::Loading);
    assert!(!m.show_window);
    assert_eq!(*e.0.mutes.lock().unwrap(), vec![true], "mute_on_join");

    e.say(CallEvent::Connecting);
    e.say(CallEvent::Live);
    e.say(CallEvent::Participants(vec![
        attendee("a", false),
        attendee("b", false),
        attendee("c", true),
    ]));
    cx.run_until_parked();
    let m = bar(cx).unwrap();
    assert_eq!(m.phase, CallBarPhase::Live);
    assert_eq!(m.mic, components::MediaState::Muted);
    assert_eq!(m.participants, 2);

    e.say(CallEvent::Muted(false));
    cx.run_until_parked();
    assert_eq!(bar(cx).unwrap().mic, components::MediaState::Active);
}

#[gpui::test]
async fn native_mute_and_leave_go_to_the_handle(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    cx.run_until_parked();
    cx.update(CallManager::toggle_mute);
    cx.run_until_parked();
    assert_eq!(*e.0.mutes.lock().unwrap(), vec![true, false]);
    assert_eq!(bar(cx).unwrap().mic, components::MediaState::Active);

    cx.update(CallManager::leave);
    cx.run_until_parked();
    assert_eq!(e.0.leaves.load(Ordering::SeqCst), 1);
    assert!(bar(cx).is_none());
    assert!(
        r.provider
            .calls()
            .iter()
            .any(|c| matches!(c, domain::test_support::Call::LeaveHuddle(_)))
    );
}

#[gpui::test]
async fn native_left_event_ends_the_call(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    e.say(CallEvent::Left);
    cx.run_until_parked();
    assert!(bar(cx).is_none());
    assert_eq!(e.0.leaves.load(Ordering::SeqCst), 1, "engine stopped");
    assert_eq!(r.fake.spawns(), 0);
}

#[gpui::test]
async fn failure_before_media_falls_back_to_the_window(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Failed {
        before_media: true,
        reason: "ice".into(),
    });
    cx.run_until_parked();
    assert_eq!(r.fake.spawns(), 1, "the call window took over");
    assert_eq!(phase(cx), Some(CallBarPhase::Loading));
    assert!(bar(cx).unwrap().show_window);
    assert_eq!(e.0.leaves.load(Ordering::SeqCst), 1);
}

#[gpui::test]
async fn native_join_error_falls_back_to_the_window(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    r.provider
        .fail_next(Op::NativeCallJoin, domain::ProviderError::Unsupported);
    join(&r.session, "C1", cx);
    assert_eq!(e.0.joins.load(Ordering::SeqCst), 0);
    assert_eq!(r.fake.spawns(), 1);
}

#[gpui::test]
async fn native_failure_without_window_opens_the_browser(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", false);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Failed {
        before_media: true,
        reason: "ice".into(),
    });
    cx.run_until_parked();
    assert_eq!(r.fake.spawns(), 0);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://fake.example.test/huddle/C1")
    );
    assert!(bar(cx).is_none());
    let notice = r.session.read_with(cx, |s, _| s.file_notice());
    assert!(matches!(notice, Some(components::FileNotice::Error(_))));
}

#[gpui::test]
async fn failure_after_media_is_an_error_without_fallback(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    e.say(CallEvent::Failed {
        before_media: false,
        reason: "dtls".into(),
    });
    cx.run_until_parked();
    assert_eq!(r.fake.spawns(), 0, "no fallback after media");
    assert_eq!(cx.opened_url(), None);
    assert!(bar(cx).is_none());
    let notice = r.session.read_with(cx, |s, _| s.file_notice());
    assert!(matches!(notice, Some(components::FileNotice::Error(_))));
}

#[gpui::test]
async fn missing_microphone_is_a_notice_and_the_call_goes_on(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    e.say(CallEvent::CaptureUnavailable("no device".into()));
    cx.run_until_parked();
    assert_eq!(bar(cx).unwrap().phase, CallBarPhase::Live);
    let notice = r.session.read_with(cx, |s, _| s.file_notice());
    assert!(matches!(notice, Some(components::FileNotice::Error(_))));
}

#[gpui::test]
async fn other_settings_and_capability_skip_native(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "window", true);
    join(&r.session, "C1", cx);
    assert_eq!(e.0.joins.load(Ordering::SeqCst), 0);
    assert_eq!(r.fake.spawns(), 1);

    // The provider cannot do native calls: the window it is.
    let (_d, fake) = init(cx, "native", true);
    let (p, s) = session(cx, "w1", caps());
    huddle(&p, "C1", false);
    cx.run_until_parked();
    join(&s, "C1", cx);
    assert_eq!(fake.spawns(), 1);
}

#[gpui::test]
async fn huddle_end_closes_a_live_native_call(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    cx.run_until_parked();
    r.provider.push(ProviderEvent::HuddleEnded {
        conversation: cid("C1"),
        id: HuddleId(Arc::from("H-C1")),
        ended: 0,
    });
    cx.run_until_parked();
    assert!(bar(cx).is_none());
    assert_eq!(e.0.leaves.load(Ordering::SeqCst), 1);
}

fn person(attendee: &str, user: &str) -> Participant {
    Participant {
        attendee_id: attendee.into(),
        external_user_id: user.into(),
        ..Default::default()
    }
}

fn devices(input: Option<&str>, output: Option<&str>) -> CallDevices {
    CallDevices {
        input: input.map(Into::into),
        output: output.map(Into::into),
    }
}

fn manager(cx: &mut TestAppContext) -> Entity<CallManager> {
    cx.update(|cx| cx.global::<CallManagerHandle>().0.clone())
}

#[gpui::test]
async fn native_attendees_map_to_members_and_the_speaker_is_marked(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    e.say(CallEvent::Participants(vec![
        person("a-me", "u1"),
        person("a-ada", "u2"),
        person("a-x", "u9"),
    ]));
    cx.run_until_parked();
    let m = bar(cx).unwrap();
    assert_eq!(m.participants, 3);
    // Everyone but this user, by the member's name; an unknown one is
    // "Someone", never a raw id.
    let names: Vec<_> = m.people.iter().map(|p| p.name.to_string()).collect();
    assert_eq!(names, ["Ada", "Someone"]);
    assert!(m.people.iter().all(|p| !p.speaking));

    e.say(CallEvent::Speaking(vec!["a-ada".into()]));
    cx.run_until_parked();
    let m = bar(cx).unwrap();
    assert!(m.people[0].speaking);
    assert!(!m.people[1].speaking);
    let _ = r;
}

#[gpui::test]
async fn a_share_offers_open_in_browser_on_the_native_bar(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    cx.run_until_parked();
    assert!(!bar(cx).unwrap().screen_share_unavailable);
    e.say(CallEvent::ScreenShare(true));
    cx.run_until_parked();
    assert!(bar(cx).unwrap().screen_share_unavailable);
    cx.update(CallManager::open_in_browser);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://fake.example.test/huddle/C1")
    );
    e.say(CallEvent::ScreenShare(false));
    cx.run_until_parked();
    assert!(!bar(cx).unwrap().screen_share_unavailable);
}

#[gpui::test]
async fn stream_errors_reopen_the_failed_device(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    e.say(CallEvent::CaptureError("unplugged".into()));
    e.say(CallEvent::PlaybackError("unplugged".into()));
    cx.run_until_parked();
    assert_eq!(
        e.reopens(),
        [
            (devices(None, None), true, false),
            (devices(None, None), false, true)
        ]
    );
    // A reopened microphone is no failure: the call goes on without a notice.
    let notice = r.session.read_with(cx, |s, _| s.file_notice());
    assert!(notice.is_none(), "{notice:?}");
}

#[gpui::test]
async fn a_failing_device_is_not_reopened_forever(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    for _ in 0..10 {
        e.say(CallEvent::CaptureError("broken".into()));
    }
    cx.run_until_parked();
    assert_eq!(e.reopens().len(), parla::call_manager::MAX_REOPENS);
    let notice = r.session.read_with(cx, |s, _| s.file_notice());
    assert!(matches!(notice, Some(components::FileNotice::Error(_))));
}

#[gpui::test]
async fn a_default_device_change_reopens_what_follows_the_default(cx: &mut TestAppContext) {
    let (dir, fake) = init_calls(cx, r#""input_device": "USB Mic""#, true);
    let engine = FakeEngine::default();
    let en = engine.clone();
    manager(cx).update(cx, |m, _| m.set_native_engine(Some(Arc::new(en))));
    let (provider, session) = session(cx, "w1", native_caps());
    huddle(&provider, "C1", false);
    cx.run_until_parked();
    join(&session, "C1", cx);
    assert_eq!(
        *engine.0.joined_with.lock().unwrap(),
        [devices(Some("USB Mic"), None)],
        "the join opens the chosen devices"
    );
    engine.say(CallEvent::Live);
    cx.run_until_parked();
    // AirPods connect: the default output moves; the chosen mic stays.
    manager(cx).update(cx, |m, cx| {
        m.audio_devices_changed(
            DeviceChange {
                default_input: true,
                default_output: true,
                list: false,
            },
            cx,
        )
    });
    assert_eq!(
        engine.reopens(),
        [(devices(Some("USB Mic"), None), false, true)]
    );
    let list = |cx: &mut TestAppContext| {
        manager(cx).update(cx, |m, cx| {
            m.audio_devices_changed(
                DeviceChange {
                    list: true,
                    ..Default::default()
                },
                cx,
            )
        });
        cx.run_until_parked();
    };
    // Another device plugged in: the chosen mic is still there, nothing
    // reopens.
    list(cx);
    assert_eq!(engine.reopens().len(), 1);
    // The chosen mic goes away, then comes back: each reopens the input.
    *engine.0.inputs.lock().unwrap() = Some(vec!["Built-in Microphone".into()]);
    list(cx);
    assert_eq!(
        engine.reopens().last(),
        Some(&(devices(Some("USB Mic"), None), true, false))
    );
    assert_eq!(engine.reopens().len(), 2);
    *engine.0.inputs.lock().unwrap() = None;
    list(cx);
    assert_eq!(engine.reopens().len(), 3);
    let _ = (dir, fake);
}

#[gpui::test]
async fn picking_a_device_saves_it_and_reopens_live(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    cx.run_until_parked();
    let menu = bar(cx)
        .unwrap()
        .devices
        .expect("native calls offer the picker");
    assert!(!menu.open);
    cx.update(CallManager::toggle_devices);
    cx.run_until_parked();
    let menu = bar(cx).unwrap().devices.unwrap();
    assert!(menu.open);
    assert_eq!(menu.inputs, ["Built-in Microphone", "USB Mic"]);
    assert_eq!(menu.outputs, ["Built-in Speakers"]);

    // Escape or a click outside closes it without a pick.
    cx.update(CallManager::close_devices);
    cx.run_until_parked();
    assert!(!bar(cx).unwrap().devices.unwrap().open);
    assert!(e.reopens().is_empty());
    cx.update(CallManager::toggle_devices);
    cx.run_until_parked();

    cx.update(|cx| CallManager::pick_input(Some("USB Mic".into()), cx));
    cx.run_until_parked();
    let menu = bar(cx).unwrap().devices.unwrap();
    assert!(!menu.open, "a pick closes the picker");
    assert_eq!(menu.input.as_deref(), Some("USB Mic"));
    assert_eq!(e.reopens(), [(devices(Some("USB Mic"), None), true, false)]);
    let file = std::fs::read_to_string(ConfigPaths::at(r._dir.path()).settings).unwrap();
    let v: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(v["calls"]["input_device"], "USB Mic");

    // Back to the default: saved as null.
    cx.update(|cx| CallManager::pick_output(None, cx));
    cx.run_until_parked();
    let file = std::fs::read_to_string(ConfigPaths::at(r._dir.path()).settings).unwrap();
    let v: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert!(v["calls"]["output_device"].is_null());
}

#[gpui::test]
async fn the_reopen_budget_survives_unrelated_device_changes(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    for _ in 0..parla::call_manager::MAX_REOPENS {
        e.say(CallEvent::CaptureError("broken".into()));
    }
    cx.run_until_parked();
    // A device plugged in that changes nothing for this call.
    manager(cx).update(cx, |m, cx| {
        m.audio_devices_changed(
            DeviceChange {
                list: true,
                ..Default::default()
            },
            cx,
        )
    });
    cx.run_until_parked();
    e.say(CallEvent::CaptureError("broken".into()));
    cx.run_until_parked();
    assert_eq!(
        e.reopens().len(),
        parla::call_manager::MAX_REOPENS,
        "budget kept"
    );
    // The default microphone really moved: a fresh budget, and a reopen.
    manager(cx).update(cx, |m, cx| {
        m.audio_devices_changed(
            DeviceChange {
                default_input: true,
                ..Default::default()
            },
            cx,
        )
    });
    e.say(CallEvent::CaptureError("broken".into()));
    cx.run_until_parked();
    assert_eq!(e.reopens().len(), parla::call_manager::MAX_REOPENS + 2);
}

#[gpui::test]
async fn the_spent_budget_notice_shows_once_per_side(cx: &mut TestAppContext) {
    use components::TimelineSource as _;
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    for _ in 0..=parla::call_manager::MAX_REOPENS {
        e.say(CallEvent::CaptureError("broken".into()));
    }
    cx.run_until_parked();
    assert!(r.session.read_with(cx, |s, _| s.file_notice()).is_some());
    r.session.update(cx, |s, cx| s.dismiss_file_notice(cx));
    cx.run_until_parked();
    for _ in 0..5 {
        e.say(CallEvent::CaptureError("broken".into()));
    }
    cx.run_until_parked();
    assert!(
        r.session.read_with(cx, |s, _| s.file_notice()).is_none(),
        "the microphone notice is not repeated"
    );
    // The speaker has its own notice.
    for _ in 0..=parla::call_manager::MAX_REOPENS {
        e.say(CallEvent::PlaybackError("broken".into()));
    }
    cx.run_until_parked();
    assert!(r.session.read_with(cx, |s, _| s.file_notice()).is_some());
}

#[gpui::test]
async fn a_settings_reload_applies_a_device_change_live(cx: &mut TestAppContext) {
    let (r, e) = native_setup(cx, "native", true);
    join(&r.session, "C1", cx);
    e.say(CallEvent::Live);
    cx.run_until_parked();
    let path = ConfigPaths::at(r._dir.path()).settings;
    std::fs::write(
        &path,
        r#"{ "locale": "en", "calls": { "join_with": "native", "output_device": "Headphones" } }"#,
    )
    .unwrap();
    cx.update(|cx| {
        cx.global_mut::<settings::SettingsStore>().reload();
    });
    cx.run_until_parked();
    assert_eq!(
        e.reopens(),
        [(devices(None, Some("Headphones")), false, true)]
    );
    assert_eq!(
        bar(cx).unwrap().devices.unwrap().output.as_deref(),
        Some("Headphones")
    );
}
