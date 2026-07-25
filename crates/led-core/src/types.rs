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

/// How the white channel of an RGBW strip is derived from a logical RGB [`PixelColor`].
///
/// Logical space stays RGB (there is no white in [`PixelColor`]); the white is computed
/// here, at the one L↔P point (the mapper). See ADR-0011.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WhiteMode {
    /// White channel is always 0 — the strip's white LED is unused; colour comes from RGB.
    None,
    /// White = `min(r, g, b)` — the neutral component common to all three channels, moved
    /// to the dedicated white LED. The RGB bytes are left unchanged (simple, non-destructive).
    Min,
}

impl WhiteMode {
    /// The white byte for this logical colour.
    #[inline]
    pub fn white(self, c: PixelColor) -> u8 {
        match self {
            WhiteMode::None => 0,
            WhiteMode::Min => c.r.min(c.g).min(c.b),
        }
    }
}

/// How a logical [`PixelColor`] is written to the wire for one strip: the channel count,
/// the RGB ordering, and (for RGBW) how the white channel is derived. Resolved once, at
/// mapping time — never in an effect (the L↔P boundary). See ADR-0011.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorFormat {
    /// 3-channel RGB in the given wire order (WS2812B, most sACN/Art-Net fixtures).
    Rgb(RgbOrder),
    /// 4-channel RGBW: the RGB triple in the given wire order, then a white channel derived
    /// from the logical colour by [`WhiteMode`] (SK6812-RGBW, TM1814).
    Rgbw(RgbOrder, WhiteMode),
}

impl ColorFormat {
    /// Number of wire channels this format writes per pixel (3 for RGB, 4 for RGBW).
    #[inline]
    pub fn channels(self) -> usize {
        match self {
            ColorFormat::Rgb(_) => 3,
            ColorFormat::Rgbw(_, _) => 4,
        }
    }

    /// Write this pixel's wire bytes into `out`, which must be at least [`channels`](Self::channels)
    /// bytes long. Allocation-free.
    #[inline]
    pub fn write(self, c: PixelColor, out: &mut [u8]) {
        match self {
            ColorFormat::Rgb(order) => {
                let b = order.bytes(c);
                out[0] = b[0];
                out[1] = b[1];
                out[2] = b[2];
            }
            ColorFormat::Rgbw(order, wm) => {
                let b = order.bytes(c);
                out[0] = b[0];
                out[1] = b[1];
                out[2] = b[2];
                out[3] = wm.white(c);
            }
        }
    }
}

impl From<RgbOrder> for ColorFormat {
    #[inline]
    fn from(o: RgbOrder) -> Self {
        ColorFormat::Rgb(o)
    }
}

/// One frame in **logical space**: colors indexed by logical pixel id. The ONLY thing the
/// engine hands to the HAL.
///
/// The optional `provenance` field records the causal chain from audio input to this frame,
/// enabling end-to-end audit, replay verification, and debug. `None` is acceptable only in
/// tests and the simulator — production frames must carry provenance.
#[derive(Clone, Debug)]
pub struct LogicalFrame {
    pub pixels:       Vec<PixelColor>,
    pub timestamp_ms: u64,
    /// Causal chain record. `None` = simulator / test only.
    pub provenance:   Option<crate::Provenance>,
}

impl LogicalFrame {
    pub fn new(pixels: Vec<PixelColor>, timestamp_ms: u64) -> Self {
        Self { pixels, timestamp_ms, provenance: None }
    }

    /// Create a frame with provenance attached.
    pub fn with_provenance(
        pixels:      Vec<PixelColor>,
        timestamp_ms: u64,
        provenance:  crate::Provenance,
    ) -> Self {
        Self { pixels, timestamp_ms, provenance: Some(provenance) }
    }
}

/// One pixel's physical destination — the output of the LayoutMapper, indexed by logical id.
///
/// `format` carries the channel count + wire ordering (RGB or RGBW). See ADR-0011: this
/// replaced the former `order: RgbOrder` field so 4-channel strips have a home without
/// touching any Frozen seam signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelPhysical {
    pub device: DeviceId,
    pub universe: u16,
    pub channel: u16, // starting channel within the universe (0-based)
    pub format: ColorFormat,
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

/// Heuristic musical section label — set by the `SectionDetector` after warm-up.
/// Mirrors `audio_core::contracts::MusicalSection`; duplicated here so `led-core`
/// stays dependency-free (leaf crate invariant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicalSection {
    Intro,
    Verse,
    Chorus,
    Bridge,
    Drop,
    Build,
    Outro,
    Unknown,
}

/// Heuristic instrument / timbral class — mirrors `audio_core::instrument::InstrumentClass`.
/// Duplicated here so `led-core` stays dependency-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstrumentClass {
    Kick, Snare, HiHat, Bass, Melody, Chord, Noise, Silence, Unknown,
}

/// What the audio layer hands to anyone. `sample_rate` travels WITH the data — no global
/// rate is ever assumed (master §3 seam).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AudioFeatures {
    pub sample_rate: u32,
    pub timestamp_ms: u64,
    pub rms: f32,
    pub beat: bool,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub spectrum: Vec<f32>,
    /// Heuristic section label from the `SectionDetector`.
    /// `None` during the warm-up window (~2.5 s); `Some(...)` afterwards.
    pub musical_section: Option<MusicalSection>,
    /// Heuristic instrument / timbral class for this frame.
    /// `None` when `InstrumentClassifier` is not active.
    pub instrument_class: Option<InstrumentClass>,
}

/// A cheap health snapshot a driver exposes (read off the hot path).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceStatus {
    pub connected: bool,
    pub frames_sent: u64,
    pub last_send_ms: u64,
}
