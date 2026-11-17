//! Network-level chaos: real ArtDmx datagrams through a lossy UDP proxy.
//!
//! `led_hal::ChaosHarness` proves the *system* degrades gracefully when
//! `send_frame` fails; these tests prove the same at the *wire*: actual
//! datagrams dropped between two real sockets — the CI-side stand-in for the
//! physical chaos runbook (cable pulls, switch reboots).

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use integration_tests::{ProxyFaults, UdpChaosProxy};
use led_core::{DeviceDriver, UniverseData};
use led_protocols::{parse_art_dmx, ArtNetDevice};

/// Receiver that counts valid ArtDmx datagrams until 100ms of silence.
fn drain_artdmx(rx: &UdpSocket) -> u64 {
    let mut buf = [0u8; 2048];
    let mut count = 0u64;
    rx.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    while let Ok((n, _)) = rx.recv_from(&mut buf) {
        if parse_art_dmx(&buf[..n]).is_some() {
            count += 1;
        }
    }
    count
}

fn send_frames(dev: &ArtNetDevice, n: u64) {
    let unis = vec![UniverseData { universe: 1, data: vec![0x55; 510] }];
    for _ in 0..n {
        // Individual sends may fail under chaos — that's the point.
        let _ = dev.send_physical(&unis);
    }
}

#[test]
fn baseline_proxy_forwards_every_datagram() {
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    let proxy = UdpChaosProxy::start(rx.local_addr().unwrap(), ProxyFaults::default()).unwrap();
    let dev = ArtNetDevice::unicast(1, proxy.addr()).unwrap();

    send_frames(&dev, 100);
    let received = drain_artdmx(&rx);

    assert_eq!(received, 100, "transparent proxy must forward all 100");
    assert_eq!(proxy.dropped(), 0);
    proxy.stop();
}

#[test]
fn thirty_pct_wire_loss_degrades_but_does_not_stop_the_stream() {
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    let proxy = UdpChaosProxy::start(
        rx.local_addr().unwrap(),
        ProxyFaults { loss_pct: 30, latency: Duration::ZERO, seed: 4242 },
    )
    .unwrap();
    let dev = ArtNetDevice::unicast(2, proxy.addr()).unwrap();

    send_frames(&dev, 300);
    let received = drain_artdmx(&rx);

    // 30% loss: expect ~210 of 300; allow a generous band.
    assert!(received > 150 && received < 290,
        "~70% must survive 30% wire loss, got {received}/300");
    assert!(proxy.dropped() > 50, "proxy must actually drop, got {}", proxy.dropped());
    // The sender never errored or stalled: sending is fire-and-forget UDP.
    assert_eq!(dev.frames_sent(), 300, "sender keeps sending through loss");
    proxy.stop();
}

#[test]
fn network_heals_and_delivery_returns_to_100pct() {
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    let proxy = UdpChaosProxy::start(
        rx.local_addr().unwrap(),
        ProxyFaults { loss_pct: 100, latency: Duration::ZERO, seed: 7 },
    )
    .unwrap();
    let dev = ArtNetDevice::unicast(3, proxy.addr()).unwrap();

    // Total outage: nothing arrives.
    send_frames(&dev, 50);
    assert_eq!(drain_artdmx(&rx), 0, "100% loss = total outage");

    // Heal the network: everything arrives again.
    proxy.heal();
    send_frames(&dev, 50);
    let after = drain_artdmx(&rx);
    assert_eq!(after, 50, "after heal, all 50 must arrive");
    proxy.stop();
}

#[test]
fn latency_injection_delays_but_delivers() {
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    let proxy = UdpChaosProxy::start(
        rx.local_addr().unwrap(),
        ProxyFaults { loss_pct: 0, latency: Duration::from_millis(2), seed: 0 },
    )
    .unwrap();
    let dev = ArtNetDevice::unicast(4, proxy.addr()).unwrap();

    let t0 = std::time::Instant::now();
    send_frames(&dev, 20);
    let received = drain_artdmx(&rx);
    let elapsed = t0.elapsed();

    assert_eq!(received, 20, "latency must not lose datagrams");
    assert!(elapsed >= Duration::from_millis(30),
        "20 × 2ms proxy delay must be observable, got {elapsed:?}");
    proxy.stop();
}

#[test]
fn same_seed_same_wire_drop_count() {
    const ENVIAR: u64 = 200;
    const LOTE: u64 = 20;
    // Injected loss is deterministic per datagram the proxy *reads*. If the OS drops some
    // before the proxy sees them (TD-034: a 200-datagram burst overflowed the receive
    // buffer on ubuntu-24.04), the PRNG walks a shorter input and the counts diverge for a
    // reason that has nothing to do with the seed. So the sender waits for the proxy to read
    // each batch before the next, and a short input is reported as a broken precondition.
    let run = |seed: u64| -> u64 {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let proxy = UdpChaosProxy::start(
            rx.local_addr().unwrap(),
            ProxyFaults { loss_pct: 40, latency: Duration::ZERO, seed },
        )
        .unwrap();
        let dev = ArtNetDevice::unicast(5, proxy.addr()).unwrap();
        let mut enviados = 0;
        while enviados < ENVIAR {
            let n = LOTE.min(ENVIAR - enviados);
            send_frames(&dev, n);
            enviados += n;
            let prazo = Instant::now() + Duration::from_secs(2);
            while proxy.received() < enviados && Instant::now() < prazo {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let _ = drain_artdmx(&rx);
        let (recebidos, dropped) = (proxy.received(), proxy.dropped());
        proxy.stop();
        assert_eq!(
            recebidos, ENVIAR,
            "pré-condição: o proxy leu {recebidos} de {ENVIAR} datagramas (seed {seed}); \
             faltam antes do proxy (descarte do SO ou envio falhado), não por perda injetada — \
             não é divergência de determinismo"
        );
        dropped
    };
    assert_eq!(run(99), run(99), "deterministic chaos: same seed, same drops");
}
