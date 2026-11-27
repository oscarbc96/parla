//! Optimistic writes in flight. Shared by the DND and notification-level
//! writes, the profile and status writes and the pins, saved items and
//! bookmarks writes: each key tracks how many answers are outstanding and
//! whether any failed.

/// Optimistic writes in flight for one key (the self DND, or one
/// conversation's prefs). No prior values are kept: when the last answer
/// is in and any write failed, the truth is refetched.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Flight {
    in_flight: u32,
    failed: bool,
    /// Another write was issued while this key already had one in flight,
    /// so no single answer is known to be the latest.
    overlapped: bool,
}

impl Flight {
    pub(crate) fn begin(&mut self) {
        if self.in_flight > 0 {
            self.overlapped = true;
        }
        self.in_flight += 1;
    }
}

/// What an answer means for its key.
pub(crate) struct Settled {
    /// This answer was the only write of its key, so its value is the
    /// latest the provider knows.
    pub(crate) authoritative: bool,
    /// Every write is answered and one failed: refetch the truth.
    pub(crate) refetch: bool,
    /// Every write is answered.
    pub(crate) idle: bool,
}

pub(crate) fn settle(slot: &mut Option<Flight>, err: bool) -> Settled {
    let mut f = slot.take().unwrap_or_default();
    f.in_flight = f.in_flight.saturating_sub(1);
    f.failed |= err;
    let authoritative = !f.overlapped && !err;
    if f.in_flight == 0 {
        return Settled {
            authoritative,
            refetch: f.failed,
            idle: true,
        };
    }
    *slot = Some(f);
    Settled {
        authoritative,
        refetch: false,
        idle: false,
    }
}
