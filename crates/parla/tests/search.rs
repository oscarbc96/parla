//! Search panel wiring: the session answers from the local store (so it works
//! offline), the window opens the panel, and a hit opens its conversation
//! scrolled to the message. Also: the default keymaps bind both features.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use components::{Search, SearchPanelEvent, SearchScope, SearchSource as _, SearchTarget};
use domain::test_support::Op;
use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, msg, ts, uid};
use domain::{Capabilities, HitSource, MessageHit, ProviderError, ProviderEvent};
use domain::{MessageId, NoExtCodec, PersistOp};
use gpui::{AppContext as _, TestAppContext};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use parla::session::WorkspaceSession;
use settings::{ConfigPaths, WorkspaceKey, parse_keymap, resolve_bindings};
use store::MemoryStore;
use store::Store;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    // Pin the language so the test does not depend on the OS locale.
    let _ = paths.ensure_exists();
    std::fs::write(&paths.settings, r#"{ "locale": "en" }"#).unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        // Tests never depend on the OS language.
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

/// A provider that knows nothing about the old message the store holds, so a
/// hit proves the answer came from the cache.
fn provider() -> Arc<FakeProvider> {
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Bob"));
    p.add_message(msg("C1", "1700000100.000000", "hello"));
    p
}

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}

/// Closes the store when the test ends (see `unreads.rs`).
struct Shutdown(SqliteStore);

impl Drop for Shutdown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

async fn cached_store(dir: &Path) -> (SqliteStore, Shutdown) {
    let db = SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let guard = Shutdown(db.clone());
    db.open(&ws(), Arc::new(NoExtCodec)).await.unwrap();
    db.apply(
        &ws(),
        PersistOp::Conversations(vec![conv("C1", "general"), conv("C2", "random")]),
    )
    .await
    .unwrap();
    db.apply(&ws(), PersistOp::Members(vec![member("u2", "Bob")]))
        .await
        .unwrap();
    let mut old = msg("C2", "1700000000.000000", "the deploy finished late");
    old.author = Some(uid("u2"));
    db.apply(
        &ws(),
        PersistOp::Messages {
            conversation: cid("C2"),
            messages: vec![old],
        },
    )
    .await
    .unwrap();
    (db, guard)
}

fn settle_until(
    cx: &mut gpui::VisualTestContext,
    mut done: impl FnMut(&mut gpui::VisualTestContext) -> bool,
) {
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

fn settle_app(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
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

#[gpui::test]
async fn session_search_resolves_hits_from_the_local_store(cx: &mut TestAppContext) {
    let _dir = init(cx);
    cx.executor().allow_parking();
    let data = tempfile::tempdir().unwrap();
    let (db, _guard) = cached_store(data.path()).await;
    let s = cx.new(|cx| WorkspaceSession::new(provider(), Arc::new(db) as Arc<dyn Store>, cx));
    cx.run_until_parked();
    s.update(cx, |s, cx| {
        s.search(SearchScope::Messages, "deploy".into(), cx)
    });
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| !s.search_results().rows.is_empty())
    });
    let r = s.read_with(cx, |s, _| s.search_results().clone());
    assert_eq!(&*r.query, "deploy");
    assert_eq!(r.rows.len(), 1);
    let row = &r.rows[0];
    assert_eq!(
        row.target,
        SearchTarget::Message(MessageId {
            conversation: cid("C2"),
            ts: ts("1700000000.000000")
        })
    );
    assert_eq!(&*row.conversation, "#random");
    assert_eq!(&*row.author, "Bob");
    assert!(row.time.starts_with("Nov 14, 2023"), "{}", row.time);
    assert!(row.snippet.contains("deploy"));
    s.update(cx, |s, cx| {
        s.search(SearchScope::Messages, "   ".into(), cx)
    });
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| s.search_results().rows.is_empty())
    });
}

#[gpui::test]
async fn search_action_opens_panel_and_a_hit_opens_its_conversation(cx: &mut TestAppContext) {
    let _dir = init(cx);
    cx.executor().allow_parking();
    let data = tempfile::tempdir().unwrap();
    let (db, _guard) = cached_store(data.path()).await;
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider()));
    let st = Arc::new(db) as Arc<dyn Store>;
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.search_panel().is_none()));
    cx.dispatch_action(Search);
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.search_panel().unwrap().clone());
    panel.update(cx, |p, cx| p.set_query("deploy", cx));
    cx.executor().advance_clock(Duration::from_millis(400));
    settle_until(cx, |cx| {
        session.read_with(cx, |s, _| !s.search_results().rows.is_empty())
    });
    let id = MessageId {
        conversation: cid("C2"),
        ts: ts("1700000000.000000"),
    };
    panel.update(cx, |_, cx| cx.emit(SearchPanelEvent::Open(id.clone())));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C2"))
    );
    let tl = view.read_with(cx, |v, _| v.timeline().unwrap().clone());
    settle_until(cx, |cx| tl.read_with(cx, |t, _| t.selected() == Some(&id)));
    // Closing returns the right column to its placeholder.
    panel.update(cx, |_, cx| cx.emit(SearchPanelEvent::Close));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.search_panel().is_none()));
}

#[test]
fn default_keymaps_bind_mark_unread_and_search() {
    for (name, text, search, mark) in [
        (
            "macos",
            include_str!("../../../assets/keymaps/default-macos.json"),
            "cmd-shift-f",
            "cmd-shift-u",
        ),
        (
            "linux",
            include_str!("../../../assets/keymaps/default-linux.json"),
            "ctrl-shift-f",
            "ctrl-shift-u",
        ),
        (
            "windows",
            include_str!("../../../assets/keymaps/default-windows.json"),
            "ctrl-shift-f",
            "ctrl-shift-u",
        ),
    ] {
        let blocks = parse_keymap(Path::new(name), text).unwrap();
        let (bindings, warnings) = resolve_bindings(&blocks, &[], &parla::actions::is_known);
        assert!(warnings.is_empty(), "{name}: {warnings:?}");
        let has = |k: &str, a: &str| bindings.iter().any(|b| b.keystrokes == k && b.action == a);
        assert!(has(search, "workspace::Search"), "{name}");
        assert!(has(mark, "timeline::MarkUnread"), "{name}");
        let latest = search.replace("-f", "-down");
        assert!(has(&latest, "timeline::JumpToLatest"), "{name}");
    }
}

fn searching_provider() -> Arc<FakeProvider> {
    let p = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            search: true,
            ..Default::default()
        }),
    );
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Bob"));
    p.add_message(msg("C1", "1700000100.000000", "hello"));
    p
}

fn remote_hit(c: &str, t: &str, text: &str) -> MessageHit {
    let mut m = msg(c, t, text);
    m.author = Some(uid("u2"));
    MessageHit {
        message: m,
        snippet: Arc::from(text),
        source: HitSource::Remote,
    }
}

async fn memory_with(extra: Vec<domain::Message>) -> Arc<dyn Store> {
    let st = MemoryStore::default();
    let mut old = msg("C2", "1700000000.000000", "the deploy finished late");
    old.author = Some(uid("u2"));
    st.apply(
        &ws(),
        PersistOp::Conversations(vec![conv("C1", "general"), conv("C2", "random")]),
    )
    .await
    .unwrap();
    st.apply(
        &ws(),
        PersistOp::Messages {
            conversation: cid("C2"),
            messages: vec![old],
        },
    )
    .await
    .unwrap();
    if !extra.is_empty() {
        st.apply(
            &ws(),
            PersistOp::Messages {
                conversation: cid("C1"),
                messages: extra,
            },
        )
        .await
        .unwrap();
    }
    Arc::new(st)
}

fn search_session(
    cx: &mut TestAppContext,
    p: &Arc<FakeProvider>,
    st: Arc<dyn Store>,
) -> gpui::Entity<WorkspaceSession> {
    let s = cx.new(|cx| WorkspaceSession::new(p.clone(), st, cx));
    cx.run_until_parked();
    s
}

async fn run_search(
    cx: &mut TestAppContext,
    s: &gpui::Entity<WorkspaceSession>,
    scope: SearchScope,
    input: &str,
) -> components::SearchResults {
    let q = input.to_string();
    s.update(cx, |s, cx| s.search(scope, q.clone(), cx));
    let want: Arc<str> = Arc::from(q.trim());
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| {
            let r = s.search_results();
            r.query == want && !r.pending
        })
    });
    s.read_with(cx, |s, _| s.search_results().clone())
}

#[gpui::test]
async fn local_hits_come_first_then_the_servers(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.set_search_hits(vec![
        remote_hit("C1", "1700000200.000000", "deploy notes"),
        remote_hit("C1", "1700000300.000000", "deploy plan"),
    ]);
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::Messages, "deploy").await;
    assert_eq!(r.rows.len(), 1, "{r:?}");
    assert!(r.rows[0].snippet.contains("finished late"));
    // Remote hits not already local, newest first.
    let remote: Vec<_> = r.remote.iter().map(|x| x.snippet.to_string()).collect();
    assert_eq!(remote, ["deploy plan", "deploy notes"]);
    assert!(!r.offline && !r.remote_error);
}

#[gpui::test]
async fn a_server_hit_already_held_locally_is_not_repeated(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.set_search_hits(vec![remote_hit(
        "C2",
        "1700000000.000000",
        "the deploy finished late",
    )]);
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::Messages, "deploy").await;
    assert_eq!((r.rows.len(), r.remote.len()), (1, 0), "{r:?}");
}

#[gpui::test]
async fn offline_searches_the_device_only_and_says_so(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.set_search_hits(vec![remote_hit("C1", "1700000200.000000", "deploy notes")]);
    let s = search_session(cx, &p, memory_with(vec![]).await);
    p.push_event(ProviderEvent::Disconnected);
    cx.run_until_parked();
    let before = p.request_count();
    let r = run_search(cx, &s, SearchScope::Messages, "deploy").await;
    assert_eq!(r.rows.len(), 1);
    assert!(r.remote.is_empty());
    assert!(r.offline);
    assert_eq!(p.request_count(), before, "no request while offline");
}

#[gpui::test]
async fn a_failing_server_keeps_the_local_rows(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.fail_next(Op::SearchMessages, ProviderError::Network("down".into()));
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::Messages, "deploy").await;
    assert_eq!(r.rows.len(), 1);
    assert!(r.remote_error);
}

#[gpui::test]
async fn empty_query_never_queries_the_server(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.set_search_hits(vec![remote_hit("C1", "1700000200.000000", "deploy notes")]);
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let before = p.request_count();
    s.update(cx, |s, cx| s.search(SearchScope::Messages, "  ".into(), cx));
    s.update(cx, |s, cx| s.search(SearchScope::People, "".into(), cx));
    cx.run_until_parked();
    assert_eq!(p.request_count(), before);
    assert!(s.read_with(cx, |s, _| s.search_results().rows.is_empty()));
}

#[gpui::test]
async fn unknown_names_are_reported_and_ignored(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::Messages, "in:#nope deploy").await;
    assert_eq!(r.warnings, ["in:#nope"]);
    assert_eq!(r.rows.len(), 1, "the unknown filter is dropped: {r:?}");
}

#[gpui::test]
async fn filters_narrow_the_local_rows(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::Messages, "in:#general deploy").await;
    assert!(r.rows.is_empty(), "the hit is in #random: {r:?}");
    let r = run_search(cx, &s, SearchScope::Messages, "in:#random deploy").await;
    assert_eq!(r.rows.len(), 1);
}

#[gpui::test]
async fn people_and_conversations_scopes_use_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::People, "bob").await;
    let all: Vec<_> = r.rows.iter().chain(&r.remote).collect();
    assert_eq!(all.len(), 1, "{r:?}");
    assert_eq!(all[0].target, SearchTarget::Person(uid("u2")));
    let r = run_search(cx, &s, SearchScope::Conversations, "rand").await;
    let all: Vec<_> = r.rows.iter().chain(&r.remote).collect();
    assert_eq!(all.len(), 1, "{r:?}");
    assert_eq!(all[0].target, SearchTarget::Conversation(cid("C2")));
}

fn many(c: &str, from: u64, n: u64) -> Vec<domain::Message> {
    (0..n)
        .map(|i| msg(c, &format!("{}.000000", from + i * 10), &format!("m{i}")))
        .collect()
}

#[gpui::test]
async fn fetch_around_uses_the_store_first(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    // 200 messages stored: the target sits well inside, so both sides are held.
    let stored = many("C1", 1_700_001_000, 200);
    let st = memory_with(stored.clone()).await;
    let s = search_session(cx, &p, st);
    let before = p.request_count();
    let target = stored[100].id.clone();
    s.update(cx, |s, cx| s.jump_to(target.clone(), cx));
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| {
            s.state()
                .windows
                .get(&cid("C1"))
                .is_some_and(|w| w.messages().iter().any(|m| m.id == target))
        })
    });
    assert_eq!(p.request_count(), before, "no provider call");
    assert_eq!(s.read_with(cx, |s, _| s.state().jump.clone()), Some(target));
    assert!(s.read_with(cx, |s, _| !s.state().windows[&cid("C1")].is_live()));
}

#[gpui::test]
async fn fetch_around_asks_the_provider_when_the_store_lacks_a_side(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.add_messages(&cid("C1"), many("C1", 1_700_001_000, 200));
    // The store only knows the target and a few after it.
    let stored = many("C1", 1_700_001_000, 200);
    let st = memory_with(stored[100..103].to_vec()).await;
    let s = search_session(cx, &p, st);
    let before = p.request_count();
    let target = stored[100].id.clone();
    s.update(cx, |s, cx| s.jump_to(target.clone(), cx));
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| s.state().jump.as_ref() == Some(&target))
    });
    assert!(p.request_count() > before);
}

#[gpui::test]
async fn fetch_newer_uses_the_store_first(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let stored = many("C1", 1_700_001_000, 300);
    let st = memory_with(stored.clone()).await;
    let s = search_session(cx, &p, st);
    // A detached window around message 40; scrolling to its bottom loads newer ones.
    s.update(cx, |s, cx| s.jump_to(stored[40].id.clone(), cx));
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| s.state().windows.contains_key(&cid("C1")))
    });
    let before = p.request_count();
    let newest = s.read_with(cx, |s, _| {
        s.state().windows[&cid("C1")].newest_ts().cloned().unwrap()
    });
    s.update(cx, |s, cx| s.focus(cid("C1"), true, true, cx));
    settle_app(cx, |cx| {
        s.read_with(cx, |s, _| {
            s.state().windows[&cid("C1")].newest_ts() != Some(&newest)
        })
    });
    assert_eq!(p.request_count(), before, "no provider call");
}

#[gpui::test]
async fn search_opens_prefilled_with_the_current_conversation(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let mut registry = ProviderRegistry::new();
    let p = searching_provider();
    registry.add(FakeFactory::default().with_provider(p));
    let (view, cx) = cx.add_window_view(|window, cx| {
        RootView::new(&registry, Arc::new(MemoryStore::default()), window, cx)
    });
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    cx.dispatch_action(Search);
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.search_panel().unwrap().clone());
    assert_eq!(
        panel.read_with(cx, |p, _| p.query().to_string()),
        "in:#general "
    );
}

#[gpui::test]
async fn choosing_a_hit_jumps_and_highlights(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let mut registry = ProviderRegistry::new();
    let p = searching_provider();
    registry.add(FakeFactory::default().with_provider(p));
    let stored = many("C1", 1_700_001_000, 200);
    let st = memory_with(stored.clone()).await;
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C2"), cx));
    cx.run_until_parked();
    cx.dispatch_action(Search);
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.search_panel().unwrap().clone());
    let target = stored[100].id.clone();
    panel.update(cx, |_, cx| cx.emit(SearchPanelEvent::Open(target.clone())));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C1"))
    );
    let tl = view.read_with(cx, |v, _| v.timeline().unwrap().clone());
    settle_until(cx, |cx| {
        tl.read_with(cx, |t, _| t.highlighted() == Some(&target))
    });
    assert!(tl.read_with(cx, |t, cx| t.is_detached(cx)));
}

#[gpui::test]
async fn remote_snippets_show_names_not_user_ids(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    p.set_search_hits(vec![remote_hit(
        "C1",
        "1700000200.000000",
        "deploy ok cc @u2 and @U0NOBODY",
    )]);
    let s = search_session(cx, &p, memory_with(vec![]).await);
    let r = run_search(cx, &s, SearchScope::Messages, "deploy").await;
    let remote: Vec<_> = r.remote.iter().map(|x| x.snippet.to_string()).collect();
    assert_eq!(remote, ["deploy ok cc @Bob and @U0NOBODY"], "{r:?}");
}

#[gpui::test]
async fn search_context_reads_the_store_around_a_hit_not_held(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let stored = many("C1", 1_700_001_000, 20);
    let st = memory_with(stored.clone()).await;
    let s = search_session(cx, &p, st);
    let target = stored[10].id.clone();
    let none = s.read_with(cx, |s, cx| s.context(&target, cx));
    assert!(none.is_empty(), "nothing held before it is read");
    s.update(cx, |s, cx| s.load_context(Some(target.clone()), cx));
    settle_app(cx, |cx| {
        s.read_with(cx, |s, cx| !s.context(&target, cx).is_empty())
    });
    let texts: Vec<String> = s.read_with(cx, |s, cx| {
        s.context(&target, cx)
            .iter()
            .map(|l| l.text.to_string())
            .collect()
    });
    assert_eq!(texts, ["m8", "m9", "m10", "m11", "m12"]);
}

/// Lets background reads finish.
fn settle(cx: &mut TestAppContext) {
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn texts(
    s: &gpui::Entity<WorkspaceSession>,
    id: &MessageId,
    cx: &mut TestAppContext,
) -> Vec<String> {
    s.read_with(cx, |s, cx| {
        s.context(id, cx)
            .iter()
            .map(|l| l.text.to_string())
            .collect()
    })
}

#[gpui::test]
async fn a_late_context_answer_does_not_replace_the_selected_one(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    // A is only on the server (slow); B is on the device.
    p.add_conversation(conv("C3", "far"));
    let far = many("C3", 1_600_000_000, 20);
    p.add_messages(&cid("C3"), far.clone());
    let near = many("C1", 1_700_001_000, 20);
    // The newest activity is in C2, so C1 opens with no window: B comes
    // from the store read, A from the server.
    let mut stored = near.clone();
    stored.push(msg("C2", "1800000000.000000", "newest"));
    let st = memory_with(stored).await;
    let s = search_session(cx, &p, st);
    let (a, b) = (far[10].id.clone(), near[10].id.clone());
    p.hold(Op::History);
    s.update(cx, |s, cx| s.load_context(Some(a.clone()), cx));
    settle(cx);
    s.update(cx, |s, cx| s.load_context(Some(b.clone()), cx));
    settle_app(cx, |cx| !texts(&s, &b, cx).is_empty());
    p.release();
    settle(cx);
    assert_eq!(texts(&s, &b, cx), ["m8", "m9", "m10", "m11", "m12"]);
    assert!(texts(&s, &a, cx).is_empty(), "the dropped read never lands");
}

#[gpui::test]
async fn moving_through_many_hits_reads_only_where_the_selection_rests(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let far = many("C1", 1_600_000_000, 60);
    p.add_messages(&cid("C1"), far.clone());
    let st = memory_with(vec![]).await;
    let s = search_session(cx, &p, st);
    settle(cx);
    // What reading one hit's context costs.
    let start = p.request_count();
    s.update(cx, |s, cx| s.load_context(Some(far[59].id.clone()), cx));
    settle(cx);
    let one = p.request_count() - start;
    let before = p.request_count();
    for m in &far[18..58] {
        s.update(cx, |s, cx| s.load_context(Some(m.id.clone()), cx));
    }
    settle(cx);
    let asked = p.request_count() - before;
    assert!(
        asked <= 2 * one,
        "40 hits passed, {asked} requests (one costs {one})"
    );
}

#[gpui::test]
async fn a_thread_reply_hit_shows_its_thread_not_the_channel(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let mut stored = many("C1", 1_700_001_000, 20);
    stored.push(msg("C2", "1800000000.000000", "newest"));
    let root = stored[5].id.ts.clone();
    for i in 0..5u64 {
        let mut r = msg(
            "C1",
            &format!("{}.000000", 1_700_001_052 + i),
            &format!("r{i}"),
        );
        r.thread_root = Some(root.clone());
        stored.push(r);
    }
    let hit = stored[23].id.clone();
    let st = memory_with(stored).await;
    let s = search_session(cx, &p, st);
    s.update(cx, |s, cx| s.load_context(Some(hit.clone()), cx));
    settle_app(cx, |cx| !texts(&s, &hit, cx).is_empty());
    assert_eq!(texts(&s, &hit, cx), ["r0", "r1", "r2", "r3", "r4"]);
}

#[gpui::test]
async fn a_hit_without_context_is_asked_for_once(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = searching_provider();
    let st = memory_with(vec![]).await;
    let s = search_session(cx, &p, st);
    let gone = MessageId {
        conversation: cid("C1"),
        ts: domain::test_support::ts("1500000000.000000"),
    };
    s.update(cx, |s, cx| s.load_context(Some(gone.clone()), cx));
    settle(cx);
    let after_first = p.request_count();
    s.update(cx, |s, cx| s.load_context(None, cx));
    s.update(cx, |s, cx| s.load_context(Some(gone.clone()), cx));
    settle(cx);
    assert_eq!(p.request_count(), after_first, "an empty answer is kept");
    assert!(texts(&s, &gone, cx).is_empty());
}

#[gpui::test]
async fn in_narrow_windows_the_context_pane_gives_way_to_the_timeline(cx: &mut TestAppContext) {
    use themes::metrics::{LAYOUT_DETAIL_PANEL, LAYOUT_RAIL, LAYOUT_SIDEBAR};
    let _dir = init(cx);
    cx.executor().allow_parking();
    let data = tempfile::tempdir().unwrap();
    let (db, _guard) = cached_store(data.path()).await;
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider()));
    let st = Arc::new(db) as Arc<dyn Store>;
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.dispatch_action(Search);
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.search_panel().unwrap().clone());
    panel.update(cx, |p, cx| p.set_query("deploy", cx));
    for w in [800.0, 1134.0] {
        cx.simulate_resize(gpui::size(gpui::px(w), gpui::px(700.0)));
        cx.run_until_parked();
        let side = cx.debug_bounds("side-panel").expect("search open");
        // The hits alone, and the panel's left border.
        assert!(
            side.size.width <= LAYOUT_DETAIL_PANEL + gpui::px(1.0),
            "{w}"
        );
        assert!(cx.debug_bounds("search-preview").is_none(), "{w}");
        // What the timeline had beside a detail panel.
        let timeline = gpui::px(w) - LAYOUT_RAIL - LAYOUT_SIDEBAR - LAYOUT_DETAIL_PANEL;
        assert!(side.left() - LAYOUT_RAIL - LAYOUT_SIDEBAR >= timeline - gpui::px(1.0));
    }
    cx.simulate_resize(gpui::size(gpui::px(1600.0), gpui::px(700.0)));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("search-preview").is_some(),
        "a wide window shows it"
    );
}
