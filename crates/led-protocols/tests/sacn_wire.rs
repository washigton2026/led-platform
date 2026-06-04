//! Proves the E1.31 driver puts correct bytes on the wire, with per-universe sequencing.
//! Sends to a real UDP socket on localhost and parses what arrives.

use std::net::UdpSocket;
use std::time::Duration;

use led_core::{DeviceDriver, UniverseData};
use led_protocols::{packet, SacnDevice};

fn recv_n(rx: &UdpSocket, n: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 1500];
    for _ in 0..n {
        let (len, _) = rx.recv_from(&mut buf).expect("datagram");
        out.push(buf[..len].to_vec());
    }
    out
}

fn by_universe(pkts: &[Vec<u8>], u: u16) -> &Vec<u8> {
    pkts.iter().find(|p| packet::universe_of(p) == u).expect("universe present")
}

fn full(universe: u16, fill: &[(usize, u8)]) -> UniverseData {
    let mut data = vec![0u8; packet::DMX_SLOTS];
    for &(i, v) in fill {
        data[i] = v;
    }
    UniverseData { universe, data }
}

#[test]
fn sacn_wire_format_and_per_universe_sequencing() {
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    rx.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let dest = rx.local_addr().unwrap();

    let cid = [0x11u8; 16];
    let dev = SacnDevice::unicast(7, dest, cid, "led-platform test").unwrap();

    let u1 = full(1, &[(0, 10), (1, 20), (2, 30)]);
    let u2 = full(2, &[(0, 200)]);

    // First send: one packet per universe.
    dev.send_physical(&[u1.clone(), u2.clone()]).unwrap();
    let first = recv_n(&rx, 2);
    let p1 = by_universe(&first, 1);
    let p2 = by_universe(&first, 2);

    // Well-formed E1.31.
    assert_eq!(p1.len(), packet::PACKET_LEN, "packet is the full 638 bytes");
    assert!(packet::acn_pid_ok(p1), "ACN packet identifier");
    assert_eq!(packet::root_vector(p1), packet::VECTOR_ROOT_E131_DATA);
    assert_eq!(packet::framing_vector(p1), packet::VECTOR_E131_DATA_PACKET);
    assert_eq!(packet::start_code(p1), 0x00, "DMX start code");

    // Data carried correctly.
    assert_eq!(&packet::dmx_slots(p1)[..3], &[10, 20, 30]);
    assert_eq!(packet::dmx_slots(p2)[0], 200);

    // Each universe's sequence starts at 1.
    assert_eq!(packet::sequence_of(p1), 1);
    assert_eq!(packet::sequence_of(p2), 1);

    // Second send: each universe's sequence increments INDEPENDENTLY to 2.
    dev.send_physical(&[u1, u2]).unwrap();
    let second = recv_n(&rx, 2);
    assert_eq!(packet::sequence_of(by_universe(&second, 1)), 2);
    assert_eq!(packet::sequence_of(by_universe(&second, 2)), 2);
}
