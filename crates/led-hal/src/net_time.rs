//! Network clock synchronisation for multi-node shows — the practical half of
//! the PTP investigation (see `docs/ptp-investigation.md`).
//!
//! Implements the classic two-way time transfer (same math as NTP and PTP's
//! delay request-response, minus hardware timestamping):
//!
//! ```text
//! client t1 ──request──▶ server t2 (receive)
//! client t4 ◀──reply──── server t3 (send)      offset = ((t2-t1)+(t3-t4))/2
//!                                              delay  = (t4-t1)-(t3-t2)
//! ```
//!
//! With software timestamps on a cabled LAN this lands within ~1 ms — well
//! inside the cluster's 5 ms drift tolerance. PTP with NIC timestamping gets
//! to ±1 µs but needs hardware support end-to-end; that trade-off is the
//! investigation's conclusion, not an implementation gap.
//!
//! ## Invariants
//! - Sync happens on the management plane, never per frame.
//! - The server never adjusts its own clock (follower calibrates to leader).
//! - A measurement carries its own `delay_ms`; callers reject high-delay
//!   samples instead of averaging them in (delay-gated, like NTP).

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::shared_clock::SharedClock;

const MAGIC: [u8; 4] = *b"LMTS"; // LuMyx Time Sync
const REQ_LEN: usize = 4 + 8; // magic + t1
const REP_LEN: usize = 4 + 8 + 8 + 8; // magic + t1 + t2 + t3

/// One two-way measurement against a time server.
#[derive(Clone, Copy, Debug)]
pub struct TimeSample {
    /// Estimated offset to ADD to the local clock to match the server (ms).
    pub offset_ms: i64,
    /// Round-trip delay of the exchange (ms) — quality gate.
    pub delay_ms: u64,
}

/// Serve the local `clock` to followers. One thread; returns after `stop()`.
pub struct TimeServer {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TimeServer {
    pub fn start(clock: Arc<SharedClock>, addr: SocketAddr) -> std::io::Result<Self> {
        let sock = UdpSocket::bind(addr)?;
        sock.set_read_timeout(Some(Duration::from_millis(50)))?;
        let bound = sock.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();

        let handle = std::thread::Builder::new()
            .name("lumyx-time-server".into())
            .spawn(move || {
                let mut buf = [0u8; 64];
                loop {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let (n, from) = match sock.recv_from(&mut buf) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let t2 = clock.now_ms();
                    if n != REQ_LEN || buf[0..4] != MAGIC {
                        continue; // not ours
                    }
                    let mut reply = [0u8; REP_LEN];
                    reply[0..4].copy_from_slice(&MAGIC);
                    reply[4..12].copy_from_slice(&buf[4..12]); // echo t1
                    reply[12..20].copy_from_slice(&t2.to_le_bytes());
                    let t3 = clock.now_ms();
                    reply[20..28].copy_from_slice(&t3.to_le_bytes());
                    let _ = sock.send_to(&reply, from);
                }
            })?;

        Ok(Self { addr: bound, stop, handle: Some(handle) })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// One two-way exchange with a [`TimeServer`]. Timeout: 500 ms.
pub fn measure_offset(
    local: &SharedClock,
    server: SocketAddr,
) -> std::io::Result<TimeSample> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;

    let t1 = local.now_ms();
    let mut req = [0u8; REQ_LEN];
    req[0..4].copy_from_slice(&MAGIC);
    req[4..12].copy_from_slice(&t1.to_le_bytes());
    sock.send_to(&req, server)?;

    let mut buf = [0u8; 64];
    let (n, _) = sock.recv_from(&mut buf)?;
    let t4 = local.now_ms();

    if n != REP_LEN || buf[0..4] != MAGIC || buf[4..12] != t1.to_le_bytes() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed time reply",
        ));
    }
    let t2 = u64::from_le_bytes(buf[12..20].try_into().unwrap());
    let t3 = u64::from_le_bytes(buf[20..28].try_into().unwrap());

    let offset_ms = ((t2 as i64 - t1 as i64) + (t3 as i64 - t4 as i64)) / 2;
    let delay_ms = (t4.saturating_sub(t1)).saturating_sub(t3.saturating_sub(t2));
    Ok(TimeSample { offset_ms, delay_ms })
}

/// Take `n` measurements and return the one with the lowest delay (NTP-style
/// quality gating: the fastest exchange has the least queueing noise).
pub fn best_of(
    local: &SharedClock,
    server: SocketAddr,
    n: usize,
) -> std::io::Result<TimeSample> {
    let mut best: Option<TimeSample> = None;
    for _ in 0..n.max(1) {
        let s = measure_offset(local, server)?;
        if best.is_none_or(|b| s.delay_ms < b.delay_ms) {
            best = Some(s);
        }
    }
    Ok(best.unwrap())
}

/// Calibrate `follower` against a leader's [`TimeServer`]: measure, then apply.
pub fn sync_to(follower: &SharedClock, server: SocketAddr, samples: usize) -> std::io::Result<TimeSample> {
    let s = best_of(follower, server, samples)?;
    follower.set_offset_ms(follower.offset_ms() + s.offset_ms);
    Ok(s)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn leader_follower() -> (Arc<SharedClock>, Arc<SharedClock>, TimeServer) {
        let leader = Arc::new(SharedClock::new());
        let follower = Arc::new(SharedClock::new());
        let server =
            TimeServer::start(leader.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
        (leader, follower, server)
    }

    #[test]
    fn loopback_offset_is_near_zero_for_synced_clocks() {
        let (_leader, follower, server) = leader_follower();
        let s = measure_offset(&follower, server.addr).expect("exchange");
        assert!(s.offset_ms.abs() <= 5, "same-machine clocks: |offset| ≤ 5ms, got {}", s.offset_ms);
        assert!(s.delay_ms <= 50, "loopback RTT must be tiny, got {}ms", s.delay_ms);
        server.stop();
    }

    #[test]
    fn injected_leader_offset_is_measured() {
        let (leader, follower, server) = leader_follower();
        leader.set_offset_ms(500); // leader runs 500ms ahead
        let s = best_of(&follower, server.addr, 3).expect("exchange");
        assert!((s.offset_ms - 500).abs() <= 10,
            "500ms lead must be measured within ±10ms, got {}", s.offset_ms);
        server.stop();
    }

    #[test]
    fn sync_to_brings_follower_within_drift_budget() {
        let (leader, follower, server) = leader_follower();
        leader.set_offset_ms(750);
        sync_to(&follower, server.addr, 3).expect("sync");
        let drift = leader.now_ms().abs_diff(follower.now_ms());
        assert!(drift <= 10, "post-sync drift must be ≤ 10ms (budget 5ms + noise), got {drift}");
        server.stop();
    }

    #[test]
    fn negative_offset_follower_ahead() {
        let (_leader, follower, server) = leader_follower();
        follower.set_offset_ms(300); // follower ahead → negative correction
        let s = best_of(&follower, server.addr, 3).expect("exchange");
        assert!((s.offset_ms + 300).abs() <= 10,
            "follower 300ms ahead → offset ≈ -300, got {}", s.offset_ms);
        server.stop();
    }

    #[test]
    fn malformed_requests_are_ignored_not_fatal() {
        let (_leader, follower, server) = leader_follower();
        // Fire garbage at the server, then a real exchange must still work.
        let junk = UdpSocket::bind("127.0.0.1:0").unwrap();
        junk.send_to(b"not a time request", server.addr).unwrap();
        junk.send_to(&[0u8; 12], server.addr).unwrap();
        let s = measure_offset(&follower, server.addr).expect("server survives junk");
        assert!(s.delay_ms <= 100);
        server.stop();
    }
}
