//! Free disk space on the store's volume, for the "Disk nearly full" banner.
//!
//! The free space is read (a `statvfs` call, off the main thread) when a
//! workspace store opens, after store writes and when the window is
//! activated, and the store lets one read through every five minutes over all
//! workspaces. There is no timer: an idle app never reads. The banner shows
//! under 1 GiB free and goes away over 1.5 GiB; in between it keeps its state,
//! so it does not flicker. On APFS, purgeable space is not counted as free.

use gpui::{App, Global};

const GIB: u64 = 1024 * 1024 * 1024;

/// The banner shows under this much free space.
pub const LOW_BYTES: u64 = GIB;

/// The banner hides over this much free space.
pub const OK_BYTES: u64 = GIB + GIB / 2;

/// Whether the banner shows after reading `free` bytes, given whether it did.
pub fn next_low(was_low: bool, free: u64) -> bool {
    if free < LOW_BYTES {
        true
    } else if free > OK_BYTES {
        false
    } else {
        was_low
    }
}

/// The last free-space reading, as a global. Absent until the first read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreUsage {
    pub free_bytes: u64,
    pub low: bool,
}

impl Global for StoreUsage {}

impl StoreUsage {
    /// Whether the banner shows.
    pub fn banner_visible(cx: &App) -> bool {
        cx.try_global::<StoreUsage>().is_some_and(|u| u.low)
    }

    /// A new reading: redraws when the banner changes.
    pub fn record(free_bytes: u64, cx: &mut App) {
        let was = cx.try_global::<StoreUsage>().is_some_and(|u| u.low);
        let low = next_low(was, free_bytes);
        cx.set_global(StoreUsage { free_bytes, low });
        if low != was {
            cx.refresh_windows();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_under_one_gib_and_hides_over_one_and_a_half() {
        assert!(next_low(false, GIB - 1));
        assert!(!next_low(false, GIB), "exactly 1 GiB is not under it");
        assert!(next_low(true, GIB + 1), "inside the band the state holds");
        assert!(!next_low(false, GIB + 1));
        assert!(next_low(true, OK_BYTES), "exactly 1.5 GiB is not over it");
        assert!(!next_low(true, OK_BYTES + 1));
    }
}
