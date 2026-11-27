use std::sync::Arc;

use domain::test_support::{cid, msg, ts};
use domain::{NoExtCodec, PersistOp};
use settings::WorkspaceKey;
use store::Store;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore, fts_query};

fn w() -> WorkspaceKey {
    WorkspaceKey::new("fake", "W1")
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

async fn put(s: &SqliteStore, messages: Vec<domain::Message>) {
    s.apply(
        &w(),
        PersistOp::Messages {
            conversation: cid("C1"),
            messages,
        },
    )
    .await
    .unwrap();
}

async fn count(s: &SqliteStore, q: &str) -> usize {
    s.search(&w(), q, 10).await.unwrap().len()
}

#[tokio::test]
async fn fts_finds_word_prefix_phrase() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    put(
        &s,
        vec![
            msg("C1", "1.000000", "the deploy finished fine"),
            msg("C1", "2.000000", "lunch at noon"),
        ],
    )
    .await;
    assert_eq!(count(&s, "deploy").await, 1);
    assert_eq!(count(&s, "depl*").await, 1);
    assert_eq!(count(&s, "\"deploy finished\"").await, 1);
    assert_eq!(count(&s, "\"finished deploy\"").await, 0);
    let hits = s.search(&w(), "deploy", 10).await.unwrap();
    assert_eq!(hits[0].id.ts, ts("1.000000"));
    assert!(hits[0].snippet.contains("deploy"));
}

#[tokio::test]
async fn fts_follows_edit_and_delete() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    put(&s, vec![msg("C1", "1.000000", "alpha message")]).await;
    assert_eq!(count(&s, "alpha").await, 1);
    put(&s, vec![msg("C1", "1.000000", "bravo message")]).await;
    assert_eq!(count(&s, "alpha").await, 0);
    assert_eq!(count(&s, "bravo").await, 1);
    s.write(&w(), |c, _| {
        c.execute("DELETE FROM messages", [])
            .map_err(|e| store::StoreError::Other(e.to_string()))?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(count(&s, "bravo").await, 0);
}

#[test]
fn fts_query_escapes_operators() {
    assert_eq!(fts_query(""), None);
    assert_eq!(fts_query("   "), None);
    assert_eq!(fts_query("deploy").as_deref(), Some("\"deploy\""));
    assert_eq!(fts_query("depl*").as_deref(), Some("\"depl\"*"));
    assert_eq!(
        fts_query("\"deploy finished\" ok").as_deref(),
        Some("\"deploy finished\" \"ok\"")
    );
    let q = fts_query("a\" OR b").unwrap();
    assert_eq!(q, "\"a\"\"\" \"OR\" \"b\"");
}

#[tokio::test]
async fn hostile_input_never_errors() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    put(&s, vec![msg("C1", "1.000000", "a b")]).await;
    for q in ["a\" OR b", "NEAR(", "*", "\"", "a AND", "col:x", "-a", "(("] {
        s.search(&w(), q, 10).await.unwrap();
    }
}

#[tokio::test]
async fn fts_text_unreadable_on_disk() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    put(&s, vec![msg("C1", "1.000000", "zyxwvutsrqponmlk here")]).await;
    s.pragma(&w(), "wal_checkpoint(TRUNCATE)").await.unwrap();
    let bytes = std::fs::read(s.path(&w())).unwrap();
    let marker = b"zyxwvutsrqponmlk";
    assert!(!bytes.windows(marker.len()).any(|x| x == marker));
}
