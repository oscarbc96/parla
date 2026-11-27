//! The startup `latest_all` query takes the conversation list from
//! `conversations` and does one `messages_window` lookup per conversation,
//! never a scan of every message index entry.

use std::sync::Arc;

use settings::WorkspaceKey;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

#[tokio::test]
async fn latest_query_scans_conversations_not_messages() {
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let ws = WorkspaceKey::new("fake", "latest");
    db.open(&ws, Arc::new(domain::NoExtCodec)).await.unwrap();
    let plan = db.explain_latest(&ws).await.unwrap();
    db.shutdown();
    let joined = plan.join(" | ");
    assert!(
        plan.iter().any(|l| l.contains("conversations")),
        "the conversation list is not read from conversations: {joined}"
    );
    assert!(joined.contains("messages_window"), "plan: {joined}");
    assert!(
        !plan
            .iter()
            .any(|l| l.contains("SCAN") && l.contains("messages")),
        "conversation list scans the messages index: {joined}"
    );
}
