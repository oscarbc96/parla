//! `is_open` survives a reopen, and an upsert that leaves `plain` unchanged
//! does not re-index the message.

use std::sync::Arc;

use domain::test_support::{cid, conv, msg};
use domain::{NoExtCodec, PersistOp};
use settings::WorkspaceKey;
use store::{Store, StoreError};
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

fn w() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}

async fn start(dir: &std::path::Path) -> SqliteStore {
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    s
}

#[tokio::test]
async fn is_open_round_trips() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    let mut closed = conv("D1", "dm");
    closed.is_open = false;
    s.apply(
        &w(),
        PersistOp::Conversations(vec![closed, conv("D2", "dm2")]),
    )
    .await
    .unwrap();
    let cs = s.conversations(&w()).await.unwrap();
    assert!(!cs.iter().find(|c| &*c.id.0 == "D1").unwrap().is_open);
    assert!(cs.iter().find(|c| &*c.id.0 == "D2").unwrap().is_open);
}

#[tokio::test]
async fn unchanged_plain_does_not_touch_the_fts_index() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    s.apply(
        &w(),
        PersistOp::Messages {
            conversation: cid("C1"),
            messages: vec![msg("C1", "1000.000001", "hello")],
        },
    )
    .await
    .unwrap();
    let (same, changed) = s
        .write(&w(), |c, _| {
            let db = |e: rusqlite::Error| StoreError::Other(e.to_string());
            let before = c.total_changes();
            c.execute("UPDATE messages SET plain = plain", [])
                .map_err(db)?;
            let same = c.total_changes() - before;
            let before = c.total_changes();
            c.execute("UPDATE messages SET plain = 'bye'", [])
                .map_err(db)?;
            Ok((same, c.total_changes() - before))
        })
        .await
        .unwrap();
    assert_eq!(same, 1, "only the message row changes");
    assert!(changed > 1, "a changed text re-indexes");
    assert_eq!(s.search(&w(), "bye", 10).await.unwrap().len(), 1);
}
