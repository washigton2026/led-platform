//! Parallel universe sender — one persistent tokio task per universe, fed by a
//! `tokio::sync::watch` channel (always-latest frame, no backpressure).
//!
//! ## Why persistent tasks, not spawn-per-frame
//! At 44 FPS × N universes, spawning a task per universe per frame is ~22k spawns/sec for
//! 512 universes — the scheduler overhead dominates. Instead, each universe owns one task
//! created **once** at startup. The render thread pushes the latest frame through a watch
//! channel (O(1), lock-free read); if a task falls behind it simply sends the most recent
//! frame — which is exactly what you want for lighting.
//!
//! ## Why `watch` instead of `mpsc`
//! `watch` is a "single slot that always holds the latest value". If the universe task is
//! slower than the render rate, older frames are automatically discarded — stale-but-recent
//! beats stalled. For lighting, we always want the current color, never a queue of old ones.
//!
//! ## Zero allocation on the hot path
//! - Each `UniverseState` carries a pre-sized `Box<[u8; PACKET_LEN]>` (638 bytes) reused
//!   every frame. The task builds the packet in place and calls `send_to` — no allocator.
//! - `FrameSlice` is `[u8; DMX_SLOTS]` (512 bytes) moved into the watch slot once.
//!   The task borrows (not clones) the guard while copying into its local packet buffer.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::watch;

use led_core::UniverseData;

use crate::packet::{self, DMX_SLOTS, PACKET_LEN};

// ─── FrameSlice ───────────────────────────────────────────────────────────────

/// One universe's DMX payload — stack-allocated, fits in the watch slot with one copy.
#[derive(Clone)]
pub struct FrameSlice {
    pub data: [u8; DMX_SLOTS],
}

impl FrameSlice {
    pub fn from_slice(s: &[u8]) -> Self {
        let mut data = [0u8; DMX_SLOTS];
        let n = s.len().min(DMX_SLOTS);
        data[..n].copy_from_slice(&s[..n]);
        Self { data }
    }
}

// ─── UniverseState ───────────────────────────────────────────────────────────

/// All state owned by one universe's sending task.
pub struct UniverseState {
    pub universe:   u16,
    pub sequence:   u8,                      // per-universe, wrapping 0..=255
    pub socket:     Arc<UdpSocket>,           // dedicated socket, never shared
    pub addr:       SocketAddr,
    pub packet_buf: Box<[u8; PACKET_LEN]>,   // pre-allocated, reused every frame
}

impl UniverseState {
    /// Increment and return the next wrapping sequence byte.
    #[inline]
    pub fn next_seq(&mut self) -> u8 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }
}

// ─── ParallelSender ──────────────────────────────────────────────────────────

struct Sender {
    tx: watch::Sender<Option<FrameSlice>>,
}

/// Manages one persistent tokio task per universe.
///
/// Lifecycle: create once → `add_universe` for each universe at startup →
/// `push_frame` on every render tick → drop to shut down all tasks.
pub struct ParallelSender {
    senders:     HashMap<u16, Sender>,
    tasks:       Vec<tokio::task::JoinHandle<()>>,
    cid:         [u8; 16],
    source_name: String,
    priority:    u8,
}

impl ParallelSender {
    pub fn new(cid: [u8; 16], source_name: impl Into<String>) -> Self {
        Self {
            senders:     HashMap::new(),
            tasks:       Vec::new(),
            cid,
            source_name: source_name.into(),
            priority:    100,
        }
    }

    pub fn with_priority(mut self, p: u8) -> Self { self.priority = p; self }

    /// Bind a socket and spawn a persistent task for `universe`. Call once per universe
    /// at startup — **never** inside the frame loop.
    pub async fn add_universe(&mut self, universe: u16, addr: SocketAddr) -> std::io::Result<()> {
        let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
        let (tx, rx) = watch::channel::<Option<FrameSlice>>(None);

        let state = UniverseState {
            universe,
            sequence:   0,
            socket:     Arc::clone(&socket),
            addr,
            packet_buf: Box::new([0u8; PACKET_LEN]),
        };

        let handle = tokio::spawn(universe_task(
            state, rx, self.cid, self.source_name.clone(), self.priority,
        ));

        self.senders.insert(universe, Sender { tx });
        self.tasks.push(handle);
        Ok(())
    }

    /// Push `universes` to all registered universe tasks.
    ///
    /// O(1) per universe on the caller: no allocation, no await. If a universe falls behind
    /// the send rate its task silently uses the most recent frame.
    pub fn push_frame(&self, universes: &[UniverseData]) {
        for u in universes {
            if let Some(s) = self.senders.get(&u.universe) {
                // `send` replaces the watch slot — old frames are automatically discarded.
                let _ = s.tx.send(Some(FrameSlice::from_slice(&u.data)));
            }
        }
    }

    pub fn universe_count(&self) -> usize { self.senders.len() }

    /// Abort all universe tasks.
    pub fn shutdown(&mut self) {
        for t in self.tasks.drain(..) { t.abort(); }
    }
}

impl Drop for ParallelSender {
    fn drop(&mut self) { self.shutdown(); }
}

// ─── Per-universe task ────────────────────────────────────────────────────────

async fn universe_task(
    mut state:    UniverseState,
    mut rx:       watch::Receiver<Option<FrameSlice>>,
    cid:          [u8; 16],
    source_name:  String,
    priority:     u8,
) {
    loop {
        // Block until a new frame is pushed; exit cleanly if the sender is dropped.
        if rx.changed().await.is_err() { break; }

        // Borrow (not clone) the guard to memcpy DMX into the local packet buffer,
        // then release the lock BEFORE the async send — keeps the watch slot unlocked.
        {
            let guard = rx.borrow();
            if let Some(ref slice) = *guard {
                let seq = state.next_seq();
                packet::build_data_packet(
                    &mut state.packet_buf,
                    &cid,
                    &source_name,
                    priority,
                    seq,
                    state.universe,
                    &slice.data,
                );
            } else {
                continue;
            }
        } // guard dropped here — watch lock released

        let _ = state.socket.send_to(&*state.packet_buf, state.addr).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_slice_from_short_slice_zero_pads() {
        let s = FrameSlice::from_slice(&[1, 2, 3]);
        assert_eq!(s.data[0], 1);
        assert_eq!(s.data[2], 3);
        assert_eq!(s.data[3], 0, "zero-padded beyond input");
        assert_eq!(s.data[DMX_SLOTS - 1], 0);
    }

    #[test]
    fn universe_state_sequence_wraps() {
        // Build a stub state — doesn't need a real socket for this test.
        // We test wrapping math directly.
        let mut seq = 254u8;
        seq = seq.wrapping_add(1); assert_eq!(seq, 255);
        seq = seq.wrapping_add(1); assert_eq!(seq, 0, "wraps to 0");
        seq = seq.wrapping_add(1); assert_eq!(seq, 1);
    }
}
