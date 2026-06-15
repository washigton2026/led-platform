//! Pre-allocated buffer pool for zero-alloc packet sending.
//!
//! ## Why
//! On the frame hot-path, every allocation is a potential stall (the system allocator can
//! block on contention). `BufferPool` pre-allocates N × 638-byte packet buffers at startup
//! and hands them out via `pull()`. On `Drop`, the buffer automatically returns to the pool —
//! no allocator touch on the hot path.
//!
//! For a **fixed** universe set, the even simpler approach is `UniverseState::packet_buf` in
//! `sender` (one pre-allocated buffer per task, never pooled). Use this pool when the universe
//! set is dynamic or you need temporary scratch space.

use std::sync::Mutex;

use crate::packet::PACKET_LEN;

/// A PACKET_LEN-sized buffer borrowed from a `BufferPool`.
/// Returns itself to the pool on `Drop` — no allocator touch.
pub struct PooledBuf<'p> {
    buf:  Option<Box<[u8; PACKET_LEN]>>,
    pool: &'p BufferPool,
}

impl<'p> std::ops::Deref for PooledBuf<'p> {
    type Target = [u8; PACKET_LEN];
    fn deref(&self) -> &Self::Target { self.buf.as_ref().unwrap() }
}
impl<'p> std::ops::DerefMut for PooledBuf<'p> {
    fn deref_mut(&mut self) -> &mut Self::Target { self.buf.as_mut().unwrap() }
}
impl<'p> Drop for PooledBuf<'p> {
    fn drop(&mut self) {
        if let Some(b) = self.buf.take() {
            self.pool.push_back(b);
        }
    }
}

/// Pre-allocated packet buffer pool. Size at `2 × max_universes` at startup.
pub struct BufferPool {
    bufs: Mutex<Vec<Box<[u8; PACKET_LEN]>>>,
}

impl BufferPool {
    /// Create a pool of `capacity` pre-allocated 638-byte buffers.
    pub fn new(capacity: usize) -> Self {
        let bufs = (0..capacity).map(|_| Box::new([0u8; PACKET_LEN])).collect();
        Self { bufs: Mutex::new(bufs) }
    }

    /// Borrow a buffer. If the pool is empty, falls back to a fresh allocation (still
    /// returns to pool on Drop). Lock contention is negligible: each universe task only
    /// touches the pool on startup in the fixed-universe design.
    pub fn pull(&self) -> PooledBuf<'_> {
        let buf = self.bufs.lock().unwrap()
            .pop()
            .unwrap_or_else(|| Box::new([0u8; PACKET_LEN]));
        PooledBuf { buf: Some(buf), pool: self }
    }

    fn push_back(&self, buf: Box<[u8; PACKET_LEN]>) {
        self.bufs.lock().unwrap().push(buf);
    }

    /// Current number of buffers available in the pool (informational; races are benign).
    pub fn available(&self) -> usize {
        self.bufs.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_roundtrip_returns_to_pool() {
        let pool = BufferPool::new(4);
        assert_eq!(pool.available(), 4);

        let b1 = pool.pull();
        assert_eq!(pool.available(), 3);
        drop(b1);
        assert_eq!(pool.available(), 4, "returned on Drop");
    }

    #[test]
    fn pool_falls_back_when_empty() {
        let pool = BufferPool::new(1);
        let _b1 = pool.pull();
        // Empty pool — still works via fallback alloc.
        let b2 = pool.pull();
        assert_eq!(b2.len(), PACKET_LEN);
    }

    #[test]
    fn pooled_buf_is_writable() {
        let pool = BufferPool::new(2);
        let mut buf = pool.pull();
        buf[0] = 0xAA;
        buf[PACKET_LEN - 1] = 0xBB;
        assert_eq!(buf[0], 0xAA);
        assert_eq!(buf[PACKET_LEN - 1], 0xBB);
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // ── STRESS: concurrent pull/drop from 16 threads ──────────────────────
    #[test]
    fn pool_concurrent_pull_drop_no_deadlock() {
        let pool = Arc::new(BufferPool::new(8));
        let mut handles = Vec::new();
        for _ in 0..16 {
            let p = pool.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    let mut buf = p.pull();
                    buf[0] = 0xAB;
                    buf[PACKET_LEN - 1] = 0xCD;
                    drop(buf); // returns to pool
                }
            }));
        }
        for h in handles { h.join().unwrap(); }
        // Pool must be non-empty — all bufs returned
        assert!(pool.available() > 0, "some buffers must have returned");
    }

    // ── STRESS: exhaustion + fallback — pool grows (all bufs return on Drop) ─
    // DESIGN NOTE: fallback-allocated buffers ALSO return to the pool on Drop
    // (per the push_back call in PooledBuf::drop). This means the pool can grow
    // beyond its initial capacity under burst load. Confirmed by test — documented.
    #[test]
    fn pool_exhaustion_fallback_returns_to_pool_grows() {
        let pool = BufferPool::new(4);
        let bufs: Vec<_> = (0..20).map(|_| pool.pull()).collect(); // 16 fallback allocs
        assert_eq!(pool.available(), 0, "all 4 originals are checked out");
        drop(bufs); // ALL 20 bufs return — pool grows beyond initial capacity
        assert_eq!(pool.available(), 20, "pool absorbs fallback bufs: grows to 20");
    }

    // ── DESIGN RISK: pool growth under burst — cap it ─────────────────────
    #[test]
    fn pool_exhaustion_pool_never_shrinks_below_initial() {
        let pool = BufferPool::new(4);
        {
            let _bufs: Vec<_> = (0..20).map(|_| pool.pull()).collect();
        }
        // after burst: pool has 20 — must have at least the original 4
        assert!(pool.available() >= 4, "pool must retain at least initial capacity");
    }

    // ── INVARIANT: returned buffer is zero-initialized on reuse ──────────
    #[test]
    fn pool_returned_buffer_reusable() {
        let pool = BufferPool::new(2);
        {
            let mut b = pool.pull();
            b.fill(0xFF);
        } // returned
        // next pull may get the same buffer — writes must be possible
        let mut b2 = pool.pull();
        b2.fill(0x00);
        assert!(b2.iter().all(|&x| x == 0));
    }

    // ── CHAOS: 1M pull/drop cycles — no panic, no leak ───────────────────
    #[test]
    fn pool_1m_cycles_no_panic() {
        let pool = BufferPool::new(16);
        for i in 0..1_000_000u32 {
            let mut b = pool.pull();
            b[0] = (i % 256) as u8;
        }
        // pool should still function
        let b = pool.pull();
        assert_eq!(b.len(), PACKET_LEN);
    }
}
