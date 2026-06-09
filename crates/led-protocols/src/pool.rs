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
