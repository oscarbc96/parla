//! Retries of a write with the same `idempotency_key` return the first
//! result without a second prompt or send: per client name, the last 64 keys
//! for 10 minutes. A key is claimed before the user is asked, so a concurrent
//! retry waits for the first call and then replays its result. Keys outlive
//! the connection, so a retry after a shim reconnect is still recognised.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub const MAX_KEYS: usize = 64;
/// Distinct client names kept at once.
pub const MAX_CLIENTS: usize = 64;
pub const TTL: Duration = Duration::from_secs(10 * 60);
pub const MAX_KEY_LEN: usize = 64;

type Stored = Option<(Instant, Value)>;

/// One key's claim: empty until the first call stores its result.
#[derive(Default)]
pub struct Slot(Arc<AsyncMutex<Stored>>);

/// Exclusive use of a key until dropped.
pub struct Held(OwnedMutexGuard<Stored>);

impl Slot {
    /// Waits for any call already working on this key.
    pub async fn acquire(&self) -> Held {
        Held(self.0.clone().lock_owned().await)
    }

    fn is_idle(&self, now: Instant) -> bool {
        Arc::strong_count(&self.0) == 1
            && self.0.try_lock().is_ok_and(|s| {
                s.as_ref()
                    .is_none_or(|(at, _)| now.saturating_duration_since(*at) >= TTL)
            })
    }
}

impl Held {
    /// The stored result, when it was stored less than [`TTL`] ago.
    pub fn replay(&self, now: Instant) -> Option<Value> {
        self.0
            .as_ref()
            .filter(|(at, _)| now.saturating_duration_since(*at) < TTL)
            .map(|(_, v)| v.clone())
    }

    pub fn store(&mut self, result: Value, now: Instant) {
        *self.0 = Some((now, result));
    }
}

struct Entry {
    key: String,
    slot: Arc<Slot>,
}

#[derive(Default)]
pub struct Idempotency {
    clients: Mutex<HashMap<String, VecDeque<Entry>>>,
}

impl Idempotency {
    /// The slot for `key` of the client called `client`, created when new.
    pub fn slot(&self, client: &str, key: &str, now: Instant) -> Arc<Slot> {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, q| {
            q.retain(|e| Arc::strong_count(&e.slot) > 1 || !e.slot.is_idle(now));
            !q.is_empty()
        });
        if !map.contains_key(client) && map.len() >= MAX_CLIENTS {
            // Any victim will do: the keys only guard retries.
            if let Some(victim) = map.keys().next().cloned() {
                map.remove(&victim);
            }
        }
        let q = map.entry(client.to_string()).or_default();
        if let Some(e) = q.iter().find(|e| e.key == key) {
            return e.slot.clone();
        }
        let slot = Arc::new(Slot::default());
        q.push_back(Entry {
            key: key.to_string(),
            slot: slot.clone(),
        });
        while q.len() > MAX_KEYS {
            q.pop_front();
        }
        slot
    }
}
