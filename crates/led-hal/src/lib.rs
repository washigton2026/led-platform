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

pub mod cluster;
pub mod engine;
pub mod hal;
pub mod heartbeat;
pub mod sim;

// Re-export the shared seams so `led_hal::*` and downstream code have one import surface.
pub use led_core::*;

pub use cluster::{ClusteredHal, SharedCluster};
pub use engine::Core;
pub use hal::Hal;
pub use heartbeat::Heartbeat;
pub use sim::SimulatorDevice;
