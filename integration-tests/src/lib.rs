//! Shared test infrastructure for cross-crate integration tests.
//!
//! [`UdpChaosProxy`] is the network-level counterpart of `led_hal::ChaosHarness`:
//! instead of intercepting `send_frame` calls in-process, it sits between two
//! real UDP sockets and drops/delays actual datagrams — the closest a CI box
//! gets to unplugging a cable. Deterministic per seed (SplitMix64, same PRNG
//! family as ChaosHarness and ShowIntentGenerator).

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Fault profile for the proxy.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProxyFaults {
    /// Percentage of datagrams to drop [0, 100].
    pub loss_pct: u8,
    /// Fixed extra delay per forwarded datagram.
    pub latency: Duration,
    /// PRNG seed — same seed, same drop pattern.
    pub seed: u64,
}

/// A UDP forwarding proxy with fault injection. Bind → point the sender at
/// `proxy.addr()` → datagrams are forwarded (or chaos-dropped) to `target`.
pub struct UdpChaosProxy {
    addr: SocketAddr,
    forwarded: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl UdpChaosProxy {
    /// Start a proxy on an ephemeral local port, forwarding to `target`.
    pub fn start(target: SocketAddr, faults: ProxyFaults) -> std::io::Result<Self> {
        let sock = UdpSocket::bind("127.0.0.1:0")?;
        sock.set_read_timeout(Some(Duration::from_millis(50)))?;
        let addr = sock.local_addr()?;
        let out = UdpSocket::bind("127.0.0.1:0")?;

        let forwarded = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let active = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));

        let (fwd, drp, act, stp) =
            (forwarded.clone(), dropped.clone(), active.clone(), stop.clone());

        let handle = std::thread::Builder::new()
            .name("udp-chaos-proxy".into())
            .spawn(move || {
                let mut rng = faults.seed;
                let mut next_rand = move || -> u64 {
                    rng = rng.wrapping_add(0x9e3779b97f4a7c15);
                    let mut z = rng;
                    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                    z ^ (z >> 31)
                };
                let mut buf = [0u8; 2048];
                loop {
                    if stp.load(Ordering::Relaxed) {
                        break;
                    }
                    let n = match sock.recv_from(&mut buf) {
                        Ok((n, _)) => n,
                        Err(_) => continue, // timeout: check stop flag again
                    };
                    let chaos_on = act.load(Ordering::Relaxed);
                    if chaos_on && faults.loss_pct > 0 && (next_rand() % 100) < faults.loss_pct as u64 {
                        drp.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if chaos_on && !faults.latency.is_zero() {
                        std::thread::sleep(faults.latency);
                    }
                    if out.send_to(&buf[..n], target).is_ok() {
                        fwd.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })?;

        Ok(Self { addr, forwarded, dropped, active, stop, handle: Some(handle) })
    }

    /// Where senders should point their datagrams.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn forwarded(&self) -> u64 {
        self.forwarded.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Turn fault injection off (transparent forwarding) — "the network healed".
    pub fn heal(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    /// Stop the proxy thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
