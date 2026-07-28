//! # led-readmodel — the read-only snapshot the operator UI polls
//!
//! Aggregates already-confirmed sources into one serialisable view:
//! - [`DeviceStatus`] (`led-core`) — per-controller connectivity/frames,
//! - [`HealthStatus`] (`led-protocols`) — heartbeat health (Ok/Warning/Critical),
//! - [`DiscoveryResult`] (`led-protocols`) — pre-show ArtPoll (responded/missing),
//! - a small [`MetricsView`] filled from `led-hal`'s `MetricsEmitter` by the caller.
//!
//! **Read-only by construction.** Nothing here mutates engine state, sends a command, or
//! touches the render/send hot path — it is exactly the surface ADR-0013/0014 lets the UI
//! consume (the UI reads this; the daemon owns the engine).
//!
//! JSON is **hand-rolled** to match the workspace convention (`MetricsEmitter::snapshot_json`,
//! `Provenance::to_json`); the workspace has no `serde` dependency and this crate stays
//! std-only. Field names are stable — the UI depends on them.

use std::net::Ipv4Addr;

use led_core::DeviceStatus;
use led_protocols::{DiscoveryResult, HealthStatus};

/// A minimal metrics view for the UI. Filled from `led-hal`'s `MetricsEmitter` by whoever
/// assembles the snapshot (kept as plain fields so this crate needs no dependency on the HAL).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricsView {
    pub fps: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub drops: u64,
    pub hb_gap_ms: u64,
}

/// One controller's read-only status, tagged with its stable device id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub id: u16,
    pub status: DeviceStatus,
}

/// The whole read-only snapshot the UI polls. Read-only by construction.
#[derive(Clone, Debug)]
pub struct ReadModel {
    pub devices: Vec<DeviceView>,
    pub health: HealthStatus,
    pub metrics: MetricsView,
    pub discovery: Option<DiscoveryResult>,
}

impl ReadModel {
    /// Serialise to one JSON object (hand-rolled, std-only). Stable field names.
    pub fn to_json(&self) -> String {
        let health = match self.health {
            HealthStatus::Ok => "ok",
            HealthStatus::Warning => "warning",
            HealthStatus::Critical => "critical",
        };
        let devices = self
            .devices
            .iter()
            .map(|d| {
                format!(
                    r#"{{"id":{},"connected":{},"frames_sent":{},"last_send_ms":{}}}"#,
                    d.id, d.status.connected, d.status.frames_sent, d.status.last_send_ms
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let m = &self.metrics;
        let discovery = match &self.discovery {
            None => "null".to_string(),
            Some(d) => format!(
                r#"{{"responded":[{}],"missing":[{}]}}"#,
                ip_list(&d.responded),
                ip_list(&d.missing)
            ),
        };
        format!(
            r#"{{"health":"{health}","devices":[{devices}],"metrics":{{"fps":{},"p50_us":{},"p99_us":{},"drops":{},"hb_gap_ms":{}}},"discovery":{discovery}}}"#,
            m.fps, m.p50_us, m.p99_us, m.drops, m.hb_gap_ms
        )
    }
}

fn ip_list(v: &[Ipv4Addr]) -> String {
    v.iter()
        .map(|ip| format!("\"{ip}\""))
        .collect::<Vec<_>>()
        .join(",")
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: u16, connected: bool, frames: u64) -> DeviceView {
        DeviceView {
            id,
            status: DeviceStatus { connected, frames_sent: frames, last_send_ms: 0 },
        }
    }

    #[test]
    fn empty_snapshot_serialises_with_stable_shape() {
        let rm = ReadModel {
            devices: vec![],
            health: HealthStatus::Ok,
            metrics: MetricsView::default(),
            discovery: None,
        };
        let j = rm.to_json();
        assert!(j.contains(r#""health":"ok""#), "{j}");
        assert!(j.contains(r#""devices":[]"#), "{j}");
        assert!(j.contains(r#""discovery":null"#), "{j}");
        assert!(j.contains(r#""fps":0"#) && j.contains(r#""p99_us":0"#), "{j}");
        assert!(braces_balanced(&j), "unbalanced JSON: {j}");
    }

    #[test]
    fn health_variants_map_to_stable_strings() {
        for (h, want) in [
            (HealthStatus::Ok, "ok"),
            (HealthStatus::Warning, "warning"),
            (HealthStatus::Critical, "critical"),
        ] {
            let rm = ReadModel {
                devices: vec![],
                health: h,
                metrics: MetricsView::default(),
                discovery: None,
            };
            assert!(rm.to_json().contains(&format!(r#""health":"{want}""#)));
        }
    }

    #[test]
    fn devices_metrics_and_discovery_are_serialised() {
        let rm = ReadModel {
            devices: vec![dev(0, true, 42), dev(7, false, 0)],
            health: HealthStatus::Warning,
            metrics: MetricsView { fps: 44, p50_us: 120, p99_us: 4100, drops: 3, hb_gap_ms: 800 },
            discovery: Some(DiscoveryResult {
                responded: vec![Ipv4Addr::new(192, 168, 2, 156)],
                missing: vec![Ipv4Addr::new(192, 168, 2, 157)],
            }),
        };
        let j = rm.to_json();
        assert!(j.contains(r#""id":0,"connected":true,"frames_sent":42"#), "{j}");
        assert!(j.contains(r#""id":7,"connected":false"#), "{j}");
        assert!(j.contains(r#""p99_us":4100"#) && j.contains(r#""drops":3"#), "{j}");
        assert!(j.contains(r#""responded":["192.168.2.156"]"#), "{j}");
        assert!(j.contains(r#""missing":["192.168.2.157"]"#), "{j}");
        assert!(braces_balanced(&j), "unbalanced JSON: {j}");
    }

    /// Cheap structural sanity (the workspace has no serde to parse with): braces/brackets balance.
    fn braces_balanced(s: &str) -> bool {
        let mut curly = 0i32;
        let mut square = 0i32;
        for c in s.chars() {
            match c {
                '{' => curly += 1,
                '}' => curly -= 1,
                '[' => square += 1,
                ']' => square -= 1,
                _ => {}
            }
            if curly < 0 || square < 0 {
                return false;
            }
        }
        curly == 0 && square == 0
    }
}
