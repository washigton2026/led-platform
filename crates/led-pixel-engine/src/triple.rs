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
