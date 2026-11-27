use std::sync::Arc;

use domain::NoExtCodec;
use domain::PersistOp;
use domain::test_support::{cid, conv, msg};
use settings::WorkspaceKey;
use store::{Store, StoreError};
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

const DAY_MS: i64 = 86_400_000;
const NOW: i64 = 100 * DAY_MS;

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

async fn sql(s: &SqliteStore, q: &'static str) {
    s.write(&w(), move |c, _| {
        c.execute_batch(q)
            .map_err(|e| StoreError::Other(e.to_string()))
    })
    .await
    .unwrap();
}

async fn put(s: &SqliteStore, conv: &str, ts: &str, text: &str) {
    s.apply(
        &w(),
        PersistOp::Messages {
            conversation: cid(conv),
            messages: vec![msg(conv, ts, text)],
        },
    )
    .await
    .unwrap();
}

async fn n(s: &SqliteStore, table: &'static str) -> i64 {
    s.read(&w(), move |c, _| {
        c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(|e| StoreError::Other(e.to_string()))
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn prune_respects_retention_and_recent_opens() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    sql(&s, "INSERT INTO conversations (id, kind, name, last_opened_ms) VALUES ('NEW',0,'n',95*86400000), ('OLD',0,'o',60*86400000), ('NEVER',0,'v',NULL)").await;
    for c in ["NEW", "OLD", "NEVER"] {
        put(&s, c, "1000.000001", "ancient").await; // far older than 30 days
        put(&s, c, &format!("{}.000001", 99 * 86_400), "fresh").await;
    }
    let deleted = s.prune(&w(), NOW, 30).await.unwrap();
    assert_eq!(deleted, 2); // OLD and NEVER ancient
    assert_eq!(n(&s, "messages").await, 4);
    let hits = s.search(&w(), "ancient", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(&*hits[0].id.conversation.0, "NEW");
}

/// The app opens a conversation before the bootstrap has written its
/// `conversations` row (a click, or a deep link, as soon as the window packs).
/// Its marker must survive that ordering: startup retention otherwise deletes
/// the page just fetched for it, because an old `ts` plus a missing row looks
/// like a conversation nobody ever opened.
#[tokio::test]
async fn prune_keeps_a_conversation_opened_before_it_was_stored() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    s.apply(
        &w(),
        PersistOp::Opened {
            conversation: cid("C"),
            at_ms: NOW,
        },
    )
    .await
    .unwrap();
    put(&s, "C", "1000.000001", "ancient").await;
    assert_eq!(s.prune(&w(), NOW, 30).await.unwrap(), 0);
    // The bootstrap's row arrives later; it does not clear the marker.
    s.apply(&w(), PersistOp::Conversations(vec![conv("C", "c")]))
        .await
        .unwrap();
    assert_eq!(s.prune(&w(), NOW, 30).await.unwrap(), 0);
    assert_eq!(n(&s, "messages").await, 1);
}

#[tokio::test]
async fn prune_drops_markers_past_the_recent_open_window() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    sql(
        &s,
        "INSERT INTO conversations (id, kind, name) VALUES ('C',0,'c')",
    )
    .await;
    s.apply(
        &w(),
        PersistOp::Opened {
            conversation: cid("C"),
            at_ms: NOW - 40 * DAY_MS,
        },
    )
    .await
    .unwrap();
    put(&s, "C", "1000.000001", "ancient").await;
    // The open is older than the window that keeps old messages, so the page
    // goes; its marker cannot matter again either and does not pile up.
    assert_eq!(s.prune(&w(), NOW, 30).await.unwrap(), 1);
    assert_eq!(n(&s, "opened").await, 0);
}

#[tokio::test]
async fn prune_keeps_outbox_and_drafts() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    sql(&s, "INSERT INTO outbox (id, conversation_id, text, created_ms) VALUES ('o1','C','t',1); INSERT INTO drafts (conversation_id, body) VALUES ('C','d')").await;
    put(&s, "C", "1000.000001", "ancient").await;
    assert_eq!(s.prune(&w(), NOW, 30).await.unwrap(), 1);
    assert_eq!(n(&s, "outbox").await, 1);
    assert_eq!(n(&s, "drafts").await, 1);
}

#[tokio::test]
async fn incremental_vacuum_shrinks_freelist() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    let big = "x".repeat(2000);
    for i in 0..300 {
        put(&s, "C", &format!("{}.000001", 1000 + i), &big).await;
    }
    let pages: i64 = s.pragma(&w(), "page_count").await.unwrap().parse().unwrap();
    assert_eq!(s.prune(&w(), NOW, 30).await.unwrap(), 300);
    let after: i64 = s.pragma(&w(), "page_count").await.unwrap().parse().unwrap();
    let free: i64 = s
        .pragma(&w(), "freelist_count")
        .await
        .unwrap()
        .parse()
        .unwrap();
    assert!(after < pages, "{after} !< {pages}");
    assert_eq!(free, 0);
}

#[tokio::test]
async fn zero_retention_days_is_clamped_to_one_day() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path()).await;
    put(&s, "C", &format!("{}.000001", 99 * 86_400), "fresh").await;
    put(&s, "C", "1000.000001", "ancient").await;
    assert_eq!(s.prune(&w(), NOW, 0).await.unwrap(), 1);
    assert_eq!(n(&s, "messages").await, 1);
}
