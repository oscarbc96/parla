//! Perf probes for the `fixture` feature: a counting
//! allocator, a frame counter and a live-heap reading. Compiled only with
//! `--features fixture`; normal builds carry none of it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

pub mod check;
pub mod driver;
pub mod report;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static INPUT_FRAMES: AtomicU64 = AtomicU64::new(0);
static LAST_INPUT_STATE: std::sync::Mutex<Option<(i32, i32, bool)>> = std::sync::Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();
static FIRST_FRAME: OnceLock<Instant> = OnceLock::new();

/// `System` plus two relaxed counters. Install it with `#[global_allocator]`
/// in the binary or test binary that reads [`allocations`] or [`live_heap`].
pub struct CountingAlloc;

// SAFETY: every call is forwarded unchanged to `System`; the counters only
// observe sizes.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(l.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(l.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(l) }
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE_BYTES.fetch_sub(l.size() as i64, Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(new as i64 - l.size() as i64, Ordering::Relaxed);
        unsafe { System.realloc(p, l, new) }
    }
}

/// Allocations since process start (needs [`CountingAlloc`] installed).
pub fn allocations() -> u64 {
    ALLOCATIONS.load(Ordering::Relaxed)
}

/// Bytes currently allocated (needs [`CountingAlloc`] installed).
pub fn live_heap() -> u64 {
    LIVE_BYTES.load(Ordering::Relaxed).max(0) as u64
}

/// Called by the root view once per drawn frame.
pub fn note_frame() {
    let _ = FIRST_FRAME.set(Instant::now());
    FRAMES.fetch_add(1, Ordering::Relaxed);
}

/// Called by the root view once per drawn frame with the pointer position and
/// whether the window is active. A frame drawn after either changed answers
/// the user (a hover, a focus change), not idle work: it is counted by
/// [`input_frames`] as well, so a person using the machine during an idle run
/// does not fail the budget.
pub fn note_frame_input(x: i32, y: i32, active: bool) {
    let state = Some((x, y, active));
    let mut last = LAST_INPUT_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some() && *last != state {
        INPUT_FRAMES.fetch_add(1, Ordering::Relaxed);
    }
    *last = state;
}

/// Frames drawn since process start that followed a pointer or focus change.
pub fn input_frames() -> u64 {
    INPUT_FRAMES.load(Ordering::Relaxed)
}

/// Frames drawn since process start.
pub fn frames_drawn() -> u64 {
    FRAMES.load(Ordering::Relaxed)
}

/// Marks the start of the process; call first thing in `main`.
pub fn mark_process_start() {
    let _ = START.set(Instant::now());
}

/// When [`mark_process_start`] ran (or now, if it never did).
pub fn process_start() -> Instant {
    *START.get_or_init(Instant::now)
}

/// When the first frame was presented, if one was.
pub fn first_frame() -> Option<Instant> {
    FIRST_FRAME.get().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one test that touches the input-frame state (it is process-wide).
    #[test]
    fn frames_after_a_pointer_or_focus_change_are_attributed_to_input() {
        note_frame_input(10, 20, true);
        let base = input_frames();
        note_frame_input(10, 20, true);
        assert_eq!(input_frames(), base, "same pointer and focus: idle work");
        note_frame_input(11, 20, true);
        assert_eq!(input_frames(), base + 1, "the pointer moved");
        note_frame_input(11, 20, false);
        assert_eq!(input_frames(), base + 2, "focus changed");
        note_frame_input(11, 20, false);
        assert_eq!(input_frames(), base + 2);
    }
}
