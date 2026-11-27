//! A conversation's topic, description and member count survive a reopen, so the header can
//! show them from the cache before the first bootstrap.

use std::sync::Arc;

use domain::test_support::conv;
use domain::{NoExtCodec, PersistOp};
use settings::WorkspaceKey;
use store::Store;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

#[tokio::test]
async fn topic_and_member_count_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let w = WorkspaceKey::new("fake", "w1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    let mut c = conv("C1", "general");
    c.topic = Arc::from("Ship it: <https://example.com|the plan>");
    c.member_count = Some(42);
    c.purpose = Arc::from("Plans for the launch");
    s.apply(&w, PersistOp::Conversations(vec![c, conv("C2", "random")]))
        .await
        .unwrap();
    let cs = s.conversations(&w).await.unwrap();
    let c1 = cs.iter().find(|c| &*c.id.0 == "C1").unwrap();
    assert_eq!(&*c1.topic, "Ship it: <https://example.com|the plan>");
    assert_eq!(c1.member_count, Some(42));
    assert_eq!(&*c1.purpose, "Plans for the launch");
    let c2 = cs.iter().find(|c| &*c.id.0 == "C2").unwrap();
    assert!(c2.topic.is_empty() && c2.purpose.is_empty());
    assert_eq!(c2.member_count, None);
}
