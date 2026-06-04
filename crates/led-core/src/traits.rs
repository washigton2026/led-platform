//! The two HAL seams + the device lifecycle trait.

use crate::types::{DeviceId, DeviceStatus, LogicalFrame, OutputError, UniverseData};

/// The upward face of the HAL — what the Core Engine talks to, and nothing else.
pub trait ProtocolOutput: Send + Sync {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError>;
    fn universe_count(&self) -> u16;
}

/// The downward face — a single device, in physical space. The frame hot path is
/// `send_physical`; it must be fast and allocation-free.
pub trait DeviceDriver: Send + Sync {
    fn id(&self) -> DeviceId;
    /// Send the universes this device owns. Already mapped; the driver only serializes. No alloc.
    fn send_physical(&self, universes: &[UniverseData]) -> Result<(), OutputError>;
    fn status(&self) -> DeviceStatus;
}

/// Per-device configuration passed on the management plane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceConfig {
    pub name: Option<String>,
    pub priority: Option<u8>,
}

/// Lifecycle / management plane. NEVER called on the frame hot path. Distinct from
/// [`DeviceDriver`] so the HAL can hold drivers without exposing reboot/firmware to the
/// frame loop.
pub trait IDevice: DeviceDriver {
    fn connect(&self) -> Result<(), OutputError>;
    fn disconnect(&self);
    fn configure(&self, cfg: &DeviceConfig) -> Result<(), OutputError>;
    fn reboot(&self) -> Result<(), OutputError>;
    /// Refuse while the device is live in a show (see led-hal/references/firmware.md).
    fn update_firmware(&self, image: &[u8]) -> Result<(), OutputError>;
}
