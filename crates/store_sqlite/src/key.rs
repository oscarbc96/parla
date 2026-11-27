//! Database keys and where they live.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use futures::future::BoxFuture;
use settings::WorkspaceKey;
use store::StoreError;
use zeroize::Zeroizing;

/// Which database a key belongs to: the cross-workspace `app.db` or one
/// workspace's database.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DbId {
    App,
    Workspace(WorkspaceKey),
}

/// A raw 256-bit database key. Wiped on drop, never printed.
#[derive(Clone)]
pub struct Key(Zeroizing<[u8; 32]>);

impl Key {
    pub fn random() -> Key {
        let mut b = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut b[..]).expect("operating system random source");
        Key(b)
    }

    /// Exactly 64 hex digits.
    pub fn from_hex(s: &str) -> Option<Key> {
        let s = s.as_bytes();
        if s.len() != 64 {
            return None;
        }
        let mut b = Zeroizing::new([0u8; 32]);
        for (i, pair) in s.chunks(2).enumerate() {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            b[i] = (hi * 16 + lo) as u8;
        }
        Some(Key(b))
    }

    pub fn to_hex(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = Zeroizing::new(String::with_capacity(64));
        for byte in self.0.iter() {
            s.push(HEX[(byte >> 4) as usize] as char);
            s.push(HEX[(byte & 15) as usize] as char);
        }
        s
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(REDACTED)")
    }
}

/// Where per-database keys are kept (the OS keychain in the app).
pub trait KeySource: Send + Sync {
    /// The stored key, creating and storing a random one when missing.
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>>;
    /// Stores a fresh random key over any existing one and returns it.
    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>>;
    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Drops any key held in memory, so the next open reads it from the
    /// source again (the idle lock). A source that holds none does nothing.
    fn forget_cached(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

impl<S: KeySource + ?Sized> KeySource for std::sync::Arc<S> {
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        (**self).get_or_create(ws)
    }
    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        (**self).replace(ws)
    }
    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        (**self).delete(ws)
    }
    fn forget_cached(&self) -> BoxFuture<'_, ()> {
        (**self).forget_cached()
    }
}

/// In-memory keys for tests; never touches the keychain.
#[derive(Default)]
pub struct MemoryKeySource(Mutex<HashMap<DbId, Key>>);

impl MemoryKeySource {
    /// Whether no key is stored (tests).
    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }

    /// Whether a key is stored for `id` (tests).
    pub fn contains(&self, id: &DbId) -> bool {
        self.0.lock().unwrap().contains_key(id)
    }
}

impl KeySource for MemoryKeySource {
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let k = self
            .0
            .lock()
            .unwrap()
            .entry(ws.clone())
            .or_insert_with(Key::random)
            .clone();
        Box::pin(async move { Ok(k) })
    }
    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let k = Key::random();
        self.0.lock().unwrap().insert(ws.clone(), k.clone());
        Box::pin(async move { Ok(k) })
    }
    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        self.0.lock().unwrap().remove(ws);
        Box::pin(async { Ok(()) })
    }
}

/// Reads each key from the inner source at most once per process: the OS
/// keychain prompts on reads, so later opens and reconnects use memory.
/// `replace` and `delete` keep the cache in step. Keys are zeroized on drop.
pub struct CachingKeySource<S> {
    inner: S,
    cache: futures::lock::Mutex<HashMap<DbId, Key>>,
}

impl<S: KeySource> CachingKeySource<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            cache: Default::default(),
        }
    }
}

impl<S: KeySource> KeySource for CachingKeySource<S> {
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            if let Some(k) = cache.get(&ws) {
                return Ok(k.clone());
            }
            let k = self.inner.get_or_create(&ws).await?;
            cache.insert(ws, k.clone());
            Ok(k)
        })
    }
    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            cache.remove(&ws);
            let k = self.inner.replace(&ws).await?;
            cache.insert(ws, k.clone());
            Ok(k)
        })
    }
    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let mut cache = self.cache.lock().await;
            cache.remove(&ws);
            self.inner.delete(&ws).await
        })
    }

    /// Empties the cache (each key is zeroized on drop) and asks the inner
    /// source to do the same.
    fn forget_cached(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.cache.lock().await.clear();
            self.inner.forget_cached().await;
        })
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counting {
        reads: Arc<AtomicUsize>,
        mem: MemoryKeySource,
    }
    impl KeySource for Counting {
        fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.mem.get_or_create(ws)
        }
        fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
            self.mem.replace(ws)
        }
        fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
            self.mem.delete(ws)
        }
    }

    #[test]
    fn one_inner_read_per_workspace_across_reopens() {
        let reads = Arc::new(AtomicUsize::new(0));
        let src = CachingKeySource::new(Counting {
            reads: reads.clone(),
            ..Default::default()
        });
        let a = DbId::Workspace(WorkspaceKey::new("p", "a"));
        let b = DbId::Workspace(WorkspaceKey::new("p", "b"));
        let k1 = futures::executor::block_on(src.get_or_create(&a)).unwrap();
        for _ in 0..5 {
            let k = futures::executor::block_on(src.get_or_create(&a)).unwrap();
            assert_eq!(k.to_hex().as_str(), k1.to_hex().as_str());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        futures::executor::block_on(src.get_or_create(&b)).unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forget_cached_makes_the_next_open_read_the_source_again() {
        let reads = Arc::new(AtomicUsize::new(0));
        let src = CachingKeySource::new(Counting {
            reads: reads.clone(),
            ..Default::default()
        });
        let a = DbId::Workspace(WorkspaceKey::new("p", "a"));
        let k1 = futures::executor::block_on(src.get_or_create(&a)).unwrap();
        futures::executor::block_on(src.forget_cached());
        let k2 = futures::executor::block_on(src.get_or_create(&a)).unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 2, "read again after forget");
        assert_eq!(
            k1.to_hex().as_str(),
            k2.to_hex().as_str(),
            "same stored key"
        );
    }

    #[test]
    fn replace_updates_cache_without_a_read() {
        let reads = Arc::new(AtomicUsize::new(0));
        let src = CachingKeySource::new(Counting {
            reads: reads.clone(),
            ..Default::default()
        });
        let a = DbId::Workspace(WorkspaceKey::new("p", "a"));
        let old = futures::executor::block_on(src.get_or_create(&a)).unwrap();
        let new = futures::executor::block_on(src.replace(&a)).unwrap();
        assert_ne!(old.to_hex().as_str(), new.to_hex().as_str());
        let got = futures::executor::block_on(src.get_or_create(&a)).unwrap();
        assert_eq!(got.to_hex().as_str(), new.to_hex().as_str());
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }
}
