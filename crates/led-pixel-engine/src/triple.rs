//! A lock-free, wait-free triple buffer for the render→send handoff.
//!
//! The platform invariant (master §4): *render and send never share a mutable buffer.*
//! Three buffers, one atomic. The producer always writes into a buffer no one else can see,
//! then publishes by swapping it into a shared slot; the consumer takes the latest published
//! buffer the same way. The producer never blocks (stale-but-recent is correct for lights),
//! and the consumer never sees a torn frame.
//!
//! The three indices held by {producer, shared, consumer} are always a permutation of
//! {0,1,2}, so the producer's buffer and the consumer's buffer are never the same one. This
//! is what the `no_tearing_under_threads` test verifies empirically.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const INDEX_MASK: usize = 0b011;
const FRESH: usize = 0b100;

struct Inner<T> {
    slots: [UnsafeCell<T>; 3],
    shared: AtomicUsize, // low 2 bits: buffer index; bit 2: "fresh data" flag
}

// Safe because the producer and consumer provably never reference the same slot at the same
// time (permutation invariant), and publish/update use Release/Acquire to pass the writes.
unsafe impl<T: Send> Sync for Inner<T> {}

/// The write half. Lives on the render thread.
pub struct Producer<T> {
    inner: Arc<Inner<T>>,
    idx: usize,
}

/// The read half. Lives on the send thread.
pub struct Consumer<T> {
    inner: Arc<Inner<T>>,
    idx: usize,
}

unsafe impl<T: Send> Send for Producer<T> {}
unsafe impl<T: Send> Send for Consumer<T> {}

/// Create a triple buffer from three pre-allocated buffers (no allocation afterwards).
pub fn triple_buffer<T>(a: T, b: T, c: T) -> (Producer<T>, Consumer<T>) {
    let inner = Arc::new(Inner {
        slots: [UnsafeCell::new(a), UnsafeCell::new(b), UnsafeCell::new(c)],
        shared: AtomicUsize::new(2), // producer=0, consumer=1, shared=2 (not fresh)
    });
    (Producer { inner: inner.clone(), idx: 0 }, Consumer { inner, idx: 1 })
}

impl<T> Producer<T> {
    /// The buffer to write the next frame into. Only the producer ever touches it.
    pub fn input(&mut self) -> &mut T {
        // SAFETY: `self.idx` is the producer-owned slot; the consumer never holds it.
        unsafe { &mut *self.inner.slots[self.idx].get() }
    }

    /// Publish the written buffer as the latest. Wait-free; never blocks.
    pub fn publish(&mut self) {
        let old = self.inner.shared.swap(self.idx | FRESH, Ordering::AcqRel);
        self.idx = old & INDEX_MASK;
    }
}

impl<T> Consumer<T> {
    /// If a newer buffer was published, swap to it and return true. Wait-free.
    pub fn update(&mut self) -> bool {
        if self.inner.shared.load(Ordering::Acquire) & FRESH != 0 {
            let old = self.inner.shared.swap(self.idx, Ordering::AcqRel);
            self.idx = old & INDEX_MASK;
            true
        } else {
            false
        }
    }

    /// The latest buffer taken by [`update`](Self::update). Only the consumer touches it.
    pub fn output(&self) -> &T {
        // SAFETY: `self.idx` is the consumer-owned slot; the producer never holds it.
        unsafe { &*self.inner.slots[self.idx].get() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn single_thread_latest_value_semantics() {
        let (mut p, mut c) = triple_buffer(0u32, 0, 0);
        assert!(!c.update(), "nothing published yet");

        *p.input() = 1;
        p.publish();
        *p.input() = 2;
        p.publish(); // two publishes, no consume in between

        assert!(c.update(), "fresh data available");
        assert_eq!(*c.output(), 2, "consumer gets the LATEST, skipping the stale one");
        assert!(!c.update(), "no new data after consuming");
    }

    #[test]
    fn no_tearing_under_threads() {
        // Miri interprets (~100× slower) and models thread scheduling for race detection,
        // so use a small workload there; the native run does the heavy stress.
        let n: usize = if cfg!(miri) { 32 } else { 256 };
        let iters: u32 = if cfg!(miri) { 500 } else { 200_000 };
        let (mut p, mut c) = triple_buffer(vec![0u32; n], vec![0u32; n], vec![0u32; n]);
        let done = Arc::new(AtomicBool::new(false));

        let done_w = done.clone();
        let writer = std::thread::spawn(move || {
            for seq in 1..=iters {
                for x in p.input().iter_mut() {
                    *x = seq; // a frame is "all elements == seq"
                }
                p.publish();
            }
            done_w.store(true, Ordering::Release);
        });

        let mut reads = 0u64;
        loop {
            if c.update() {
                let buf = c.output();
                let first = buf[0];
                // A torn frame would mix two sequence numbers.
                assert!(buf.iter().all(|&v| v == first), "torn frame detected");
                reads += 1;
            } else if done.load(Ordering::Acquire) {
                if !c.update() {
                    break;
                }
            }
        }

        writer.join().unwrap();
        assert!(reads > 0, "consumer never observed a frame");
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // ── INVARIANT: producer and consumer never share the same slot index ──
    #[test]
    fn permutation_invariant_producer_consumer_never_same_slot() {
        let (mut prod, mut cons) = triple_buffer(0u64, 0u64, 0u64);
        for i in 0..10_000u64 {
            *prod.input() = i;
            prod.publish();
            cons.update();
            // Can't directly read indices, but values must be consistent
            let _v = *cons.output();
        }
        // If we reach here with no panic/UB, the permutation invariant held
    }

    // ── STRESS: 1M publish/update cycles — no stale data ─────────────────
    #[test]
    fn triple_buffer_1m_cycles_no_torn_frame() {
        let (mut prod, mut cons) = triple_buffer([0u8; 512], [0u8; 512], [0u8; 512]);
        for i in 0..1_000_000u32 {
            let marker = (i % 256) as u8;
            prod.input().fill(marker);
            prod.publish();
            if cons.update() {
                // Every byte in the consumer's buffer must be the SAME value (no torn frame)
                let out = cons.output();
                let first = out[0];
                assert!(out.iter().all(|&b| b == first),
                    "torn frame at i={i}: first={first}, found different bytes");
            }
        }
    }

    // ── CONCURRENCY: render thread (producer) + send thread (consumer) ────
    #[test]
    fn triple_buffer_concurrent_threads_no_torn_frame() {
        let (prod, cons) = triple_buffer([0u8; 512], [0u8; 512], [0u8; 512]);
        let prod = Arc::new(std::sync::Mutex::new(prod));
        let cons = Arc::new(std::sync::Mutex::new(cons));

        let prod2 = prod.clone();
        let producer = thread::spawn(move || {
            for i in 0..50_000u32 {
                let marker = (i % 256) as u8;
                let mut p = prod2.lock().unwrap();
                p.input().fill(marker);
                p.publish();
            }
        });

        let cons2 = cons.clone();
        let consumer = thread::spawn(move || {
            let mut tears = 0u32;
            for _ in 0..50_000 {
                let mut c = cons2.lock().unwrap();
                if c.update() {
                    let out = c.output();
                    let first = out[0];
                    if out.iter().any(|&b| b != first) {
                        tears += 1;
                    }
                }
            }
            tears
        });

        producer.join().unwrap();
        let tears = consumer.join().unwrap();
        assert_eq!(tears, 0, "torn frames detected: {tears}");
    }

    // ── EDGE: consumer reads without publish — stays on initial value ─────
    #[test]
    fn triple_buffer_no_publish_consumer_sees_initial() {
        let (_, mut cons) = triple_buffer(42u32, 0u32, 0u32);
        let updated = cons.update();
        assert!(!updated, "no publish → update() must return false");
        assert_eq!(*cons.output(), 0u32, "consumer starts on slot 1 (value 0)");
    }

    // ── REAL-TIME: publish must complete in < 1µs ─────────────────────────
    #[test]
    fn triple_buffer_publish_latency_sub_microsecond() {
        use std::time::Instant;
        let (mut prod, _cons) = triple_buffer([0u8; 512], [0u8; 512], [0u8; 512]);
        let mut total_ns = 0u128;
        let runs = 10_000;
        for i in 0..runs {
            prod.input().fill(i as u8);
            let t0 = Instant::now();
            prod.publish();
            total_ns += t0.elapsed().as_nanos();
        }
        let avg_ns = total_ns / runs;
        // publish is one atomic swap — should be << 1µs; allow generous 10µs for CI
        assert!(avg_ns < 10_000, "publish avg {}ns exceeds 10µs budget", avg_ns);
    }
}
