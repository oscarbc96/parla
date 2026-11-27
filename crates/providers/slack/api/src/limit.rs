//! Per-tier rate limiting with request priorities.
//!
//! One `governor` direct limiter per tier, a second set at half the rate that
//! `Priority::Background` calls draw from first, and a keyed limiter that spaces
//! posts per channel (a short burst, then one per second). The clock follows `tokio::time`, so tests can pause time.

use std::num::NonZeroU32;
use std::time::Duration;

use governor::clock::Clock;
use governor::middleware::NoOpMiddleware;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};

use crate::method::{Priority, Tier};

/// Calls per minute for each tier, and posts per second per channel.
///
/// The defaults are Slack's published tiers. The web-client endpoints are not
/// documented per tier, so each `Method` constant states the tier it is assumed
/// to be in, and 429s observed in recon captures correct it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimits {
    pub t1: u32,
    pub t2: u32,
    pub t3: u32,
    pub t4: u32,
    pub post_per_channel_per_sec: u32,
    /// Posts a channel may take at once before the per-second spacing starts.
    pub post_burst: u32,
}

impl Default for RateLimits {
    fn default() -> Self {
        RateLimits {
            t1: 1,
            t2: 20,
            t3: 50,
            t4: 100,
            post_per_channel_per_sec: 1,
            post_burst: 5,
        }
    }
}

/// A governor clock that reads `tokio::time::Instant`, so it obeys `time::pause`.
#[derive(Debug, Clone)]
struct TokioClock {
    base: tokio::time::Instant,
}

impl TokioClock {
    fn new() -> Self {
        TokioClock {
            base: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    type Instant = Duration;

    fn now(&self) -> Duration {
        tokio::time::Instant::now().duration_since(self.base)
    }
}

type Mw = NoOpMiddleware<Duration>;
type Direct = RateLimiter<NotKeyed, InMemoryState, TokioClock, Mw>;
type Keyed = RateLimiter<String, DefaultKeyedStateStore<String>, TokioClock, Mw>;

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n.max(1)).unwrap_or(NonZeroU32::MIN)
}

/// What a 429 holds: a method, and for posts also the channel.
type HoldKey = (&'static str, Option<String>);

pub(crate) struct Limiter {
    clock: TokioClock,
    tiers: [Direct; 4],
    background: [Direct; 4],
    post: Keyed,
    /// Posts that name no channel share this one limiter.
    post_any: Direct,
    /// A method is not sent before its instant: set when the server answers 429
    /// for it. Server limits are per method, so one method's hold never delays
    /// another (a rate-limited history fetch must not stall a user send).
    blocked_until: std::sync::Mutex<std::collections::HashMap<HoldKey, tokio::time::Instant>>,
}

impl Limiter {
    pub(crate) fn new(limits: RateLimits) -> Limiter {
        let clock = TokioClock::new();
        let per_min = [limits.t1, limits.t2, limits.t3, limits.t4];
        let tiers =
            per_min.map(|n| Direct::direct_with_clock(Quota::per_minute(nz(n)), clock.clone()));
        let background =
            per_min.map(|n| Direct::direct_with_clock(Quota::per_minute(nz(n / 2)), clock.clone()));
        let post_quota = Quota::per_second(nz(limits.post_per_channel_per_sec))
            .allow_burst(nz(limits.post_burst));
        let post = Keyed::dashmap_with_clock(post_quota, clock.clone());
        let post_any = Direct::direct_with_clock(post_quota, clock.clone());
        Limiter {
            clock,
            tiers,
            background,
            post,
            post_any,
            blocked_until: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The server said 429 for `method`: hold that method for `d`, so in-flight
    /// work stops hitting it. Other methods are not held.
    pub(crate) fn penalize(&self, method: &'static str, key: Option<&str>, d: Duration) {
        let now = tokio::time::Instant::now();
        let until = now + d;
        let mut g = self.blocked_until.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|_, t| *t > now);
        let e = g.entry((method, key.map(str::to_string))).or_insert(until);
        if until > *e {
            *e = until;
        }
    }

    async fn wait_unblocked(&self, method: &'static str, key: Option<&str>) {
        let hold = (method, key.map(str::to_string));
        loop {
            let until = self
                .blocked_until
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&hold)
                .copied();
            match until {
                Some(u) if u > tokio::time::Instant::now() => tokio::time::sleep_until(u).await,
                _ => return,
            }
        }
    }

    /// Wait until a call of `method` (in `tier`) may be sent. `key` is the channel for posts.
    pub(crate) async fn acquire(
        &self,
        method: &'static str,
        tier: Tier,
        key: Option<&str>,
        prio: Priority,
    ) {
        self.wait_unblocked(method, key).await;
        let idx = match tier {
            Tier::T1 => Some(0),
            Tier::T2 => Some(1),
            Tier::T3 => Some(2),
            Tier::T4 => Some(3),
            Tier::PostMessage => None,
        };
        if let Some(i) = idx {
            if prio == Priority::Background {
                self.wait(&self.background[i]).await;
            }
            self.wait(&self.tiers[i]).await;
        } else if let Some(key) = key {
            self.wait_keyed(key).await;
        } else {
            self.wait(&self.post_any).await;
        }
    }

    async fn wait(&self, l: &Direct) {
        while let Err(not_until) = l.check() {
            tokio::time::sleep(not_until.wait_time_from(self.clock.now())).await;
        }
    }

    async fn wait_keyed(&self, key: &str) {
        let key = key.to_string();
        while let Err(not_until) = self.post.check_key(&key) {
            tokio::time::sleep(not_until.wait_time_from(self.clock.now())).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    const M: &str = "x.method";

    fn limiter(t2: u32) -> Arc<Limiter> {
        Arc::new(Limiter::new(RateLimits {
            t2,
            ..RateLimits::default()
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn background_capped_at_half_tier() {
        let l = limiter(4);
        let done = Arc::new(AtomicU32::new(0));
        for _ in 0..4 {
            let (l, done) = (l.clone(), done.clone());
            tokio::spawn(async move {
                l.acquire(M, Tier::T2, None, Priority::Background).await;
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(done.load(Ordering::SeqCst), 2, "burst is half the tier");
        tokio::time::sleep(Duration::from_secs(28)).await;
        assert_eq!(done.load(Ordering::SeqCst), 2, "still capped at 29s");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(done.load(Ordering::SeqCst), 3, "one more token at 30s");
    }

    #[tokio::test(start_paused = true)]
    async fn user_calls_use_the_full_tier() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        for _ in 0..4 {
            l.acquire(M, Tier::T2, None, Priority::User).await;
        }
        assert_eq!(start.elapsed(), Duration::ZERO);
        l.acquire(M, Tier::T2, None, Priority::Visible).await;
        assert!(start.elapsed() >= Duration::from_secs(15));
    }

    #[tokio::test(start_paused = true)]
    async fn post_without_channel_shares_a_global_limit() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        for _ in 0..RateLimits::default().post_burst {
            l.acquire(M, Tier::PostMessage, None, Priority::User).await;
        }
        assert_eq!(start.elapsed(), Duration::ZERO);
        l.acquire(M, Tier::PostMessage, None, Priority::User).await;
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn penalty_holds_its_method_and_never_shortens() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        l.penalize(M, None, Duration::from_secs(7));
        l.penalize(M, None, Duration::from_secs(2));
        l.acquire(M, Tier::T3, None, Priority::User).await;
        assert_eq!(
            start.elapsed(),
            Duration::from_secs(7),
            "a shorter penalty never shortens"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn post_limited_per_channel() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        for _ in 0..RateLimits::default().post_burst {
            l.acquire(M, Tier::PostMessage, Some("C1"), Priority::User)
                .await;
        }
        l.acquire(M, Tier::PostMessage, Some("C2"), Priority::User)
            .await;
        assert_eq!(start.elapsed(), Duration::ZERO, "other channel not delayed");
        l.acquire(M, Tier::PostMessage, Some("C1"), Priority::User)
            .await;
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }

    /// Someone typing several short messages in a row: the first few go out
    /// at once (the server allows short bursts), then one per second.
    #[tokio::test(start_paused = true)]
    async fn post_allows_a_short_burst_per_channel() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        for _ in 0..5 {
            l.acquire(M, Tier::PostMessage, Some("C1"), Priority::User)
                .await;
        }
        assert_eq!(start.elapsed(), Duration::ZERO, "a burst is not spaced");
        l.acquire(M, Tier::PostMessage, Some("C1"), Priority::User)
            .await;
        assert_eq!(
            start.elapsed(),
            Duration::from_secs(1),
            "then one per second"
        );
    }

    /// A 429 on a background method (history, users) must not hold a user
    /// send for the whole Retry-After: server limits are per method.
    #[tokio::test(start_paused = true)]
    async fn background_429_does_not_delay_a_post() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        l.penalize("users.list", None, Duration::from_secs(30));
        l.acquire(
            "chat.postMessage",
            Tier::PostMessage,
            Some("C1"),
            Priority::User,
        )
        .await;
        assert_eq!(start.elapsed(), Duration::ZERO, "post waited on the hold");
    }

    /// A 429 on one channel's posts holds that channel only.
    #[tokio::test(start_paused = true)]
    async fn post_429_holds_only_its_channel() {
        let l = limiter(4);
        let start = tokio::time::Instant::now();
        l.penalize("chat.postMessage", Some("C1"), Duration::from_secs(9));
        l.acquire(
            "chat.postMessage",
            Tier::PostMessage,
            Some("C2"),
            Priority::User,
        )
        .await;
        assert_eq!(start.elapsed(), Duration::ZERO, "other channel held");
        l.acquire(
            "chat.postMessage",
            Tier::PostMessage,
            Some("C1"),
            Priority::User,
        )
        .await;
        assert_eq!(start.elapsed(), Duration::from_secs(9));
    }
}
