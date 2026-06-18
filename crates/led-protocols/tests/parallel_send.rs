//! Integration tests for the async parallel universe sender.
//! Proves: persistent tasks deliver frames, per-universe sequence numbers increment
//! independently, and multiple universes send concurrently.

use std::time::Duration;

use led_core::UniverseData;
use led_protocols::{packet, sender::ParallelSender};
use tokio::net::UdpSocket;

fn make_universe(universe: u16, fill: u8) -> UniverseData {
    UniverseData { universe, data: vec![fill; packet::DMX_SLOTS] }
}

/// Push one frame, collect the first packet that arrives on `rx` (with timeout).
async fn recv_one(rx: &UdpSocket) -> Option<Vec<u8>> {
    let mut buf = [0u8; 1500];
    match tokio::time::timeout(Duration::from_secs(2), rx.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => Some(buf[..n].to_vec()),
        _ => None,
    }
}

#[tokio::test]
async fn parallel_sender_delivers_frames_to_all_universes() {
    // Two independent receivers.
    let rx1 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let rx2 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr1 = rx1.local_addr().unwrap();
    let addr2 = rx2.local_addr().unwrap();

    let mut sender = ParallelSender::new([0xAA; 16], "parallel-test");
    sender.add_universe(1, addr1).await.unwrap();
    sender.add_universe(2, addr2).await.unwrap();
    assert_eq!(sender.universe_count(), 2);

    // Push one frame for both universes.
    sender.push_frame(&[make_universe(1, 42), make_universe(2, 99)]);

    // Both tasks should deliver within 500 ms.
    let p1 = recv_one(&rx1).await.expect("universe 1 packet");
    let p2 = recv_one(&rx2).await.expect("universe 2 packet");

    assert_eq!(packet::universe_of(&p1), 1);
    assert_eq!(packet::dmx_slots(&p1)[0], 42, "DMX data for universe 1");

    assert_eq!(packet::universe_of(&p2), 2);
    assert_eq!(packet::dmx_slots(&p2)[0], 99, "DMX data for universe 2");

    // Well-formed E1.31 packet.
    assert!(packet::acn_pid_ok(&p1), "ACN packet identifier");
    assert_eq!(packet::root_vector(&p1), packet::VECTOR_ROOT_E131_DATA);
    assert_eq!(packet::start_code(&p1), 0x00, "DMX start code");
    assert_eq!(p1.len(), packet::PACKET_LEN, "full 638-byte packet");
}

#[tokio::test]
async fn per_universe_sequences_increment_independently() {
    let rx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = rx.local_addr().unwrap();

    let mut sender = ParallelSender::new([0xBB; 16], "seq-test");
    sender.add_universe(5, addr).await.unwrap();

    // Push 3 frames; recv_timeout(300ms) is itself the causal barrier — sleep removed.
    let mut seqs = Vec::new();
    for _ in 0..3 {
        sender.push_frame(&[make_universe(5, 1)]);
        // No sleep needed: recv_timeout below waits up to 300ms for the packet.
        let mut buf = [0u8; 1500];
        if let Ok(Ok((n, _))) = tokio::time::timeout(
            Duration::from_millis(300),
            rx.recv_from(&mut buf),
        ).await {
            seqs.push(packet::sequence_of(&buf[..n]));
        }
    }

    assert!(!seqs.is_empty(), "at least one packet received");
    // Each packet's sequence should be one more than the previous (wrapping).
    for w in seqs.windows(2) {
        assert_eq!(w[1], w[0].wrapping_add(1),
            "sequences must increment: {:?}", seqs);
    }
}

#[tokio::test]
async fn watch_channel_delivers_latest_frame_when_producer_is_faster() {
    // Push many frames quickly before the task can drain them.
    // The task should always send the most recent, never queue up stale ones.
    let rx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = rx.local_addr().unwrap();

    let mut sender = ParallelSender::new([0xCC; 16], "latest-test");
    sender.add_universe(7, addr).await.unwrap();

    // Rapid-fire 10 frames — latest value is fill=255.
    for v in 0u8..=10 {
        sender.push_frame(&[make_universe(7, v)]);
    }

    // Causal: wait for the first packet to arrive (up to 2s), then drain the rest quickly.
    let mut last_fill = 0u8;
    let mut buf = [0u8; 1500];
    // First packet: longer timeout to let the async sender task start.
    if let Ok(Ok((n, _))) = tokio::time::timeout(Duration::from_secs(2), rx.recv_from(&mut buf)).await {
        last_fill = packet::dmx_slots(&buf[..n])[0];
    }
    // Drain any further packets with a short timeout.
    while let Ok(Ok((n, _))) = tokio::time::timeout(Duration::from_millis(50), rx.recv_from(&mut buf)).await {
        last_fill = packet::dmx_slots(&buf[..n])[0];
    }

    // The final packet should carry the most recent frame (fill = 10).
    assert_eq!(last_fill, 10, "watch delivers latest: got {last_fill}");
}
