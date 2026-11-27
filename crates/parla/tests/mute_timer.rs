//! The timed mute ("Mute for 1 hour") ends by one timer at its stored
//! deadline: an expired deadline is cleared on load, the timer re-arms after
//! a change and is dropped when the deadline is cleared. No polling.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use components::NotifyControls as _;
use domain::test_support::{FakeProvider, cid, member, uid};
use domain::{ChatProvider, NotifyExtras, PersistOp};
use gpui::{AppContext as _, Entity, TestAppContext};
use jiff::Timestamp;
use parla::session::WorkspaceSession;
use store::{MemoryStore, Store};

const T: i64 = 1_794_736_800;

struct Clock(AtomicI64);

impl domain::Now for Clock {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

struct Rig {
    ws: settings::WorkspaceKey,
    store: Arc<MemoryStore>,
    clock: Arc<Clock>,
    session: Entity<WorkspaceSession>,
}

fn timed(until: i64) -> NotifyExtras {
    NotifyExtras {
        mute_until: Some(until),
        ..Default::default()
    }
}

async fn start(
    cx: &mut TestAppContext,
    store: Arc<MemoryStore>,
    at: i64,
    seed: Vec<(&str, NotifyExtras)>,
) -> Rig {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_member(member("u1", "Me"));
    let ws = provider.workspace().clone();
    for (c, extras) in seed {
        store
            .apply(
                &ws,
                PersistOp::LocalNotifyExtrasPut {
                    conversation: cid(c),
                    extras,
                },
            )
            .await
            .unwrap();
    }
    let clock = Arc::new(Clock(AtomicI64::new(at)));
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, clock.clone());
    });
    let p: Arc<dyn ChatProvider> = provider;
    let st: Arc<dyn Store> = store.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    Rig {
        ws,
        store,
        clock,
        session,
    }
}

fn armed(r: &Rig, cx: &mut TestAppContext) -> Option<i64> {
    r.session.read_with(cx, |s, _| s.mute_armed_at())
}

fn extras(r: &Rig, cx: &mut TestAppContext, c: &str) -> NotifyExtras {
    r.session.read_with(cx, |s, _| s.notify_extras(&cid(c)))
}

fn set(r: &Rig, cx: &mut TestAppContext, c: &str, e: NotifyExtras) {
    r.session
        .update(cx, |s, cx| s.set_notify_extras(cid(c), e, cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn an_expired_deadline_is_cleared_on_load_and_keeps_the_rest(cx: &mut TestAppContext) {
    let seed = NotifyExtras {
        sound_off: true,
        mute_until: Some(T - 10),
        ..Default::default()
    };
    let r = start(cx, Arc::default(), T, vec![("C1", seed)]).await;
    assert_eq!(
        extras(&r, cx, "C1"),
        NotifyExtras {
            sound_off: true,
            ..Default::default()
        }
    );
    assert_eq!(armed(&r, cx), None);
    // The cleared row is stored, so a restart does not see the old deadline.
    let stored = r.store.local_notify_extras(&r.ws).await.unwrap();
    assert_eq!(stored.get(&cid("C1")).unwrap().mute_until, None);
}

#[gpui::test]
async fn the_timer_rearms_after_a_change_and_fires_once(cx: &mut TestAppContext) {
    let r = start(cx, Arc::default(), T, vec![("C1", timed(T + 600))]).await;
    assert_eq!(armed(&r, cx), Some(T + 600));
    // An earlier deadline moves the timer to it.
    set(&r, cx, "C2", timed(T + 60));
    assert_eq!(armed(&r, cx), Some(T + 60));
    r.clock.0.store(T + 59, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(59));
    cx.run_until_parked();
    assert_eq!(extras(&r, cx, "C2").mute_until, Some(T + 60), "not early");
    r.clock.0.store(T + 60, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(extras(&r, cx, "C2"), NotifyExtras::default());
    assert_eq!(armed(&r, cx), Some(T + 600), "re-armed for the next one");
    r.clock.0.store(T + 600, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(540));
    cx.run_until_parked();
    assert_eq!(extras(&r, cx, "C1"), NotifyExtras::default());
    assert_eq!(armed(&r, cx), None);
    // Idle afterwards: more time passing changes nothing.
    cx.executor().advance_clock(Duration::from_secs(86_400));
    cx.run_until_parked();
    assert_eq!(armed(&r, cx), None);
}

#[gpui::test]
async fn clearing_the_deadline_drops_the_timer(cx: &mut TestAppContext) {
    let r = start(cx, Arc::default(), T, vec![]).await;
    assert_eq!(armed(&r, cx), None);
    set(&r, cx, "C1", timed(T + 3_600));
    assert_eq!(armed(&r, cx), Some(T + 3_600));
    set(&r, cx, "C1", NotifyExtras::default());
    assert_eq!(armed(&r, cx), None);
}
