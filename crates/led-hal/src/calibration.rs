//! Calibração por-output (ADR-0019): gamma + brightness dobrados numa única LUT, aplicada
//! por device na borda de saída.
//!
//! ## Onde isto entra
//!
//! ```text
//! layout.apply(frame, scratch)   // mapeamento — inalterado
//!   └─▶ por device: scratch[range] ─(LUT)─▶ send_physical    ← aqui
//! ```
//!
//! O `scratch` de cada device é contíguo (`CompiledLayout::device_range`), então a correção é
//! um passe linear sobre bytes já mapeados. **Nenhum contrato Frozen é tocado** — nem
//! `CompiledLayout`, nem `PixelPhysical`, nem o `led-core`.
//!
//! ## Por que uma LUT combinada
//!
//! `lut[i] = ((i/255)^gamma * brightness * 255).round()` é pré-computada **no startup**. No hot
//! path resta **uma leitura indexada por canal** — sem `powf`, sem multiplicação em ponto
//! flutuante, sem alocação. A tabela tem 256 bytes e cabe em L1.
//!
//! ## O que isto NÃO é
//!
//! Não é proteção elétrica: `brightness` alto não impede sobrecorrente (isso é a fonte e o ABL
//! do controlador). E não é intensidade de efeito — essa continua em `led-pixel-engine`
//! (`color::scale`), em espaço lógico, e não foi tocada.

use std::collections::HashMap;

use led_core::DeviceId;

/// Tabela de correção de 256 entradas para um output, com gamma e brightness já combinados.
#[derive(Clone)]
pub struct CalibrationLut {
    lut: [u8; 256],
}

impl std::fmt::Debug for CalibrationLut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalibrationLut")
            .field("lut[1]", &self.lut[1])
            .field("lut[128]", &self.lut[128])
            .field("lut[255]", &self.lut[255])
            .finish()
    }
}

impl CalibrationLut {
    /// Constrói a tabela. Valores absurdos são saneados por `clamp` — o validador do profile
    /// (ADR-0018) já os recusa antes, isto é a defesa de quem constrói sem validar.
    ///
    /// `gamma` ≤ 0 e não-finito viram `1.0` (linear); `brightness` é fixado em `0.0..=1.0`.
    pub fn new(gamma: f32, brightness: f32) -> Self {
        let g = if gamma.is_finite() && gamma > 0.0 { gamma } else { 1.0 };
        let b = if brightness.is_finite() { brightness.clamp(0.0, 1.0) } else { 1.0 };
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            let v = (i as f32 / 255.0).powf(g) * b;
            *slot = (v * 255.0 + 0.5) as u8;
        }
        Self { lut }
    }

    /// A tabela identidade — `apply` não muda byte algum. Útil para provar que a presença da
    /// calibração, por si só, não altera a saída.
    pub fn identity() -> Self {
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = i as u8;
        }
        Self { lut }
    }

    /// Corrige um bloco de canais **no lugar**. Hot path: uma leitura indexada por byte,
    /// zero alocação.
    #[inline]
    pub fn apply_in_place(&self, channels: &mut [u8]) {
        for b in channels.iter_mut() {
            *b = self.lut[*b as usize];
        }
    }

    /// O valor corrigido de um canal (para testes e diagnóstico).
    #[inline]
    pub fn map(&self, channel: u8) -> u8 {
        self.lut[channel as usize]
    }
}

/// Calibração por device. Um device ausente do mapa **não é corrigido** — e o custo, nesse
/// caso, é zero: a ramificação acontece por device, nunca por pixel.
#[derive(Clone, Debug, Default)]
pub struct Calibration {
    per_device: HashMap<DeviceId, CalibrationLut>,
}

impl Calibration {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra gamma+brightness para um device. Os valores vêm como `f32` de propósito: o
    /// `led-hal` não depende de `led-hardware-profile` — quem cabla profile→HAL é o app.
    pub fn set(&mut self, device: DeviceId, gamma: f32, brightness: f32) {
        self.per_device.insert(device, CalibrationLut::new(gamma, brightness));
    }

    /// Registra uma LUT já construída.
    pub fn set_lut(&mut self, device: DeviceId, lut: CalibrationLut) {
        self.per_device.insert(device, lut);
    }

    /// A LUT deste device, se houver.
    #[inline]
    pub fn lut(&self, device: DeviceId) -> Option<&CalibrationLut> {
        self.per_device.get(&device)
    }

    /// `true` se nenhum device tem calibração — nesse caso o HAL nem consulta o mapa.
    pub fn is_empty(&self) -> bool {
        self.per_device.is_empty()
    }

    pub fn len(&self) -> usize {
        self.per_device.len()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_lut_changes_nothing() {
        let lut = CalibrationLut::identity();
        let mut bytes: Vec<u8> = (0..=255).collect();
        let before = bytes.clone();
        lut.apply_in_place(&mut bytes);
        assert_eq!(bytes, before, "a identidade não pode alterar byte algum");
    }

    #[test]
    fn gamma_one_brightness_one_is_the_identity() {
        let lut = CalibrationLut::new(1.0, 1.0);
        for i in 0..=255u8 {
            assert_eq!(lut.map(i), i, "gamma=1, brightness=1 deve ser transparente (byte {i})");
        }
    }

    #[test]
    fn gamma_preserves_the_endpoints_and_darkens_the_middle() {
        let lut = CalibrationLut::new(2.2, 1.0);
        assert_eq!(lut.map(0), 0, "preto continua preto");
        assert_eq!(lut.map(255), 255, "branco continua branco");
        assert!(lut.map(128) < 128, "gamma 2.2 escurece o meio-tom");
    }

    #[test]
    fn gamma_is_monotonic() {
        let lut = CalibrationLut::new(2.2, 1.0);
        for i in 1..=255u8 {
            assert!(lut.map(i) >= lut.map(i - 1), "a correção não pode inverter a rampa em {i}");
        }
    }

    #[test]
    fn brightness_scales_the_top_of_the_range() {
        let half = CalibrationLut::new(1.0, 0.5);
        assert_eq!(half.map(0), 0);
        assert_eq!(half.map(255), 128, "255 * 0.5 = 127.5 → 128");
        let off = CalibrationLut::new(1.0, 0.0);
        assert_eq!(off.map(255), 0, "brightness 0 apaga");
    }

    #[test]
    fn absurd_values_are_sanitised_not_panicking() {
        for (g, b) in [(0.0, 1.0), (-3.0, 1.0), (f32::NAN, 1.0), (2.2, 5.0), (2.2, f32::NAN)] {
            let lut = CalibrationLut::new(g, b);
            // Nenhum pânico e a tabela continua utilizável em toda a faixa.
            assert_eq!(lut.map(0), 0);
            let _ = lut.map(255);
        }
        assert_eq!(
            CalibrationLut::new(2.2, 5.0).map(255),
            255,
            "brightness acima de 1 é fixado em 1, não estoura"
        );
    }

    #[test]
    fn a_device_without_calibration_has_no_lut() {
        let mut c = Calibration::new();
        assert!(c.is_empty());
        c.set(1, 2.2, 1.0);
        assert_eq!(c.len(), 1);
        assert!(c.lut(1).is_some());
        assert!(c.lut(2).is_none(), "device não registrado não é corrigido");
    }

    #[test]
    fn apply_in_place_corrects_a_whole_block() {
        let lut = CalibrationLut::new(1.0, 0.5);
        let mut bytes = vec![255u8, 100, 0];
        lut.apply_in_place(&mut bytes);
        assert_eq!(bytes, vec![128, 50, 0]);
    }
}
