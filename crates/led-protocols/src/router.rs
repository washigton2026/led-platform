//! `RouterDevice` — a [`DeviceDriver`] that fans out universes to multiple
//! protocol backends (sACN, DDP, or future Art-Net) based on a per-universe
//! routing table.
//!
//! ## Use case
//!
//! A rig with mixed controllers: WLED fixtures on DDP, professional fixtures on
//! sACN, and legacy DMX bridges on Art-Net. `RouterDevice` presents a single
//! `DeviceDriver` to the HAL while dispatching each universe to the correct backend.
//!
//! ```text
//! HAL → RouterDevice.send_physical([u0, u1, u2, u3, …])
//!              │
//!              ├── universe 0 → SacnSend (E1.31)
//!              ├── universe 1 → DdpSend  (WLED)
//!              └── universe 2 → SacnSend (E1.31)
//! ```
//!
//! ## Invariants (lumyx-network-architect)
//! - Each universe is dispatched to **exactly one backend** (no fan-out duplication).
//! - Universe indices in the routing table are dense, starting from 0 — matching
//!   the slice layout the HAL hands to `send_physical`.
//! - An unknown universe index falls back to the **default backend** (avoids silent loss).
//! - All backends are `Send + Sync`; the router itself is `Send + Sync`.

use led_core::{DeviceDriver, DeviceId, DeviceStatus, OutputError, UniverseData};

// ── Protocol backend ───────────────────────────────────────────────────────────

/// A single-universe send function — the minimal abstraction for one protocol.
pub trait ProtocolBackend: Send + Sync {
    /// Send one universe of data. Called per-universe by the router.
    fn send_universe(&self, universe_data: &UniverseData) -> Result<(), OutputError>;
    /// Human-readable protocol name (for diagnostics).
    fn protocol_name(&self) -> &'static str;
}

// ── Concrete backends ─────────────────────────────────────────────────────────

use std::net::{SocketAddr, UdpSocket};

/// sACN (E1.31) backend — one per universe, owns its sequence counter.
/// Uses `Mutex<u8>` for interior mutability so the type is `Sync`.
pub struct SacnBackend {
    socket:   UdpSocket,
    universe: u16,
    seq:      std::sync::Mutex<u8>,
}

impl SacnBackend {
    pub fn new(dest: SocketAddr, universe: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(dest)?;
        Ok(Self { socket, universe, seq: std::sync::Mutex::new(0) })
    }
}

impl ProtocolBackend for SacnBackend {
    fn send_universe(&self, universe_data: &UniverseData) -> Result<(), OutputError> {
        use crate::packet::{build_data_packet, DMX_SLOTS, PACKET_LEN};

        let mut seq = self.seq.lock().unwrap();
        let s = *seq;
        *seq = seq.wrapping_add(1);
        drop(seq);

        // Pad/truncate to DMX_SLOTS
        let mut dmx = [0u8; DMX_SLOTS];
        let copy_len = universe_data.data.len().min(DMX_SLOTS);
        dmx[..copy_len].copy_from_slice(&universe_data.data[..copy_len]);

        let cid = [0x4C, 0x55, 0x4D, 0x59, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // "LUMY"
        let mut buf = [0u8; PACKET_LEN];
        build_data_packet(&mut buf, &cid, "LUMYX Router", 100, s, self.universe, &dmx);
        self.socket.send(&buf)
            .map_err(|e| OutputError::Transport(e.to_string()))?;
        Ok(())
    }
    fn protocol_name(&self) -> &'static str { "sACN/E1.31" }
}

/// DDP backend — owns its sequence counter **and a pre-allocated packet buffer**, both
/// behind one `Mutex` (interior mutability so the type is `Sync`). Zero heap allocation on
/// the send path: the buffer is sized once at construction and reused for every fragment.
/// (Mirrors [`crate::ddp::DdpDevice`], which was already alloc-free; the previous router
/// backend regressed by rebuilding a `Vec<PixelColor>` + packet `Vec<u8>` per frame — C2.)
pub struct DdpBackend {
    socket:       UdpSocket,
    pixel_offset: u32,
    /// Wire channels per pixel (3 = RGB, 4 = RGBW). Used to drop DMX padding at whole-pixel
    /// boundaries and keep DDP fragments pixel-aligned. DDP is byte-addressed, so the format
    /// bytes are already produced upstream by the mapper (ADR-0011); the backend only needs
    /// the stride, not the colour order.
    stride:       usize,
    state:        std::sync::Mutex<DdpBackendState>,
}

struct DdpBackendState {
    seq: u8,
    buf: Box<[u8; 10 + crate::ddp::DDP_MAX_PAYLOAD]>,
}

impl DdpBackend {
    /// RGB backend (3 channels/pixel) — the common case.
    pub fn new(dest: SocketAddr, pixel_offset: u32) -> std::io::Result<Self> {
        Self::with_channels(dest, pixel_offset, 3)
    }

    /// Backend with an explicit channels-per-pixel stride (e.g. 4 for RGBW, ADR-0011).
    pub fn with_channels(
        dest: SocketAddr,
        pixel_offset: u32,
        channels_per_pixel: usize,
    ) -> std::io::Result<Self> {
        assert!(channels_per_pixel >= 1, "DDP: channels_per_pixel must be ≥ 1");
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(dest)?;
        Ok(Self {
            socket,
            pixel_offset,
            stride: channels_per_pixel,
            state: std::sync::Mutex::new(DdpBackendState {
                seq: 0,
                buf: Box::new([0u8; 10 + crate::ddp::DDP_MAX_PAYLOAD]),
            }),
        })
    }
}

impl ProtocolBackend for DdpBackend {
    fn send_universe(&self, universe_data: &UniverseData) -> Result<(), OutputError> {
        use crate::ddp::{build_ddp_packet_bytes, DDP_MAX_PAYLOAD};

        let channels = &universe_data.data;
        // Send only whole pixels; a trailing partial pixel (DMX padding) is dropped. Stride is
        // 3 for RGB, 4 for RGBW — the `% 3` here would have corrupted RGBW by half a pixel.
        let usable = channels.len() - channels.len() % self.stride;
        if usable == 0 {
            return Ok(());
        }
        // Largest DDP payload that is a whole number of pixels — keeps every fragment
        // pixel-aligned regardless of stride (1461 for RGB, 1460 for RGBW).
        let frag = DDP_MAX_PAYLOAD - DDP_MAX_PAYLOAD % self.stride;

        let mut st = self.state.lock().unwrap();
        let base_offset = self.pixel_offset * self.stride as u32; // byte offset into the buffer
        let mut sent = 0usize;
        while sent < usable {
            let end = (sent + frag).min(usable);
            let payload = &channels[sent..end];
            let byte_offset = base_offset + sent as u32;
            let seq = st.seq;
            let len = build_ddp_packet_bytes(st.buf.as_mut(), seq, byte_offset, payload);
            self.socket
                .send(&st.buf[..len])
                .map_err(|e| OutputError::Transport(e.to_string()))?;
            st.seq = st.seq.wrapping_add(1);
            sent = end;
        }
        Ok(())
    }
    fn protocol_name(&self) -> &'static str { "DDP" }
}

// ── RouterDevice ──────────────────────────────────────────────────────────────

/// Routing entry: a universe index maps to one backend.
pub struct RouteEntry {
    /// Universe index within the slice handed to `send_physical` (0-based).
    pub universe_idx: usize,
    pub backend:      Box<dyn ProtocolBackend>,
}

/// A [`DeviceDriver`] that dispatches each universe to a protocol-specific backend.
pub struct RouterDevice {
    id:       DeviceId,
    /// Routing table: sorted by `universe_idx` for O(log n) lookup.
    routes:   Vec<RouteEntry>,
    /// Default backend for universes not in the routing table.
    default:  Option<Box<dyn ProtocolBackend>>,
}

impl RouterDevice {
    /// Create a new router with the given routes.
    ///
    /// `routes` may be in any order; they are sorted internally.
    /// If `default` is `Some`, universes not in the table use it.
    /// If `default` is `None`, unknown universes are silently skipped.
    pub fn new(
        id:      DeviceId,
        mut routes: Vec<RouteEntry>,
        default: Option<Box<dyn ProtocolBackend>>,
    ) -> Self {
        routes.sort_by_key(|r| r.universe_idx);
        Self { id, routes, default }
    }

    fn find_backend(&self, universe_idx: usize) -> Option<&dyn ProtocolBackend> {
        self.routes
            .binary_search_by_key(&universe_idx, |r| r.universe_idx)
            .ok()
            .map(|i| self.routes[i].backend.as_ref())
            .or_else(|| self.default.as_deref())
    }
}

impl DeviceDriver for RouterDevice {
    fn id(&self) -> DeviceId { self.id }

    fn send_physical(&self, universes: &[UniverseData]) -> Result<(), OutputError> {
        for (idx, universe_data) in universes.iter().enumerate() {
            if let Some(backend) = self.find_backend(idx) {
                backend.send_universe(universe_data)?;
            }
            // Universes with no backend and no default are silently skipped —
            // this is intentional (not every universe needs sending).
        }
        Ok(())
    }

    fn status(&self) -> DeviceStatus { DeviceStatus { connected: true, frames_sent: 0, last_send_ms: 0 } }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::{DeviceDriver, OutputError, UniverseData};
    use std::sync::{Arc, Mutex};

    // ── Spy backend for testing ───────────────────────────────────────────────

    #[derive(Default)]
    struct SpyBackend {
        calls: Arc<Mutex<Vec<(String, Vec<u8>)>>>, // (protocol_name, data)
        name:  &'static str,
    }

    impl SpyBackend {
        fn new(name: &'static str) -> (Self, Arc<Mutex<Vec<(String, Vec<u8>)>>>) {
            let calls = Arc::new(Mutex::new(vec![]));
            (Self { calls: calls.clone(), name }, calls)
        }
    }

    impl ProtocolBackend for SpyBackend {
        fn send_universe(&self, u: &UniverseData) -> Result<(), OutputError> {
            self.calls.lock().unwrap().push((self.name.to_owned(), u.data.clone()));
            Ok(())
        }
        fn protocol_name(&self) -> &'static str { self.name }
    }

    fn universe(data: Vec<u8>) -> UniverseData {
        UniverseData { universe: 1, data }
    }

    // ── Basic routing ─────────────────────────────────────────────────────────

    #[test]
    fn router_dispatches_to_correct_backend() {
        let (spy_a, calls_a) = SpyBackend::new("BackendA");
        let (spy_b, calls_b) = SpyBackend::new("BackendB");

        let router = RouterDevice::new(
            0,
            vec![
                RouteEntry { universe_idx: 0, backend: Box::new(spy_a) },
                RouteEntry { universe_idx: 1, backend: Box::new(spy_b) },
            ],
            None,
        );

        let universes = vec![
            universe(vec![1, 2, 3]),
            universe(vec![4, 5, 6]),
        ];
        router.send_physical(&universes).unwrap();

        let a = calls_a.lock().unwrap();
        let b = calls_b.lock().unwrap();
        assert_eq!(a.len(), 1, "BackendA must receive exactly 1 universe");
        assert_eq!(b.len(), 1, "BackendB must receive exactly 1 universe");
        assert_eq!(a[0].1, vec![1, 2, 3]);
        assert_eq!(b[0].1, vec![4, 5, 6]);
    }

    #[test]
    fn router_default_backend_handles_unmapped_universe() {
        let (spy_default, calls_default) = SpyBackend::new("default");

        let router = RouterDevice::new(0, vec![], Some(Box::new(spy_default)));
        router.send_physical(&[universe(vec![7, 8, 9])]).unwrap();

        let calls = calls_default.lock().unwrap();
        assert_eq!(calls.len(), 1, "default backend must handle unmapped universe");
        assert_eq!(calls[0].1, vec![7, 8, 9]);
    }

    #[test]
    fn router_no_default_skips_unmapped_universe() {
        let router = RouterDevice::new(0, vec![], None);
        // No panic, no error — unmapped universe silently skipped
        router.send_physical(&[universe(vec![1, 2, 3])]).unwrap();
    }

    #[test]
    fn router_routes_sorted_regardless_of_insert_order() {
        let (spy_0, calls_0) = SpyBackend::new("u0");
        let (spy_1, calls_1) = SpyBackend::new("u1");
        let (spy_2, calls_2) = SpyBackend::new("u2");

        // Insert out of order
        let router = RouterDevice::new(
            0,
            vec![
                RouteEntry { universe_idx: 2, backend: Box::new(spy_2) },
                RouteEntry { universe_idx: 0, backend: Box::new(spy_0) },
                RouteEntry { universe_idx: 1, backend: Box::new(spy_1) },
            ],
            None,
        );

        let universes = vec![
            universe(vec![0]),
            universe(vec![1]),
            universe(vec![2]),
        ];
        router.send_physical(&universes).unwrap();

        assert_eq!(calls_0.lock().unwrap()[0].1, vec![0u8]);
        assert_eq!(calls_1.lock().unwrap()[0].1, vec![1u8]);
        assert_eq!(calls_2.lock().unwrap()[0].1, vec![2u8]);
    }

    #[test]
    fn router_mixed_sacn_ddp_routes_compile() {
        // Structural test: confirms RouterDevice accepts both backend types.
        // We don't need real sockets here — just type-check the wiring.
        let (spy_sacn, _) = SpyBackend::new("sacn");
        let (spy_ddp,  _) = SpyBackend::new("ddp");

        let router = RouterDevice::new(
            42,
            vec![
                RouteEntry { universe_idx: 0, backend: Box::new(spy_sacn) },
                RouteEntry { universe_idx: 1, backend: Box::new(spy_ddp) },
            ],
            None,
        );

        assert_eq!(router.id(), 42);
        // Two universes — one per protocol
        router.send_physical(&[universe(vec![]), universe(vec![])]).unwrap();
    }

    #[test]
    fn router_partial_routes_only_send_mapped() {
        // 4 universes, only 0 and 2 mapped
        let (spy_0, calls_0) = SpyBackend::new("u0");
        let (spy_2, calls_2) = SpyBackend::new("u2");

        let router = RouterDevice::new(
            0,
            vec![
                RouteEntry { universe_idx: 0, backend: Box::new(spy_0) },
                RouteEntry { universe_idx: 2, backend: Box::new(spy_2) },
            ],
            None, // no default
        );

        let universes = vec![
            universe(vec![10]), // → u0
            universe(vec![20]), // unmapped, skipped
            universe(vec![30]), // → u2
            universe(vec![40]), // unmapped, skipped
        ];
        router.send_physical(&universes).unwrap();

        assert_eq!(calls_0.lock().unwrap().len(), 1);
        assert_eq!(calls_2.lock().unwrap().len(), 1);
        assert_eq!(calls_0.lock().unwrap()[0].1, vec![10u8]);
        assert_eq!(calls_2.lock().unwrap()[0].1, vec![30u8]);
    }

    #[test]
    fn router_device_id_and_status() {
        let router = RouterDevice::new(99, vec![], None);
        assert_eq!(router.id(), 99);
        assert!(router.status().connected);
    }

    // ── UDP loopback: real sACN + DDP backends ────────────────────────────────

    #[test]
    fn sacn_backend_sends_valid_packet() {
        use std::net::UdpSocket;
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();
        let addr = recv.local_addr().unwrap();

        let backend = SacnBackend::new(addr, 1).unwrap();
        let payload = vec![0xFFu8, 0x00, 0xAA].repeat(170);
        let u = UniverseData { universe: 1, data: payload[..510].to_vec() };
        backend.send_universe(&u).unwrap();

        let mut buf = [0u8; 638];
        let n = recv.recv(&mut buf).expect("must receive sACN packet");
        // Verify ACN PID bytes (E1.31 header)
        assert_eq!(&buf[..4], &[0x00, 0x10, 0x00, 0x00], "sACN preamble must be valid");
        assert!(n > 10, "sACN packet must be longer than header");
    }

    #[test]
    fn ddp_backend_sends_valid_packet() {
        use std::net::UdpSocket;
        use crate::ddp::parse_ddp_packet;
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();
        let addr = recv.local_addr().unwrap();

        let backend = DdpBackend::new(addr, 0).unwrap();
        let u = UniverseData { universe: 0, data: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01] };
        backend.send_universe(&u).unwrap();

        let mut buf = [0u8; 512];
        let n = recv.recv(&mut buf).expect("must receive DDP packet");
        let pkt = parse_ddp_packet(&buf[..n]).expect("must parse as DDP");
        assert_eq!(pkt.payload, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
    }

    #[test]
    fn ddp_backend_rgbw_sends_whole_4byte_pixels() {
        // RGBW stride=4: an 8-byte (2-pixel) universe must send all 8 bytes — the old `% 3`
        // truncation would have dropped 2 bytes (8 % 3 = 2), corrupting the second pixel.
        use crate::ddp::parse_ddp_packet;
        use std::net::UdpSocket;
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();
        let addr = recv.local_addr().unwrap();

        let backend = DdpBackend::with_channels(addr, 0, 4).unwrap();
        let u = UniverseData { universe: 0, data: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        backend.send_universe(&u).unwrap();

        let mut buf = [0u8; 512];
        let n = recv.recv(&mut buf).expect("must receive DDP packet");
        let pkt = parse_ddp_packet(&buf[..n]).expect("must parse as DDP");
        assert_eq!(
            pkt.payload,
            &[1, 2, 3, 4, 5, 6, 7, 8],
            "RGBW (stride 4) must not truncate to 3-byte pixels"
        );
    }
}
