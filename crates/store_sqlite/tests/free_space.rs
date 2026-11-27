use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use store_sqlite::{FREE_SPACE_CHECK_MS, FreeSpace, MemoryKeySource, SqliteConfig, SqliteStore};

struct Fixed(AtomicU64);

impl FreeSpace for Fixed {
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

fn store(dir: &Path) -> SqliteStore {
    SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    )
}

#[test]
fn the_check_is_throttled_across_every_clone_of_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let a = store(dir.path());
    let b = a.clone();
    a.set_free_space(Arc::new(Fixed(AtomicU64::new(777))));
    let t0 = 10_000_000;
    assert_eq!(a.check_free_space(t0), Some(777), "the first check reads");
    assert_eq!(
        b.check_free_space(t0 + 1000),
        None,
        "a clone is throttled too"
    );
    assert_eq!(a.check_free_space(t0 + FREE_SPACE_CHECK_MS - 1), None);
    assert_eq!(b.check_free_space(t0 + FREE_SPACE_CHECK_MS), Some(777));
    assert_eq!(a.check_free_space(t0 + FREE_SPACE_CHECK_MS + 1), None);
}

#[test]
fn the_system_source_reads_the_volume_of_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let free = store_sqlite::SystemFreeSpace.free_bytes(dir.path());
    assert!(free.is_some_and(|b| b > 0), "{free:?}");
    let missing = store_sqlite::SystemFreeSpace.free_bytes(&dir.path().join("not/yet/created"));
    assert!(
        missing.is_some_and(|b| b > 0),
        "a directory that does not exist yet reads its nearest parent"
    );
}
