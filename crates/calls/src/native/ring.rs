//! Fixed-capacity single-producer single-consumer sample ring.
//!
//! Preallocated and lock-free: the audio callbacks push and pop without
//! allocating or taking a lock. Samples are `f32` bits in `AtomicU32` slots,
//! so it needs no `unsafe`. Exactly one thread may push and one may pop.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub struct Ring {
    slots: Box<[AtomicU32]>,
    mask: usize,
    /// Total samples ever written / read; wrapping.
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
}

impl Ring {
    /// A ring holding at least `capacity` samples (rounded up to a power of
    /// two).
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(2).next_power_of_two();
        Self {
            slots: (0..cap).map(|_| AtomicU32::new(0)).collect(),
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Samples currently queued.
    pub fn len(&self) -> usize {
        self.head
            .load(Ordering::Acquire)
            .wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    /// Samples dropped because the ring was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Producer side: writes what fits, counts the rest as dropped and
    /// returns the number written.
    pub fn push_slice(&self, src: &[f32]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let used = head.wrapping_sub(self.tail.load(Ordering::Acquire));
        let n = src.len().min(self.capacity() - used);
        for (i, s) in src[..n].iter().enumerate() {
            self.slots[head.wrapping_add(i) & self.mask].store(s.to_bits(), Ordering::Relaxed);
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        if n < src.len() {
            self.dropped
                .fetch_add((src.len() - n) as u64, Ordering::Relaxed);
        }
        n
    }

    /// Consumer side: reads up to `dst.len()` samples, returns the count.
    pub fn pop_slice(&self, dst: &mut [f32]) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let avail = self.head.load(Ordering::Acquire).wrapping_sub(tail);
        let n = dst.len().min(avail);
        for (i, d) in dst[..n].iter_mut().enumerate() {
            *d = f32::from_bits(
                self.slots[tail.wrapping_add(i) & self.mask].load(Ordering::Relaxed),
            );
        }
        self.tail.store(tail.wrapping_add(n), Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_pop_keeps_order_across_the_wrap() {
        let r = Ring::new(8);
        let mut out = [0.0; 8];
        let mut next = 0.0;
        for _ in 0..20 {
            let chunk: Vec<f32> = (0..5)
                .map(|_| {
                    next += 1.0;
                    next
                })
                .collect();
            assert_eq!(r.push_slice(&chunk), 5);
            assert_eq!(r.pop_slice(&mut out[..5]), 5);
            assert_eq!(&out[..5], &chunk[..]);
        }
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn full_ring_drops_and_counts() {
        let r = Ring::new(4);
        assert_eq!(r.push_slice(&[1.0; 6]), 4);
        assert_eq!(r.dropped(), 2);
        assert_eq!(r.len(), 4);
        let mut out = [0.0; 8];
        assert_eq!(r.pop_slice(&mut out), 4);
        assert_eq!(r.pop_slice(&mut out), 0);
    }

    #[test]
    fn producer_and_consumer_threads_agree() {
        let r = std::sync::Arc::new(Ring::new(64));
        let p = r.clone();
        let t = std::thread::spawn(move || {
            let mut i = 0u32;
            while i < 10_000 {
                if p.push_slice(&[i as f32]) == 1 {
                    i += 1;
                } else {
                    std::thread::yield_now();
                }
            }
        });
        let mut seen = 0u32;
        let mut buf = [0.0f32; 16];
        while seen < 10_000 {
            let n = r.pop_slice(&mut buf);
            for v in &buf[..n] {
                assert_eq!(*v, seen as f32);
                seen += 1;
            }
            if n == 0 {
                std::thread::yield_now();
            }
        }
        t.join().unwrap();
    }
}
