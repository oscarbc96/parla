use std::sync::Arc;

use domain::NoExtCodec;
use domain::test_support::FakeFactory;
use parla::providers::ProviderRegistry;
use settings::WorkspaceKey;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

#[tokio::test]
async fn removing_a_workspace_wipes_its_store_and_leaves_others() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let a = WorkspaceKey::new("fake", "A");
    let b = WorkspaceKey::new("fake", "B");
    let factory = FakeFactory::default();
    factory.set_accounts(
        [&a, &b]
            .map(|k| domain::AccountInfo {
                key: k.clone(),
                name: Arc::from(k.id.0.as_str()),
                org: None,
                icon: None,
            })
            .to_vec(),
    );
    let mut registry = ProviderRegistry::new();
    registry.add(factory);
    for k in [&a, &b] {
        sqlite.open(k, Arc::new(NoExtCodec)).await.unwrap();
    }
    let (pa, pb) = (sqlite.path(&a), sqlite.path(&b));
    assert!(pa.exists() && pb.exists());

    // A live session blocks removal and nothing is deleted.
    registry.set_running(&a, true);
    assert!(matches!(
        registry.remove_workspace_opt(&a, Some(&sqlite)).await,
        Err(parla::providers::RemoveError::Running)
    ));
    assert!(pa.exists());
    assert_eq!(registry.accounts().len(), 2);

    registry.set_running(&a, false);
    registry
        .remove_workspace_opt(&a, Some(&sqlite))
        .await
        .unwrap();

    assert!(!pa.exists(), "removed workspace store is deleted");
    assert!(pb.exists(), "other workspace store is intact");
    let left: Vec<_> = registry.accounts().into_iter().map(|(_, k)| k).collect();
    assert_eq!(left, vec![b]);
}
