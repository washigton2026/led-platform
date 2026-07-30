//! # led-hal — the Hardware Abstraction Layer
//!
//! The single boundary between the Core Engine and physical devices. Built on the
//! contracts in [`led_core`] (re-exported here for convenience). This crate owns the
//! [`Hal`] facade (sole [`ProtocolOutput`] impl), the [`SimulatorDevice`], the
//! [`Heartbeat`], and a [`Core`] stand-in that demonstrates the boundary.
//!
//! ```text
//! Core ── LogicalFrame ──▶ Hal ──(apply mapping once)──▶ DeviceDriver fan-out ──▶ device
//! ```
//!
//! Proven invariants live in `tests/`: mapping applied once, fan-out by ownership,
//! heartbeat never zeros, zero allocation on the hot path, Core reaches hardware only
//! through `ProtocolOutput`.

pub mod calibration;
pub mod chaos;
pub mod cluster;
pub mod cluster_sync;
pub mod engine;
pub mod hal;
pub mod observability;
pub mod heartbeat;
pub mod metrics;
pub mod net_time;
pub mod network_guard;
pub mod prometheus;
pub mod shared_clock;
pub mod sim;

// Re-export the shared seams so `led_hal::*` and downstream code have one import surface.
pub use led_core::*;

pub use calibration::{Calibration, CalibrationLut};
pub use cluster::{ClusteredHal, ClusterHeartbeat, SharedCluster};
pub use chaos::{ChaosHarness, ChaosResult, FaultConfig, run_experiment};
pub use cluster_sync::{SegmentHealth, SegmentState, SyncedCluster};
pub use engine::Core;
pub use hal::Hal;
pub use heartbeat::{Heartbeat, HeartbeatHandle};
pub use network_guard::{NetworkGuard, NetworkPolicyError, PermissiveGuard, WifiBlockGuard};
pub use metrics::MetricsEmitter;
pub use observability::{
    ActiveSpan, AlertCondition, AlertEngine, AlertRule, AlertSeverity,
    ObservabilityReport, Span, SpanCollector,
};
pub use net_time::{best_of, measure_offset, sync_to, TimeSample, TimeServer};
pub use prometheus::{prometheus_text, serve_metrics, MetricsServer};
pub use shared_clock::{calibrate_offset, SharedClock};
pub use sim::SimulatorDevice;
