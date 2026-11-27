//! The wall clock behind a trait, so UI tests can pin the time.

use jiff::Timestamp;

/// A source of the current time.
pub trait Now: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The real clock. The only place UI-facing code reads the system time.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemNow;

impl Now for SystemNow {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock that always returns the same instant.
#[cfg(feature = "test-support")]
#[derive(Clone, Copy, Debug)]
pub struct FixedNow(pub Timestamp);

#[cfg(feature = "test-support")]
impl Now for FixedNow {
    fn now(&self) -> Timestamp {
        self.0
    }
}
