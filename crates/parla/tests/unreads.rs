//! Sidebar unreads, mark unread and draft markers on the session, and the
//! composer drafts kept by the main window.

use std::sync::Arc;

use components::{ClosePanel, SidebarSource as _};
use domain::test_support::{Call, FakeFactory, FakeProvider, cid, conv, member, msg, ts, uid};
use domain::{DraftKey, MessageId};
use gpui::{AppContext as _, TestAppContext};
use parla::actions::RebuildCache;
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use parla::session::{StoreHealth, WorkspaceSession};
use settings::ConfigPaths;
use store::{MemoryStore, Store};
use store_sqlite::{KeySource as _, MemoryKeySource, SqliteConfig, SqliteStore};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        // Tests never depend on the OS language.
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn provider() -> Arc<FakeProvider> {
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Other"));
    p.add_message(msg("C1", "1.000000", "one"));
    p.add_message(msg("C1", "2.000000", "two"));
    p.add_message(msg("C2", "1.000000", "other"));
    p
}

fn key(c: &str) -> DraftKey {
    DraftKey {
        conversation: cid(c),
        thread: None,
    }
}

fn store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

#[gpui::test]
async fn draft_marker_in_sidebar_rows(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let s = cx.new(|cx| WorkspaceSession::new(p, store(), cx));
    cx.run_until_parked();
    s.update(cx, |s, cx| s.set_draft(key("C2"), "wip", cx));
    let rows = s.read_with(cx, |s, cx| s.rows(cx));
    let marked: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            components::SidebarRow::Conversation {
                id, draft: true, ..
            } => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(marked, vec![cid("C2")]);
    s.update(cx, |s, cx| s.set_draft(key("C2"), "  ", cx));
    let rows = s.read_with(cx, |s, cx| s.rows(cx));
    assert!(
        rows.iter()
            .all(|r| !matches!(r, components::SidebarRow::Conversation { draft: true, .. }))
    );
}

#[gpui::test]
async fn mark_unread_moves_the_marker_on_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let s = cx.new(|cx| WorkspaceSession::new(p.clone(), store(), cx));
    cx.run_until_parked();
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    s.update(cx, |s, cx| {
        s.mark_unread(
            MessageId {
                conversation: cid("C1"),
                ts: ts("2.000000"),
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(
        p.calls().iter().any(|c| matches!(
            c,
            Call::MarkRead { conversation, ts: t } if *conversation == cid("C1") && *t == ts("1.000000")
        )),
        "{:?}",
        p.calls()
    );
    assert!(s.read_with(cx, |s, _| s.state().conversations[&cid("C1")].unread));
}

async fn ready_view(
    cx: &mut TestAppContext,
    p: Arc<FakeProvider>,
    st: Arc<dyn Store>,
) -> (gpui::Entity<RootView>, &mut gpui::VisualTestContext) {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let (view, vcx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui::test]
async fn composer_draft_saved_on_switch_and_restored(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = ready_view(cx, provider(), store()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    composer.update(cx, |c, cx| c.set_text("half a thought".into(), cx));
    session.update(cx, |s, cx| s.open(cid("C2"), cx));
    cx.run_until_parked();
    assert_eq!(
        session.read_with(cx, |s, _| s.draft(&key("C1")).map(|d| d.text.to_string())),
        Some("half a thought".to_string())
    );
    assert_eq!(composer.read_with(cx, |c, cx| c.text(cx).to_string()), "");
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    assert_eq!(
        composer.read_with(cx, |c, cx| c.text(cx).to_string()),
        "half a thought"
    );
}

#[gpui::test]
async fn quit_flushes_the_open_drafts(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let st = Arc::new(MemoryStore::default());
    let (view, cx) = ready_view(cx, provider(), st.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    composer.update(cx, |c, cx| c.set_text("typed".into(), cx));
    let task = view.update(cx, |v, cx| v.save_drafts(cx));
    task.await;
    let ws = session.read_with(cx, |s, _| s.state().workspace.clone());
    let saved = st.drafts(&ws).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(&*saved[0].1.text, "typed");
}

#[gpui::test]
async fn thread_draft_saved_on_close_and_restored(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = ready_view(cx, provider(), store()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let root = MessageId {
        conversation: cid("C1"),
        ts: ts("1.000000"),
    };
    view.update_in(cx, |v, window, cx| {
        v.open_thread_panel(root.clone(), window, cx)
    });
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.thread_panel().unwrap().clone());
    let composer = panel.read_with(cx, |p, _| p.composer().clone());
    composer.update(cx, |c, cx| c.set_text("reply wip".into(), cx));
    cx.dispatch_action(ClosePanel);
    cx.run_until_parked();
    let tkey = DraftKey {
        conversation: cid("C1"),
        thread: Some(ts("1.000000")),
    };
    assert_eq!(
        session.read_with(cx, |s, _| s.draft(&tkey).map(|d| d.text.to_string())),
        Some("reply wip".to_string())
    );
    view.update_in(cx, |v, window, cx| {
        v.open_thread_panel(root.clone(), window, cx)
    });
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.thread_panel().unwrap().clone());
    let composer = panel.read_with(cx, |p, _| p.composer().clone());
    assert_eq!(
        composer.read_with(cx, |c, cx| c.text(cx).to_string()),
        "reply wip"
    );
}

/// Closes the store when the test ends. gpui keeps a test's entities (and so
/// their store clones) alive to process exit, and SQLCipher's `atexit` hook
/// frees its crypto state under a still-running store thread (SIGSEGV).
struct Shutdown(SqliteStore);

impl Drop for Shutdown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

fn sqlite(dir: &std::path::Path, keys: &Arc<MemoryKeySource>) -> SqliteStore {
    SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        keys.clone(),
    )
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
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("condition not reached");
}

#[gpui::test]
async fn rebuild_cache_action_rebuilds_an_unreadable_store(cx: &mut TestAppContext) {
    let _dir = init(cx);
    cx.executor().allow_parking();
    let data = tempfile::tempdir().unwrap();
    let keys = Arc::new(MemoryKeySource::default());
    let ws = settings::WorkspaceKey::new("fake", "w1");
    // A first launch creates the file under the first key, then the key is lost.
    let db = sqlite(data.path(), &keys);
    futures::executor::block_on(db.open(&ws, Arc::new(domain::NoExtCodec))).unwrap();
    futures::executor::block_on(db.close(&ws)).unwrap();
    drop(db);
    futures::executor::block_on(keys.replace(&store_sqlite::DbId::Workspace(ws.clone()))).unwrap();

    let db = sqlite(data.path(), &keys);
    let _shutdown = Shutdown(db.clone());
    cx.update(|cx| cx.set_global(parla::root_view::SqliteHandle(db.clone())));
    let (view, cx) = ready_view(cx, provider(), Arc::new(db) as Arc<dyn Store>).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    settle_until(cx, |cx| {
        session.read_with(cx, |s, _| s.store_health() == StoreHealth::Unreadable)
    });
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    cx.dispatch_action(RebuildCache);
    settle_until(cx, |cx| {
        session.read_with(cx, |s, _| s.store_health() == StoreHealth::Ok)
    });
}

#[gpui::test]
async fn next_unread_opens_from_anywhere(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = ready_view(cx, provider(), store()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    session.update(cx, |s, cx| {
        s.mark_unread(
            MessageId {
                conversation: cid("C1"),
                ts: ts("2.000000"),
            },
            cx,
        )
    });
    session.update(cx, |s, cx| s.open(cid("C2"), cx));
    cx.run_until_parked();
    cx.dispatch_action(components::NextUnread);
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C1"))
    );
}
