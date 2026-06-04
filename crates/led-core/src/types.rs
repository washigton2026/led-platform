//! Shared value types (the seam payloads).

/// A device's stable identifier within the HAL.
pub type DeviceId = u16;

/// 8-bit RGB color in **logical space** — no chip RGB order baked in.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PixelColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl PixelColor {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Per-strip channel order. Resolved once, at mapping time — never in an effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RgbOrder {
    Rgb,
    Grb, // the common WS281x order
    Bgr,
}

impl RgbOrder {
    /// Reorder a logical color into the three wire bytes for this strip.
    #[inline]
    pub fn bytes(self, c: PixelColor) -> [u8; 3] {
        match self {
            RgbOrder::Rgb => [c.r, c.g, c.b],
            RgbOrder::Grb => [c.g, c.r, c.b],
            RgbOrder::Bgr => [c.b, c.g, c.r],
        }
    }
}

/// One frame in **logical space**: colors indexed by logical pixel id. The ONLY thing the
/// engine hands to the HAL.
#[derive(Clone, Debug)]
pub struct LogicalFrame {
    pub pixels: Vec<PixelColor>,
    pub timestamp_ms: u64,
}

impl LogicalFrame {
    pub fn new(pixels: Vec<PixelColor>, timestamp_ms: u64) -> Self {
        Self { pixels, timestamp_ms }
    }
}

/// One pixel's physical destination — the output of the LayoutMapper, indexed by logical id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelPhysical {
    pub device: DeviceId,
    pub universe: u16,
    pub channel: u16, // starting channel within the universe (0-based)
    pub order: RgbOrder,
}

/// One universe's worth of channel bytes, in **physical space**. Sized once; reused.
#[derive(Clone, Debug)]
pub struct UniverseData {
    pub universe: u16,
    pub data: Vec<u8>,
}

/// Errors surfaced upward from the output edge.
#[derive(Debug, PartialEq, Eq)]
pub enum OutputError {
    /// A device referenced by the layout is not present in the HAL.
    DeviceNotConnected(DeviceId),
    /// A driver's transport failed (e.g. a socket send error), with a short reason.
    Transport(String),
}

/// What the audio layer hands to anyone. `sample_rate` travels WITH the data — no global
/// rate is ever assumed (master §3 seam).
#[derive(Clone, Debug, PartialEq)]
pub struct AudioFeatures {
    pub sample_rate: u32,
    pub timestamp_ms: u64,
    pub rms: f32,
    pub beat: bool,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub spectrum: Vec<f32>,
}

/// A cheap health snapshot a driver exposes (read off the hot path).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceStatus {
    pub connected: bool,
    pub frames_sent: u64,
    pub last_send_ms: u64,
}
