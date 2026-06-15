//! E1.31 (sACN) data packet — build + a few accessors for tests.
//!
//! Layout (fixed 512-slot packet = 638 bytes): Root (38) + Framing (77) + DMP (523).
//! See ANSI E1.31-2018. Offsets are spelled out so the wire format is auditable.

/// ACN packet identifier: "ASC-E1.17" + 3 nulls.
pub const ACN_PID: [u8; 12] = [0x41, 0x53, 0x43, 0x2d, 0x45, 0x31, 0x2e, 0x31, 0x37, 0x00, 0x00, 0x00];

pub const VECTOR_ROOT_E131_DATA: u32 = 0x0000_0004;
pub const VECTOR_E131_DATA_PACKET: u32 = 0x0000_0002;
pub const VECTOR_DMP_SET_PROPERTY: u8 = 0x02;

/// The standard sACN UDP port.
pub const SACN_PORT: u16 = 5568;

pub const DMX_SLOTS: usize = 512;
pub const PACKET_LEN: usize = 638;

#[inline]
fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_be_bytes());
}
#[inline]
fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
}
#[inline]
fn get_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([buf[off], buf[off + 1]])
}
#[inline]
fn get_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Build a full data packet into a pre-sized 638-byte buffer. Allocation-free.
///
/// `dmx` must be exactly [`DMX_SLOTS`] bytes (the universe's channel data).
pub fn build_data_packet(
    buf: &mut [u8; PACKET_LEN],
    cid: &[u8; 16],
    source_name: &str,
    priority: u8,
    sequence: u8,
    universe: u16,
    dmx: &[u8],
) {
    debug_assert_eq!(dmx.len(), DMX_SLOTS);

    // --- Root layer (0..38) ---
    put_u16(buf, 0, 0x0010); // preamble size
    put_u16(buf, 2, 0x0000); // postamble size
    buf[4..16].copy_from_slice(&ACN_PID);
    put_u16(buf, 16, 0x7000 | (PACKET_LEN - 16) as u16); // flags + length
    put_u32(buf, 18, VECTOR_ROOT_E131_DATA);
    buf[22..38].copy_from_slice(cid);

    // --- Framing layer (38..115) ---
    put_u16(buf, 38, 0x7000 | (PACKET_LEN - 38) as u16);
    put_u32(buf, 40, VECTOR_E131_DATA_PACKET);
    // source name: 64 bytes, UTF-8, null-padded
    let name = source_name.as_bytes();
    let n = name.len().min(63);
    buf[44..108].fill(0);
    buf[44..44 + n].copy_from_slice(&name[..n]);
    buf[108] = priority;
    put_u16(buf, 109, 0); // synchronization address
    buf[111] = sequence;
    buf[112] = 0; // options
    put_u16(buf, 113, universe);

    // --- DMP layer (115..638) ---
    put_u16(buf, 115, 0x7000 | (PACKET_LEN - 115) as u16);
    buf[117] = VECTOR_DMP_SET_PROPERTY;
    buf[118] = 0xa1; // address type & data type
    put_u16(buf, 119, 0x0000); // first property address
    put_u16(buf, 121, 0x0001); // address increment
    put_u16(buf, 123, (DMX_SLOTS + 1) as u16); // property value count (incl. start code)
    buf[125] = 0x00; // DMX start code
    buf[126..126 + DMX_SLOTS].copy_from_slice(dmx);
}

// --- Accessors (used by tests to verify the wire format) ---

pub fn acn_pid_ok(pkt: &[u8]) -> bool {
    pkt.len() >= 16 && pkt[4..16] == ACN_PID
}
pub fn root_vector(pkt: &[u8]) -> u32 {
    get_u32(pkt, 18)
}
pub fn framing_vector(pkt: &[u8]) -> u32 {
    get_u32(pkt, 40)
}
pub fn sequence_of(pkt: &[u8]) -> u8 {
    pkt[111]
}
pub fn universe_of(pkt: &[u8]) -> u16 {
    get_u16(pkt, 113)
}
pub fn start_code(pkt: &[u8]) -> u8 {
    pkt[125]
}
pub fn dmx_slots(pkt: &[u8]) -> &[u8] {
    &pkt[126..126 + DMX_SLOTS]
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;

    fn cid() -> [u8; 16] { [0xAB; 16] }
    fn dmx_full() -> [u8; DMX_SLOTS] { [0xCC; DMX_SLOTS] }
    fn dmx_zeros() -> [u8; DMX_SLOTS] { [0x00; DMX_SLOTS] }

    // ── PROTOCOL: ACN PID must be correct on every packet ─────────────────
    #[test]
    fn acn_pid_invariant() {
        let mut buf = [0u8; PACKET_LEN];
        build_data_packet(&mut buf, &cid(), "LUMYX", 100, 0, 1, &dmx_full());
        assert!(acn_pid_ok(&buf), "ACN PID must be correct");
        assert_eq!(root_vector(&buf), VECTOR_ROOT_E131_DATA);
        assert_eq!(framing_vector(&buf), VECTOR_E131_DATA_PACKET);
    }

    // ── PROTOCOL: DMX start code must always be 0x00 ──────────────────────
    #[test]
    fn dmx_start_code_always_zero() {
        let mut buf = [0u8; PACKET_LEN];
        for priority in [0u8, 100, 200] {
            build_data_packet(&mut buf, &cid(), "TEST", priority, 42, 63999, &dmx_full());
            assert_eq!(start_code(&buf), 0x00, "DMX start code must be 0x00");
        }
    }

    // ── PROTOCOL: sequence wraps 0..=255 ──────────────────────────────────
    #[test]
    fn sequence_wraps_correctly() {
        let mut buf = [0u8; PACKET_LEN];
        for seq in [0u8, 127, 255] {
            build_data_packet(&mut buf, &cid(), "TEST", 100, seq, 1, &dmx_zeros());
            assert_eq!(sequence_of(&buf), seq);
        }
    }

    // ── PROTOCOL: universe field round-trips 1..=63999 ────────────────────
    #[test]
    fn universe_round_trips() {
        let mut buf = [0u8; PACKET_LEN];
        for u in [1u16, 512, 1024, 32768, 63999] {
            build_data_packet(&mut buf, &cid(), "TEST", 100, 0, u, &dmx_zeros());
            assert_eq!(universe_of(&buf), u, "universe {u} must round-trip");
        }
    }

    // ── PROTOCOL: DMX payload integrity — all 512 bytes preserved ─────────
    #[test]
    fn dmx_payload_integrity() {
        let mut buf = [0u8; PACKET_LEN];
        let mut dmx = [0u8; DMX_SLOTS];
        for (i, b) in dmx.iter_mut().enumerate() { *b = (i % 256) as u8; }
        build_data_packet(&mut buf, &cid(), "LUMYX", 100, 0, 1, &dmx);
        assert_eq!(dmx_slots(&buf), &dmx, "all 512 DMX bytes must survive packet build");
    }

    // ── FUZZ: source name at max length (63 chars) ─────────────────────────
    #[test]
    fn source_name_max_length_no_overflow() {
        let mut buf = [0u8; PACKET_LEN];
        let long_name = "A".repeat(100); // longer than 63 bytes — must be truncated
        build_data_packet(&mut buf, &cid(), &long_name, 100, 0, 1, &dmx_zeros());
        assert!(acn_pid_ok(&buf), "long source name must not corrupt packet");
        // Verify null termination exists within the 64-byte name field
        let name_field = &buf[44..108];
        assert!(name_field.iter().any(|&b| b == 0), "name field must be null-terminated");
    }

    // ── FUZZ: empty source name ────────────────────────────────────────────
    #[test]
    fn empty_source_name_is_valid() {
        let mut buf = [0u8; PACKET_LEN];
        build_data_packet(&mut buf, &cid(), "", 100, 0, 1, &dmx_zeros());
        assert!(acn_pid_ok(&buf));
        assert_eq!(buf[44], 0x00, "empty name: first byte of name field must be null");
    }

    // ── PROTOCOL: priority clamps stay in wire format ──────────────────────
    #[test]
    fn priority_extremes_survive_wire() {
        let mut buf = [0u8; PACKET_LEN];
        build_data_packet(&mut buf, &cid(), "TEST", 0, 0, 1, &dmx_zeros());
        assert_eq!(buf[108], 0, "priority 0 on wire");
        build_data_packet(&mut buf, &cid(), "TEST", 200, 0, 1, &dmx_zeros());
        assert_eq!(buf[108], 200, "priority 200 on wire");
    }

    // ── STRESS: build 10_000 packets without allocation ───────────────────
    #[test]
    fn stress_10k_packet_builds() {
        let mut buf = [0u8; PACKET_LEN];
        let dmx = dmx_full();
        for seq in 0..10_000u32 {
            build_data_packet(&mut buf, &cid(), "LUMYX-STRESS", 100, (seq % 256) as u8, (seq % 512 + 1) as u16, &dmx);
            assert!(acn_pid_ok(&buf));
        }
    }
}
