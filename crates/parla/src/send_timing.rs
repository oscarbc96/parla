//! Where the time of a sent message goes, as one INFO line per send. Phases
//! and milliseconds only: no text, ids, names or URLs. The provider adds its
//! own `post timing:` line (limiter wait, HTTP round trip, retries).
//!
//! `send timing: queue_ms=Q echo_ms=E finished_ms=F attempts=N outcome=O`,
//! all measured from the moment the composer enqueued the send: `queue_ms` is
//! the wait until the first call started (inflight item, offline, retry
//! timer), `echo_ms` the realtime echo (`none` when it had not arrived),
//! `finished_ms` the answer applied to the state. An echo that arrives after
//! the answer adds `send late echo: echo_ms=E`. A send with no enqueue seen
//! (a user Retry, an item reloaded from the store) starts its clock when its
//! first call starts, so its `queue_ms` is 0.

use std::collections::HashMap;
use std::time::Instant;

use domain::{ClientMsgId, Message, ProviderError};

/// Sends tracked at once; older ones are dropped (a send never settled).
const KEEP: usize = 16;

struct Clock {
    enqueued: Instant,
    started: Option<Instant>,
    echo: Option<Instant>,
    attempts: u32,
}

#[derive(Default)]
pub(crate) struct SendTiming {
    open: HashMap<ClientMsgId, Clock>,
    /// Sends answered whose echo may still come, with the enqueue instant.
    answered: HashMap<ClientMsgId, Instant>,
}

fn ms(from: Instant, to: Instant) -> u128 {
    to.saturating_duration_since(from).as_millis()
}

impl SendTiming {
    pub(crate) fn enqueued(&mut self, id: &ClientMsgId) {
        if self.open.len() >= KEEP {
            self.open.clear();
        }
        self.open.insert(
            id.clone(),
            Clock {
                enqueued: Instant::now(),
                started: None,
                echo: None,
                attempts: 0,
            },
        );
    }

    /// The outbox handed the send to the provider (every attempt counts).
    pub(crate) fn started(&mut self, id: &ClientMsgId) {
        let was_new = !self.open.contains_key(id);
        if was_new {
            self.enqueued(id);
        }
        if let Some(c) = self.open.get_mut(id) {
            // A send never enqueued starts at its enqueue instant, so its
            // queue time is exactly zero rather than the gap between two clock reads.
            let at = if was_new { c.enqueued } else { Instant::now() };
            c.started.get_or_insert(at);
            c.attempts += 1;
        }
    }

    /// The realtime echo of this send arrived.
    /// Returns the late-echo line when the send had already been answered.
    pub(crate) fn echo(&mut self, id: &ClientMsgId) -> Option<String> {
        let now = Instant::now();
        if let Some(c) = self.open.get_mut(id) {
            c.echo.get_or_insert(now);
            None
        } else {
            let t = self.answered.remove(id)?;
            Some(format!("send late echo: echo_ms={}", ms(t, now)))
        }
    }

    /// The provider's answer was applied. A retryable failure keeps the clock
    /// running for the next attempt.
    /// Returns the `send timing` line.
    pub(crate) fn finished(
        &mut self,
        id: &ClientMsgId,
        result: &Result<Message, ProviderError>,
    ) -> Option<String> {
        let now = Instant::now();
        let retry = matches!(
            result,
            Err(ProviderError::Network(_) | ProviderError::RateLimited)
        );
        let outcome = match result {
            Ok(_) => "ok",
            Err(ProviderError::Network(_)) => "network",
            Err(ProviderError::RateLimited) => "rate_limited",
            Err(_) => "failed",
        };
        let c = self.open.get(id)?;
        let echo = c
            .echo
            .map_or_else(|| "none".to_string(), |e| ms(c.enqueued, e).to_string());
        let queue = c.started.map_or(0, |s| ms(c.enqueued, s));
        let line = format!(
            "send timing: queue_ms={queue} echo_ms={echo} finished_ms={} attempts={} outcome={outcome}",
            ms(c.enqueued, now),
            c.attempts,
        );
        if retry {
            return Some(line);
        }
        if let Some(c) = self.open.remove(id)
            && c.echo.is_none()
            && result.is_ok()
        {
            if self.answered.len() >= KEEP {
                self.answered.clear();
            }
            self.answered.insert(id.clone(), c.enqueued);
        }
        Some(line)
    }
}

/// Settles a send whose task was lost: the first attempts are a network
/// failure (the outbox sends it again), then a hard failure, so a task that
/// keeps getting lost shows Retry instead of looping.
#[derive(Default)]
pub(crate) struct LostSends {
    counts: HashMap<ClientMsgId, u32>,
}

/// Lost attempts before the item is marked failed.
pub(crate) const MAX_LOST: u32 = 3;

impl LostSends {
    pub(crate) fn lost(&mut self, id: &ClientMsgId) -> ProviderError {
        if self.counts.len() >= KEEP {
            self.counts.clear();
        }
        let n = self.counts.entry(id.clone()).or_insert(0);
        *n += 1;
        if *n >= MAX_LOST {
            self.counts.remove(id);
            ProviderError::Other("send_task_lost".into())
        } else {
            ProviderError::Network("send_task_lost".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn id(s: &str) -> ClientMsgId {
        ClientMsgId(Arc::from(s))
    }

    fn ok() -> Result<Message, ProviderError> {
        Ok(domain::test_support::msg("c1", "1.000000", "x"))
    }

    fn field(line: &str, key: &str) -> String {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn a_send_in_order_makes_one_line() {
        let mut t = SendTiming::default();
        t.enqueued(&id("a"));
        t.started(&id("a"));
        assert!(t.echo(&id("a")).is_none());
        let line = t.finished(&id("a"), &ok()).unwrap();
        assert!(line.starts_with("send timing: queue_ms="), "{line}");
        assert_eq!(field(&line, "attempts"), "1");
        assert_eq!(field(&line, "outcome"), "ok");
        assert_ne!(field(&line, "echo_ms"), "none");
        assert!(field(&line, "finished_ms").parse::<u64>().is_ok(), "{line}");
        assert!(t.finished(&id("a"), &ok()).is_none(), "one line per send");
    }

    #[test]
    fn a_retry_keeps_the_enqueue_clock() {
        let mut t = SendTiming::default();
        t.enqueued(&id("a"));
        t.started(&id("a"));
        let first = t
            .finished(&id("a"), &Err(ProviderError::Network("x".into())))
            .unwrap();
        assert_eq!(field(&first, "outcome"), "network");
        std::thread::sleep(std::time::Duration::from_millis(15));
        t.started(&id("a"));
        let last = t.finished(&id("a"), &ok()).unwrap();
        assert_eq!(field(&last, "attempts"), "2");
        assert_eq!(field(&last, "echo_ms"), "none");
        let ms_of = |l: &str, k: &str| field(l, k).parse::<u64>().unwrap();
        assert!(ms_of(&last, "finished_ms") >= 15);
        assert!(ms_of(&last, "finished_ms") >= ms_of(&first, "finished_ms"));
    }

    #[test]
    fn an_echo_after_the_answer_makes_the_late_line_once() {
        let mut t = SendTiming::default();
        t.enqueued(&id("a"));
        t.started(&id("a"));
        t.finished(&id("a"), &ok());
        let late = t.echo(&id("a")).unwrap();
        assert!(late.starts_with("send late echo: echo_ms="), "{late}");
        assert!(t.echo(&id("a")).is_none());
    }

    #[test]
    fn an_echo_before_the_answer_is_on_the_line_and_no_late_line_follows() {
        let mut t = SendTiming::default();
        t.enqueued(&id("a"));
        t.started(&id("a"));
        t.echo(&id("a"));
        let line = t.finished(&id("a"), &ok()).unwrap();
        assert_ne!(field(&line, "echo_ms"), "none");
        assert!(t.echo(&id("a")).is_none());
    }

    #[test]
    fn a_send_never_enqueued_starts_its_clock_at_the_first_call() {
        let mut t = SendTiming::default();
        t.started(&id("r"));
        let line = t.finished(&id("r"), &ok()).unwrap();
        assert_eq!(field(&line, "queue_ms"), "0");
        assert_eq!(field(&line, "attempts"), "1");
    }

    #[test]
    fn tracking_is_bounded() {
        let mut t = SendTiming::default();
        for i in 0..(KEEP * 3) {
            t.enqueued(&id(&format!("s{i}")));
            t.started(&id(&format!("s{i}")));
            t.finished(&id(&format!("s{i}")), &ok());
        }
        assert!(t.open.len() <= KEEP && t.answered.len() <= KEEP);
        for i in 0..(KEEP * 3) {
            t.enqueued(&id(&format!("o{i}")));
        }
        assert!(t.open.len() <= KEEP);
        assert!(t.open.contains_key(&id(&format!("o{}", KEEP * 3 - 1))));
    }

    #[test]
    fn a_lost_task_is_retried_twice_then_fails() {
        let mut l = LostSends::default();
        assert!(matches!(l.lost(&id("a")), ProviderError::Network(_)));
        assert!(matches!(l.lost(&id("a")), ProviderError::Network(_)));
        assert!(matches!(l.lost(&id("a")), ProviderError::Other(_)));
        assert!(matches!(l.lost(&id("a")), ProviderError::Network(_)));
        assert!(matches!(l.lost(&id("b")), ProviderError::Network(_)));
    }
}
