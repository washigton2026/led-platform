//! `SacnDevice` — an E1.31 (sACN) [`DeviceDriver`]. It serializes each universe into a
//! correct E1.31 data packet and sends it over UDP. Sequence numbers are tracked
//! **per universe** (the invariant from led-protocols), wrapping 0..=255.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};

use led_core::{DeviceDriver, DeviceId, DeviceStatus, OutputError, UniverseData};

use crate::packet::{self, DMX_SLOTS, PACKET_LEN, SACN_PORT};

/// sACN multicast TTL. Low by default (local segments); raise only deliberately for routed
/// multicast. Multicast still requires IGMP snooping on the path (see `/security`).
pub const SACN_MULTICAST_TTL: u32 = 16;

/// Where a device sends each universe.
enum Dest {
    /// Every universe to one address (handy for WLED/DDP-style targets and for tests).
    Unicast(SocketAddr),
    /// Each universe to its own sACN multicast group, 239.255.<hi>.<lo>:5568.
    Multicast,
}

struct SacnState {
    seqs: HashMap<u16, u8>,    // per-universe sequence, wrapping
    buf: Box<[u8; PACKET_LEN]>, // reused packet buffer
    frames_sent: u64,
}

pub struct SacnDevice {
    id: DeviceId,
    socket: UdpSocket,
    dest: Dest,
    cid: [u8; 16],
    source_name: String,
    priority: u8,
    state: Mutex<SacnState>,
}

/// The standard sACN multicast address for a universe: 239.255.<hi>.<lo>:5568.
pub fn multicast_addr(universe: u16) -> SocketAddr {
    let hi = (universe >> 8) as u8;
    let lo = (universe & 0xff) as u8;
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, hi, lo), SACN_PORT))
}

impl SacnDevice {
    /// Unicast sender: every universe is sent to `dest`. (Multicast per-universe is the
    /// production default — see [`multicast_addr`]; this slice uses unicast so a test can
    /// receive on localhost.)
    pub fn unicast(
        id: DeviceId,
        dest: SocketAddr,
        cid: [u8; 16],
        source_name: impl Into<String>,
    ) -> std::io::Result<Arc<Self>> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        Ok(Self::build(id, socket, Dest::Unicast(dest), cid, source_name.into()))
    }

    /// Multicast sender: each universe goes to its own sACN group (239.255.<hi>.<lo>:5568).
    /// This is the production default for scaling to many controllers — but the network
    /// path MUST have IGMP snooping, or multicast floods/dies (see `/security`).
    pub fn multicast(
        id: DeviceId,
        cid: [u8; 16],
        source_name: impl Into<String>,
    ) -> std::io::Result<Arc<Self>> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_multicast_ttl_v4(SACN_MULTICAST_TTL)?;
        socket.set_multicast_loop_v4(true)?; // same-host receivers (and tests) get a copy
        Ok(Self::build(id, socket, Dest::Multicast, cid, source_name.into()))
    }

    fn build(id: DeviceId, socket: UdpSocket, dest: Dest, cid: [u8; 16], source_name: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            socket,
            dest,
            cid,
            source_name,
            priority: 100,
            state: Mutex::new(SacnState {
                seqs: HashMap::new(),
                buf: Box::new([0u8; PACKET_LEN]),
                frames_sent: 0,
            }),
        })
    }

    fn dest_for(&self, universe: u16) -> SocketAddr {
        match self.dest {
            Dest::Unicast(addr) => addr,
            Dest::Multicast => multicast_addr(universe),
        }
    }

    /// The local address the sending socket is bound to (useful in tests).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl DeviceDriver for SacnDevice {
    fn id(&self) -> DeviceId {
        self.id
    }

    fn send_physical(&self, universes: &[UniverseData]) -> Result<(), OutputError> {
        let mut st = self.state.lock().unwrap();
        let SacnState { seqs, buf, frames_sent } = &mut *st;

        for u in universes {
            if u.data.len() != DMX_SLOTS {
                return Err(OutputError::Transport(format!(
                    "universe {} has {} channels, expected {DMX_SLOTS}",
                    u.universe,
                    u.data.len()
                )));
            }
            // Per-universe sequence: independent wrapping counter.
            let seq = seqs.entry(u.universe).or_insert(0);
            *seq = seq.wrapping_add(1);

            packet::build_data_packet(
                buf,
                &self.cid,
                &self.source_name,
                self.priority,
                *seq,
                u.universe,
                &u.data,
            );
            self.socket
                .send_to(&buf[..], self.dest_for(u.universe))
                .map_err(|e| OutputError::Transport(e.to_string()))?;
        }

        *frames_sent += 1;
        Ok(())
    }

    fn status(&self) -> DeviceStatus {
        let st = self.state.lock().unwrap();
        DeviceStatus { connected: true, frames_sent: st.frames_sent, last_send_ms: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multicast_group_is_per_universe() {
        // sACN: 239.255.<hi>.<lo>:5568
        assert_eq!(multicast_addr(1), "239.255.0.1:5568".parse().unwrap());
        assert_eq!(multicast_addr(0x0102), "239.255.1.2:5568".parse().unwrap());
        assert_eq!(multicast_addr(0), "239.255.0.0:5568".parse().unwrap());
    }
}
