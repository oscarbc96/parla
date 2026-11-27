//! `WorkspaceSession` over the encrypted `SqliteStore`: cold start from the
//! cache, restart-surviving outbox and drafts, unreadable-store banner and
//! rebuild, and sync on reconnect. `MemoryKeySource`, temp dirs, `FakeProvider`;
//! no keychain, no network.

mod support;

use std::sync::Arc;

use domain::test_support::{Call, Op};
use domain::test_support::{FakeProvider, cid, conv, member, msg, ts, uid};
use domain::{ChatProvider, ConversationCounts, DraftKey, ProviderError, ProviderEvent};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::session::{StoreHealth, WorkspaceSession};
use settings::WorkspaceKey;
use store::Store as _;
use store_sqlite::{KeySource, MemoryKeySource, SqliteConfig, SqliteStore};
use support::{settle, settle_until};

struct Env {
    /// Every store started for this test, shut down when the test ends.
    stores: std::sync::Mutex<Vec<SqliteStore>>,
    dir: tempfile::TempDir,
    keys: Arc<MemoryKeySource>,
    provider: Arc<FakeProvider>,
}

fn env() -> Env {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_conversation(conv("C2", "random"));
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    Env {
        stores: Default::default(),
        dir: tempfile::tempdir().unwrap(),
        keys: Arc::new(MemoryKeySource::default()),
        provider,
    }
}

impl Drop for Env {
    /// gpui does not drop a test's entities before the process exits, so the
    /// sessions keep their store clones alive. SQLCipher's `atexit` hook then
    /// frees its crypto state under the still-running store thread (SIGSEGV),
    /// so close the stores explicitly.
    fn drop(&mut self) {
        for s in self.stores.lock().unwrap().drain(..) {
            s.shutdown();
        }
    }
}

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}

fn sqlite(e: &Env) -> SqliteStore {
    let s = SqliteStore::start(
        SqliteConfig {
            dir: e.dir.path().to_path_buf(),
            cache_mb: 8,
        },
        e.keys.clone(),
    );
    e.stores.lock().unwrap().push(s.clone());
    s
}

fn start(cx: &mut TestAppContext, e: &Env, db: &SqliteStore) -> Entity<WorkspaceSession> {
    cx.executor().allow_parking();
    let p: Arc<dyn ChatProvider> = e.provider.clone();
    let db = db.clone();
    let codec = domain::NoExtCodec;
    cx.new(|cx| WorkspaceSession::new_sqlite(p, db, Arc::new(codec), cx))
}

fn block<T>(f: impl std::future::Future<Output = T>) -> T {
    futures::executor::block_on(f)
}

/// A read answers `Closed` until the session has opened its store: the key
/// and the migrations run on the store's writer thread, so how long that takes
/// depends on machine load. Wait for the store instead of the clock.
fn store_read<T>(r: Result<T, store::StoreError>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(store::StoreError::Closed) => None,
        Err(e) => panic!("store read failed: {e}"),
    }
}

#[gpui::test]
async fn cold_start_paints_from_cache_before_bootstrap(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s1 = start(cx, &e, &db);
    // Bootstrap must have landed before the open, or the open races it.
    settle_until(cx, |cx| {
        s1.read_with(cx, |s, _| s.state().conversations.len() == 2)
    });
    s1.update(cx, |s, cx| s.open(cid("C1"), cx));
    settle_until(cx, |_| {
        block(db.latest_all(&ws())).map(|m| m.len()).unwrap_or(0) == 1
    });
    drop(s1);
    db.shutdown(); // a real restart closes the first launch's store
    // Second launch: the network is down, bootstrap fails.
    e.provider
        .fail_next(Op::Bootstrap, ProviderError::Network("offline".into()));
    let db = sqlite(&e);
    let s2 = start(cx, &e, &db);
    // Conversations and the open window's messages load from the cache in
    // separate store jobs, so wait for both before asserting.
    settle_until(cx, |cx| {
        s2.read_with(cx, |s, _| {
            s.state().conversations.len() == 2
                && s.state()
                    .windows
                    .get(&cid("C1"))
                    .is_some_and(|w| !w.messages().is_empty())
        })
    });
    s2.read_with(cx, |s, _| {
        assert_eq!(s.store_health(), StoreHealth::Ok);
        let w = s.state().windows.get(&cid("C1")).expect("cached window");
        assert_eq!(w.messages().len(), 1);
    });
}

#[gpui::test]
async fn reconnect_catches_up_without_duplicates(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    settle(cx);
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    settle_until(cx, |_| {
        block(db.latest_all(&ws())).map(|m| m.len()).unwrap_or(0) == 1
    });
    e.provider.add_message(msg("C1", "2.000000", "missed a"));
    e.provider.add_message(msg("C1", "3.000000", "missed b"));
    e.provider.set_counts(vec![
        ConversationCounts {
            conversation: cid("C1"),
            latest: Some(ts("3.000000")),
            last_read: None,
            unread: false,
            mentions: 0,
        },
        ConversationCounts {
            conversation: cid("C2"),
            latest: None,
            last_read: None,
            unread: false,
            mentions: 0,
        },
    ]);
    e.provider
        .push(ProviderEvent::Connected { reconnect: true });
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| {
            s.state().windows[&cid("C1")].messages().len() == 3
        })
    });
    settle(cx);
    s.read_with(cx, |s, _| {
        let t: Vec<_> = s.state().windows[&cid("C1")]
            .messages()
            .iter()
            .map(|m| m.id.ts.0.to_string())
            .collect();
        assert_eq!(t, ["1.000000", "2.000000", "3.000000"]);
    });
    // Bounded: one request for the one conversation that moved, none for C2.
    let between: Vec<_> = e
        .provider
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::HistoryBetween { conversation, .. } => Some(conversation),
            _ => None,
        })
        .collect();
    assert_eq!(between, vec![cid("C1")]);
}

#[gpui::test]
async fn reconnect_records_gap_when_far_behind(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    // The store opens on its thread and the first sync follows the bootstrap:
    // both must land before an open and a reconnect mean anything.
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.sync_progress().done() && !s.connecting())
    });
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    settle_until(cx, |_| {
        block(db.latest_all(&ws())).map(|m| m.len()).unwrap_or(0) == 1
    });
    for i in 0..650 {
        e.provider
            .add_message(msg("C1", &format!("{}.000000", 10 + i), "m"));
    }
    e.provider.set_counts(vec![ConversationCounts {
        conversation: cid("C1"),
        latest: Some(ts("659.000000")),
        last_read: None,
        unread: false,
        mentions: 0,
    }]);
    e.provider
        .push(ProviderEvent::Connected { reconnect: true });
    settle_until(cx, |_| {
        store_read(block(db.gaps(&ws(), &cid("C1")))).is_some_and(|g| !g.is_empty())
    });
    let gaps = store_read(block(db.gaps(&ws(), &cid("C1")))).unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].after, ts("1.000000"));
}

#[gpui::test]
async fn outbox_survives_restart(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s1 = start(cx, &e, &db);
    settle(cx);
    s1.update(cx, |s, cx| {
        s.open(cid("C1"), cx);
        for t in ["one", "two", "three"] {
            s.send(cid("C1"), t.to_string(), cx);
        }
    });
    settle_until(cx, |_| {
        store_read(block(db.outbox(&ws()))).is_some_and(|o| o.len() == 3)
    });
    drop(s1);
    db.shutdown(); // a real restart closes the first launch's store
    let db = sqlite(&e);
    let s2 = start(cx, &e, &db);
    e.provider
        .push(ProviderEvent::Connected { reconnect: false });
    settle_until(cx, |_| {
        e.provider
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Send { .. }))
            .count()
            >= 3
    });
    settle(cx);
    let sent: Vec<String> = e
        .provider
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Send { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["one", "two", "three"]);
    drop(s2);
}

#[gpui::test]
async fn draft_survives_restart(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let key = DraftKey {
        conversation: cid("C1"),
        thread: None,
    };
    let s1 = start(cx, &e, &db);
    settle(cx);
    s1.update(cx, |s, cx| s.set_draft(key.clone(), "half a thought", cx));
    settle_until(cx, |_| {
        store_read(block(db.drafts(&ws()))).is_some_and(|d| d.len() == 1)
    });
    drop(s1);
    db.shutdown(); // a real restart closes the first launch's store
    let db = sqlite(&e);
    let s2 = start(cx, &e, &db);
    settle_until(cx, |cx| s2.read_with(cx, |s, _| s.draft(&key).is_some()));
    s2.read_with(cx, |s, _| {
        assert_eq!(
            s.draft(&key).map(|d| d.text.to_string()).as_deref(),
            Some("half a thought")
        );
    });
}

#[gpui::test]
async fn draft_keeps_picked_mentions_across_restart(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let key = DraftKey {
        conversation: cid("C1"),
        thread: None,
    };
    let body = domain::ComposedText::new(
        "ping @Ana and see docs",
        vec![
            domain::ComposedSpan {
                range: 5..9,
                inline: domain::Inline::User {
                    id: domain::UserId("U1".into()),
                    label: Some("Ana".into()),
                },
            },
            domain::ComposedSpan {
                range: 18..22,
                inline: domain::Inline::Link {
                    url: "https://example.com/d".into(),
                    label: Some("docs".into()),
                    style: Default::default(),
                },
            },
        ],
    );
    let s1 = start(cx, &e, &db);
    settle(cx);
    s1.update(cx, |s, cx| s.set_draft(key.clone(), body.clone(), cx));
    settle_until(cx, |_| {
        store_read(block(db.drafts(&ws()))).is_some_and(|d| d.len() == 1)
    });
    drop(s1);
    db.shutdown();
    let db = sqlite(&e);
    let s2 = start(cx, &e, &db);
    settle_until(cx, |cx| s2.read_with(cx, |s, _| s.draft(&key).is_some()));
    s2.read_with(cx, |s, _| assert_eq!(s.draft(&key), Some(body)));
}

async fn make_unreadable(cx: &mut TestAppContext, e: &Env) -> Vec<u8> {
    let db = sqlite(e);
    let s1 = start(cx, e, &db);
    settle(cx);
    s1.update(cx, |s, cx| s.open(cid("C1"), cx));
    settle_until(cx, |_| {
        block(db.latest_all(&ws())).map(|m| m.len()).unwrap_or(0) == 1
    });
    let path = db.path(&ws());
    drop(s1);
    cx.run_until_parked();
    block(db.close(&ws())).unwrap(); // checkpoints the WAL into the file
    db.shutdown(); // a real restart closes the first launch's store
    block(e.keys.replace(&store_sqlite::DbId::Workspace(ws()))).unwrap();
    std::fs::read(path).unwrap()
}

#[gpui::test]
async fn unreadable_store_shows_banner_and_keeps_running(cx: &mut TestAppContext) {
    let e = env();
    let before = make_unreadable(cx, &e).await;
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.state().conversations.len() == 2)
    });
    s.read_with(cx, |s, _| {
        assert_eq!(s.store_health(), StoreHealth::Unreadable);
        let chain = i18n::chain(
            &i18n::LocalePref::Code("en".parse().unwrap()),
            &[],
            i18n::LocaleSet::embedded(),
        );
        let l = i18n::Localizer::new(chain, &[parla::catalog()]).unwrap();
        let text = parla::store_banner::text(&l, &s.state().name);
        assert!(text.contains("cannot be opened"), "{text}");
        assert!(text.contains("cannot be recovered"), "{text}");
    });
    // Messages still load from the provider and the file is untouched.
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| {
            s.state()
                .windows
                .get(&cid("C1"))
                .is_some_and(|w| w.messages().len() == 1)
        })
    });
    assert_eq!(std::fs::read(db.path(&ws())).unwrap(), before);
}

#[gpui::test]
async fn rebuild_resets_key_and_file(cx: &mut TestAppContext) {
    let e = env();
    let before = make_unreadable(cx, &e).await;
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.store_health() == StoreHealth::Unreadable)
    });
    s.update(cx, |s, cx| s.rebuild_cache(cx));
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.store_health() == StoreHealth::Ok)
    });
    assert_ne!(std::fs::read(db.path(&ws())).unwrap(), before);
    drop(s);
    db.shutdown(); // a real restart closes the first launch's store
    // The next launch opens the rebuilt file with the new key.
    let db = sqlite(&e);
    let s2 = start(cx, &e, &db);
    settle(cx);
    s2.read_with(cx, |s, _| assert_eq!(s.store_health(), StoreHealth::Ok));
}

#[gpui::test]
async fn store_writes_happen_off_main_thread(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    settle(cx);
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    settle_until(cx, |_| db.last_writer_thread().is_some());
    assert_eq!(db.last_writer_thread().as_deref(), Some("parla-store"));
}

#[gpui::test]
async fn startup_prunes_old_messages_in_background(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s1 = start(cx, &e, &db);
    settle_until(cx, |cx| {
        s1.read_with(cx, |s, _| s.state().conversations.len() == 2)
    });
    // C2 was never opened, so its ancient message is past retention.
    block(db.apply(
        &ws(),
        domain::PersistOp::Messages {
            conversation: cid("C2"),
            messages: vec![msg("C2", "1000.000001", "ancient")],
        },
    ))
    .unwrap();
    let count = |db: &SqliteStore| {
        block(db.read(&ws(), |c, _| {
            c.query_row(
                "SELECT count(*) FROM messages WHERE conversation_id = 'C2'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| store::StoreError::Other(e.to_string()))
        }))
        .unwrap()
    };
    assert_eq!(count(&db), 1);
    drop(s1);
    cx.run_until_parked();
    let _s2 = start(cx, &e, &db);
    settle_until(cx, |_| count(&db) == 0);
}

/// A first launch (the store opens empty) must still reach the bootstrap and
/// paint the conversations the provider returns.
#[gpui::test]
async fn empty_store_still_bootstraps_and_paints(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| {
            s.state().conversations.len() == 2 && !s.connecting()
        })
    });
}

#[gpui::test]
async fn failed_reopen_after_lean_falls_back_to_memory(cx: &mut TestAppContext) {
    let e = env();
    let db = sqlite(&e);
    let s = start(cx, &e, &db);
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.state().conversations.len() == 2)
    });
    s.update(cx, |s, cx| s.set_lean(true, cx));
    settle(cx);
    // The key changes while the store is closed, so the reopen cannot read the file.
    block(e.keys.replace(&store_sqlite::DbId::Workspace(ws()))).unwrap();
    s.update(cx, |s, cx| s.set_lean(false, cx));
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.store_health() == StoreHealth::Unreadable)
    });
}
