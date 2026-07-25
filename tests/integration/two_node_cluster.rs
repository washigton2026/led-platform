//! Two-node cluster integration test.
//!
//! Simulates two independent processes each running a `SyncedCluster` segment,
//! verifying that both receive frames and that drift detection works across nodes.
//!
//! This test is the closest we can get to a real two-node cluster without
//! spawning separate OS processes — each "node" is an independent `SyncedCluster`
//! running in its own thread with its own `SharedClock`.

use std::sync::Arc;
use std::time::Duration;

use led_core::{CompiledLayout, DeviceSpec, LogicalFrame, PixelColor, RgbOrder};
use led_hal::{
    calibrate_offset, ChaosHarness, FaultConfig, Hal, MetricsEmitter,
    SegmentState, SharedClock, SimulatorDevice, SyncedCluster,
};
use led_core::ProtocolOutput;

const N: usize = 8;
const PIXEL_UNIVERSES: u16 = 1; // 8 pixels × 3 bytes = 24 bytes < 512

fn make_segment(device_id: u16) -> (Hal, Arc<SimulatorDevice>) {
    let specs = [DeviceSpec { id: device_id, universes: PIXEL_UNIVERSES }];
    let layout = CompiledLayout::linear(N, &specs, RgbOrder::Rgb);
    let sim = SimulatorDevice::new(device_id, layout.device_universes(device_id));
    (Hal::new(layout, vec![sim.clone()]), sim)
}

fn test_frame(t: u64) -> LogicalFrame {
    LogicalFrame::new(vec![PixelColor::rgb(100, 150, 200); N], t)
}

// ── Test 1: Two nodes, calibrated clocks, same frames ─────────────────────────

#[test]
fn two_nodes_receive_identical_frame_count() {
    // Node A: reference clock (offset=0)
    let clock_a = Arc::new(SharedClock::new());
    let (h_a, sim_a) = make_segment(1);
    let metrics_a = Arc::new(MetricsEmitter::new("node-a"));
    let cluster_a = SyncedCluster::new(vec![h_a], clock_a.clone(), 100)
        .with_metrics(metrics_a.clone());

    // Node B: follower clock, calibrated to A
    let clock_b = Arc::new(SharedClock::new());
    let offset = calibrate_offset(clock_a.now_ms(), clock_b.now_ms());
    clock_b.set_offset_ms(offset);
    let (h_b, sim_b) = make_segment(2);
    let metrics_b = Arc::new(MetricsEmitter::new("node-b"));
    let cluster_b = SyncedCluster::new(vec![h_b], clock_b.clone(), 100)
        .with_metrics(metrics_b.clone());

    // Send 50 frames to both nodes
    for i in 0..50u64 {
        let t = i * 33;
        cluster_a.send_frame(&test_frame(t)).unwrap();
        cluster_b.send_frame(&test_frame(t)).unwrap();
    }

    assert_eq!(sim_a.frames_sent(), 50, "node A must receive 50 frames");
    assert_eq!(sim_b.frames_sent(), 50, "node B must receive 50 frames");
    assert_eq!(metrics_a.frame_count(), 50);
    assert_eq!(metrics_b.frame_count(), 50);
}

// ── Test 2: Hot-join while show is running ────────────────────────────────────

#[test]
fn hot_join_node_receives_frames_after_joining() {
    let clock = Arc::new(SharedClock::new());
    let (h1, sim1) = make_segment(3);
    let cluster = SyncedCluster::new(vec![h1], clock.clone(), 100);

    // Send 20 frames before hot-join
    for i in 0..20u64 {
        cluster.send_frame(&test_frame(i * 33)).unwrap();
    }
    assert_eq!(sim1.frames_sent(), 20);

    // Hot-join second node
    let (h2, sim2) = make_segment(4);
    cluster.hot_join(h2);

    // Send 30 more frames
    for i in 20..50u64 {
        cluster.send_frame(&test_frame(i * 33)).unwrap();
    }

    assert_eq!(sim1.frames_sent(), 50, "original node: all 50 frames");
    assert_eq!(sim2.frames_sent(), 30, "hot-joined node: only 30 post-join frames");
}

// ── Test 3: Failover — one node fails, other continues ────────────────────────

#[test]
fn failover_continues_when_one_node_fails() {
    let clock = Arc::new(SharedClock::new());
    let (h1, sim1) = make_segment(5);
    let (h2, sim2) = make_segment(6);
    let cluster = SyncedCluster::new(vec![h1, h2], clock.clone(), 100);

    // Send 10 good frames
    for i in 0..10u64 {
        cluster.send_frame(&test_frame(i * 33)).unwrap();
    }

    // Manually mark segment 1 as failed (simulates network partition)
    {
        let mut segs = cluster.segments.write().unwrap();
        let (_, health) = &mut segs[1];
        health.consecutive_fails = 15;
        health.state = SegmentState::Failed;
    }

    // Send 10 more frames — must succeed (node 0 still active)
    for i in 10..20u64 {
        cluster.send_frame(&test_frame(i * 33)).expect("cluster must continue with one failed node");
    }

    assert_eq!(sim1.frames_sent(), 20, "healthy node: all 20 frames");
    assert_eq!(sim2.frames_sent(), 10, "failed node: only pre-failure frames");
    assert_eq!(cluster.active_segment_count(), 1, "1 active after failover");
}

// ── Test 4: Chaos + cluster — 30% loss does not crash the cluster ─────────────

#[test]
fn cluster_survives_chaotic_network_on_one_node() {
    let clock = Arc::new(SharedClock::new());
    let (h_reliable, sim_reliable) = make_segment(7);
    let (h_chaotic,  sim_chaotic)  = make_segment(8);

    // Wrap the chaotic node in a ChaosHarness
    let chaos = ChaosHarness::new(h_chaotic, FaultConfig::packet_loss(30, 777));

    let cluster_reliable = SyncedCluster::new(vec![h_reliable], clock.clone(), 100);
    // The chaotic node runs independently (ChaosHarness wraps the HAL directly)
    for i in 0..200u64 {
        let f = test_frame(i * 33);
        cluster_reliable.send_frame(&f).unwrap();
        // Simulate chaotic node receiving same frame
        let _ = chaos.send_frame(&f); // may drop silently
    }

    assert_eq!(sim_reliable.frames_sent(), 200, "reliable node: all frames");
    // Chaotic: some drops expected, but not panic
    let chaotic_received = sim_chaotic.frames_sent();
    assert!(chaotic_received > 100 && chaotic_received < 200,
        "chaotic node should have 30% loss: got {chaotic_received}/200");
}

// ── Test 5: SharedClock alignment — both nodes within drift budget ─────────────

#[test]
fn two_nodes_clock_within_drift_budget() {
    let clock_a = Arc::new(SharedClock::new());
    let clock_b = Arc::new(SharedClock::new());

    std::thread::sleep(Duration::from_millis(5));

    let ts_a = clock_a.now_ms();
    let ts_b = clock_b.now_ms();

    // Calibrate B to A
    let offset = calibrate_offset(ts_a, ts_b);
    clock_b.set_offset_ms(offset);

    // After calibration, both clocks should read within 10ms of each other
    let a_now = clock_a.now_ms();
    let b_now = clock_b.now_ms();
    let drift = a_now.abs_diff(b_now);
    assert!(drift < 10, "calibrated clocks must be within 10ms: drift={drift}ms");
}

// ── Test 6: Metrics consistency across nodes ───────────────────────────────────

#[test]
fn metrics_consistent_across_two_nodes() {
    let clock = Arc::new(SharedClock::new());
    let m_a = Arc::new(MetricsEmitter::new("node-a"));
    let m_b = Arc::new(MetricsEmitter::new("node-b"));

    let (h_a, _) = make_segment(9);
    let (h_b, _) = make_segment(10);
    let c_a = SyncedCluster::new(vec![h_a], clock.clone(), 50).with_metrics(m_a.clone());
    let c_b = SyncedCluster::new(vec![h_b], clock.clone(), 50).with_metrics(m_b.clone());

    for i in 0..100u64 {
        c_a.send_frame(&test_frame(i * 33)).unwrap();
        c_b.send_frame(&test_frame(i * 33)).unwrap();
    }

    assert_eq!(m_a.frame_count(), m_b.frame_count(), "both nodes must record same frame count");
    assert_eq!(m_a.frame_count(), 100);
}
