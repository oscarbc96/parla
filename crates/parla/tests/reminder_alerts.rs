//! Local reminders raise one desktop notification when due: one timer for the
//! earliest, none when nothing is pending, and never twice (also after a
//! restart). No polling.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use components::RemindersSource;
use domain::notify::Notification;
use domain::test_support::{FakeProvider, member, uid};
use domain::{ChatProvider, Now, PersistOp, Reminder, ReminderId};
use gpui::{App, AppContext as _, Entity, TestAppContext};
use jiff::Timestamp;
use parla::notifications::{Notifications, Notifier};
use parla::session::WorkspaceSession;
use settings::WorkspaceKey;
use store::{MemoryStore, Store};

const T: i64 = 1_794_736_800;

struct Clock(AtomicI64);

impl Now for Clock {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

#[derive(Clone, Default)]
struct Shown(Rc<RefCell<Vec<String>>>);

impl Notifier for Shown {
    fn show(&self, n: &Notification, _: &mut App) {
        self.0.borrow_mut().push(n.text.to_string());
    }
    fn dismiss_conversation(&self, _: &WorkspaceKey, _: &domain::ConversationId, _: &mut App) {}
    fn set_badge(&self, _: Option<u32>, _: &mut App) {}
}

struct Rig {
    ws: WorkspaceKey,
    store: Arc<MemoryStore>,
    clock: Arc<Clock>,
    shown: Shown,
    session: Entity<WorkspaceSession>,
}

fn rem(id: &str, due: i64) -> Reminder {
    Reminder {
        id: ReminderId(Arc::from(id)),
        text: Arc::from(format!("text {id}")),
        due: Some(due),
        recurring: false,
        completed: false,
        fired: false,
        interval_secs: 0,
        for_user: uid("u1"),
        link: None,
        snoozed_until: None,
    }
}

async fn start(
    cx: &mut TestAppContext,
    store: Arc<MemoryStore>,
    clock: Arc<Clock>,
    seed: Vec<Reminder>,
) -> Rig {
    start_with(cx, store, clock, seed, None).await
}

async fn start_with(
    cx: &mut TestAppContext,
    store: Arc<MemoryStore>,
    clock: Arc<Clock>,
    seed: Vec<Reminder>,
    settings_json: Option<&str>,
) -> Rig {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(
        domain::Capabilities {
            dnd: true,
            ..Default::default()
        },
    ));
    provider.add_member(member("u1", "Me"));
    let ws = provider.workspace().clone();
    for r in seed {
        store
            .apply(&ws, PersistOp::LocalReminderPut(r))
            .await
            .unwrap();
    }
    let shown = Shown::default();
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, clock.clone());
        Notifications::new(Rc::new(shown.clone())).install(cx);
        if let Some(json) = settings_json {
            let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
            let paths = settings::ConfigPaths::at(dir.path());
            paths.ensure_exists().unwrap();
            std::fs::write(&paths.settings, json).unwrap();
            cx.set_global(settings::SettingsStore::load(paths));
        }
    });
    let p: Arc<dyn ChatProvider> = provider;
    let st: Arc<dyn Store> = store.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    Rig {
        ws,
        store,
        clock,
        shown,
        session,
    }
}

fn clock(at: i64) -> Arc<Clock> {
    Arc::new(Clock(AtomicI64::new(at)))
}

fn armed(r: &Rig, cx: &mut TestAppContext) -> Option<i64> {
    r.session.read_with(cx, |s, _| s.reminder_armed_at())
}

async fn fired_in_store(r: &Rig, cx: &mut TestAppContext) -> Vec<(String, bool)> {
    let _ = cx;
    r.store
        .local_reminders(&r.ws)
        .await
        .unwrap()
        .into_iter()
        .map(|x| (x.id.0.to_string(), x.fired))
        .collect()
}

#[gpui::test]
async fn nothing_pending_arms_nothing(cx: &mut TestAppContext) {
    let r = start(cx, Arc::default(), clock(T), vec![]).await;
    assert_eq!(armed(&r, cx), None);
    assert!(r.shown.0.borrow().is_empty());
}

#[gpui::test]
async fn one_timer_for_the_earliest_then_the_next(cx: &mut TestAppContext) {
    let r = start(
        cx,
        Arc::default(),
        clock(T),
        vec![rem("b", T + 200), rem("a", T + 50)],
    )
    .await;
    assert_eq!(armed(&r, cx), Some(T + 50));
    r.clock.0.store(T + 49, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(49));
    cx.run_until_parked();
    assert!(r.shown.0.borrow().is_empty());
    r.clock.0.store(T + 50, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(*r.shown.0.borrow(), ["text a"]);
    assert_eq!(armed(&r, cx), Some(T + 200));
    r.clock.0.store(T + 200, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(150));
    cx.run_until_parked();
    assert_eq!(*r.shown.0.borrow(), ["text a", "text b"]);
    assert_eq!(armed(&r, cx), None);
    // Idle afterwards: more time passing alerts nothing.
    cx.executor().advance_clock(Duration::from_secs(86_400));
    cx.run_until_parked();
    assert_eq!(r.shown.0.borrow().len(), 2);
}

#[gpui::test]
async fn past_due_at_startup_alerts_once_and_not_after_a_restart(cx: &mut TestAppContext) {
    let store = Arc::new(MemoryStore::default());
    let r = start(cx, store.clone(), clock(T), vec![rem("a", T - 60)]).await;
    assert_eq!(*r.shown.0.borrow(), ["text a"]);
    assert_eq!(armed(&r, cx), None);
    assert_eq!(fired_in_store(&r, cx).await, [("a".to_string(), true)]);
    // A restart over the same store does not alert again.
    let r2 = start(cx, store, clock(T + 10), vec![]).await;
    assert!(r2.shown.0.borrow().is_empty());
    assert_eq!(armed(&r2, cx), None);
}

#[gpui::test]
async fn adding_and_completing_rearm_the_timer(cx: &mut TestAppContext) {
    let r = start(cx, Arc::default(), clock(T), vec![]).await;
    r.session.update(cx, |s, cx| {
        RemindersSource::add(s, "stretch".into(), "in 20 minutes".into(), cx)
    });
    cx.run_until_parked();
    assert_eq!(armed(&r, cx), Some(T + 20 * 60));
    let id = r
        .session
        .read_with(cx, |s, _| s.reminders().unwrap()[0].id.clone());
    r.session
        .update(cx, |s, cx| RemindersSource::complete(s, id, cx));
    cx.run_until_parked();
    assert_eq!(armed(&r, cx), None, "a completed reminder never alerts");
}

#[gpui::test]
async fn a_pause_holds_the_alert_and_delivers_it_once_when_it_ends(cx: &mut TestAppContext) {
    // The fake provider has no clock: a 30 minute pause ends at second 1800.
    let r = start(cx, Arc::default(), clock(1_000), vec![rem("a", 1_060)]).await;
    r.session
        .update(cx, |s, cx| components::NotifyControls::pause(s, 30, cx));
    cx.run_until_parked();
    r.clock.0.store(1_060, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert!(r.shown.0.borrow().is_empty(), "paused: nothing is shown");
    assert_eq!(fired_in_store(&r, cx).await, [("a".to_string(), false)]);
    // The pause ends: the alert comes once, with no further timers.
    r.clock.0.store(1_800, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(740));
    cx.run_until_parked();
    assert_eq!(*r.shown.0.borrow(), ["text a"]);
    assert_eq!(fired_in_store(&r, cx).await, [("a".to_string(), true)]);
    cx.executor().advance_clock(Duration::from_secs(86_400));
    cx.run_until_parked();
    assert_eq!(r.shown.0.borrow().len(), 1);
}

#[gpui::test]
async fn resuming_early_delivers_the_held_alert(cx: &mut TestAppContext) {
    let r = start(cx, Arc::default(), clock(1_000), vec![rem("a", 1_060)]).await;
    r.session
        .update(cx, |s, cx| components::NotifyControls::pause(s, 30, cx));
    cx.run_until_parked();
    r.clock.0.store(1_120, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    assert!(r.shown.0.borrow().is_empty());
    r.session
        .update(cx, |s, cx| components::NotifyControls::resume(s, cx));
    cx.run_until_parked();
    assert_eq!(*r.shown.0.borrow(), ["text a"]);
}

#[gpui::test]
async fn desktop_notifications_off_silence_the_alert(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, r#"{"notifications": {"desktop": "none"}}"#).unwrap();
    let r = start(cx, Arc::default(), clock(T), vec![]).await;
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    r.session.update(cx, |s, cx| {
        RemindersSource::add(s, "quiet".into(), "in 1 minutes".into(), cx)
    });
    cx.run_until_parked();
    r.clock.0.store(T + 60, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert!(r.shown.0.borrow().is_empty());
}

#[gpui::test]
async fn clicking_the_reminder_alert_opens_the_reminders_view(cx: &mut TestAppContext) {
    let r = start(cx, Arc::default(), clock(T), vec![rem("a", T - 60)]).await;
    assert_eq!(*r.shown.0.borrow(), ["text a"]);
    let opened = Rc::new(RefCell::new(0));
    let o = opened.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(
            &r.session,
            move |_, ev: &parla::session::SessionEvent, _| {
                if matches!(ev, parla::session::SessionEvent::ShowReminders) {
                    *o.borrow_mut() += 1;
                }
            },
        )
    });
    let tag = format!("{}/reminder:a/a", r.ws);
    assert!(cx.update(|cx| Notifications::handle_response(&tag, cx)));
    cx.run_until_parked();
    assert_eq!(*opened.borrow(), 1);
}

#[gpui::test]
async fn a_reminder_alert_hides_its_text_when_the_preview_is_off(cx: &mut TestAppContext) {
    let r = start_with(
        cx,
        Arc::default(),
        clock(T),
        vec![rem("a", T + 50)],
        Some(r#"{ "notifications": { "show_preview": false } }"#),
    )
    .await;
    r.clock.0.store(T + 50, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(50));
    cx.run_until_parked();
    assert_eq!(*r.shown.0.borrow(), [""], "alert shown, with no text");
}
