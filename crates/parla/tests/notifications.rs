//! Notification wiring against `FakeProvider` and a recording notifier; no
//! real banners.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

mod support;

use components::NotifyControls;
use domain::notify::{Notification, SessionExpiredNotice};
use domain::test_support::Call as ProviderCallRecord;
use domain::test_support::{FakeProvider, cid, conv, member, msg, ts, uid};
use domain::{
    Capabilities, ChatProvider, ConversationCounts, ConversationId, ConversationKind, Message,
    ProviderEvent,
};
use gpui::{App, AppContext as _, Entity, TestAppContext};
use parla::notifications::{
    GpuiNotifier, NoopNotifier, Notifications, Notifier, session_expired_body, session_expired_tag,
    session_expired_title, tag,
};
use parla::session::WorkspaceSession;
use settings::WorkspaceKey;
use store::{MemoryStore, Store};

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Show(String),
    ShowExpired(WorkspaceKey),
    Dismiss(ConversationId),
    DismissTag(String),
    Badge(Option<u32>),
}

#[derive(Clone, Default)]
struct FakeNotifier(Rc<RefCell<Vec<Call>>>);

impl Notifier for FakeNotifier {
    fn show(&self, n: &Notification, _: &mut App) {
        self.0.borrow_mut().push(Call::Show(tag(n)));
    }
    fn show_session_expired(&self, n: &SessionExpiredNotice, _: &mut App) {
        self.0
            .borrow_mut()
            .push(Call::ShowExpired(n.workspace.clone()));
    }
    fn dismiss_conversation(&self, _: &WorkspaceKey, c: &ConversationId, _: &mut App) {
        self.0.borrow_mut().push(Call::Dismiss(c.clone()));
    }
    fn dismiss_tag(&self, tag: &str, _: &mut App) {
        self.0.borrow_mut().push(Call::DismissTag(tag.into()));
    }
    fn set_badge(&self, count: Option<u32>, _: &mut App) {
        self.0.borrow_mut().push(Call::Badge(count));
    }
}

impl FakeNotifier {
    fn shows(&self) -> usize {
        self.0
            .borrow()
            .iter()
            .filter(|c| matches!(c, Call::Show(_)))
            .count()
    }
    fn expired_shows(&self) -> usize {
        self.0
            .borrow()
            .iter()
            .filter(|c| matches!(c, Call::ShowExpired(_)))
            .count()
    }
    fn last_badge(&self) -> Option<Option<u32>> {
        self.0.borrow().iter().rev().find_map(|c| match c {
            Call::Badge(b) => Some(*b),
            _ => None,
        })
    }
}

struct Fixture {
    provider: Arc<FakeProvider>,
    session: Entity<WorkspaceSession>,
    fake: FakeNotifier,
}

fn setup(cx: &mut TestAppContext) -> Fixture {
    setup_with(cx, Capabilities::default(), None)
}

/// A session with `caps`; `store` is shared with an earlier session when given.
fn setup_with(
    cx: &mut TestAppContext,
    caps: Capabilities,
    store: Option<Arc<dyn Store>>,
) -> Fixture {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(caps));
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    provider.add_conversation(dm);
    let mut dm2 = conv("D2", "dm2");
    dm2.kind = ConversationKind::Direct;
    provider.add_conversation(dm2);
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Other"));
    let fake = FakeNotifier::default();
    cx.update(|cx| Notifications::new(Rc::new(fake.clone())).install(cx));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = store.unwrap_or_else(|| Arc::new(MemoryStore::default()));
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    Fixture {
        provider,
        session,
        fake,
    }
}

fn post(f: &Fixture, cx: &mut TestAppContext, m: Message) {
    f.provider.push(ProviderEvent::MessagePosted(m));
    cx.run_until_parked();
}

#[gpui::test]
async fn live_mention_notifies_once(cx: &mut TestAppContext) {
    let f = setup(cx);
    post(&f, cx, msg("D1", "2.000000", "hello"));
    assert_eq!(f.fake.shows(), 1);
}

/// A second workspace sharing the same `Notifications` global. The session is
/// returned so the caller keeps the entity (and its event subscription) alive.
fn add_session(cx: &mut TestAppContext, id: &str) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let provider = Arc::new(FakeProvider::new(id, uid("u-self")));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, Arc::new(MemoryStore::default()), cx));
    cx.run_until_parked();
    (provider, session)
}

#[gpui::test]
async fn session_expired_notifies_once_until_signed_in_again(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider.push(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert_eq!(f.fake.expired_shows(), 1);

    // The same expiry repeated by the provider does not post again.
    f.provider.push(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert_eq!(f.fake.expired_shows(), 1);

    // Signing this workspace back in releases the latch: a later expiry posts.
    let ws = WorkspaceKey::new("fake", "w1");
    cx.update(|cx| {
        let hub = cx.global::<Notifications>().clone();
        hub.signed_in(&ws, cx)
    });
    f.provider.push(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert_eq!(f.fake.expired_shows(), 2);
}

#[gpui::test]
async fn signed_in_dismisses_the_delivered_expired_notification(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider.push(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    let ws = WorkspaceKey::new("fake", "w1");
    cx.update(|cx| {
        let hub = cx.global::<Notifications>().clone();
        hub.signed_in(&ws, cx)
    });
    assert!(
        f.fake
            .0
            .borrow()
            .contains(&Call::DismissTag(session_expired_tag(&ws)))
    );
}

#[gpui::test]
async fn session_expired_click_opens_sign_in_for_that_workspace(cx: &mut TestAppContext) {
    let f = setup(cx);
    let (other, _other_session) = add_session(cx, "w2");
    let opened: Rc<RefCell<Vec<WorkspaceKey>>> = Rc::default();
    cx.update(|cx| {
        let hub = cx.global::<Notifications>().clone();
        let seen = opened.clone();
        hub.set_sign_in_opener(Rc::new(move |ws, _| seen.borrow_mut().push(ws.clone())));
    });

    other.push(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert_eq!(f.fake.expired_shows(), 1);

    let ws = WorkspaceKey::new("fake", "w2");
    let tag = session_expired_tag(&ws);
    assert!(cx.update(|cx| Notifications::handle_response(&tag, cx)));
    assert_eq!(*opened.borrow(), vec![ws]);

    // A tag of a workspace whose session did not expire is not a click we own.
    let other_tag = session_expired_tag(&WorkspaceKey::new("fake", "w1"));
    assert!(!cx.update(|cx| Notifications::handle_response(&other_tag, cx)));
    assert_eq!(opened.borrow().len(), 1);
}

#[test]
fn session_expired_notification_is_localised() {
    let n = SessionExpiredNotice {
        workspace: WorkspaceKey::new("fake", "w1"),
        workspace_name: Arc::from("Acme"),
    };
    assert_eq!(
        session_expired_title(&n, &localizer("en")),
        "Sign in to Acme again"
    );
    assert_eq!(
        session_expired_title(&n, &localizer("es")),
        "Inicia sesión en Acme de nuevo"
    );
    assert_eq!(
        session_expired_title(&n, &localizer("ca")),
        "Torna a iniciar la sessió a Acme"
    );
    assert_eq!(
        session_expired_body(&localizer("en")),
        "Your session ended. Click to sign in."
    );
    assert_eq!(
        session_expired_body(&localizer("es")),
        "Tu sesión terminó. Haz clic para iniciar sesión."
    );
    assert_eq!(
        session_expired_body(&localizer("ca")),
        "La teva sessió ha acabat. Fes clic per iniciar la sessió."
    );
}

#[gpui::test]
async fn focused_conversation_does_not_notify(cx: &mut TestAppContext) {
    let f = setup(cx);
    let w = cx.add_empty_window();
    w.update(|window, _| window.activate_window());
    f.session.update(cx, |s, cx| s.open(cid("D1"), cx));
    cx.run_until_parked();
    post(&f, cx, msg("D1", "2.000000", "hello"));
    assert_eq!(f.fake.shows(), 0);
    post(&f, cx, msg("D2", "2.000000", "elsewhere"));
    assert_eq!(f.fake.shows(), 1);
}

#[gpui::test]
async fn catch_up_never_notifies(cx: &mut TestAppContext) {
    let f = setup(cx);
    for i in 0..50 {
        f.provider
            .add_message(msg("D1", &format!("3.{i:06}"), "missed"));
    }
    f.provider.set_counts(vec![ConversationCounts {
        conversation: cid("D1"),
        latest: Some(ts("3.000049")),
        last_read: None,
        unread: true,
        mentions: 50,
    }]);
    f.provider
        .push(ProviderEvent::Connected { reconnect: true });
    cx.run_until_parked();
    assert_eq!(f.fake.shows(), 0);
    assert_eq!(f.fake.last_badge(), Some(Some(50)));
}

#[gpui::test]
async fn newer_replaces_older_per_conversation(cx: &mut TestAppContext) {
    let f = setup(cx);
    post(&f, cx, msg("D1", "2.000000", "one"));
    post(&f, cx, msg("D1", "3.000000", "two"));
    // The registry keeps only the newest per conversation; the real notifier
    // dismisses the older tag when a newer one arrives.
    let tags: Vec<String> = f
        .fake
        .0
        .borrow()
        .iter()
        .filter_map(|c| match c {
            Call::Show(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tags.len(), 2);
    assert_ne!(tags[0], tags[1]);
}

fn sample(ws: &WorkspaceKey, c: &str, t: &str) -> Notification {
    Notification {
        workspace: ws.clone(),
        conversation: cid(c),
        message: domain::test_support::mid(c, t),
        thread: None,
        author: "t".into(),
        channel: None,
        text: "b".into(),
        file_count: 0,
        silent: false,
    }
}

#[gpui::test]
async fn click_opens_conversation_and_thread(cx: &mut TestAppContext) {
    let f = setup(cx);
    let mut reply = msg("D1", "5.000000", "in thread");
    reply.thread_root = Some(ts("4.000000"));
    post(&f, cx, reply.clone());
    assert_eq!(f.fake.shows(), 1);
    let t = tag(&Notification {
        workspace: WorkspaceKey::new("fake", "w1"),
        conversation: cid("D1"),
        message: reply.id,
        thread: None,
        author: "".into(),
        channel: None,
        text: "".into(),
        file_count: 0,
        silent: false,
    });
    let handled = cx.update(|cx| Notifications::handle_response(&t, cx));
    assert!(handled);
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        assert_eq!(s.selected(), Some(&cid("D1")));
        assert!(
            s.state()
                .threads
                .contains_key(&domain::test_support::mid("D1", "4.000000"))
        );
    });
    assert!(!cx.update(|cx| Notifications::handle_response("nope", cx)));
}

#[gpui::test]
async fn read_elsewhere_dismisses(cx: &mut TestAppContext) {
    let f = setup(cx);
    post(&f, cx, msg("D1", "2.000000", "hello"));
    f.provider.push(ProviderEvent::ReadMarker {
        conversation: cid("D1"),
        ts: ts("2.000000"),
    });
    cx.run_until_parked();
    assert!(f.fake.0.borrow().contains(&Call::Dismiss(cid("D1"))));
}

#[gpui::test]
async fn badge_tracks_mentions(cx: &mut TestAppContext) {
    let f = setup(cx);
    post(&f, cx, msg("D1", "2.000000", "a"));
    post(&f, cx, msg("D2", "2.000000", "b"));
    assert_eq!(f.fake.last_badge(), Some(Some(2)));
    f.provider.push(ProviderEvent::ReadMarker {
        conversation: cid("D1"),
        ts: ts("2.000000"),
    });
    f.provider.push(ProviderEvent::ReadMarker {
        conversation: cid("D2"),
        ts: ts("2.000000"),
    });
    cx.run_until_parked();
    assert_eq!(f.fake.last_badge(), Some(None));
}

#[gpui::test]
async fn unbundled_is_noop_without_panic(cx: &mut TestAppContext) {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    provider.add_conversation(dm);
    provider.add_member(member("u2", "Other"));
    cx.update(|cx| Notifications::new(parla::notifications::notifier_for(false)).install(cx));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let _s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    provider.push(ProviderEvent::MessagePosted(msg("D1", "2.000000", "x")));
    cx.run_until_parked();
    let _ = NoopNotifier;
}

fn localizer(code: &str) -> i18n::Localizer {
    support::localizer(code, &[parla::catalog()])
}

fn notification(channel: Option<&str>, text: &str, file_count: usize) -> Notification {
    Notification {
        author: "Ann".into(),
        channel: channel.map(Into::into),
        text: text.into(),
        file_count,
        ..sample(&WorkspaceKey::new("fake", "w1"), "C1", "1.000000")
    }
}

#[test]
fn hidden_preview_leaves_the_body_empty() {
    let l = localizer("en");
    // No text and no files (what `show_preview: false` produces): no body.
    assert_eq!(
        parla::notifications::body(&notification(Some("gen"), "", 0), &l),
        ""
    );
    // A file-only message still counts its files.
    assert_ne!(
        parla::notifications::body(&notification(Some("gen"), "", 2), &l),
        ""
    );
}

#[test]
fn notification_title_localised() {
    let n = notification(Some("gen"), "hi", 0);
    assert_eq!(
        parla::notifications::title(&n, &localizer("en")),
        "Ann in #gen"
    );
    assert_eq!(
        parla::notifications::title(&n, &localizer("es")),
        "Ann en #gen"
    );
    assert_eq!(
        parla::notifications::title(&n, &localizer("ca")),
        "Ann a #gen"
    );
    // A direct conversation shows the author alone, whatever the language.
    let dm = notification(None, "hi", 0);
    assert_eq!(parla::notifications::title(&dm, &localizer("ca")), "Ann");
}

#[test]
fn notification_body_is_text_or_localised_file_count() {
    let text = notification(None, "hello", 0);
    assert_eq!(parla::notifications::body(&text, &localizer("ca")), "hello");
    let one = notification(None, "", 1);
    assert_eq!(parla::notifications::body(&one, &localizer("en")), "1 file");
    assert_eq!(
        parla::notifications::body(&one, &localizer("es")),
        "1 archivo"
    );
    let many = notification(None, "", 3);
    assert_eq!(
        parla::notifications::body(&many, &localizer("en")),
        "3 files"
    );
    assert_eq!(
        parla::notifications::body(&many, &localizer("ca")),
        "3 fitxers"
    );
}

// pause notifications and per-conversation levels.

fn caps(dnd: bool, notify_prefs: bool) -> Capabilities {
    Capabilities {
        dnd,
        notify_prefs,
        ..Default::default()
    }
}

fn nothing() -> domain::ConversationPrefs {
    domain::ConversationPrefs {
        level: domain::NotifyLevel::Nothing,
        muted: false,
    }
}

/// The fake provider has no clock: its snooze ends at `minutes * 60`, so the
/// tests run near the epoch.
fn clock(cx: &mut TestAppContext, secs: i64) {
    cx.update(|cx| {
        components::set_now(
            cx,
            Arc::new(domain::FixedNow(
                jiff::Timestamp::from_second(secs).unwrap(),
            )),
        )
    });
}

#[gpui::test]
async fn snoozed_workspace_does_not_notify(cx: &mut TestAppContext) {
    clock(cx, 1_000);
    let f = setup_with(cx, caps(true, false), None);
    post(&f, cx, msg("D1", "2.000000", "before"));
    assert_eq!(f.fake.shows(), 1);
    f.session.update(cx, |s, cx| s.pause(30, cx));
    cx.run_until_parked();
    assert!(f.provider.calls().contains(&ProviderCallRecord::Snooze(30)));
    post(&f, cx, msg("D2", "3.000000", "during the pause"));
    assert_eq!(f.fake.shows(), 1, "paused: nothing is shown");
    f.session.update(cx, |s, cx| s.resume(cx));
    cx.run_until_parked();
    assert!(f.provider.calls().contains(&ProviderCallRecord::EndSnooze));
    post(&f, cx, msg("D2", "4.000000", "after"));
    assert_eq!(f.fake.shows(), 2, "resumed");
}

#[gpui::test]
async fn level_nothing_conversation_does_not_notify(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.update(cx, |s, cx| {
        s.set_conversation_prefs(cid("D1"), nothing(), cx)
    });
    cx.run_until_parked();
    post(&f, cx, msg("D1", "2.000000", "silenced"));
    assert_eq!(f.fake.shows(), 0);
    post(&f, cx, msg("D2", "3.000000", "other conversation"));
    assert_eq!(f.fake.shows(), 1);
}

#[gpui::test]
async fn levels_stay_local_without_the_provider_capability(cx: &mut TestAppContext) {
    let f = setup_with(cx, caps(true, false), None);
    f.session.update(cx, |s, cx| {
        s.set_conversation_prefs(cid("D1"), nothing(), cx)
    });
    cx.run_until_parked();
    // The provider keeps no levels: its (unverified) call is never made.
    assert!(
        !f.provider
            .calls()
            .iter()
            .any(|c| matches!(c, ProviderCallRecord::SetNotifyPrefs { .. })),
        "{:?}",
        f.provider.calls()
    );
    f.session.read_with(cx, |s, _| {
        assert_eq!(s.conversation_prefs(&cid("D1")), nothing());
        assert!(s.state().local_notify_prefs.contains_key(&cid("D1")));
    });
}

#[gpui::test]
async fn levels_go_to_the_provider_when_it_keeps_them(cx: &mut TestAppContext) {
    let f = setup_with(cx, caps(true, true), None);
    f.session.update(cx, |s, cx| {
        s.set_conversation_prefs(cid("D1"), nothing(), cx)
    });
    cx.run_until_parked();
    assert!(f.provider.calls().iter().any(|c| matches!(
        c,
        ProviderCallRecord::SetNotifyPrefs { conversation, .. } if *conversation == cid("D1")
    )));
    f.session.read_with(cx, |s, _| {
        assert!(s.state().local_notify_prefs.is_empty());
        assert_eq!(s.conversation_prefs(&cid("D1")), nothing());
    });
}

#[gpui::test]
async fn local_levels_survive_a_restart(cx: &mut TestAppContext) {
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let f = setup_with(cx, Capabilities::default(), Some(store.clone()));
    f.session.update(cx, |s, cx| {
        s.set_conversation_prefs(cid("D1"), nothing(), cx)
    });
    cx.run_until_parked();
    // A new session over the same store starts with the level.
    let g = setup_with(cx, Capabilities::default(), Some(store));
    g.session.read_with(cx, |s, _| {
        assert_eq!(s.conversation_prefs(&cid("D1")), nothing());
    });
    post(&g, cx, msg("D1", "2.000000", "still silenced"));
    assert_eq!(g.fake.shows(), 0);
}

#[gpui::test]
async fn pause_controls_follow_the_capability(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.read_with(cx, |s, _| assert!(!s.can_pause()));
    let g = setup_with(cx, caps(true, false), None);
    g.session.read_with(cx, |s, _| assert!(s.can_pause()));
}

#[gpui::test]
async fn paused_label_and_the_wake_when_the_pause_ends(cx: &mut TestAppContext) {
    use components::SidebarSource as _;
    cx.update(components::init);
    clock(cx, 1_000);
    let f = setup_with(cx, caps(true, false), None);
    assert_eq!(f.session.read_with(cx, |s, cx| s.paused_label(cx)), None);
    f.session.update(cx, |s, cx| s.pause(30, cx));
    cx.run_until_parked();
    let label = f.session.read_with(cx, |s, cx| s.paused_label(cx));
    assert!(
        label
            .as_deref()
            .is_some_and(|l| l.starts_with("Notifications paused until")),
        "{label:?}"
    );
    // One timer wakes the views when the pause ends; the clock moving alone does not.
    let seen = Rc::new(RefCell::new(0));
    let s2 = seen.clone();
    cx.update(|cx| {
        cx.observe(&f.session, move |_, _| *s2.borrow_mut() += 1)
            .detach();
    });
    clock(cx, 1_801);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31 * 60));
    cx.run_until_parked();
    assert!(
        *seen.borrow() >= 1,
        "the session notified when the pause ended"
    );
    assert_eq!(f.session.read_with(cx, |s, cx| s.paused_label(cx)), None);
}

#[gpui::test]
async fn forget_clears_mentions_and_delivered(cx: &mut TestAppContext) {
    let f = setup(cx);
    post(&f, cx, msg("D1", "2.000000", "a"));
    assert_eq!(f.fake.last_badge(), Some(Some(1)));
    let ws = WorkspaceKey::new("fake", "w1");
    cx.update(|cx| cx.global::<Notifications>().clone().forget(&ws, cx));
    assert_eq!(f.fake.last_badge(), Some(None));
    assert!(f.fake.0.borrow().contains(&Call::Dismiss(cid("D1"))));
}

#[derive(Clone, Default)]
struct DockPort(Rc<RefCell<Vec<bool>>>);

impl parla::tray::TrayPort for DockPort {
    fn show(&self, _: &parla::tray::TrayView, _: &mut App) {}
    fn remove(&self) {}
    fn hide_app(&self, _: &mut App) {}
    fn set_dock(&self, visible: bool) {
        self.0.borrow_mut().push(visible);
    }
}

#[gpui::test]
async fn click_restores_the_dock_icon_hidden_with_the_window(cx: &mut TestAppContext) {
    let f = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{ "locale": "en", "dock_icon": "with_window" }"#,
    )
    .unwrap();
    let paths = settings::ConfigPaths::at(dir.path());
    cx.update(|cx| parla::init_without_watcher(cx, paths));
    let port = DockPort::default();
    cx.update(|cx| parla::tray::Tray::new(Rc::new(port.clone())).install(cx));
    let w = cx.add_window(|_, _| gpui::Empty);
    let mut vcx = gpui::VisualTestContext::from_window(w.into(), cx);
    vcx.update(|window, cx| parla::tray::Tray::guard_window(window, cx));
    assert!(!vcx.simulate_close());
    assert_eq!(*port.0.borrow(), vec![false]);
    post(&f, cx, msg("D1", "7.000000", "hello"));
    let t = tag(&Notification {
        workspace: WorkspaceKey::new("fake", "w1"),
        conversation: cid("D1"),
        message: msg("D1", "7.000000", "hello").id,
        thread: None,
        author: "".into(),
        channel: None,
        text: "".into(),
        file_count: 0,
        silent: false,
    });
    assert!(cx.update(|cx| Notifications::handle_response(&t, cx)));
    assert_eq!(*port.0.borrow(), vec![false, true]);
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn bare_binary_cannot_notify(cx: &mut TestAppContext) {
    // The test process has no bundle identifier, like `cargo run`.
    cx.update(|_| assert!(!parla::notifications::can_notify()));
}

fn notif(conv: &str, ts_s: &str, thread: Option<&str>) -> Notification {
    Notification {
        workspace: WorkspaceKey::new("fake", "w1"),
        conversation: cid(conv),
        message: msg(conv, ts_s, "hello").id,
        thread: thread.map(ts),
        author: "Other".into(),
        channel: None,
        text: "hello".into(),
        file_count: 0,
        silent: false,
    }
}

fn deliver(cx: &mut TestAppContext, n: &Notification) {
    cx.update(|cx| cx.global::<Notifications>().clone().show(n.clone(), cx));
}

fn act(cx: &mut TestAppContext, n: &Notification, action: &str, text: Option<&str>) -> bool {
    let t = tag(n);
    cx.update(|cx| Notifications::handle_action(&t, Some(action), text.map(str::to_string), cx))
}

fn connect(f: &Fixture, cx: &mut TestAppContext) {
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
}

fn calls(f: &Fixture) -> Vec<ProviderCallRecord> {
    f.provider.calls()
}

fn system_actions(cx: &mut TestAppContext, n: &Notification) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), r#"{ "locale": "en" }"#).unwrap();
    let paths = settings::ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_app_identity("com.justparla.parla", "Parla");
        GpuiNotifier::new().show(n, cx)
    });
    cx.delivered_system_notifications()
        .into_iter()
        .rev()
        .find(|d| d.tag.as_ref() == tag(n))
        .map(|d| d.actions.iter().map(|a| a.id.to_string()).collect())
        .unwrap_or_default()
}

#[gpui::test]
async fn message_notification_carries_reply_and_mark_read(cx: &mut TestAppContext) {
    let n = notif("D1", "5.000000", None);
    assert_eq!(system_actions(cx, &n), ["reply", "mark-read"]);
}

#[gpui::test]
async fn reminder_notification_has_no_actions(cx: &mut TestAppContext) {
    let mut n = notif("D1", "5.000000", None);
    n.conversation = cid("reminder:r1");
    assert!(system_actions(cx, &n).is_empty());
}

#[gpui::test]
async fn missed_huddle_notification_has_no_actions(cx: &mut TestAppContext) {
    let n = notif("D1", "missed-H1", None);
    assert!(system_actions(cx, &n).is_empty());
}

#[gpui::test]
async fn reply_with_text_sends_to_the_conversation(cx: &mut TestAppContext) {
    let f = setup(cx);
    connect(&f, cx);
    let n = notif("D1", "5.000000", None);
    deliver(cx, &n);
    assert!(act(cx, &n, "reply", Some("on my way")));
    cx.run_until_parked();
    assert!(calls(&f).iter().any(|c| matches!(
        c,
        ProviderCallRecord::Send { conversation, text, thread: None, .. }
            if *conversation == cid("D1") && text == "on my way"
    )));
}

#[gpui::test]
async fn reply_to_a_thread_message_replies_in_the_thread(cx: &mut TestAppContext) {
    let f = setup(cx);
    connect(&f, cx);
    let n = notif("D1", "6.000000", Some("5.000000"));
    deliver(cx, &n);
    assert!(act(cx, &n, "reply", Some("thread answer")));
    cx.run_until_parked();
    assert!(calls(&f).iter().any(|c| matches!(
        c,
        ProviderCallRecord::Reply { conversation, root, text, .. }
            if *conversation == cid("D1") && *root == ts("5.000000") && text == "thread answer"
    )));
}

#[gpui::test]
async fn reply_without_text_sends_nothing(cx: &mut TestAppContext) {
    let f = setup(cx);
    let n = notif("D1", "5.000000", None);
    deliver(cx, &n);
    assert!(act(cx, &n, "reply", None));
    assert!(act(cx, &n, "reply", Some("   ")));
    cx.run_until_parked();
    assert!(!calls(&f).iter().any(|c| matches!(
        c,
        ProviderCallRecord::Send { .. } | ProviderCallRecord::Reply { .. }
    )));
}

#[gpui::test]
async fn mark_read_action_marks_the_conversation_read(cx: &mut TestAppContext) {
    let f = setup(cx);
    post(&f, cx, msg("D1", "5.000000", "hello"));
    let n = notif("D1", "5.000000", None);
    deliver(cx, &n);
    assert!(act(cx, &n, "mark-read", None));
    cx.run_until_parked();
    assert!(calls(&f).iter().any(|c| matches!(
        c,
        ProviderCallRecord::MarkRead { conversation, .. } if *conversation == cid("D1")
    )));
}

#[gpui::test]
async fn mark_read_action_on_a_thread_marks_the_thread_read(cx: &mut TestAppContext) {
    let f = setup(cx);
    let n = notif("D1", "6.000000", Some("5.000000"));
    deliver(cx, &n);
    assert!(act(cx, &n, "mark-read", None));
    cx.run_until_parked();
    assert!(calls(&f).iter().any(|c| matches!(
        c,
        ProviderCallRecord::MarkThreadRead { conversation, root, ts: t }
            if *conversation == cid("D1") && *root == ts("5.000000") && *t == ts("6.000000")
    )));
}

#[gpui::test]
async fn unknown_or_expired_action_is_ignored(cx: &mut TestAppContext) {
    let f = setup(cx);
    let n = notif("D1", "5.000000", None);
    // Never delivered: unknown id.
    assert!(!act(cx, &n, "reply", Some("hi")));
    deliver(cx, &n);
    // Replaced by a newer notification of the same conversation: expired id.
    let newer = notif("D1", "7.000000", None);
    deliver(cx, &newer);
    assert!(!act(cx, &n, "mark-read", None));
    // Unknown action id on a live tag.
    assert!(!act(cx, &newer, "bogus", None));
    cx.run_until_parked();
    assert!(calls(&f).is_empty());
}

#[derive(Clone, Default)]
struct FakeSound(Rc<RefCell<usize>>);

impl parla::notifications::NotificationSound for FakeSound {
    fn play(&self) {
        *self.0.borrow_mut() += 1;
    }
}

#[gpui::test]
async fn a_sounding_notification_plays_the_sound_once(cx: &mut TestAppContext) {
    let sound = FakeSound::default();
    let ws = WorkspaceKey::new("fake", "w1");
    cx.update(|cx| {
        Notifications::new(Rc::new(FakeNotifier::default()))
            .with_sound(Rc::new(sound.clone()))
            .install(cx);
        cx.global::<Notifications>()
            .clone()
            .show(sample(&ws, "D1", "1.000000"), cx);
    });
    assert_eq!(*sound.0.borrow(), 1);
}

#[gpui::test]
async fn a_silent_notification_plays_no_sound(cx: &mut TestAppContext) {
    let sound = FakeSound::default();
    let ws = WorkspaceKey::new("fake", "w1");
    cx.update(|cx| {
        Notifications::new(Rc::new(FakeNotifier::default()))
            .with_sound(Rc::new(sound.clone()))
            .install(cx);
        let mut n = sample(&ws, "D1", "1.000000");
        n.silent = true;
        cx.global::<Notifications>().clone().show(n, cx);
    });
    assert_eq!(*sound.0.borrow(), 0);
}
