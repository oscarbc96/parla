//! Fixed-target jitter buffer: reorder, loss, duplicates and overflow.
//!
//! It never owns a timer. The media thread calls [`JitterBuffer::pop`] once
//! per 20 ms frame the output device asks for, so playout is clocked by the
//! speaker, not by the microphone.

use std::collections::BTreeMap;

/// Frames buffered before playout starts (60 ms of 20 ms frames).
pub const TARGET_FRAMES: usize = 3;
/// Hard bound on buffered frames; beyond it the oldest is dropped.
pub const MAX_FRAMES: usize = 10;
/// Consecutive late packets after which the buffer assumes the sender
/// restarted its sequence and resyncs to it.
pub const RESYNC_AFTER: usize = 8;

/// What to play for the next 20 ms.
#[derive(Debug, PartialEq, Eq)]
pub enum Playout {
    /// An in-order Opus payload.
    Frame(Vec<u8>),
    /// A missing frame: conceal it (Opus PLC).
    Lost,
    /// Nothing to play (still filling to the target).
    Empty,
}

/// Counters, for logs and tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitterStats {
    pub duplicates: u64,
    pub late: u64,
    pub overflow: u64,
    pub lost: u64,
    pub resyncs: u64,
}

pub struct JitterBuffer {
    frames: BTreeMap<u64, Vec<u8>>,
    last_seq: Option<u16>,
    last_ext: u64,
    next: u64,
    playing: bool,
    /// Playback has begun at least once, so older packets are late even
    /// while rebuffering after an underrun.
    started: bool,
    late_streak: usize,
    stats: JitterStats,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self {
            frames: BTreeMap::new(),
            last_seq: None,
            last_ext: 0,
            next: 0,
            playing: false,
            started: false,
            late_streak: 0,
            stats: JitterStats::default(),
        }
    }

    /// Forgets the stream (a new SSRC); counters are kept.
    pub fn reset(&mut self) {
        self.frames.clear();
        self.last_seq = None;
        self.last_ext = 0;
        self.next = 0;
        self.playing = false;
        self.started = false;
        self.late_streak = 0;
    }

    /// The buffered payload right after the frame last popped, if present:
    /// after [`Playout::Lost`] it carries the in-band FEC for the lost one.
    pub fn next_payload(&self) -> Option<&[u8]> {
        self.frames.get(&self.next).map(Vec::as_slice)
    }

    pub fn push(&mut self, seq: u16, payload: Vec<u8>) {
        let ext = match self.last_seq {
            None => {
                // Offset so earlier sequence numbers cannot underflow.
                self.last_ext = u64::from(seq) + 65_536;
                self.last_seq = Some(seq);
                self.last_ext
            }
            Some(last) => {
                let diff = i64::from(seq.wrapping_sub(last) as i16);
                let ext = (self.last_ext as i64 + diff).max(0) as u64;
                if ext > self.last_ext {
                    self.last_ext = ext;
                    self.last_seq = Some(seq);
                }
                ext
            }
        };
        if self.started && ext < self.next {
            self.stats.late += 1;
            self.late_streak += 1;
            if self.late_streak >= RESYNC_AFTER {
                self.stats.resyncs += 1;
                self.reset();
                self.push(seq, payload);
            }
            return;
        }
        self.late_streak = 0;
        if self.frames.contains_key(&ext) {
            self.stats.duplicates += 1;
            return;
        }
        self.frames.insert(ext, payload);
        if self.frames.len() > MAX_FRAMES {
            self.frames.pop_first();
            self.stats.overflow += 1;
            if let Some(&min) = self.frames.keys().next() {
                self.next = self.next.max(min);
            }
        }
    }

    pub fn pop(&mut self) -> Playout {
        if !self.playing {
            if self.frames.len() < TARGET_FRAMES {
                return Playout::Empty;
            }
            self.playing = true;
            self.started = true;
            self.next = *self.frames.keys().next().expect("non-empty");
        }
        if let Some(frame) = self.frames.remove(&self.next) {
            self.next += 1;
            return Playout::Frame(frame);
        }
        match self.frames.keys().next().copied() {
            Some(min) if min - self.next > MAX_FRAMES as u64 => {
                // Too far ahead to conceal: jump to the data we have.
                self.next = min + 1;
                Playout::Frame(self.frames.remove(&min).expect("present"))
            }
            Some(_) => {
                self.stats.lost += 1;
                self.next += 1;
                Playout::Lost
            }
            None => {
                // Underrun: conceal this frame, then refill to the target.
                self.stats.lost += 1;
                self.playing = false;
                Playout::Lost
            }
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn stats(&self) -> JitterStats {
        self.stats
    }
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: u8) -> Vec<u8> {
        vec![n]
    }

    fn fill(jb: &mut JitterBuffer, seqs: &[u16]) {
        for &s in seqs {
            jb.push(s, p(s as u8));
        }
    }

    #[test]
    fn waits_for_target_then_plays_in_order() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[10, 11]);
        assert_eq!(jb.pop(), Playout::Empty);
        fill(&mut jb, &[12]);
        assert_eq!(jb.pop(), Playout::Frame(p(10)));
        assert_eq!(jb.pop(), Playout::Frame(p(11)));
        assert_eq!(jb.pop(), Playout::Frame(p(12)));
    }

    #[test]
    fn reorders_within_the_target() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 3, 2]);
        assert_eq!(jb.pop(), Playout::Frame(p(1)));
        assert_eq!(jb.pop(), Playout::Frame(p(2)));
        assert_eq!(jb.pop(), Playout::Frame(p(3)));
    }

    #[test]
    fn missing_frame_is_concealed() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 3, 4]);
        assert_eq!(jb.pop(), Playout::Frame(p(1)));
        assert_eq!(jb.pop(), Playout::Lost);
        assert_eq!(jb.pop(), Playout::Frame(p(3)));
        assert_eq!(jb.pop(), Playout::Frame(p(4)));
        assert_eq!(jb.stats().lost, 1);
    }

    #[test]
    fn duplicates_and_late_packets_are_dropped() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 2, 2, 3]);
        assert_eq!(jb.stats().duplicates, 1);
        assert_eq!(jb.pop(), Playout::Frame(p(1)));
        assert_eq!(jb.pop(), Playout::Frame(p(2)));
        jb.push(1, p(1));
        assert_eq!(jb.stats().late, 1);
        assert_eq!(jb.len(), 1);
    }

    #[test]
    fn overflow_drops_the_oldest() {
        let mut jb = JitterBuffer::new();
        for s in 0..(MAX_FRAMES as u16 + 4) {
            jb.push(s, p(s as u8));
        }
        assert_eq!(jb.len(), MAX_FRAMES);
        assert_eq!(jb.stats().overflow, 4);
        assert_eq!(jb.pop(), Playout::Frame(p(4)));
    }

    #[test]
    fn sequence_wraparound_keeps_order() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[65534, 65535, 0, 1]);
        for s in [65534u16, 65535, 0, 1] {
            assert_eq!(jb.pop(), Playout::Frame(p(s as u8)));
        }
    }

    #[test]
    fn underrun_conceals_once_then_rebuffers() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 2, 3]);
        for _ in 0..3 {
            jb.pop();
        }
        assert_eq!(jb.pop(), Playout::Lost);
        assert_eq!(jb.pop(), Playout::Empty);
        fill(&mut jb, &[4, 5, 6]);
        assert_eq!(jb.pop(), Playout::Frame(p(4)));
    }

    #[test]
    fn large_gap_resyncs_instead_of_concealing_forever() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 2, 3]);
        for _ in 0..3 {
            jb.pop();
        }
        fill(&mut jb, &[500, 501, 502]);
        let mut lost = 0;
        loop {
            match jb.pop() {
                Playout::Lost => lost += 1,
                Playout::Frame(f) => {
                    assert_eq!(f, p(500u16 as u8));
                    break;
                }
                Playout::Empty => {}
            }
            assert!(lost <= MAX_FRAMES, "concealed too long");
        }
    }

    #[test]
    fn late_packets_stay_rejected_after_an_underrun() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 2, 3]);
        for _ in 0..4 {
            jb.pop();
        }
        // Underran and is rebuffering; seq 2 was already played.
        jb.push(2, p(2));
        assert_eq!(jb.stats().late, 1);
        assert_eq!(jb.len(), 0);
    }

    #[test]
    fn consecutive_late_packets_resync_to_the_new_stream() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1000, 1001, 1002, 1003]);
        for _ in 0..4 {
            jb.pop();
        }
        // The sender restarted its sequence numbers far behind.
        for s in 0..RESYNC_AFTER as u16 + 3 {
            jb.push(s, p(s as u8));
        }
        assert_eq!(jb.stats().resyncs, 1);
        assert!(jb.len() >= TARGET_FRAMES);
        assert!(matches!(jb.pop(), Playout::Frame(_)));
    }

    #[test]
    fn reset_forgets_the_stream_but_keeps_counters() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[500, 501, 502]);
        jb.pop();
        jb.push(500, p(0));
        jb.reset();
        assert!(jb.is_empty());
        assert_eq!(jb.stats().late, 1);
        fill(&mut jb, &[7, 8, 9]);
        assert_eq!(jb.pop(), Playout::Frame(p(7)));
    }

    #[test]
    fn next_payload_peeks_the_frame_after_a_gap() {
        let mut jb = JitterBuffer::new();
        fill(&mut jb, &[1, 3, 4]);
        assert_eq!(jb.pop(), Playout::Frame(p(1)));
        assert_eq!(jb.pop(), Playout::Lost);
        assert_eq!(jb.next_payload(), Some(&p(3)[..]));
    }
}
