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

pub mod contract_version;
pub mod mapping;
pub mod provenance;
pub mod traits;
pub mod types;

pub use mapping::{CompiledLayout, DeviceSpec, UNIVERSE_SIZE};
pub use contract_version::{
    certified_contracts, ContractRecord, ContractStability,
    LED_CORE_CONTRACT_VERSION, HAL_CONTRACT_VERSION,
    LOGICAL_FRAME_VERSION, AUDIO_FEATURES_V0_VERSION,
    PROVENANCE_VERSION, MUSICAL_SECTION_VERSION,
};
pub use provenance::{compute_pixel_hash, FrameSource, Provenance};
pub use traits::{DeviceConfig, DeviceDriver, IDevice, ProtocolOutput};
pub use types::{
    AudioFeatures, ColorFormat, DeviceId, DeviceStatus, InstrumentClass, LogicalFrame,
    MusicalSection, OutputError, PixelColor, PixelPhysical, RgbOrder, UniverseData, WhiteMode,
};
