//! Rate limits and the confirmation queue cap: each MCP client and each plugin has its own limits,
//! below the user's budget. Limits are checked on each call, so an idle
//! requester costs nothing. Moved from `mcp::limits`.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use governor::{
    Quota, RateLimiter,
    clock::{Clock, DefaultClock, FakeRelativeClock},
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed, keyed::DashMapStateStore},
};
use settings::WorkspaceKey;

pub const READS_PER_MIN: u32 = 120;
pub const READ_BURST: u32 = 20;
pub const WRITES_PER_MIN: u32 = 10;
pub const WRITE_BURST: u32 = 3;
pub const PROVIDER_READS_PER_MIN_PER_WORKSPACE: u32 = 20;
pub const MAX_PENDING_CONFIRMATIONS: usize = 3;
/// Plugin design section 9.2: 60 read calls per minute per plugin.
pub const PLUGIN_READS_PER_MIN: u32 = 60;
pub const PLUGIN_READ_BURST: u32 = READ_BURST;
/// Plugin design section 9.3: 20 write calls per minute per plugin.
pub const PLUGIN_WRITES_PER_MIN: u32 = 20;
pub const PLUGIN_WRITE_BURST: u32 = WRITE_BURST;

/// One per MCP IPC connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

/// Whose budget a call draws on: an MCP client or a plugin (by id).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequesterKey {
    Mcp(ClientId),
    Plugin(Arc<str>),
}

impl RequesterKey {
    pub fn plugin(id: impl Into<Arc<str>>) -> Self {
        RequesterKey::Plugin(id.into())
    }
}

impl From<ClientId> for RequesterKey {
    fn from(id: ClientId) -> Self {
        RequesterKey::Mcp(id)
    }
}

/// A request refused for now; try again after `retry_after_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("rate limited: retry after {retry_after_ms} ms")]
pub struct Limited {
    pub retry_after_ms: u64,
}

type Direct<C> = RateLimiter<NotKeyed, InMemoryState, C, NoOpMiddleware<<C as Clock>::Instant>>;
type Keyed<C> = RateLimiter<
    WorkspaceKey,
    DashMapStateStore<WorkspaceKey>,
    C,
    NoOpMiddleware<<C as Clock>::Instant>,
>;

struct ClientState<C: Clock> {
    reads: Direct<C>,
    writes: Direct<C>,
    pending: Arc<AtomicUsize>,
}

/// Frees a confirmation slot when dropped.
#[derive(Debug)]
pub struct PendingGuard {
    slot: Arc<AtomicUsize>,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.slot.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct Limits<C: Clock = DefaultClock> {
    clock: C,
    clients: Mutex<HashMap<RequesterKey, Arc<ClientState<C>>>>,
    provider: Keyed<C>,
}

impl Limits<DefaultClock> {
    /// A limiter on the system clock.
    pub fn system() -> Self {
        Self::with_clock(DefaultClock::default())
    }
}

impl Limits<FakeRelativeClock> {
    /// A limiter on a clock the test advances by hand.
    pub fn fake() -> (Self, FakeRelativeClock) {
        let clock = FakeRelativeClock::default();
        (Self::with_clock(clock.clone()), clock)
    }
}

fn quota(per_min: u32, burst: u32) -> Quota {
    Quota::per_minute(NonZeroU32::new(per_min).expect("nonzero"))
        .allow_burst(NonZeroU32::new(burst).expect("nonzero"))
}

fn retry_ms(d: Duration) -> u64 {
    // Round up so waiting the advertised time always succeeds.
    u64::try_from(d.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

impl<C: Clock + Clone> Limits<C> {
    pub fn with_clock(clock: C) -> Self {
        let provider = RateLimiter::dashmap_with_clock(
            quota(
                PROVIDER_READS_PER_MIN_PER_WORKSPACE,
                PROVIDER_READS_PER_MIN_PER_WORKSPACE,
            ),
            clock.clone(),
        );
        Self {
            clock,
            clients: Mutex::new(HashMap::new()),
            provider,
        }
    }

    fn client(&self, id: RequesterKey) -> Arc<ClientState<C>> {
        let ((reads, read_burst), (writes, write_burst)) = match id {
            RequesterKey::Mcp(_) => ((READS_PER_MIN, READ_BURST), (WRITES_PER_MIN, WRITE_BURST)),
            RequesterKey::Plugin(_) => (
                (PLUGIN_READS_PER_MIN, PLUGIN_READ_BURST),
                (PLUGIN_WRITES_PER_MIN, PLUGIN_WRITE_BURST),
            ),
        };
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(id)
            .or_insert_with(|| {
                Arc::new(ClientState {
                    reads: RateLimiter::direct_with_clock(
                        quota(reads, read_burst),
                        self.clock.clone(),
                    ),
                    writes: RateLimiter::direct_with_clock(
                        quota(writes, write_burst),
                        self.clock.clone(),
                    ),
                    pending: Arc::new(AtomicUsize::new(0)),
                })
            })
            .clone()
    }

    fn check(&self, l: &Direct<C>) -> Result<(), Limited> {
        l.check().map_err(|nu| Limited {
            retry_after_ms: retry_ms(nu.wait_time_from(self.clock.now())),
        })
    }

    pub fn read(&self, client: impl Into<RequesterKey>) -> Result<(), Limited> {
        self.check(&self.client(client.into()).reads)
    }

    pub fn write(&self, client: impl Into<RequesterKey>) -> Result<(), Limited> {
        self.check(&self.client(client.into()).writes)
    }

    /// Charges `n` provider requests to `ws`, shared by every client. Checked
    /// before a call that may reach the provider; on `Limited` the caller asks
    /// the backend with `remote = false` and answers from cache.
    pub fn provider_reads(&self, ws: &WorkspaceKey, n: u32) -> Result<(), Limited> {
        let Some(n) = NonZeroU32::new(n) else {
            return Ok(());
        };
        match self.provider.check_key_n(ws, n) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(nu)) => Err(Limited {
                retry_after_ms: retry_ms(nu.wait_time_from(self.clock.now())),
            }),
            // More than a full budget in one go can never succeed; ask again
            // after a whole window.
            Err(_) => Err(Limited {
                retry_after_ms: 60_000,
            }),
        }
    }

    /// Claims a confirmation slot; dropping the guard frees it.
    pub fn pending(&self, client: impl Into<RequesterKey>) -> Result<PendingGuard, Limited> {
        let slot = self.client(client.into()).pending.clone();
        let taken = slot.try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < MAX_PENDING_CONFIRMATIONS).then_some(n + 1)
        });
        match taken {
            Ok(_) => Ok(PendingGuard { slot }),
            // A slot frees when the user answers or the prompt times out;
            // there is no useful wait time to report, so suggest a short one.
            Err(_) => Err(Limited {
                retry_after_ms: 5_000,
            }),
        }
    }

    /// Drops a disconnected client's or an unloaded plugin's state.
    pub fn forget(&self, client: impl Into<RequesterKey>) {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&client.into());
    }

    /// Requesters with live state (for tests and diagnostics).
    #[cfg(feature = "test-support")]
    pub fn client_count(&self) -> usize {
        self.clients.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}
