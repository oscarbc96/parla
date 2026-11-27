//! Deterministic perf counters: allocations per event,
//! idle means idle, bounded windows and caches, cold-start reads and the
//! replayed soak. Fake time, a counting allocator and `FakeProvider`; the
//! ceilings live in `perf_budget.toml`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use domain::test_support::fixture::{
    self, conversation_id, member_id, perf_workspace, perf_workspaces,
};
use domain::test_support::{FakeFactory, FakeProvider, cid, ts, uid};
use domain::{
    ChatProvider, ClientMsgId, Conversation, ConversationId, DraftKey, Gap, Member, Message,
    MessageId, MessageTs, OutboxItem, PersistOp, PlainParser, ProviderEvent, SearchHit,
};
use futures::future::BoxFuture;
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use parla::perf::{CountingAlloc, allocations, frames_drawn, live_heap};
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView};
use parla::session::WorkspaceSession;
use settings::{ConfigPaths, WorkspaceKey};
use store::{Draft, MemoryStore, Store, StoreError};

mod support;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

// ---- budget file ----------------------------------------------------------

const BUDGET: &str = include_str!("perf_budget.toml");

fn budget_file() -> toml::Table {
    BUDGET.parse().expect("perf_budget.toml parses")
}

fn ceiling(key: &str) -> u64 {
    budget_file()["ceilings"][key]
        .as_integer()
        .unwrap_or_else(|| panic!("ceilings.{key}")) as u64
}

// ---- helpers --------------------------------------------------------------

/// A store that counts reads and writes. With `keep` it also stores (a real
/// `MemoryStore`); without, it discards writes so long replays cannot grow it.
struct CountingStore {
    inner: MemoryStore,
    keep: bool,
    reads: AtomicU64,
    writes: AtomicU64,
}

impl CountingStore {
    fn new(keep: bool) -> Arc<Self> {
        Arc::new(Self {
            inner: MemoryStore::default(),
            keep,
            reads: AtomicU64::new(0),
            writes: AtomicU64::new(0),
        })
    }

    fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }

    fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    fn read(&self) {
        self.reads.fetch_add(1, Ordering::Relaxed);
    }

    fn write(&self) {
        self.writes.fetch_add(1, Ordering::Relaxed);
    }
}

type R<'a, T> = BoxFuture<'a, Result<T, StoreError>>;

impl Store for CountingStore {
    fn apply(&self, ws: &WorkspaceKey, op: PersistOp) -> R<'_, ()> {
        self.write();
        if self.keep {
            self.inner.apply(ws, op)
        } else {
            Box::pin(async { Ok(()) })
        }
    }
    fn conversations(&self, ws: &WorkspaceKey) -> R<'_, Vec<Conversation>> {
        self.read();
        self.inner.conversations(ws)
    }
    fn members(&self, ws: &WorkspaceKey) -> R<'_, Vec<Member>> {
        self.read();
        self.inner.members(ws)
    }
    fn messages_before(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> R<'_, Vec<Message>> {
        self.read();
        self.inner.messages_before(ws, c, before, limit)
    }
    fn message(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> R<'_, Option<Message>> {
        self.read();
        self.inner.message(ws, c, ts)
    }
    fn latest_ts(&self, ws: &WorkspaceKey, c: &ConversationId) -> R<'_, Option<MessageTs>> {
        self.read();
        self.inner.latest_ts(ws, c)
    }
    fn thread(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        root: &MessageTs,
        limit: usize,
    ) -> R<'_, Vec<Message>> {
        self.read();
        self.inner.thread(ws, c, root, limit)
    }
    fn latest_all(&self, ws: &WorkspaceKey) -> R<'_, HashMap<ConversationId, MessageTs>> {
        self.read();
        self.inner.latest_all(ws)
    }
    fn gaps(&self, ws: &WorkspaceKey, c: &ConversationId) -> R<'_, Vec<Gap>> {
        self.read();
        self.inner.gaps(ws, c)
    }
    fn outbox(&self, ws: &WorkspaceKey) -> R<'_, Vec<OutboxItem>> {
        self.read();
        self.inner.outbox(ws)
    }
    fn outbox_put(&self, ws: &WorkspaceKey, item: OutboxItem) -> R<'_, ()> {
        self.write();
        self.inner.outbox_put(ws, item)
    }
    fn outbox_remove(&self, ws: &WorkspaceKey, id: &ClientMsgId) -> R<'_, ()> {
        self.write();
        self.inner.outbox_remove(ws, id)
    }
    fn drafts(&self, ws: &WorkspaceKey) -> R<'_, Vec<Draft>> {
        self.read();
        self.inner.drafts(ws)
    }
    fn draft_put(
        &self,
        ws: &WorkspaceKey,
        key: DraftKey,
        body: Option<domain::ComposedText>,
        edited: i64,
    ) -> R<'_, ()> {
        self.write();
        self.inner.draft_put(ws, key, body, edited)
    }
    fn search(&self, ws: &WorkspaceKey, q: &str, limit: usize) -> R<'_, Vec<SearchHit>> {
        self.read();
        self.inner.search(ws, q, limit)
    }
    fn local_notify_prefs(
        &self,
        ws: &WorkspaceKey,
    ) -> R<'_, std::collections::HashMap<ConversationId, domain::ConversationPrefs>> {
        self.read();
        self.inner.local_notify_prefs(ws)
    }
    fn local_notify_extras(
        &self,
        ws: &WorkspaceKey,
    ) -> R<'_, std::collections::HashMap<ConversationId, domain::NotifyExtras>> {
        self.read();
        self.inner.local_notify_extras(ws)
    }
    fn local_starred(&self, ws: &WorkspaceKey) -> R<'_, std::collections::HashSet<ConversationId>> {
        self.read();
        self.inner.local_starred(ws)
    }
    fn sidebar_collapsed(&self, ws: &WorkspaceKey) -> R<'_, std::collections::BTreeSet<String>> {
        self.read();
        self.inner.sidebar_collapsed(ws)
    }
    fn sidebar_collapsed_put(
        &self,
        ws: &WorkspaceKey,
        section: String,
        collapsed: bool,
    ) -> R<'_, ()> {
        self.write();
        self.inner.sidebar_collapsed_put(ws, section, collapsed)
    }
    fn local_reminders(&self, ws: &WorkspaceKey) -> R<'_, Vec<domain::Reminder>> {
        self.read();
        self.inner.local_reminders(ws)
    }
    fn local_schedules(&self, ws: &WorkspaceKey) -> R<'_, Vec<domain::LocalSchedule>> {
        self.read();
        self.inner.local_schedules(ws)
    }
    fn messages_after(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        after: &MessageTs,
        limit: usize,
    ) -> R<'_, Vec<Message>> {
        self.read();
        self.inner.messages_after(ws, c, after, limit)
    }
    fn recent_emoji(&self, ws: &WorkspaceKey, limit: usize) -> R<'_, Vec<domain::EmojiRef>> {
        self.read();
        self.inner.recent_emoji(ws, limit)
    }
    fn emoji_used(&self, ws: &WorkspaceKey, e: domain::EmojiRef, at_ms: i64) -> R<'_, ()> {
        self.write();
        self.inner.emoji_used(ws, e, at_ms)
    }
    fn search_filtered(
        &self,
        ws: &WorkspaceKey,
        q: &domain::SearchQuery,
        limit: usize,
    ) -> R<'_, Vec<SearchHit>> {
        self.read();
        self.inner.search_filtered(ws, q, limit)
    }
    fn user_groups(&self, ws: &WorkspaceKey) -> R<'_, Vec<domain::UserGroup>> {
        self.read();
        self.inner.user_groups(ws)
    }
    fn conversation_files(
        &self,
        ws: &WorkspaceKey,
        c: &domain::ConversationId,
        before: Option<domain::MessageTs>,
        limit: usize,
    ) -> R<'_, domain::Page<domain::FileHit>> {
        self.read();
        self.inner.conversation_files(ws, c, before, limit)
    }
    fn wipe(&self, ws: &WorkspaceKey) -> R<'_, ()> {
        self.write();
        self.inner.wipe(ws)
    }
}

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| parla::init_without_watcher(cx, paths));
    dir
}

fn posted(conversation: &str, n: u64) -> ProviderEvent {
    // Newer than every fixture message (they end around 1.7e9 s + 74,000 s).
    let ts = MessageTs(Arc::from(format!(
        "{}.{:06}",
        1_800_000_000 + n,
        n % 1_000_000
    )));
    let mut m = Message::new(
        MessageId {
            conversation: cid(conversation),
            ts,
        },
        Some(uid(&member_id(1 + (n % 50) as usize))),
        Arc::from("deploy is *green* <https://example.com/p/1|link> :tada:"),
    );
    m.plain = m.text.clone();
    ProviderEvent::MessagePosted(m)
}

fn session(
    cx: &mut TestAppContext,
    p: &Arc<FakeProvider>,
    st: &Arc<CountingStore>,
) -> Entity<WorkspaceSession> {
    let (p, st): (Arc<dyn ChatProvider>, Arc<dyn Store>) = (p.clone(), st.clone());
    let s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    s
}

async fn window(
    cx: &mut TestAppContext,
    p: Arc<FakeProvider>,
    st: Arc<dyn Store>,
) -> (Entity<RootView>, &mut VisualTestContext) {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let (view, vcx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| assert_eq!(*v.phase(), Phase::Ready));
    (view, vcx)
}

// ---- call window -------------

/// A joined huddle must leave nothing behind: no armed timer, no task, no
/// redraw, no helper. Fake time, a fake helper over pipes.
#[gpui::test]
async fn no_timers_after_hang_up(cx: &mut TestAppContext) {
    let rig = support::call_fake::rig(cx);
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = window(cx, rig.provider.clone(), store).await;
    let _session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    cx.run_until_parked();
    assert!(frames_drawn() > 0, "the window draws before the call");
    assert!(
        !cx.update(|_, app| support::call_fake::in_call(app)),
        "no call yet"
    );

    cx.update(|_, app| support::call_fake::join(&rig, app));
    cx.run_until_parked();
    assert_eq!(rig.fake.spawns(), 1, "the window rung spawned one helper");
    rig.fake.live();
    cx.run_until_parked();

    // Not trivially idle: the call is real before the hang-up.
    assert!(
        cx.update(|_, app| support::call_fake::in_call(app)),
        "in a call after joining"
    );
    let bar = cx
        .update(|_, app| support::call_fake::bar_model(app))
        .expect("bar model during the call");
    assert_eq!(bar.phase, components::CallBarPhase::Live);

    cx.update(|_, app| support::call_fake::leave(app));
    cx.run_until_parked();
    assert!(
        rig.fake.lines().iter().any(|l| l.contains("leave")),
        "the helper was told to leave: {:?}",
        rig.fake.lines()
    );
    rig.fake.exit();
    cx.run_until_parked();
    assert_eq!(rig.fake.exits(), 1, "the helper exited after leaving");
    assert_eq!(rig.fake.alive(), 0, "no helper process left");

    // An hour of fake time: no frame, no task and no timer may be pending.
    let frames = frames_drawn();
    for _ in 0..60 {
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
    }
    assert_eq!(
        frames_drawn(),
        frames,
        "cpu.idle after hang-up: frames drawn"
    );
    assert!(
        !cx.update(|_, app| support::call_fake::in_call(app)),
        "CallManager is still in a call"
    );
    assert!(
        cx.update(|_, app| support::call_fake::bar_model(app))
            .is_none(),
        "bar model after hang-up"
    );
    assert!(
        !cx.update(|_, app| support::call_fake::ringer_timer_armed(app)),
        "Ringer timer left armed"
    );
    // `in_call` is exactly "the controller's phase is not Idle" and the bar
    // model is `None` once it is Idle, so the two pin the phase to Idle.
    assert!(!cx.executor().tick(), "a task or timer is left behind");
}

/// Ten more join/leave cycles must not grow the parent's live heap beyond the
/// ceiling in `perf_budget.toml`, and every helper must be gone at the end.
#[gpui::test]
async fn parent_allocations_flat_across_ten_calls(cx: &mut TestAppContext) {
    let rig = support::call_fake::rig(cx);
    let cycle = |cx: &mut TestAppContext| {
        let spawned = rig.fake.spawns() + 1;
        cx.update(|app| support::call_fake::join(&rig, app));
        cx.run_until_parked();
        assert_eq!(rig.fake.spawns(), spawned, "one helper per call");
        rig.fake.live();
        cx.run_until_parked();
        assert!(
            cx.update(|app| support::call_fake::in_call(app)),
            "the call is live"
        );
        cx.update(support::call_fake::leave);
        cx.run_until_parked();
        rig.fake.exit();
        cx.run_until_parked();
        assert!(!cx.update(|app| support::call_fake::in_call(app)));
        // The 10 s after hang-up the budget allows: nothing may be left by then.
        cx.executor().advance_clock(Duration::from_secs(10));
        cx.run_until_parked();
    };
    for _ in 0..2 {
        cycle(cx);
    }
    assert_eq!(rig.fake.spawns(), 2, "two warm-up cycles");

    let before = live_heap();
    let allocs = allocations();
    for _ in 0..10 {
        cycle(cx);
    }
    let used = allocations() - allocs;
    let delta = live_heap().saturating_sub(before);
    println!(
        "call.parent measured: {delta} bytes of live heap over 10 join/leave cycles \
         ({used} allocations, debug test build)"
    );
    let cap = ceiling("heap_call_cycles_bytes");
    assert!(
        delta <= cap,
        "call.parent: live heap grew {delta} bytes over 10 calls > ceiling {cap}"
    );
    assert_eq!(rig.fake.spawns(), 12, "one helper per cycle");
    assert_eq!(rig.fake.alive(), 0, "helpers left running");
    assert_eq!(
        rig.fake.exits() + rig.fake.kills(),
        12,
        "every helper exited or was killed"
    );
}

// ---- counters -------------------------------------------------------------

#[gpui::test]
async fn allocations_per_live_event_within_budget(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(perf_workspace(1));
    let st = CountingStore::new(false);
    let s = session(cx, &p, &st);
    p.push_event(ProviderEvent::Connected { reconnect: false });
    s.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
    cx.run_until_parked();
    const N: u64 = 10_000;
    let events: Vec<ProviderEvent> = (0..N).map(|n| posted(&conversation_id(0), n)).collect();
    let before = allocations();
    for ev in events {
        p.push_event(ev);
    }
    cx.run_until_parked();
    let per_event = (allocations() - before) / N;
    println!("alloc.event measured: {per_event} allocations/event");
    let len = s.read_with(cx, |s, _| {
        s.state().windows[&cid(&conversation_id(0))]
            .messages()
            .len()
    });
    assert_eq!(len, 200, "events reached the window");
    assert!(
        per_event <= ceiling("alloc_per_event"),
        "alloc.event: {per_event} > ceiling {}",
        ceiling("alloc_per_event")
    );
}

#[gpui::test]
async fn idle_session_schedules_nothing(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(perf_workspace(2));
    let st = CountingStore::new(true);
    let (view, cx) = window(cx, p.clone(), st.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
    cx.run_until_parked();
    let (frames, writes, reads, calls) =
        (frames_drawn(), st.writes(), st.reads(), p.request_count());
    cx.executor().advance_clock(Duration::from_secs(3600));
    cx.run_until_parked();
    assert_eq!(frames_drawn(), frames, "cpu.idle: frames drawn while idle");
    assert_eq!(st.writes(), writes, "cpu.idle: store writes while idle");
    assert_eq!(st.reads(), reads, "cpu.idle: store reads while idle");
    assert_eq!(
        p.request_count(),
        calls,
        "net.idle: provider calls while idle"
    );
    // A timer left armed would have fired inside the hour and drawn or called.
    assert!(!cx.executor().tick(), "no runnable task is left behind");
}

/// A bot-alert channel: long messages full of links and emoji shortcodes.
fn alert_messages(conversation: &str, n: u64) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let text: String = (0..40)
                .map(|k| {
                    format!(
                        "Finding {k} :warning: see <https://example.com/a/{i}/{k}|runbook {k}> \
                         and https://example.com/b/{i}/{k} for the affected resource arn:aws:ec2:eu-west-1:{i}\n"
                    )
                })
                .collect();
            let mut m = Message::new(
                MessageId {
                    conversation: cid(conversation),
                    ts: MessageTs(Arc::from(format!("{}.{:06}", 1_700_100_000 + i, i))),
                },
                Some(uid(&member_id(1))),
                Arc::from(text),
            );
            m.plain = m.text.clone();
            m
        })
        .collect()
}

#[gpui::test]
async fn long_alert_messages_stay_idle_and_cheap_to_redraw(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(perf_workspace(5));
    let c = conversation_id(0);
    p.add_messages(&cid(&c), alert_messages(&c, 60));
    let st = CountingStore::new(true);
    let (view, cx) = window(cx, p.clone(), st.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid(&c), cx));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    // Idle means idle: nothing keeps drawing once the conversation settled.
    let frames = frames_drawn();
    cx.executor().advance_clock(Duration::from_secs(3600));
    cx.run_until_parked();
    assert_eq!(
        frames_drawn(),
        frames,
        "cpu.idle: frames drawn while a long-alert conversation sits open"
    );
    assert!(!cx.executor().tick(), "no runnable task is left behind");
    // A forced full redraw (what an animation tick or a resize causes) of the
    // visible long messages stays within the allocation ceiling.
    const REDRAWS: u64 = 20;
    let before = allocations();
    for _ in 0..REDRAWS {
        cx.update(|_, cx| cx.refresh_windows());
        cx.run_until_parked();
    }
    let per_redraw = (allocations() - before) / REDRAWS;
    println!("alloc.redraw_long_alerts measured: {per_redraw} allocations/redraw");
    assert!(
        per_redraw <= ceiling("alloc_per_long_alert_redraw"),
        "alloc.redraw_long_alerts: {per_redraw} > ceiling {}",
        ceiling("alloc_per_long_alert_redraw")
    );
}

#[gpui::test]
async fn hidden_window_still_updates_state(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(perf_workspace(3));
    let st = CountingStore::new(false);
    let (view, cx) = window(cx, p.clone(), st.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
    cx.run_until_parked();
    cx.simulate_visibility_change(gpui::WindowVisibility::Hidden);
    cx.run_until_parked();
    for n in 0..1_000 {
        p.push_event(posted(&conversation_id(1 + (n % 5) as usize), n));
    }
    cx.run_until_parked();
    let unread = session.read_with(cx, |s, _| {
        s.state()
            .conversations
            .values()
            .filter(|c| c.unread)
            .count()
    });
    assert!(unread > 0, "state still updates while hidden");
    // Not drawing while hidden is the platform's job: GPUI stops the frames of an
    // occluded or minimised window, which the test platform does not model. The `idle-hidden` perf scenario measures it on a real window.
}

#[gpui::test]
async fn window_memory_is_bounded(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(perf_workspace(4));
    let st = CountingStore::new(false);
    let s = session(cx, &p, &st);
    p.push_event(ProviderEvent::Connected { reconnect: false });
    let cap = cx.update(|cx| {
        cx.global::<settings::SettingsStore>()
            .current()
            .performance
            .message_window
    }) as usize;
    for i in 0..50 {
        let c = conversation_id(i);
        s.update(cx, |s, cx| s.open(cid(&c), cx));
        cx.run_until_parked();
        for n in 0..(cap as u64 + 100) {
            p.push_event(posted(&c, n));
        }
        cx.run_until_parked();
    }
    s.read_with(cx, |s, _| {
        assert_eq!(s.state().windows.len(), 50);
        for (c, w) in &s.state().windows {
            assert!(
                w.messages().len() <= cap,
                "{c:?}: {} > {cap}",
                w.messages().len()
            );
        }
    });
}

#[test]
fn caches_stay_at_their_caps() {
    // mem.cache: the render cache is an LRU of 512 rows, whatever is scrolled.
    let mut cache = components::RenderCache::new();
    let parser = PlainParser;
    for i in 0..2_000u32 {
        let m = Message::new(
            MessageId {
                conversation: cid("C0000"),
                ts: ts(&format!("{}.000000", 1_700_000_000 + i)),
            },
            Some(uid("U0001")),
            Arc::from("hello"),
        );
        cache.get_or_parse(&m, &parser);
    }
    assert!(cache.len() <= 512, "render cache {} > 512", cache.len());
    // The settings behind the other caps keep their spec values.
    let d = settings::Settings::default();
    assert_eq!(d.performance.message_window, 200);
    assert_eq!(d.performance.image_cache_mb, 32);
    assert_eq!(d.storage.cache_mb, 8);
}

#[test]
fn fixture_matches_the_spec_shape() {
    let p = perf_workspace(9);
    let calls = p.request_count();
    assert_eq!(calls, 0, "building the fixture makes no provider request");
    let again = perf_workspace(9);
    // Same seed, same data: the window of busy channel 0 has equal text.
    let page = |p: &FakeProvider| {
        futures::executor::block_on(p.history(&cid(&conversation_id(0)), None, 5)).unwrap()
    };
    let (a, b) = (page(&p), page(&again));
    assert_eq!(
        a.messages
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>(),
        b.messages
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>()
    );
    let boot = futures::executor::block_on(p.bootstrap()).unwrap();
    assert_eq!(boot.conversations.len(), fixture::CONVERSATIONS);
    assert_eq!(boot.members.len(), fixture::MEMBERS);
    let mut total = 0;
    for i in 0..fixture::CONVERSATIONS {
        let mut before = None;
        loop {
            let page =
                futures::executor::block_on(p.history(&cid(&conversation_id(i)), before, 1000))
                    .unwrap();
            total += page.messages.len();
            if !page.has_more {
                break;
            }
            before = page.messages.first().map(|m| m.id.ts.clone());
        }
    }
    assert_eq!(total, fixture::MESSAGES);
}

#[gpui::test]
async fn cold_start_reads_bounded(cx: &mut TestAppContext) {
    // start.content: a cold start from the cache loads the sidebar and exactly
    // one window (the most recent conversation), never the other 299.
    use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};
    let _dir = init(cx);
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let keys = Arc::new(MemoryKeySource::default());
    let start = |keys: &Arc<MemoryKeySource>| {
        SqliteStore::start(
            SqliteConfig {
                dir: dir.path().to_path_buf(),
                cache_mb: 8,
            },
            keys.clone(),
        )
    };
    let p = Arc::new(perf_workspace(5));
    let ws = p.workspace().clone();
    let codec: Arc<dyn domain::ExtCodec> = Arc::new(domain::NoExtCodec);
    // First launch fills the store and opens the busiest channel.
    let db = start(&keys);
    let s1 = {
        let (pp, db, codec) = (
            p.clone() as Arc<dyn ChatProvider>,
            db.clone(),
            codec.clone(),
        );
        cx.new(|cx| WorkspaceSession::new_sqlite(pp, db, codec, cx))
    };
    settle(cx, |_| sidebar_stored(&db, &ws));
    s1.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
    settle(cx, |_| {
        futures::executor::block_on(store::Store::latest_all(&db, &ws)).map_or(0, |m| m.len()) >= 1
    });
    drop(s1);
    db.shutdown();
    // Second launch, offline: everything shown comes from the store.
    p.fail_next(
        domain::test_support::Op::Bootstrap,
        domain::ProviderError::Network("offline".into()),
    );
    let db = start(&keys);
    let s2 = {
        let (pp, db, codec) = (p as Arc<dyn ChatProvider>, db.clone(), codec.clone());
        cx.new(|cx| WorkspaceSession::new_sqlite(pp, db, codec, cx))
    };
    settle(cx, |cx| {
        s2.read_with(cx, |s, _| {
            !s.state().conversations.is_empty() && !s.state().windows.is_empty()
        })
    });
    s2.read_with(cx, |s, _| {
        assert_eq!(
            s.state().windows.len(),
            1,
            "only the last-open window is read"
        );
        let w = s.state().windows.values().next().unwrap();
        assert!(w.messages().len() <= 200);
    });
    let plan = futures::executor::block_on(db.explain_window(&ws))
        .unwrap()
        .join(" | ");
    assert!(plan.contains("messages_window"), "plan: {plan}");
    db.shutdown();
}

fn settle(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("condition not reached");
}

fn sidebar_stored(db: &store_sqlite::SqliteStore, ws: &WorkspaceKey) -> bool {
    // Bootstrap and catch-up have written the sidebar once it lists everything.
    futures::executor::block_on(store::Store::conversations(db, ws))
        .is_ok_and(|c| c.len() == fixture::CONVERSATIONS)
}

#[test]
fn window_query_uses_the_window_index() {
    use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let ws = WorkspaceKey::new("fake", "perf");
    futures::executor::block_on(db.open(&ws, Arc::new(domain::NoExtCodec))).unwrap();
    let plan = futures::executor::block_on(db.explain_window(&ws)).unwrap();
    db.shutdown();
    let joined = plan.join(" | ");
    assert!(joined.contains("messages_window"), "plan: {joined}");
    assert!(!joined.contains("SCAN messages"), "full scan: {joined}");
}

#[test]
fn latest_query_uses_the_window_index() {
    use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let ws = WorkspaceKey::new("fake", "perf");
    futures::executor::block_on(db.open(&ws, Arc::new(domain::NoExtCodec))).unwrap();
    let plan = futures::executor::block_on(db.explain_latest(&ws)).unwrap();
    db.shutdown();
    let joined = plan.join(" | ");
    assert!(joined.contains("messages_window"), "plan: {joined}");
    assert!(
        !plan.iter().any(|l| l.trim() == "SCAN messages"),
        "full scan: {joined}"
    );
}

#[gpui::test]
#[ignore = "soak: run by `mise run test:perf`"]
async fn replayed_soak_returns_to_baseline(cx: &mut TestAppContext) {
    // mem.soak (counter form): 3 workspaces, 1 event/s each, a switch every
    // 2 minutes for 8 h of fake time, then 10 minutes idle.
    let _dir = init(cx);
    let mut parts = Vec::new();
    for w in 0..3u64 {
        let p = Arc::new(perf_workspace(10 + w));
        let st = CountingStore::new(false);
        let s = session(cx, &p, &st);
        p.push_event(ProviderEvent::Connected { reconnect: false });
        parts.push((p, s));
    }
    let mut hour_mark = 0u64;
    for sec in 0..(8 * 3600u64) {
        if sec % 120 == 0 {
            let c = conversation_id(((sec / 120) % 8) as usize);
            for (_, s) in &parts {
                s.update(cx, |s, cx| s.open(cid(&c), cx));
            }
        }
        for (p, _) in &parts {
            p.push_event(posted(&conversation_id(((sec / 120) % 8) as usize), sec));
        }
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        if sec == 3600 {
            hour_mark = live_heap();
        }
    }
    cx.executor().advance_clock(Duration::from_secs(600));
    cx.run_until_parked();
    let end = live_heap();
    let drift = (end as f64 - hour_mark as f64) / hour_mark as f64 * 100.0;
    println!("mem.soak live heap: 1 h {hour_mark} B, end {end} B, drift {drift:.2} %");
    assert!(drift <= 2.0, "live heap drifted {drift:.2} %");
}

// ---- three workspaces and lean mode ---------------------------

const LEAN_AFTER_10: &str = r#"{ "performance": { "lean_after_minutes": 10 } }"#;

/// Three perf workspaces under one manager over a counting store (no encrypted
/// store, so every read and write is visible). Nothing is started yet.
fn manager(
    cx: &mut TestAppContext,
    providers: &[Arc<FakeProvider>],
    st: &Arc<CountingStore>,
) -> (Entity<parla::workspaces::Workspaces>, tempfile::TempDir) {
    let dir = init(cx);
    let mut registry = ProviderRegistry::new();
    let mut factory = FakeFactory::default();
    for p in providers {
        factory = factory.with_provider(p.clone());
    }
    registry.add(factory);
    let (data, cache) = (dir.path().join("data"), dir.path().join("cache"));
    let store: Arc<dyn Store> = st.clone();
    let w = cx
        .new(|cx| parla::workspaces::Workspaces::new(&registry, store, cx).with_dirs(data, cache));
    (w, dir)
}

fn perf_providers(seed: u64, n: usize) -> Vec<Arc<FakeProvider>> {
    perf_workspaces(seed, n).into_iter().map(Arc::new).collect()
}

fn start_all(cx: &mut TestAppContext, w: &Entity<parla::workspaces::Workspaces>) {
    w.update(cx, |w, cx| w.start_all(cx));
    cx.run_until_parked();
}

#[test]
fn perf_workspaces_are_distinct_and_deterministic() {
    let all = perf_workspaces(7, 3);
    assert_eq!(all.len(), 3);
    let keys: Vec<_> = all.iter().map(|p| p.workspace().clone()).collect();
    assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2]);
    assert_eq!(
        all[0].workspace(),
        perf_workspace(7).workspace(),
        "the first one is the single perf workspace"
    );
}

#[gpui::test]
async fn three_hidden_workspaces_go_lean_and_release_windows(cx: &mut TestAppContext) {
    let providers = perf_providers(20, 3);
    let e = support::env_with(cx, LEAN_AFTER_10, providers.clone());
    let w = support::workspaces(cx, &e);
    for s in w.read_with(cx, |w, _| w.sessions().cloned().collect::<Vec<_>>()) {
        s.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
    }
    let c0 = cid(&conversation_id(0));
    support::settle_until(cx, |cx| {
        w.read_with(cx, |w, cx| {
            w.sessions()
                .all(|s| s.read(cx).state().windows.contains_key(&c0))
        })
    });
    w.update(cx, |w, cx| w.set_window_visible(false, cx));
    cx.executor().advance_clock(Duration::from_secs(10 * 60));
    support::settle_until(cx, |cx| {
        w.read_with(cx, |w, cx| {
            w.lean_timers() == 0
                && w.sessions()
                    .all(|s| s.read(cx).is_lean() && s.read(cx).state().windows.is_empty())
        })
    });
    // Closing the lean workspaces' stores is asynchronous too.
    support::settle_until(cx, |_| {
        providers.iter().all(|p| {
            futures::executor::block_on(e.sqlite.read(p.workspace(), |_, _| Ok(1u8))).is_err()
        })
    });
    w.read_with(cx, |w, cx| {
        assert_eq!(w.sessions().count(), 3);
        assert_eq!(w.lean_timers(), 0, "every timer fired and is gone");
        for s in w.sessions() {
            let s = s.read(cx);
            assert!(s.is_lean(), "hidden workspace is lean");
            assert!(s.state().windows.is_empty(), "mem.ws: windows released");
        }
    });
    for p in &providers {
        assert!(
            !e.sqlite.read(p.workspace(), |_, _| Ok(1u8)).await.is_ok(),
            "mem.ws: the store of a lean workspace is closed"
        );
    }
    // The render cache belongs to the timeline view of the shown conversation;
    // a lean workspace has no view, so there is nothing of it left to release.
}

#[gpui::test]
async fn lean_live_event_allocations_within_budget(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(perf_workspace(21));
    let st = CountingStore::new(false);
    let s = session(cx, &p, &st);
    p.push_event(ProviderEvent::Connected { reconnect: false });
    s.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
    cx.run_until_parked();
    s.update(cx, |s, cx| s.set_lean(true, cx));
    cx.run_until_parked();
    const N: u64 = 10_000;
    let events: Vec<ProviderEvent> = (0..N).map(|n| posted(&conversation_id(0), n)).collect();
    let before = allocations();
    for ev in events {
        p.push_event(ev);
    }
    cx.run_until_parked();
    let per_event = (allocations() - before) / N;
    println!("alloc.event (lean) measured: {per_event} allocations/event");
    s.read_with(cx, |s, _| {
        assert!(s.is_lean());
        assert!(s.state().windows.is_empty(), "lean holds no window");
    });
    assert!(
        per_event <= ceiling("alloc_per_lean_event"),
        "alloc.event (lean): {per_event} > ceiling {}",
        ceiling("alloc_per_lean_event")
    );
}

#[gpui::test]
async fn idle_three_workspaces_schedule_nothing(cx: &mut TestAppContext) {
    let providers = perf_providers(22, 3);
    let st = CountingStore::new(true);
    let (w, _dir) = manager(cx, &providers, &st);
    start_all(cx, &w);
    let sessions: Vec<_> = w.read_with(cx, |w, _| w.sessions().cloned().collect());
    assert_eq!(sessions.len(), 3);
    w.update(cx, |w, cx| w.set_window_visible(false, cx));
    cx.executor()
        .advance_clock(Duration::from_secs(60 * 60 * 24));
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert_eq!(w.lean_timers(), 0, "cpu.idle: no pending timers");
        assert!(w.sessions().all(|s| s.read(cx).is_lean()));
    });
    let (frames, writes, reads) = (frames_drawn(), st.writes(), st.reads());
    let calls: Vec<u64> = providers.iter().map(|p| p.request_count()).collect();
    cx.executor().advance_clock(Duration::from_secs(3600));
    cx.run_until_parked();
    assert_eq!(frames_drawn(), frames, "cpu.idle: frames drawn while lean");
    assert_eq!(st.writes(), writes, "cpu.idle: store writes while lean");
    assert_eq!(st.reads(), reads, "cpu.idle: store reads while lean");
    let now: Vec<u64> = providers.iter().map(|p| p.request_count()).collect();
    assert_eq!(now, calls, "net.idle: provider calls while lean");
    assert!(!cx.executor().tick(), "no runnable task is left behind");
}

#[gpui::test]
async fn extra_workspace_heap_within_ceiling(cx: &mut TestAppContext) {
    // mem.ws (counter form): live heap added by a shown and by a lean workspace.
    let providers = perf_providers(23, 3);
    let st = CountingStore::new(true);
    let (w, _dir) = manager(cx, &providers, &st);
    let start = |cx: &mut TestAppContext, i: usize| {
        let key = providers[i].workspace().clone();
        w.update(cx, |w, cx| w.start(key, cx));
        cx.run_until_parked();
        if let Some(s) = w.read_with(cx, |w, _| w.sessions().nth(i).cloned()) {
            s.update(cx, |s, cx| s.open(cid(&conversation_id(0)), cx));
            cx.run_until_parked();
        }
    };
    start(cx, 0);
    let one = live_heap();
    start(cx, 1);
    let two = live_heap();
    start(cx, 2);
    // Visible: only the active workspace is looked at. Hidden: all go lean.
    w.update(cx, |w, cx| w.set_window_visible(false, cx));
    cx.executor().advance_clock(Duration::from_secs(60 * 60));
    cx.run_until_parked();
    let lean3 = live_heap();
    w.read_with(cx, |w, cx| {
        assert_eq!(w.sessions().count(), 3);
        assert!(w.sessions().all(|s| s.read(cx).is_lean()));
    });
    let shown = two.saturating_sub(one);
    let lean = lean3.saturating_sub(one) / 2;
    println!("mem.ws measured: shown +{shown} B, lean +{lean} B");
    assert!(
        shown <= ceiling("heap_ws_shown_bytes"),
        "mem.ws shown: +{shown} B > {}",
        ceiling("heap_ws_shown_bytes")
    );
    assert!(
        lean <= ceiling("heap_ws_lean_bytes"),
        "mem.ws lean: +{lean} B > {}",
        ceiling("heap_ws_lean_bytes")
    );
}
