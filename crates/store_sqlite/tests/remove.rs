use std::sync::Arc;

use domain::NoExtCodec;
use settings::WorkspaceKey;
use store_sqlite::{KeySource, MemoryKeySource, SqliteConfig, SqliteStore};

fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

fn start(dir: &std::path::Path) -> (SqliteStore, Arc<MemoryKeySource>) {
    let keys = Arc::new(MemoryKeySource::default());
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        keys.clone(),
    );
    (s, keys)
}

async fn put_draft(s: &SqliteStore, w: &WorkspaceKey) {
    s.write(w, |c, _| {
        c.execute(
            "INSERT INTO drafts (conversation_id, body) VALUES ('C','hi')",
            [],
        )
        .map_err(|e| store::StoreError::Other(e.to_string()))?;
        Ok(())
    })
    .await
    .unwrap();
}

fn sibling(p: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut o = p.to_path_buf().into_os_string();
    o.push(suffix);
    o.into()
}

#[tokio::test]
async fn remove_deletes_file_wal_and_key() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path());
    let w = ws("W1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    put_draft(&s, &w).await;
    let path = s.path(&w);
    assert!(path.exists());
    let old = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    s.remove(&w).await.unwrap();
    assert!(!path.exists());
    assert!(!sibling(&path, "-wal").exists());
    assert!(!sibling(&path, "-shm").exists());
    // The key is gone: asking again creates a different one.
    let new = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    assert_ne!(old, new);
    // The workspace is closed.
    assert_eq!(
        s.read(&w, |_, _| Ok(())).await,
        Err(store::StoreError::Closed)
    );
}

#[tokio::test]
async fn remove_leaves_other_workspaces() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path());
    let (a, b) = (ws("A"), ws("B"));
    for w in [&a, &b] {
        s.open(w, Arc::new(NoExtCodec)).await.unwrap();
        put_draft(&s, w).await;
    }
    let key_b = keys
        .get_or_create(&store_sqlite::DbId::Workspace(b.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    s.remove(&a).await.unwrap();
    assert!(s.path(&b).exists());
    assert_eq!(
        keys.get_or_create(&store_sqlite::DbId::Workspace(b.clone()))
            .await
            .unwrap()
            .to_hex()
            .to_string(),
        key_b
    );
    let n = s
        .read(&b, |c, _| {
            c.query_row("SELECT count(*) FROM drafts", [], |r| r.get::<_, i64>(0))
                .map_err(|e| store::StoreError::Other(e.to_string()))
        })
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn remove_works_when_not_open() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path());
    let w = ws("W1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    s.close(&w).await.unwrap();
    let old = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    s.remove(&w).await.unwrap();
    assert!(!s.path(&w).exists());
    let new = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    assert_ne!(old, new);
}

/// A database left at schema version 3 (M2) gains the M3 tables on open.
#[tokio::test]
async fn migration_0004_applies_on_m2_database() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path());
    let w = ws("W1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    s.close(&w).await.unwrap();
    // Rewind: drop the M3 objects and set the version back to 3.
    let key = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap();
    {
        let c = rusqlite::Connection::open(s.path(&w)).unwrap();
        c.execute_batch(&format!("PRAGMA key = \"x'{}'\"", key.to_hex().as_str()))
            .unwrap();
        c.execute_batch(
            "DROP TABLE local_schedules; DROP TABLE recent_emoji; DROP TABLE local_notify_prefs; DROP TABLE local_notify_extras; DROP TABLE local_starred; DROP TABLE local_sidebar_collapsed; DROP TABLE local_reminders; DROP TABLE user_groups; DROP TABLE opened; DROP INDEX messages_author; DROP INDEX messages_files; \
             ALTER TABLE conversations DROP COLUMN topic; \
             ALTER TABLE conversations DROP COLUMN member_count; \
             ALTER TABLE conversations DROP COLUMN purpose; \
             ALTER TABLE conversations DROP COLUMN is_open; DROP INDEX gaps_conversation; \
             ALTER TABLE drafts DROP COLUMN spans; ALTER TABLE drafts DROP COLUMN edited_at; \
             ALTER TABLE members DROP COLUMN title; ALTER TABLE members DROP COLUMN tz; ALTER TABLE members DROP COLUMN avatar; ALTER TABLE members DROP COLUMN avatar_lg; \
             PRAGMA user_version = 3;",
        )
        .unwrap();
    }
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    let n = s
        .read(&w, |c, _| {
            c.query_row(
                "SELECT (SELECT count(*) FROM local_schedules) + (SELECT count(*) FROM recent_emoji) \
                 + (SELECT count(*) FROM local_notify_prefs) + (SELECT count(*) FROM local_starred) \
                 + (SELECT count(*) FROM local_sidebar_collapsed) + (SELECT count(*) FROM local_reminders)",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| store::StoreError::Other(e.to_string()))
        })
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(s.pragma(&w, "user_version").await.unwrap(), "22");
}
