//! Art-Net `ArtPoll` / `ArtPollReply` for **source-conflict detection**: before driving a
//! universe, ask who else is on the wire. Two apps on one universe is a war (flicker,
//! safe-mode); detect it at startup and refuse, naming the other IP.
//!
//! This is a faithful *subset* of Art-Net 4 — enough fields, at their real offsets, to
//! interoperate for discovery/conflict purposes. Note Art-Net stores the OpCode
//! **little-endian** (unlike sACN's big-endian wire fields).

use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

pub const ARTNET_ID: [u8; 8] = *b"Art-Net\0";
pub const OP_POLL: u16 = 0x2000;
pub const OP_POLL_REPLY: u16 = 0x2100;
pub const ARTNET_PORT: u16 = 6454;

pub const ART_POLL_LEN: usize = 14;
pub const ART_POLL_REPLY_LEN: usize = 239;

#[inline]
fn put_u16_le(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn get_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn has_artnet_id(pkt: &[u8]) -> bool {
    pkt.len() >= 10 && pkt[0..8] == ARTNET_ID
}

/// The OpCode of an Art-Net packet (little-endian), or `None` if it isn't Art-Net.
pub fn opcode(pkt: &[u8]) -> Option<u16> {
    has_artnet_id(pkt).then(|| get_u16_le(pkt, 8))
}

/// Build an `ArtPoll` (the discovery request).
pub fn build_art_poll(buf: &mut [u8; ART_POLL_LEN]) {
    *buf = [0u8; ART_POLL_LEN];
    buf[0..8].copy_from_slice(&ARTNET_ID);
    put_u16_le(buf, 8, OP_POLL);
    buf[10] = 0; // ProtVerHi
    buf[11] = 14; // ProtVerLo
    buf[12] = 0; // TalkToMe
    buf[13] = 0; // Priority (DpLow)
}

/// A parsed `ArtPollReply` — what we need to spot a conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtPollReply {
    pub ip: Ipv4Addr,
    pub short_name: String,
    /// 15-bit Art-Net port-addresses this node **outputs**.
    pub universes: Vec<u16>,
}

/// 15-bit port-address from net/sub/universe parts.
fn port_address(net: u8, sub: u8, uni: u8) -> u16 {
    ((net as u16 & 0x7f) << 8) | ((sub as u16 & 0x0f) << 4) | (uni as u16 & 0x0f)
}

/// Build an `ArtPollReply` advertising `port_addresses` as outputs from `ip`. (Assumes the
/// universes share a Net/Sub, taken from the first — the common single-net case.)
pub fn build_art_poll_reply(
    buf: &mut [u8; ART_POLL_REPLY_LEN],
    ip: Ipv4Addr,
    port_addresses: &[u16],
    short_name: &str,
) {
    *buf = [0u8; ART_POLL_REPLY_LEN];
    buf[0..8].copy_from_slice(&ARTNET_ID);
    put_u16_le(buf, 8, OP_POLL_REPLY);
    buf[10..14].copy_from_slice(&ip.octets());
    put_u16_le(buf, 14, ARTNET_PORT);

    let first = port_addresses.first().copied().unwrap_or(0);
    buf[18] = ((first >> 8) & 0x7f) as u8; // NetSwitch
    buf[19] = ((first >> 4) & 0x0f) as u8; // SubSwitch

    // ShortName (18 bytes, null-padded) at offset 26.
    let name = short_name.as_bytes();
    let n = name.len().min(17);
    buf[26..26 + n].copy_from_slice(&name[..n]);

    let ports = port_addresses.len().min(4);
    buf[173] = ports as u8; // NumPortsLo
    for (i, pa) in port_addresses.iter().take(4).enumerate() {
        buf[174 + i] = 0x80; // PortType: output
        buf[182 + i] = 0x80; // GoodOutput: data is being transmitted
        buf[190 + i] = (*pa & 0x0f) as u8; // SwOut: universe nibble
    }
}

/// Parse an `ArtPollReply`, or `None` if `pkt` isn't one.
pub fn parse_art_poll_reply(pkt: &[u8]) -> Option<ArtPollReply> {
    if pkt.len() < 194 || opcode(pkt) != Some(OP_POLL_REPLY) {
        return None;
    }
    let ip = Ipv4Addr::new(pkt[10], pkt[11], pkt[12], pkt[13]);
    let net = pkt[18];
    let sub = pkt[19];
    let num_ports = (pkt[173] as usize).min(4);

    let name_end = pkt[26..44].iter().position(|&b| b == 0).map(|p| 26 + p).unwrap_or(44);
    let short_name = String::from_utf8_lossy(&pkt[26..name_end]).into_owned();

    let universes = (0..num_ports).map(|i| port_address(net, sub, pkt[190 + i])).collect();
    Some(ArtPollReply { ip, short_name, universes })
}

/// A universe we want to drive that is already being output by another node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictReport {
    pub universe: u16,
    pub other_ip: Ipv4Addr,
    pub node_name: String,
}

/// Pure conflict logic: which of `my_universes` are already driven by a discovered node.
pub fn find_conflicts(my_universes: &[u16], replies: &[ArtPollReply]) -> Vec<ConflictReport> {
    let mut out = Vec::new();
    for reply in replies {
        for &u in &reply.universes {
            if my_universes.contains(&u) {
                out.push(ConflictReport {
                    universe: u,
                    other_ip: reply.ip,
                    node_name: reply.short_name.clone(),
                });
            }
        }
    }
    out
}

/// Broadcast an `ArtPoll`, collect replies for `timeout`, and report conflicts on
/// `my_universes`. (Plumbing — exercised against real/fake nodes; the build/parse/conflict
/// pieces are unit-tested.)
pub fn poll_conflicts(my_universes: &[u16], timeout: Duration) -> std::io::Result<Vec<ConflictReport>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, ARTNET_PORT))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(timeout))?;

    let mut poll = [0u8; ART_POLL_LEN];
    build_art_poll(&mut poll);
    socket.send_to(&poll, (Ipv4Addr::BROADCAST, ARTNET_PORT))?;

    let mut replies = Vec::new();
    let mut buf = [0u8; 1024];
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Some(r) = parse_art_poll_reply(&buf[..n]) {
                    replies.push(r);
                }
            }
            Err(_) => break, // timeout
        }
    }
    Ok(find_conflicts(my_universes, &replies))
}

// ── Pre-show discovery ──────────────────────────────────────────────────────
//
// Before the first frame, ask the wire "are the controllers I'm about to drive
// actually there?" — an ArtPoll broadcast, then match ArtPollReply source IPs
// against the expected set. Kills the silent-dark-stage footgun: a controller
// that is powered off / on the wrong subnet / on dead WiFi answers nothing, and
// the operator learns it BEFORE the show, not during.

/// Which expected controllers answered discovery, and which did not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryResult {
    /// Expected IPs that sent a valid ArtPollReply.
    pub responded: Vec<Ipv4Addr>,
    /// Expected IPs that stayed silent (powered off, wrong subnet, dead link).
    pub missing: Vec<Ipv4Addr>,
}

impl DiscoveryResult {
    /// True when every expected controller answered.
    pub fn all_present(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Pure presence logic: partition `expected` into responded/missing given the
/// replies collected from the wire. A reply from an IP that is NOT in `expected`
/// is ignored — a stray/rogue node answering can never mask a missing
/// controller (the whole point of the check).
pub fn presence(expected: &[Ipv4Addr], replies: &[ArtPollReply]) -> DiscoveryResult {
    let mut responded = Vec::new();
    let mut missing = Vec::new();
    for &ip in expected {
        if replies.iter().any(|r| r.ip == ip) {
            responded.push(ip);
        } else {
            missing.push(ip);
        }
    }
    DiscoveryResult { responded, missing }
}

/// Broadcast an `ArtPoll`, collect replies for `timeout`, and report which of
/// `expected` answered. Wire plumbing mirrors [`poll_conflicts`]; the presence
/// partition is unit-tested via [`presence`].
pub fn discover_controllers(
    expected: &[Ipv4Addr],
    timeout: Duration,
) -> std::io::Result<DiscoveryResult> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, ARTNET_PORT))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(timeout))?;

    let mut poll = [0u8; ART_POLL_LEN];
    build_art_poll(&mut poll);
    socket.send_to(&poll, (Ipv4Addr::BROADCAST, ARTNET_PORT))?;

    let mut replies = Vec::new();
    let mut buf = [0u8; 1024];
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Some(r) = parse_art_poll_reply(&buf[..n]) {
                    replies.push(r);
                }
            }
            Err(_) => break, // timeout
        }
    }
    Ok(presence(expected, &replies))
}

// ── ArtDmx output ─────────────────────────────────────────────────────────────
//
// The live-data half of Art-Net: one `ArtDmx` packet per universe per frame, sent
// over UDP unicast to the controller (WLED, Falcon, FPP all accept unicast ArtDmx).
// Same invariants as sACN: sequence is **per universe** (wrapping 1..=255 — 0 would
// disable sequence checking at the receiver), one universe per datagram, and the
// packet buffer is pre-allocated once (no alloc on the hot path).

pub const OP_DMX: u16 = 0x5000;
/// ArtDmx header is 18 bytes; data follows.
pub const ART_DMX_HEADER_LEN: usize = 18;
/// DMX payload per ArtDmx packet: 2..=512 bytes, and the length **must be even**.
pub const ART_DMX_MAX_SLOTS: usize = 512;
/// Full-size ArtDmx packet (530 bytes — comfortably under a 1500 MTU).
pub const ART_DMX_LEN: usize = ART_DMX_HEADER_LEN + ART_DMX_MAX_SLOTS;
/// Art-Net 4 protocol revision carried in ProtVerLo.
pub const ART_PROT_VER: u8 = 14;

/// Serialize one `ArtDmx` packet into `buf`. Returns the wire length.
///
/// `universe` is the 15-bit port-address (SubUni = low 8 bits, Net = high 7 bits) —
/// the same numbering xLights/WLED expose as "universe" for Art-Net controllers.
/// `data` is the DMX payload (≤ 512 bytes); it is padded by one zero byte on the
/// wire if its length is odd, because the spec requires an even length.
pub fn build_art_dmx(
    buf: &mut [u8; ART_DMX_LEN],
    universe: u16,
    sequence: u8,
    physical: u8,
    data: &[u8],
) -> usize {
    debug_assert!(data.len() <= ART_DMX_MAX_SLOTS, "ArtDmx payload > 512");
    let even_len = (data.len() + 1) & !1; // round odd up to even
    buf[0..8].copy_from_slice(&ARTNET_ID);
    put_u16_le(buf, 8, OP_DMX);
    buf[10] = 0; // ProtVerHi
    buf[11] = ART_PROT_VER; // ProtVerLo
    buf[12] = sequence;
    buf[13] = physical;
    buf[14] = (universe & 0xFF) as u8; // SubUni
    buf[15] = ((universe >> 8) & 0x7F) as u8; // Net
    buf[16] = (even_len >> 8) as u8; // Length is BIG-endian (unlike the LE OpCode)
    buf[17] = (even_len & 0xFF) as u8;
    buf[ART_DMX_HEADER_LEN..ART_DMX_HEADER_LEN + data.len()].copy_from_slice(data);
    if even_len > data.len() {
        buf[ART_DMX_HEADER_LEN + data.len()] = 0; // pad byte
    }
    ART_DMX_HEADER_LEN + even_len
}

/// Parse an `ArtDmx` packet: `(universe, sequence, data)`. `None` if not ArtDmx.
pub fn parse_art_dmx(pkt: &[u8]) -> Option<(u16, u8, &[u8])> {
    if opcode(pkt)? != OP_DMX || pkt.len() < ART_DMX_HEADER_LEN {
        return None;
    }
    let universe = (pkt[14] as u16) | (((pkt[15] & 0x7F) as u16) << 8);
    let len = ((pkt[16] as usize) << 8) | pkt[17] as usize;
    if !(2..=ART_DMX_MAX_SLOTS).contains(&len) || pkt.len() < ART_DMX_HEADER_LEN + len {
        return None;
    }
    Some((universe, pkt[12], &pkt[ART_DMX_HEADER_LEN..ART_DMX_HEADER_LEN + len]))
}

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use led_core::{DeviceDriver, DeviceId, DeviceStatus, OutputError, UniverseData};

struct ArtNetState {
    /// Per-universe sequence, wrapping 1..=255 (0 disables checking — never emitted).
    seqs: HashMap<u16, u8>,
    /// Reused packet buffer — no alloc on the hot path.
    buf: Box<[u8; ART_DMX_LEN]>,
    frames_sent: u64,
}

/// Art-Net output [`DeviceDriver`]: one unicast `ArtDmx` per universe per frame.
pub struct ArtNetDevice {
    id: DeviceId,
    socket: UdpSocket,
    dest: SocketAddr,
    state: Mutex<ArtNetState>,
}

impl ArtNetDevice {
    /// Unicast sender: every universe goes to `dest` (a controller's IP:6454).
    pub fn unicast(id: DeviceId, dest: SocketAddr) -> std::io::Result<Arc<Self>> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        Ok(Arc::new(Self {
            id,
            socket,
            dest,
            state: Mutex::new(ArtNetState {
                seqs: HashMap::new(),
                buf: Box::new([0u8; ART_DMX_LEN]),
                frames_sent: 0,
            }),
        }))
    }

    pub fn frames_sent(&self) -> u64 {
        self.state.lock().unwrap().frames_sent
    }
}

impl DeviceDriver for ArtNetDevice {
    fn id(&self) -> DeviceId {
        self.id
    }

    fn send_physical(&self, universes: &[UniverseData]) -> Result<(), OutputError> {
        let mut st = self.state.lock().unwrap();
        let st = &mut *st;
        for u in universes {
            if u.data.len() > ART_DMX_MAX_SLOTS {
                return Err(OutputError::Transport(format!(
                    "universe {} payload {} > 512",
                    u.universe,
                    u.data.len()
                )));
            }
            let seq = st.seqs.entry(u.universe).or_insert(0);
            *seq = if *seq == 255 { 1 } else { *seq + 1 }; // wrap 1..=255, skip 0
            let len = build_art_dmx(&mut st.buf, u.universe, *seq, 0, &u.data);
            self.socket
                .send_to(&st.buf[..len], self.dest)
                .map_err(|e| OutputError::Transport(format!("artnet send: {e}")))?;
        }
        st.frames_sent += 1;
        Ok(())
    }

    fn status(&self) -> DeviceStatus {
        DeviceStatus {
            connected: true,
            frames_sent: self.state.lock().unwrap().frames_sent,
            last_send_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_poll_has_id_and_opcode() {
        let mut p = [0u8; ART_POLL_LEN];
        build_art_poll(&mut p);
        assert_eq!(&p[0..8], &ARTNET_ID);
        assert_eq!(opcode(&p), Some(OP_POLL));
        assert_eq!(opcode(b"not artnet"), None);
    }

    #[test]
    fn reply_build_parse_roundtrip() {
        let mut buf = [0u8; ART_POLL_REPLY_LEN];
        let ip = Ipv4Addr::new(192, 168, 1, 45);
        build_art_poll_reply(&mut buf, ip, &[1, 3], "Falcon");
        let r = parse_art_poll_reply(&buf).expect("parses");
        assert_eq!(r.ip, ip);
        assert_eq!(r.short_name, "Falcon");
        assert!(r.universes.contains(&1) && r.universes.contains(&3));
    }

    #[test]
    fn find_conflicts_names_the_offender() {
        let mut buf = [0u8; ART_POLL_REPLY_LEN];
        build_art_poll_reply(&mut buf, Ipv4Addr::new(10, 0, 0, 7), &[1, 3], "OtherApp");
        let reply = parse_art_poll_reply(&buf).unwrap();

        let conflicts = find_conflicts(&[3, 5], &[reply]);
        assert_eq!(conflicts.len(), 1, "only universe 3 overlaps");
        assert_eq!(conflicts[0].universe, 3);
        assert_eq!(conflicts[0].other_ip, Ipv4Addr::new(10, 0, 0, 7));

        assert!(find_conflicts(&[5, 9], &[parse_art_poll_reply(&buf).unwrap()]).is_empty());
    }

    // ── Pre-show discovery ────────────────────────────────────────────────────

    fn reply_from(ip: [u8; 4]) -> ArtPollReply {
        let mut buf = [0u8; ART_POLL_REPLY_LEN];
        build_art_poll_reply(&mut buf, Ipv4Addr::from(ip), &[1], "ctrl");
        parse_art_poll_reply(&buf).unwrap()
    }

    #[test]
    fn presence_partitions_expected_into_responded_and_missing() {
        // The user's rig: 5 controllers .156–.160; only 4 answer.
        let expected: Vec<Ipv4Addr> = (156..=160).map(|n| Ipv4Addr::new(192, 168, 2, n)).collect();
        let replies = vec![
            reply_from([192, 168, 2, 156]),
            reply_from([192, 168, 2, 157]),
            reply_from([192, 168, 2, 158]),
            reply_from([192, 168, 2, 159]),
            // .160 stayed silent (dead WiFi / powered off)
        ];
        let result = presence(&expected, &replies);
        assert_eq!(result.responded.len(), 4);
        assert_eq!(result.missing, vec![Ipv4Addr::new(192, 168, 2, 160)]);
        assert!(!result.all_present(), "one silent controller → not all present");
    }

    #[test]
    fn presence_all_present_when_every_expected_answers() {
        let expected = vec![Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)];
        let replies = vec![reply_from([10, 0, 0, 1]), reply_from([10, 0, 0, 2])];
        assert!(presence(&expected, &replies).all_present());
    }

    #[test]
    fn negative_control_rogue_reply_cannot_mask_a_missing_controller() {
        // A stray node at .99 answers, but the expected .160 does NOT. The rogue
        // reply must NOT be counted as .160 present — .160 stays missing.
        // If this ever reports all_present, the presence check is worthless.
        let expected = vec![Ipv4Addr::new(192, 168, 2, 160)];
        let replies = vec![reply_from([192, 168, 2, 99])]; // wrong device answering
        let result = presence(&expected, &replies);
        assert_eq!(result.missing, vec![Ipv4Addr::new(192, 168, 2, 160)],
            "a reply from the wrong IP must never satisfy an expected one");
        assert!(!result.all_present());
    }

    #[test]
    fn presence_empty_expected_is_trivially_present() {
        assert!(presence(&[], &[]).all_present(), "nothing expected → nothing missing");
    }

    // ── ArtDmx wire format ────────────────────────────────────────────────────

    #[test]
    fn art_dmx_wire_format_exact_offsets() {
        let mut buf = [0u8; ART_DMX_LEN];
        let data = [10u8, 20, 30, 40];
        let len = build_art_dmx(&mut buf, 0x0102, 7, 0, &data);

        assert_eq!(len, ART_DMX_HEADER_LEN + 4);
        assert_eq!(&buf[0..8], &ARTNET_ID);
        assert_eq!(opcode(&buf), Some(OP_DMX));
        assert_eq!(buf[10], 0, "ProtVerHi");
        assert_eq!(buf[11], ART_PROT_VER, "ProtVerLo = 14");
        assert_eq!(buf[12], 7, "sequence");
        assert_eq!(buf[14], 0x02, "SubUni = low byte");
        assert_eq!(buf[15], 0x01, "Net = high 7 bits");
        assert_eq!(buf[16], 0x00, "length hi (big-endian)");
        assert_eq!(buf[17], 0x04, "length lo");
        assert_eq!(&buf[18..22], &data);
    }

    #[test]
    fn art_dmx_odd_payload_padded_to_even() {
        let mut buf = [0u8; ART_DMX_LEN];
        let data = [1u8, 2, 3]; // odd length
        let len = build_art_dmx(&mut buf, 1, 1, 0, &data);
        assert_eq!(len, ART_DMX_HEADER_LEN + 4, "3 bytes rounds up to 4 on the wire");
        assert_eq!(buf[16] as usize * 256 + buf[17] as usize, 4);
        assert_eq!(buf[21], 0, "pad byte is zero");
    }

    #[test]
    fn art_dmx_build_parse_roundtrip() {
        let mut buf = [0u8; ART_DMX_LEN];
        let data: Vec<u8> = (0..=255u8).chain(0..=255u8).collect(); // 512 slots
        let len = build_art_dmx(&mut buf, 149, 42, 0, &data);
        assert_eq!(len, ART_DMX_LEN, "full universe = 530-byte packet, MTU-safe");
        let (uni, seq, parsed) = parse_art_dmx(&buf[..len]).expect("parses");
        assert_eq!(uni, 149, "universe 149 = the robot rig's highest universe");
        assert_eq!(seq, 42);
        assert_eq!(parsed, &data[..]);
    }

    #[test]
    fn art_dmx_parse_rejects_garbage() {
        assert!(parse_art_dmx(b"not artnet at all").is_none());
        let mut poll = [0u8; ART_POLL_LEN];
        build_art_poll(&mut poll);
        assert!(parse_art_dmx(&poll).is_none(), "ArtPoll is not ArtDmx");
    }

    // ── ArtNetDevice ──────────────────────────────────────────────────────────

    fn loopback_pair() -> (Arc<ArtNetDevice>, UdpSocket) {
        let rx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        rx.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let dev = ArtNetDevice::unicast(21, rx.local_addr().unwrap()).unwrap();
        (dev, rx)
    }

    #[test]
    fn artnet_device_sends_one_packet_per_universe() {
        let (dev, rx) = loopback_pair();
        let unis = vec![
            UniverseData { universe: 1, data: vec![0xAA; 510] },
            UniverseData { universe: 2, data: vec![0xBB; 510] },
        ];
        dev.send_physical(&unis).unwrap();

        let mut got = Vec::new();
        let mut pkt = [0u8; 600];
        for _ in 0..2 {
            let (n, _) = rx.recv_from(&mut pkt).unwrap();
            let (uni, seq, data) = parse_art_dmx(&pkt[..n]).expect("valid ArtDmx");
            assert_eq!(seq, 1, "first frame → sequence 1");
            got.push((uni, data[0]));
        }
        got.sort();
        assert_eq!(got, vec![(1, 0xAA), (2, 0xBB)]);
        assert_eq!(dev.frames_sent(), 1);
    }

    #[test]
    fn artnet_sequence_is_per_universe_and_wraps_skipping_zero() {
        let (dev, rx) = loopback_pair();
        let mut pkt = [0u8; 600];

        // 300 frames on universe 1 only — universe 2 must stay untouched at seq 1..
        for _ in 0..300 {
            dev.send_physical(&[UniverseData { universe: 1, data: vec![1, 2] }]).unwrap();
        }
        let mut last_seq = 0u8;
        let mut seen_zero = false;
        for _ in 0..300 {
            let (n, _) = rx.recv_from(&mut pkt).unwrap();
            let (_, seq, _) = parse_art_dmx(&pkt[..n]).unwrap();
            if seq == 0 { seen_zero = true; }
            last_seq = seq;
        }
        assert!(!seen_zero, "sequence 0 disables checking — must never be emitted");
        // 300 sends wrapping 1..=255: 255 then restart at 1 → 300-255=45
        assert_eq!(last_seq, 45, "wrap 1..=255 (never 0)");

        // now universe 2 starts fresh at 1 — per-universe, not global
        dev.send_physical(&[UniverseData { universe: 2, data: vec![9, 9] }]).unwrap();
        let (n, _) = rx.recv_from(&mut pkt).unwrap();
        let (uni, seq, _) = parse_art_dmx(&pkt[..n]).unwrap();
        assert_eq!((uni, seq), (2, 1), "universe 2 has its own counter");
    }

    #[test]
    fn artnet_device_rejects_oversized_universe() {
        let (dev, _rx) = loopback_pair();
        let too_big = vec![UniverseData { universe: 1, data: vec![0; 513] }];
        assert!(dev.send_physical(&too_big).is_err(), "513 slots must be refused");
    }
}
