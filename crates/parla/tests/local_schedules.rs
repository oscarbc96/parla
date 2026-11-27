//! The local schedule runner: one timer, one send, a prompt for late items.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use domain::test_support::{Call, FakeProvider, cid, uid};
use domain::{
    Capabilities, ChatProvider, LocalSchedule, LocalScheduleState, Now, PersistOp, ScheduleId,
    Upload,
};
use gpui::{AppContext as _, Entity, TestAppContext};
use jiff::Timestamp;
use parla::session::WorkspaceSession;
use store::{MemoryStore, Store};

const T: i64 = 1_794_736_800;

struct Clock(AtomicI64);

impl Now for Clock {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

fn sid(s: &str) -> ScheduleId {
    ScheduleId(Arc::from(s))
}

fn files() -> Vec<Upload> {
    vec![Upload {
        path: PathBuf::from("/tmp/parla-test-report.pdf"),
        name: Arc::from("report.pdf"),
        mime: Arc::from("application/pdf"),
        size: 10,
        filetype: None,
    }]
}

fn item(id: &str, at: i64, with_files: bool) -> LocalSchedule {
    LocalSchedule {
        id: sid(id),
        conversation: cid("C1"),
        thread: None,
        text: Arc::from("scheduled text"),
        files: if with_files { files() } else { vec![] },
        post_at: at,
        created_ms: 0,
        state: LocalScheduleState::Pending,
    }
}

struct Rig {
    provider: Arc<FakeProvider>,
    store: Arc<MemoryStore>,
    clock: Arc<Clock>,
    session: Entity<WorkspaceSession>,
}

/// A session over a store that already holds `items`, at unix time `T`.
async fn start(cx: &mut TestAppContext, items: Vec<LocalSchedule>) -> Rig {
    let clock = Arc::new(Clock(AtomicI64::new(T)));
    let c: Arc<dyn Now> = clock.clone();
    cx.update(|cx| components::set_now(cx, c));
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            scheduled_send: true,
            ..Default::default()
        }),
    );
    let store = Arc::new(MemoryStore::default());
    let ws = provider.workspace().clone();
    for l in items {
        store
            .apply(&ws, PersistOp::LocalSchedulePut(l))
            .await
            .expect("seed");
    }
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = store.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    Rig {
        provider,
        store,
        clock,
        session,
    }
}

fn uploads(r: &Rig) -> usize {
    r.provider
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Upload { .. }))
        .count()
}

async fn stored(r: &Rig) -> Vec<LocalSchedule> {
    let ws = r.provider.workspace().clone();
    r.store.local_schedules(&ws).await.unwrap()
}

#[gpui::test]
async fn due_local_sends_once_with_files(cx: &mut TestAppContext) {
    let r = start(cx, vec![item("l1", T - 10, true)]).await;
    assert_eq!(uploads(&r), 1, "due at startup, inside the grace period");
    let calls = r.provider.calls();
    match calls.iter().find(|c| matches!(c, Call::Upload { .. })) {
        Some(Call::Upload { files, text, .. }) => {
            assert_eq!(files.len(), 1);
            assert_eq!(text.as_deref(), Some("scheduled text"));
        }
        other => panic!("no upload: {other:?}"),
    }
    assert!(stored(&r).await.is_empty(), "removed once sent");
    // Another tick and a long wait never send it again.
    r.session.update(cx, |s, cx| s.local_tick(cx));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(3600));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 1);
}

#[gpui::test]
async fn text_only_local_item_sends_as_a_message(cx: &mut TestAppContext) {
    let r = start(cx, vec![item("l1", T - 10, false)]).await;
    let sends = r
        .provider
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Send { text, .. } if text == "scheduled text"))
        .count();
    assert_eq!(sends, 1);
    assert_eq!(uploads(&r), 0);
    assert!(stored(&r).await.is_empty());
}

#[gpui::test]
async fn single_timer_for_next_due(cx: &mut TestAppContext) {
    let r = start(
        cx,
        vec![item("l2", T + 200, true), item("l1", T + 50, true)],
    )
    .await;
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), Some(T + 50));
    assert_eq!(uploads(&r), 0);
    // Not yet.
    r.clock.0.store(T + 49, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(49));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 0);
    // The first fires, the timer moves to the next.
    r.clock.0.store(T + 50, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 1);
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), Some(T + 200));
    r.clock.0.store(T + 200, Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(150));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 2);
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), None);
}

#[gpui::test]
async fn late_after_restart_prompts(cx: &mut TestAppContext) {
    let r = start(
        cx,
        vec![item("l1", T - 4000, true), item("l2", T - 10, true)],
    )
    .await;
    // The late one waits for an answer; the one inside the grace period went.
    assert_eq!(uploads(&r), 1);
    let late = r
        .session
        .read_with(cx, |s, _| s.late_prompt().map(<[_]>::to_vec));
    assert_eq!(late, Some(vec![sid("l1")]));
    // Asked once: another tick does not ask or send.
    r.session.update(cx, |s, cx| s.local_tick(cx));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 1);
    r.session.update(cx, |s, cx| s.answer_late(true, cx));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 2);
    assert!(r.session.read_with(cx, |s, _| s.late_prompt().is_none()));
    assert!(stored(&r).await.is_empty());
}

#[gpui::test]
async fn late_kept_stays_scheduled_and_is_not_asked_again(cx: &mut TestAppContext) {
    let r = start(cx, vec![item("l1", T - 4000, true)]).await;
    assert!(r.session.read_with(cx, |s, _| s.late_prompt().is_some()));
    r.session.update(cx, |s, cx| s.answer_late(false, cx));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 0);
    assert!(r.session.read_with(cx, |s, _| s.late_prompt().is_none()));
    assert_eq!(stored(&r).await.len(), 1, "still scheduled");
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), None);
    r.session.update(cx, |s, cx| s.local_tick(cx));
    cx.run_until_parked();
    assert!(r.session.read_with(cx, |s, _| s.late_prompt().is_none()));
    assert_eq!(uploads(&r), 0);
    // It can still be sent from the Scheduled view.
    r.session
        .update(cx, |s, cx| s.send_schedule_now(sid("l1"), cx));
    cx.run_until_parked();
    assert_eq!(uploads(&r), 1);
    assert!(stored(&r).await.is_empty());
}

#[gpui::test]
async fn no_timer_without_local_items(cx: &mut TestAppContext) {
    let r = start(cx, vec![]).await;
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), None);
    assert!(r.session.read_with(cx, |s, _| s.late_prompt().is_none()));
    // Scheduling a local one arms it; deleting it disarms.
    r.session.update(cx, |s, cx| {
        s.schedule(cid("C1"), None, "x".into(), files(), T + 500, cx)
    });
    cx.run_until_parked();
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), Some(T + 500));
    let id = r
        .session
        .read_with(cx, |s, _| s.state().local_schedules[0].id.clone());
    r.session.update(cx, |s, cx| s.delete_schedule(id, cx));
    cx.run_until_parked();
    assert_eq!(r.session.read_with(cx, |s, _| s.armed_at()), None);
}

fn sent(r: &Rig) -> usize {
    r.provider
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Send { text, .. } if text == "server text"))
        .count()
}

#[gpui::test]
async fn send_now_on_a_server_schedule_posts_only_after_the_delete(cx: &mut TestAppContext) {
    use domain::test_support::Op;
    let r = start(cx, vec![]).await;
    r.provider
        .push(domain::ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
    r.session.update(cx, |s, cx| {
        s.schedule(cid("C1"), None, "server text".into(), vec![], T + 600, cx)
    });
    cx.run_until_parked();
    let id = r
        .session
        .read_with(cx, |s, _| s.state().scheduled[0].id.clone());
    // A failed delete leaves it scheduled and sends nothing (no double send).
    r.provider.fail_next(
        Op::DeleteScheduled,
        domain::ProviderError::Network("down".into()),
    );
    r.session
        .update(cx, |s, cx| s.send_schedule_now(id.clone(), cx));
    cx.run_until_parked();
    assert_eq!(sent(&r), 0);
    assert_eq!(r.session.read_with(cx, |s, _| s.state().scheduled.len()), 1);
    // Once the delete works, the message goes out exactly once.
    r.session.update(cx, |s, cx| s.send_schedule_now(id, cx));
    cx.run_until_parked();
    assert_eq!(sent(&r), 1, "{:?}", r.provider.calls());
    assert_eq!(r.session.read_with(cx, |s, _| s.state().scheduled.len()), 0);
}

#[gpui::test]
async fn a_failed_local_schedule_read_is_retried(cx: &mut TestAppContext) {
    let r = start(cx, vec![]).await;
    assert!(r.session.read_with(cx, |s, _| s.local_schedules_loaded()));
    r.session.update(cx, |s, cx| {
        s.local_schedules_read(Err(store::StoreError::Closed), cx)
    });
    assert!(!r.session.read_with(cx, |s, _| s.local_schedules_loaded()));
}
