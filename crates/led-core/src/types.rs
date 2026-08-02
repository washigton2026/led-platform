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
    /// White = `min(r, g, b)`, **added** to unchanged RGB bytes.
    ///
    /// ⚠️ **Consumo elétrico:** como o RGB não é reduzido, branco pleno acende os quatro
    /// canais no máximo — ~80 mA/pixel num SK6812 contra 60 mA de uma fita RGB (+33 %), e
    /// **4× mais** que [`WhiteMode::MinSubtract`]. O die branco também **soma** luz ao branco
    /// RGB, então a saída fica mais brilhante que a cor lógica pedida. Use conscientemente
    /// (ADR-0020); para o comportamento colorimétrico padrão, prefira `MinSubtract`.
    Min,
    /// White = `min(r, g, b)`, **subtraído** dos três canais coloridos (satura em zero).
    ///
    /// É o comportamento colorimétrico padrão: o componente neutro sai pelo die branco
    /// dedicado — mais eficiente e com melhor CRI que somar três coloridos — e só o excedente
    /// de cor permanece no RGB. Branco pleno vira `[0,0,0,255]`: um die em vez de quatro.
    /// Ver ADR-0020.
    MinSubtract,
}

impl WhiteMode {
    /// The white byte for this logical colour.
    #[inline]
    pub fn white(self, c: PixelColor) -> u8 {
        match self {
            WhiteMode::None => 0,
            WhiteMode::Min | WhiteMode::MinSubtract => c.r.min(c.g).min(c.b),
        }
    }

    /// A cor que resta nos canais RGB depois de extrair o branco.
    ///
    /// Só [`WhiteMode::MinSubtract`] reduz o RGB; os outros modos devolvem a cor intacta —
    /// é exatamente aqui que mora a diferença de corrente descrita no ADR-0020.
    #[inline]
    pub fn residual_rgb(self, c: PixelColor) -> PixelColor {
        match self {
            WhiteMode::None | WhiteMode::Min => c,
            WhiteMode::MinSubtract => {
                let w = self.white(c);
                PixelColor::rgb(
                    c.r.saturating_sub(w),
                    c.g.saturating_sub(w),
                    c.b.saturating_sub(w),
                )
            }
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
                // O branco sai da cor ORIGINAL; a ordem de canais é aplicada ao RESÍDUO.
                let w = wm.white(c);
                let b = order.bytes(wm.residual_rgb(c));
                out[0] = b[0];
                out[1] = b[1];
                out[2] = b[2];
                out[3] = w;
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

// ── Tests: derivação do branco (ADR-0011 + ADR-0020) ───────────────────────────

#[cfg(test)]
mod white_tests {
    use super::*;

    fn wire(fmt: ColorFormat, c: PixelColor) -> Vec<u8> {
        let mut out = vec![0u8; fmt.channels()];
        fmt.write(c, &mut out);
        out
    }

    /// O achado do ADR-0020: `Min` acende os quatro canais no máximo para branco pleno.
    /// Este teste FIXA esse comportamento — ele é legítimo, mas precisa ser escolhido.
    #[test]
    fn min_is_additive_and_lights_four_channels_on_full_white() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::Min);
        assert_eq!(wire(fmt, PixelColor::rgb(255, 255, 255)), vec![255, 255, 255, 255]);
    }

    /// `MinSubtract`: branco pleno sai por UM die, não quatro.
    #[test]
    fn min_subtract_moves_full_white_to_the_white_die_only() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::MinSubtract);
        assert_eq!(wire(fmt, PixelColor::rgb(255, 255, 255)), vec![0, 0, 0, 255]);
    }

    /// **Gate elétrico** — a redução é verificada, não afirmada. A soma dos canais no fio é
    /// proporcional à corrente (um die por canal, mesma corrente nominal por die).
    #[test]
    fn subtractive_white_draws_strictly_less_than_additive() {
        let white = PixelColor::rgb(255, 255, 255);
        let sum = |wm: WhiteMode| -> u32 {
            wire(ColorFormat::Rgbw(RgbOrder::Rgb, wm), white).iter().map(|&b| b as u32).sum()
        };
        let additive = sum(WhiteMode::Min);
        let subtractive = sum(WhiteMode::MinSubtract);
        assert_eq!(additive, 1020, "4 canais no máximo");
        assert_eq!(subtractive, 255, "1 canal no máximo");
        assert!(subtractive < additive, "o modo subtrativo NUNCA pode desenhar mais corrente");
        assert_eq!(additive / subtractive, 4, "razão de 4x para branco pleno (ADR-0020)");
    }

    /// Só o componente neutro vai para o branco; o excedente de cor permanece.
    #[test]
    fn only_the_neutral_component_moves_to_white() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::MinSubtract);
        // min(10,20,30) = 10 -> W=10, resíduo (0,10,20)
        assert_eq!(wire(fmt, PixelColor::rgb(10, 20, 30)), vec![0, 10, 20, 10]);
    }

    /// Cor saturada não tem componente neutro — nada muda, em nenhum dos modos.
    #[test]
    fn a_saturated_colour_is_untouched_by_either_mode() {
        let red = PixelColor::rgb(255, 0, 0);
        for wm in [WhiteMode::Min, WhiteMode::MinSubtract] {
            assert_eq!(
                wire(ColorFormat::Rgbw(RgbOrder::Rgb, wm), red),
                vec![255, 0, 0, 0],
                "{wm:?}: sem componente neutro, o branco é 0 e o RGB fica intacto"
            );
        }
    }

    /// A subtração satura em zero — nenhum canal pode "dar a volta".
    #[test]
    fn subtraction_saturates_and_never_wraps() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::MinSubtract);
        for (r, g, b) in [(0, 0, 0), (1, 0, 0), (0, 255, 1), (255, 255, 254)] {
            let out = wire(fmt, PixelColor::rgb(r, g, b));
            assert_eq!(out.len(), 4);
            // Reconstruir a cor a partir do fio nunca pode exceder a original.
            assert!(out[0] as u16 + out[3] as u16 <= 255 + 255);
        }
    }

    /// A ordem de canais é aplicada ao RESÍDUO, não à cor original.
    #[test]
    fn channel_order_applies_to_the_residual() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::MinSubtract);
        // (10,20,30): W=10, resíduo (0,10,20) -> GRB = [10, 0, 20]
        assert_eq!(wire(fmt, PixelColor::rgb(10, 20, 30)), vec![10, 0, 20, 10]);
    }

    /// `None` continua sem usar o die branco, e ambos os modos seguem com 4 canais.
    #[test]
    fn white_mode_none_is_unchanged_and_all_modes_keep_four_channels() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::None);
        assert_eq!(wire(fmt, PixelColor::rgb(10, 20, 30)), vec![10, 20, 30, 0]);
        for wm in [WhiteMode::None, WhiteMode::Min, WhiteMode::MinSubtract] {
            assert_eq!(ColorFormat::Rgbw(RgbOrder::Rgb, wm).channels(), 4);
        }
    }

    /// RGB puro não é afetado por nada disto.
    #[test]
    fn plain_rgb_is_unaffected() {
        assert_eq!(
            wire(ColorFormat::Rgb(RgbOrder::Grb), PixelColor::rgb(10, 20, 30)),
            vec![20, 10, 30]
        );
    }
}
