//! `app.db`: encryption, its own key, the shared writer thread and the cache split.

use std::path::Path;
use std::sync::Arc;

use domain::NoExtCodec;
use settings::WorkspaceKey;
use store::{AppStore, AuditEntry, AuditOutcome, StoreError, app_contract};
use store_sqlite::{DbId, KeySource, MemoryKeySource, SqliteConfig, SqliteStore};

fn start(dir: &Path, cache_mb: u32) -> (SqliteStore, Arc<MemoryKeySource>) {
    let keys = Arc::new(MemoryKeySource::default());
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb,
        },
        keys.clone(),
    );
    (s, keys)
}

fn entry(content: Option<&str>) -> AuditEntry {
    AuditEntry {
        at_ms: 1,
        client: Arc::from("client"),
        peer_pid: None,
        tool: Arc::from("read_messages"),
        workspace: Some(WorkspaceKey::new("fake", "a")),
        target: None,
        outcome: AuditOutcome::Ok,
        detail: None,
        decision: None,
        duration_ms: 1,
        content: content.map(Arc::from),
    }
}

struct Fixture {
    _store: SqliteStore,
    app: Arc<dyn AppStore>,
    _dir: tempfile::TempDir,
}

async fn sqlite() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = start(dir.path(), 8);
    store.open_app().await.unwrap();
    let app = store.app();
    Fixture {
        _store: store,
        app,
        _dir: dir,
    }
}

macro_rules! case {
    ($($name:ident),* $(,)?) => {$(
        #[tokio::test]
        async fn $name() {
            let f = sqlite().await;
            app_contract::$name(&*f.app).await;
        }
    )*};
}

case!(
    append_then_page_newest_first,
    page_filters_workspace_tool_and_writes,
    page_continues_before_id,
    prune_drops_only_older,
    forget_workspace_drops_only_its_rows,
    clear_empties,
);

#[tokio::test]
async fn app_db_is_encrypted_with_own_key() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path(), 8);
    s.open_app().await.unwrap();
    s.close_app().await.unwrap();
    let bytes = std::fs::read(s.app_path()).unwrap();
    assert!(bytes.len() >= 16);
    assert_ne!(&bytes[..16], b"SQLite format 3\0");
    // A key of its own, apart from every workspace key.
    let app = keys.get_or_create(&DbId::App).await.unwrap();
    let w = keys
        .get_or_create(&DbId::Workspace(WorkspaceKey::new("fake", "a")))
        .await
        .unwrap();
    assert_ne!(app.to_hex().as_str(), w.to_hex().as_str());
    assert_eq!(s.app_path(), d.path().join("app.db"));
}

#[tokio::test]
async fn app_db_wrong_key_is_unreadable_not_wiped() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path(), 8);
    s.open_app().await.unwrap();
    s.app().audit_append(entry(None)).await.unwrap();
    s.close_app().await.unwrap();
    let before = std::fs::read(s.app_path()).unwrap();
    keys.replace(&DbId::App).await.unwrap();
    assert_eq!(s.open_app().await, Err(StoreError::Unreadable));
    assert_eq!(std::fs::read(s.app_path()).unwrap(), before);
    // rebuild_app starts over with a fresh key.
    s.rebuild_app().await.unwrap();
    assert!(
        s.app()
            .audit_page(Default::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn audit_rows_unreadable_on_disk() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    s.open_app().await.unwrap();
    let marker = "Zq7Lm2Xv9Kp4Wd8R";
    s.app().audit_append(entry(Some(marker))).await.unwrap();
    s.close_app().await.unwrap(); // last connection closed: WAL checkpointed
    let mut raw = std::fs::read(s.app_path()).unwrap();
    let wal = d.path().join("app.db-wal");
    if let Ok(w) = std::fs::read(wal) {
        raw.extend(w);
    }
    assert!(!raw.windows(marker.len()).any(|w| w == marker.as_bytes()));
}

#[tokio::test]
async fn app_writes_run_on_store_thread() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    s.open_app().await.unwrap();
    s.app().audit_append(entry(None)).await.unwrap();
    assert_eq!(s.last_writer_thread().as_deref(), Some("parla-store"));
}

#[tokio::test]
async fn cache_budget_counts_app_db() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    s.open_app().await.unwrap();
    let w = WorkspaceKey::new("fake", "a");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    // 8 MiB across two databases: -4096 KiB each.
    assert_eq!(s.pragma(&w, "cache_size").await.unwrap(), "-4096");
    assert_eq!(s.app_pragma("cache_size").await.unwrap(), "-4096");
}
