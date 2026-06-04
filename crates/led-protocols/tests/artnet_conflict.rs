//! Proves source-conflict detection over the wire: a node's ArtPollReply is sent on UDP,
//! parsed, and `find_conflicts` names the offending IP for an overlapping universe.

use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

use led_protocols::artnet::{build_art_poll_reply, parse_art_poll_reply, ART_POLL_REPLY_LEN};
use led_protocols::find_conflicts;

#[test]
fn discovers_a_conflicting_node_over_the_wire() {
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    rx.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let dest = rx.local_addr().unwrap();

    // A "node" announces it outputs universe 3, from 10.0.0.7.
    let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut reply = [0u8; ART_POLL_REPLY_LEN];
    build_art_poll_reply(&mut reply, Ipv4Addr::new(10, 0, 0, 7), &[3], "Falcon F48");
    tx.send_to(&reply, dest).unwrap();

    let mut buf = [0u8; 1024];
    let (n, _) = rx.recv_from(&mut buf).unwrap();
    let parsed = parse_art_poll_reply(&buf[..n]).expect("valid ArtPollReply");
    assert_eq!(parsed.short_name, "Falcon F48");

    // We intend to drive universes 3 and 5 → conflict on 3, naming the IP.
    let conflicts = find_conflicts(&[3, 5], &[parsed]);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].universe, 3);
    assert_eq!(conflicts[0].other_ip, Ipv4Addr::new(10, 0, 0, 7));
}
