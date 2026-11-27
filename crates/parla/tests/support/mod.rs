//! Shared setup for the multi-workspace tests: three fake accounts over a real
//! encrypted store in scratch directories (never the owner's data).
#![allow(dead_code)]

pub mod call_fake;
pub mod mcp_env;

use std::path::PathBuf;
use std::sync::Arc;

use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, msg, uid};
use domain::{AccountInfo, ChatProvider, ConversationKind};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::providers::ProviderRegistry;
use parla::root_view::SqliteHandle;
use parla::workspaces::Workspaces;
use settings::{ConfigPaths, WorkspaceKey};
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

pub struct Env {
    pub dir: tempfile::TempDir,
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub providers: Vec<Arc<FakeProvider>>,
    pub registry: ProviderRegistry,
    pub sqlite: SqliteStore,
    pub keys: Arc<MemoryKeySource>,
}

impl Drop for Env {
    /// SQLCipher's `atexit` hook frees its crypto state under a still running
    /// store thread, so close the store explicitly.
    fn drop(&mut self) {
        self.sqlite.shutdown();
    }
}

pub fn key(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

/// Three accounts `w1..w3`, each with a DM `D1` holding one message.
pub fn env(cx: &mut TestAppContext, settings_json: &str) -> Env {
    let providers = ["w1", "w2", "w3"]
        .iter()
        .map(|id| {
            let p = Arc::new(FakeProvider::new(id, uid("u1")));
            let mut dm = conv("D1", "dm");
            dm.kind = ConversationKind::Direct;
            p.add_conversation(dm);
            p.add_member(member("u1", "Me"));
            p.add_member(member("u2", "Other"));
            p.add_message(msg("D1", "1.000000", "hi"));
            p
        })
        .collect();
    env_with(cx, settings_json, providers)
}

/// One account per provider over a real encrypted store in scratch dirs.
pub fn env_with(
    cx: &mut TestAppContext,
    settings_json: &str,
    providers: Vec<Arc<FakeProvider>>,
) -> Env {
    env_with_parser(cx, settings_json, providers, None)
}

/// [`env_with`] whose providers parse message markup with `parser`.
pub fn env_with_parser(
    cx: &mut TestAppContext,
    settings_json: &str,
    providers: Vec<Arc<FakeProvider>>,
    parser: Option<Arc<dyn domain::ContentParser>>,
) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let (config, data, cache) = (
        dir.path().join("config"),
        dir.path().join("data"),
        dir.path().join("cache"),
    );
    for d in [&config, &data, &cache] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(config.join("settings.json"), settings_json).unwrap();
    let paths = ConfigPaths::at(&config);
    cx.update(|cx| parla::init_without_watcher(cx, paths));
    let keys = Arc::new(MemoryKeySource::default());
    let sqlite = SqliteStore::start(
        SqliteConfig {
            dir: data.join("store"),
            cache_mb: 8,
        },
        keys.clone(),
    );
    let sq = sqlite.clone();
    cx.update(|cx| cx.set_global(SqliteHandle(sq)));
    let mut factory = FakeFactory::default();
    if let Some(p) = parser {
        factory = factory.with_parser(p);
    }
    let mut infos = Vec::new();
    for p in &providers {
        let key = p.workspace().clone();
        infos.push(AccountInfo {
            name: Arc::from(key.id.0.as_str()),
            key,
            org: None,
            icon: None,
        });
        factory = factory.with_provider(p.clone());
    }
    factory.set_accounts(infos);
    let mut registry = ProviderRegistry::new();
    registry.add(factory);
    cx.executor().allow_parking();
    Env {
        dir,
        config,
        data,
        cache,
        providers,
        registry,
        sqlite,
        keys,
    }
}

pub fn workspaces(cx: &mut TestAppContext, e: &Env) -> Entity<Workspaces> {
    let (registry, data, cache) = (e.registry.clone(), e.data.clone(), e.cache.clone());
    let w = cx.new(|cx| {
        let store: Arc<dyn store::Store> = Arc::new(store::MemoryStore::default());
        let mut w = Workspaces::new(&registry, store, cx).with_dirs(data, cache);
        w.start_all(cx);
        w
    });
    settle(cx);
    // Every store loads on its own threads: a busy machine takes longer than
    // the fixed settle above, so wait for the loaded state itself.
    let ready = w.clone();
    settle_until(cx, move |cx| {
        ready.read_with(cx, |w, cx| {
            // Loaded store and finished bootstrap (conversations known): an
            // event pushed earlier would find no conversation to belong to.
            w.ready_to_lock(cx)
                && w.sessions()
                    .all(|s| s.read(cx).sync_progress().directory.is_some())
        })
    });
    w
}

/// The store runs on its own threads, so give it real time to answer.
pub fn settle(cx: &mut TestAppContext) {
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.run_until_parked();
}

/// Run the executor until `done` holds. The store and the fake providers
/// answer from their own threads, so how long that takes depends on machine
/// load: wait for the condition itself, bounded only by a generous deadline
/// that a hung test hits instead of hanging the suite.
pub fn settle_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "condition not reached within 60 s"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// `settle_until` for a window's `VisualTestContext`.
pub fn settle_visual_until(
    cx: &mut gpui::VisualTestContext,
    mut done: impl FnMut(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "condition not reached within 60 s"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub async fn store_open(e: &Env, id: &str) -> bool {
    e.sqlite.read(&key(id), |_, _| Ok(1u8)).await.is_ok()
}

/// Wait (real time, bounded) until `id`'s encrypted store is open or closed.
/// Opening and closing answer from the store's own threads, so how long that
/// takes depends on machine load: wait for the answer, not the clock.
pub fn wait_store(cx: &mut TestAppContext, e: &Env, id: &str, open: bool) {
    let (db, k) = (e.sqlite.clone(), key(id));
    settle_until(cx, move |_| {
        match futures::executor::block_on(db.read(&k, |_, _| Ok(1u8))) {
            Ok(_) => open,
            Err(store::StoreError::Closed) => !open,
            Err(e) => panic!("store read failed: {e}"),
        }
    });
}

pub fn wait_store_open(cx: &mut TestAppContext, e: &Env, id: &str) {
    wait_store(cx, e, id, true);
}

pub fn wait_store_closed(cx: &mut TestAppContext, e: &Env, id: &str) {
    wait_store(cx, e, id, false);
}

pub fn dm(c: &str) -> domain::ConversationId {
    cid(c)
}

/// A localizer over `catalogs` in `code`, from the embedded locales.
pub fn localizer(code: &str, catalogs: &[&'static i18n::Catalog]) -> i18n::Localizer {
    let chain = i18n::chain(
        &i18n::LocalePref::Code(code.parse().unwrap()),
        &[],
        i18n::LocaleSet::embedded(),
    );
    i18n::Localizer::new(chain, catalogs).unwrap()
}

/// A session over an in-memory store, for provider effects that need no cache.
pub fn memory_session(
    cx: &mut TestAppContext,
    provider: Arc<FakeProvider>,
) -> Entity<parla::session::WorkspaceSession> {
    let p: Arc<dyn ChatProvider> = provider;
    let st: Arc<dyn store::Store> = Arc::new(store::MemoryStore::default());
    let session = cx.new(|cx| parla::session::WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    session
}
