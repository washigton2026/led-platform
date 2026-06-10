//! A fixed-capacity, single-producer/single-consumer lock-free ring buffer of `f32`
//! samples. The CPAL audio callback (producer) pushes captured samples; the analysis
//! thread (consumer) pops fixed-size hops. The backing storage is allocated once in
//! [`RingBuffer::new`] — `push_slice`/`pop_exact` never allocate.
//!
//! Indices are monotonically increasing `usize` counters masked on access (classic SPSC
//! ring buffer); `capacity` must be a power of two.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct RingBuffer {
    buf: Box<[UnsafeCell<f32>]>,
    mask: usize,
    write: AtomicUsize,
    read: AtomicUsize,
}

// SAFETY: `buf` cells are written only by the single producer (push_slice) at indices in
// `[write_old, write_new)` and read only by the single consumer (pop_exact) at indices in
// `[read_old, read_new)`. The producer publishes new data via `write.store(Release)` and
// the consumer observes it via `write.load(Acquire)` (and symmetrically for `read`), so the
// two sides never touch overlapping indices concurrently.
unsafe impl Sync for RingBuffer {}

impl RingBuffer {
    /// `capacity` must be a power of two (panics otherwise).
    pub fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two(), "RingBuffer capacity must be a power of two");
        let buf = (0..capacity).map(|_| UnsafeCell::new(0.0f32)).collect::<Vec<_>>().into_boxed_slice();
        Self { buf, mask: capacity - 1, write: AtomicUsize::new(0), read: AtomicUsize::new(0) }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Number of samples currently available to read.
    pub fn available(&self) -> usize {
        let w = self.write.load(Ordering::Acquire);
        let r = self.read.load(Ordering::Relaxed);
        w.wrapping_sub(r)
    }

    /// Producer side. Pushes as many samples from `data` as fit, dropping the tail of
    /// `data` if the buffer is full. Returns the number of samples written.
    pub fn push_slice(&self, data: &[f32]) -> usize {
        let r = self.read.load(Ordering::Acquire);
        let w = self.write.load(Ordering::Relaxed);
        let free = self.capacity() - w.wrapping_sub(r);
        let n = data.len().min(free);
        for (i, &sample) in data[..n].iter().enumerate() {
            let idx = (w.wrapping_add(i)) & self.mask;
            // SAFETY: index is within `[w, w+n)`, exclusively owned by the producer.
            unsafe { *self.buf[idx].get() = sample };
        }
        self.write.store(w.wrapping_add(n), Ordering::Release);
        n
    }

    /// Consumer side. If at least `out.len()` samples are available, fills `out` and
    /// advances the read cursor, returning `true`. Otherwise leaves the buffer untouched
    /// and returns `false`.
    pub fn pop_exact(&self, out: &mut [f32]) -> bool {
        let w = self.write.load(Ordering::Acquire);
        let r = self.read.load(Ordering::Relaxed);
        if w.wrapping_sub(r) < out.len() {
            return false;
        }
        for (i, slot) in out.iter_mut().enumerate() {
            let idx = (r.wrapping_add(i)) & self.mask;
            // SAFETY: index is within `[r, r+out.len())`, exclusively owned by the consumer,
            // and was published by the producer's Release store observed via the Acquire
            // load of `write` above.
            *slot = unsafe { *self.buf[idx].get() };
        }
        self.read.store(r.wrapping_add(out.len()), Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_pop_round_trip() {
        let rb = RingBuffer::new(16);
        assert_eq!(rb.push_slice(&[1.0, 2.0, 3.0, 4.0]), 4);
        assert_eq!(rb.available(), 4);

        let mut out = [0.0f32; 4];
        assert!(rb.pop_exact(&mut out));
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rb.available(), 0);
    }

    #[test]
    fn pop_exact_returns_false_without_consuming_when_short() {
        let rb = RingBuffer::new(16);
        rb.push_slice(&[1.0, 2.0]);

        let mut out = [0.0f32; 4];
        assert!(!rb.pop_exact(&mut out));
        assert_eq!(rb.available(), 2, "a failed pop must not consume partial data");

        rb.push_slice(&[3.0, 4.0]);
        assert!(rb.pop_exact(&mut out));
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn push_slice_truncates_when_full() {
        let rb = RingBuffer::new(4);
        assert_eq!(rb.push_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), 4);
        assert_eq!(rb.available(), 4);
    }

    #[test]
    fn wraps_around_correctly_over_many_cycles() {
        let rb = RingBuffer::new(8);
        let mut next_push = 0.0f32;
        let mut next_pop = 0.0f32;

        for _ in 0..1000 {
            // Push a hop of 3, pop a hop of 3 — exercises wraparound since 8 % 3 != 0.
            let chunk: [f32; 3] = std::array::from_fn(|i| next_push + i as f32);
            assert_eq!(rb.push_slice(&chunk), 3);
            next_push += 3.0;

            let mut out = [0.0f32; 3];
            assert!(rb.pop_exact(&mut out));
            let expected: [f32; 3] = std::array::from_fn(|i| next_pop + i as f32);
            assert_eq!(out, expected);
            next_pop += 3.0;
        }
    }

    #[test]
    fn spsc_stress_no_loss_or_reorder_under_threads() {
        use std::sync::Arc;

        const CHUNK: usize = 4;
        // Miri interprets (~100x slower) and models thread scheduling for race detection,
        // so use a small workload there; the native run does the heavy stress.
        let iters: u64 = if cfg!(miri) { 200 } else { 20_000 };

        let rb = Arc::new(RingBuffer::new(64));

        let producer_rb = rb.clone();
        let producer = std::thread::spawn(move || {
            let mut next = 0u64;
            while next < iters {
                let chunk: [f32; CHUNK] = std::array::from_fn(|i| (next + i as u64) as f32);
                let mut written = 0;
                while written < CHUNK {
                    written += producer_rb.push_slice(&chunk[written..]);
                }
                next += CHUNK as u64;
            }
        });

        let mut next_expected = 0u64;
        let mut out = [0.0f32; CHUNK];
        while next_expected < iters {
            if rb.pop_exact(&mut out) {
                for (i, &v) in out.iter().enumerate() {
                    assert_eq!(v, (next_expected + i as u64) as f32, "data must arrive in order, unmodified");
                }
                next_expected += CHUNK as u64;
            } else {
                std::thread::yield_now();
            }
        }
        producer.join().unwrap();
    }
}
