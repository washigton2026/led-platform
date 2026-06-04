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
}
