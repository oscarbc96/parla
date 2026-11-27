//! Per-message parsed rich text, kept in a bounded LRU so only rows that were
//! laid out are ever parsed and old ones are dropped.

use std::num::NonZeroUsize;
use std::sync::Arc;

use domain::{ContentParser, Message, MessageId, MessageTs, RichText};
use lru::LruCache;

pub const CAPACITY: usize = 512;

pub struct RenderCache {
    cache: LruCache<(MessageId, Option<MessageTs>), Arc<RichText>>,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(CAPACITY).expect("non-zero")),
        }
    }

    /// Cached rich text of `m`; an edit changes the key, so it parses again.
    pub fn get_or_parse(&mut self, m: &Message, p: &dyn ContentParser) -> Arc<RichText> {
        let key = (m.id.clone(), m.edited.clone());
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let parsed = Arc::new(p.parse(m));
        self.cache.put(key, parsed.clone());
        parsed
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}
