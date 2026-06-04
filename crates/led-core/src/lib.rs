//! # led-core — shared seams for the LED platform
//!
//! The thin, stable contracts every other crate depends on (and nothing depends on them
//! in reverse). Mirrors §3 of the `led-strip-platform` master skill:
//!
//! - [`LogicalFrame`] — what the engine hands down (logical space).
//! - [`ProtocolOutput`] — the HAL's upward face.
//! - [`DeviceDriver`] — the HAL's downward face (physical space).
//! - [`CompiledLayout`] — the compiled, apply-once logical→physical mapping. `led-layout`
//!   produces one; the HAL consumes it. This crate owns only the *compiled artifact*, not
//!   the high-level layout model.

pub mod mapping;
pub mod traits;
pub mod types;

pub use mapping::{CompiledLayout, DeviceSpec, UNIVERSE_SIZE};
pub use traits::{DeviceConfig, DeviceDriver, IDevice, ProtocolOutput};
pub use types::{
    AudioFeatures, DeviceId, DeviceStatus, LogicalFrame, OutputError, PixelColor, PixelPhysical,
    RgbOrder, UniverseData,
};
