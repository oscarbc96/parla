use std::path::Path;
use std::sync::Arc;

use domain::NoExtCodec;
use settings::WorkspaceKey;
use store_sqlite::{Key, KeySource, MemoryKeySource, SqliteConfig, SqliteStore};

fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

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

fn codec() -> Arc<NoExtCodec> {
    Arc::new(NoExtCodec)
}

async fn pragma(s: &SqliteStore, w: &WorkspaceKey, name: &str) -> String {
    s.pragma(w, name).await.unwrap()
}

#[tokio::test]
async fn opens_encrypted_db() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    s.close(&w).await.unwrap();
    let bytes = std::fs::read(s.path(&w)).unwrap();
    assert!(bytes.len() >= 16);
    assert_ne!(&bytes[..16], b"SQLite format 3\0");
}

#[tokio::test]
async fn wrong_key_is_unreadable_not_wiped() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path(), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    s.write(&w, |tx, _| {
        tx.execute(
            "INSERT INTO drafts (conversation_id, body) VALUES ('C','hi')",
            [],
        )
        .map_err(|e| store::StoreError::Other(e.to_string()))?;
        Ok(())
    })
    .await
    .unwrap();
    s.close(&w).await.unwrap();
    let before = std::fs::read(s.path(&w)).unwrap();
    keys.replace(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap();
    assert_eq!(
        s.open(&w, codec()).await,
        Err(store::StoreError::Unreadable)
    );
    assert_eq!(std::fs::read(s.path(&w)).unwrap(), before);
}

#[tokio::test]
async fn rebuild_starts_empty_with_new_key() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path(), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    s.write(&w, |tx, _| {
        tx.execute(
            "INSERT INTO drafts (conversation_id, body) VALUES ('C','hi')",
            [],
        )
        .map_err(|e| store::StoreError::Other(e.to_string()))?;
        Ok(())
    })
    .await
    .unwrap();
    s.close(&w).await.unwrap();
    let old = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    keys.replace(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap();
    assert_eq!(
        s.open(&w, codec()).await,
        Err(store::StoreError::Unreadable)
    );
    s.rebuild(&w).await.unwrap();
    let new = keys
        .get_or_create(&store_sqlite::DbId::Workspace(w.clone()))
        .await
        .unwrap()
        .to_hex()
        .to_string();
    assert_ne!(old, new);
    let n = s
        .read(&w, |c, _| {
            c.query_row("SELECT count(*) FROM drafts", [], |r| r.get::<_, i64>(0))
                .map_err(|e| store::StoreError::Other(e.to_string()))
        })
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn migrations_apply_once() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    s.close(&w).await.unwrap();
    s.open(&w, codec()).await.unwrap();
    assert_eq!(pragma(&s, &w, "user_version").await, "22");
}

#[tokio::test]
async fn pragmas_are_set() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    assert_eq!(pragma(&s, &w, "journal_mode").await, "wal");
    assert_eq!(pragma(&s, &w, "synchronous").await, "1");
    assert_eq!(pragma(&s, &w, "foreign_keys").await, "1");
    assert_eq!(pragma(&s, &w, "mmap_size").await, "0");
    assert_eq!(pragma(&s, &w, "auto_vacuum").await, "2");
}

#[tokio::test]
async fn cache_budget_split_across_dbs() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    let (a, b) = (ws("A"), ws("B"));
    s.open(&a, codec()).await.unwrap();
    assert_eq!(pragma(&s, &a, "cache_size").await, "-8192");
    s.open(&b, codec()).await.unwrap();
    assert_eq!(pragma(&s, &a, "cache_size").await, "-4096");
    assert_eq!(pragma(&s, &b, "cache_size").await, "-4096");
    s.close(&b).await.unwrap();
    assert_eq!(pragma(&s, &a, "cache_size").await, "-8192");
}

#[tokio::test]
async fn writes_run_on_store_thread() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    s.write(&w, |_, _| Ok(())).await.unwrap();
    assert_eq!(s.last_writer_thread().as_deref(), Some("parla-store"));
}

#[cfg(unix)]
#[tokio::test]
async fn file_is_0600_dir_0700() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(&d.path().join("store"), 8);
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&s.path(&w)), 0o600);
    assert_eq!(mode(s.path(&w).parent().unwrap()), 0o700);
}

#[test]
fn key_debug_is_redacted() {
    let k = Key::random();
    assert_eq!(format!("{k:?}"), "Key(REDACTED)");
    assert_eq!(k.to_hex().len(), 64);
    assert!(Key::from_hex(&k.to_hex()).is_some());
    assert!(Key::from_hex("zz").is_none());
}

#[test]
fn schema_names_no_provider() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut banned = vec![["sl", "ack"].concat()];
    for e in std::fs::read_dir(root.join("../providers")).unwrap() {
        let e = e.unwrap();
        if e.path().is_dir() {
            banned.push(e.file_name().to_string_lossy().to_lowercase());
        }
    }
    let mut n = 0;
    let mut plugins = false;
    for set in ["migrations/workspace", "migrations/app"] {
        for e in std::fs::read_dir(root.join(set)).unwrap() {
            let p = e.unwrap().path();
            plugins |= p.ends_with("app/0002_plugins.sql");
            let text = std::fs::read_to_string(&p).unwrap().to_lowercase();
            for b in &banned {
                assert!(!text.contains(b.as_str()), "{} names {b}", p.display());
            }
            n += 1;
        }
    }
    assert!(n > 0);
    assert!(plugins, "the plugin tables migration is checked");
}

/// A leaked clone (a gpui entity that outlives its test) keeps `Drop` from
/// ever running; `shutdown` must still close the databases and end the
/// threads, because SQLCipher's `atexit` hook frees global crypto state under
/// any thread still running (SIGSEGV in `parla`'s `store_wiring` tests).
#[tokio::test]
async fn shutdown_closes_databases_even_with_live_clones() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path(), 8);
    let leaked = s.clone();
    let w = ws("W1");
    s.open(&w, codec()).await.unwrap();
    s.write(&w, |c, _| {
        c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1)")
            .map_err(|e| store::StoreError::Other(e.to_string()))
    })
    .await
    .unwrap();
    let wal = {
        let mut p = s.path(&w).into_os_string();
        p.push("-wal");
        std::path::PathBuf::from(p)
    };
    s.shutdown();
    // Last connection closed: the WAL was checkpointed and removed.
    assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0);
    assert!(matches!(
        leaked.open(&w, codec()).await,
        Err(store::StoreError::Closed)
    ));
    assert!(matches!(
        leaked.read(&w, |_, _| Ok(())).await,
        Err(store::StoreError::Closed)
    ));
}
