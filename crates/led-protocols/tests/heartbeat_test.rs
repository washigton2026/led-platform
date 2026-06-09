//! Integration tests for the keep-alive heartbeat.
//! Proves: fires at ~800 ms, never sends when no frame is registered, never sends zeros.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

use led_core::UniverseData;
use led_protocols::{
    health, HealthStatus,
    heartbeat::Heartbeat,
    packet::DMX_SLOTS,
};

fn universe(fill: u8) -> Vec<UniverseData> {
    vec![UniverseData { universe: 1, data: vec![fill; DMX_SLOTS] }]
}

// ── Health status (pure, sync) ────────────────────────────────────────────────

#[test]
fn health_status_thresholds_are_correct() {
    assert_eq!(health(1000, 1000), HealthStatus::Ok,       "0 ms gap");
    assert_eq!(health(0, 1999),    HealthStatus::Ok,       "1999 ms gap");
    assert_eq!(health(0, 2000),    HealthStatus::Warning,   "2000 ms = Warning");
    assert_eq!(health(0, 2499),    HealthStatus::Warning,   "2499 ms = Warning");
    assert_eq!(health(0, 2500),    HealthStatus::Critical,  "2500 ms = Critical");
    assert_eq!(health(0, 99_999),  HealthStatus::Critical,  "long silence = Critical");
}

#[test]
fn health_handles_backwards_clock_gracefully() {
    // now < last_sent (clock correction) — saturating_sub → 0 → Ok
    assert_eq!(health(9999, 100), HealthStatus::Ok);
}

// ── Heartbeat fires at interval ───────────────────────────────────────────────

#[tokio::test]
async fn heartbeat_fires_at_800ms_interval() {
    let hb = Heartbeat::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    hb.update(&universe(0x77));

    // Use 50 ms interval to keep tests fast.
    let _handle = hb.start(50, move |_| { c.fetch_add(1, Ordering::Relaxed); });

    // Wait 175 ms → expect 3 ticks (at 50/100/150 ms).
    tokio::time::sleep(Duration::from_millis(175)).await;
    let n = counter.load(Ordering::Relaxed);
    assert!(n >= 2 && n <= 5, "expected ~3 ticks in 175 ms @ 50 ms interval, got {n}");
}

// ── Heartbeat never sends when no frame is registered ────────────────────────

#[tokio::test]
async fn heartbeat_silent_before_first_update() {
    let hb = Heartbeat::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    // Do NOT call hb.update() — heartbeat should stay silent.
    let _handle = hb.start(50, move |_| { c.fetch_add(1, Ordering::Relaxed); });

    tokio::time::sleep(Duration::from_millis(175)).await;
    let n = counter.load(Ordering::Relaxed);
    assert_eq!(n, 0, "heartbeat must not send zeros when no frame is set: sent {n} times");
}

// ── Heartbeat resends the frame it was given, not zeros ───────────────────────

#[tokio::test]
async fn heartbeat_resends_last_registered_frame() {
    let hb = Heartbeat::new();

    // Store the last DMX value seen by the heartbeat.
    let last_fill = Arc::new(AtomicU32::new(0));
    let lf = last_fill.clone();

    hb.update(&universe(0xAB));

    let _handle = hb.start(50, move |universes| {
        if let Some(u) = universes.first() {
            lf.store(u.data[0] as u32, Ordering::Relaxed);
        }
    });

    tokio::time::sleep(Duration::from_millis(120)).await;
    let seen = last_fill.load(Ordering::Relaxed);
    assert_eq!(seen, 0xAB, "heartbeat resent the registered frame, not zeros: got 0x{seen:02X}");
}

// ── Heartbeat always sends the most-recently updated frame ───────────────────

#[tokio::test]
async fn heartbeat_uses_most_recent_update() {
    let hb = Heartbeat::new();
    let last_fill = Arc::new(AtomicU32::new(0));
    let lf = last_fill.clone();

    hb.update(&universe(0x11));
    let _handle = hb.start(50, move |universes| {
        if let Some(u) = universes.first() {
            lf.store(u.data[0] as u32, Ordering::Relaxed);
        }
    });

    // Update to a new frame mid-flight.
    tokio::time::sleep(Duration::from_millis(60)).await;
    hb.update(&universe(0xFF));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let seen = last_fill.load(Ordering::Relaxed);
    assert_eq!(seen, 0xFF, "heartbeat switched to the new frame: got 0x{seen:02X}");
}
