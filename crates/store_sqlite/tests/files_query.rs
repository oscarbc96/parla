//! The Files tab query reads through its partial index, never the whole
//! messages table.

use std::sync::Arc;

use settings::WorkspaceKey;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

#[tokio::test]
async fn files_query_uses_the_files_index() {
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let ws = WorkspaceKey::new("fake", "files");
    db.open(&ws, Arc::new(domain::NoExtCodec)).await.unwrap();
    let plan = db.explain_files(&ws).await.unwrap().join(" | ");
    db.shutdown();
    assert!(plan.contains("messages_files"), "plan: {plan}");
    assert!(!plan.contains("SCAN messages"), "full scan: {plan}");
}
