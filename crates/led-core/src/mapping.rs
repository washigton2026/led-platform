//! The compiled logical→physical mapping.
//!
//! [`CompiledLayout::compile`] takes per-pixel assignments (the LayoutMapper's output) and
//! builds a flat index table. Universes are grouped so each device owns a **contiguous**
//! block of scratch — the HAL fan-out is then a slice, not a gather. Per frame,
//! [`CompiledLayout::apply`] is an allocation-free scatter.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::{ColorFormat, DeviceId, LogicalFrame, PixelPhysical, RgbOrder, UniverseData};

/// DMX-style universe size in channels.
pub const UNIVERSE_SIZE: usize = 512;

struct Target {
    uni_index: usize,
    byte: usize,
    format: ColorFormat,
}

/// How many universes a device owns (used by the [`CompiledLayout::linear`] convenience).
#[derive(Clone, Copy, Debug)]
pub struct DeviceSpec {
    pub id: DeviceId,
    pub universes: u16,
}

/// The compiled, ready-to-apply mapping.
pub struct CompiledLayout {
    targets: Vec<Target>,                          // indexed by logical pixel id
    universes: Vec<u16>,                           // uni_index → universe number
    device_ranges: Vec<(DeviceId, Range<usize>)>,  // device → contiguous uni_index range
    apply_count: AtomicU64,                         // instrumentation: proves "applied once"
}

impl CompiledLayout {
    /// Compile per-pixel assignments (LayoutMapper output) into the apply-once artifact.
    ///
    /// Devices are ordered first-seen; within a device, universes are ordered first-seen.
    /// Each device thus owns a contiguous `uni_index` range in scratch.
    pub fn compile(assignments: &[PixelPhysical]) -> Self {
        // Per device, the ordered set of distinct universes it uses.
        let mut per_device: Vec<(DeviceId, Vec<u16>)> = Vec::new();
        for a in assignments {
            let di = match per_device.iter().position(|(id, _)| *id == a.device) {
                Some(i) => i,
                None => {
                    per_device.push((a.device, Vec::new()));
                    per_device.len() - 1
                }
            };
            if !per_device[di].1.contains(&a.universe) {
                per_device[di].1.push(a.universe);
            }
        }

        let mut universes = Vec::new();
        let mut device_ranges = Vec::new();
        let mut index_of: HashMap<(DeviceId, u16), usize> = HashMap::new();
        for (dev, unis) in &per_device {
            let start = universes.len();
            for &u in unis {
                index_of.insert((*dev, u), universes.len());
                universes.push(u);
            }
            device_ranges.push((*dev, start..universes.len()));
        }

        let mut targets = Vec::with_capacity(assignments.len());
        for a in assignments {
            let uni_index = index_of[&(a.device, a.universe)];
            targets.push(Target { uni_index, byte: a.channel as usize, format: a.format });
        }

        Self { targets, universes, device_ranges, apply_count: AtomicU64::new(0) }
    }

    /// Convenience: lay `pixel_count` RGB pixels out contiguously (3 ch/pixel) across
    /// devices, each owning a contiguous block of universes numbered globally from 0.
    ///
    /// This is the RGB-only convenience; its signature is **Frozen** (ADR-0007). For RGBW
    /// or other channel counts, use [`linear_format`](Self::linear_format).
    pub fn linear(pixel_count: usize, devices: &[DeviceSpec], order: RgbOrder) -> Self {
        Self::linear_format(pixel_count, devices, ColorFormat::Rgb(order))
    }

    /// Like [`linear`](Self::linear) but with an explicit [`ColorFormat`], so RGBW/4-channel
    /// strips lay out correctly — `format.channels()` bytes per pixel instead of a hardcoded 3.
    /// Added in `led-core` 1.3.0 (ADR-0011); additive — changes no Frozen signature.
    pub fn linear_format(pixel_count: usize, devices: &[DeviceSpec], format: ColorFormat) -> Self {
        let stride = format.channels();
        let mut slots: Vec<(DeviceId, u16)> = Vec::new(); // (device, universe number)
        let mut uni_no: u16 = 0;
        for d in devices {
            for _ in 0..d.universes {
                slots.push((d.id, uni_no));
                uni_no += 1;
            }
        }

        let mut assignments = Vec::with_capacity(pixel_count);
        let mut slot = 0usize;
        let mut byte: u16 = 0;
        for _ in 0..pixel_count {
            if byte as usize + stride > UNIVERSE_SIZE {
                slot += 1;
                byte = 0;
            }
            assert!(slot < slots.len(), "layout: pixels exceed universe capacity");
            let (device, universe) = slots[slot];
            assignments.push(PixelPhysical { device, universe, channel: byte, format });
            byte += stride as u16;
        }
        Self::compile(&assignments)
    }

    /// Allocate the per-universe scratch buffers ONCE, at startup. The hot path reuses these.
    pub fn make_scratch(&self) -> Vec<UniverseData> {
        self.universes
            .iter()
            .map(|&u| UniverseData { universe: u, data: vec![0u8; UNIVERSE_SIZE] })
            .collect()
    }

    /// Apply the mapping for one frame into pre-allocated scratch. Allocation-free.
    pub fn apply(&self, frame: &LogicalFrame, scratch: &mut [UniverseData]) {
        self.apply_count.fetch_add(1, Ordering::Relaxed);
        for (id, px) in frame.pixels.iter().enumerate() {
            if let Some(t) = self.targets.get(id) {
                let chan = &mut scratch[t.uni_index].data;
                let n = t.format.channels();
                t.format.write(*px, &mut chan[t.byte..t.byte + n]);
            }
        }
    }

    pub fn universe_count(&self) -> u16 {
        self.universes.len() as u16
    }

    /// The uni_index range a device owns (for fan-out slicing of scratch).
    pub fn device_range(&self, id: DeviceId) -> Option<Range<usize>> {
        self.device_ranges.iter().find(|(d, _)| *d == id).map(|(_, r)| r.clone())
    }

    /// The universe numbers a device owns (for constructing a matching device buffer).
    pub fn device_universes(&self, id: DeviceId) -> &[u16] {
        let r = self.device_range(id).expect("unknown device id");
        &self.universes[r]
    }

    /// All devices referenced by this layout, in compile order.
    pub fn devices(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.device_ranges.iter().map(|(d, _)| *d)
    }

    /// How many times [`apply`](Self::apply) has run — tests use this to prove "applied once".
    pub fn apply_count(&self) -> u64 {
        self.apply_count.load(Ordering::Relaxed)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ColorFormat, PixelColor, RgbOrder, WhiteMode};

    fn frame(colors: &[PixelColor]) -> LogicalFrame {
        LogicalFrame::new(colors.to_vec(), 0)
    }

    #[test]
    fn rgb_layout_writes_three_bytes_per_pixel_unchanged() {
        // Pre-ADR-0011 behaviour must stay byte-identical: GRB = [g, r, b], 3 bytes/pixel.
        let layout = CompiledLayout::linear(2, &[DeviceSpec { id: 0, universes: 1 }], RgbOrder::Grb);
        let mut scratch = layout.make_scratch();
        let f = frame(&[PixelColor::rgb(10, 20, 30), PixelColor::rgb(40, 50, 60)]);
        layout.apply(&f, &mut scratch);
        assert_eq!(&scratch[0].data[0..6], &[20, 10, 30, 50, 40, 60]);
    }

    #[test]
    fn rgbw_layout_writes_four_bytes_per_pixel_with_derived_white() {
        // SK6812-style: GRB colour order + W = min(r, g, b), 4 bytes/pixel.
        let fmt = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        assert_eq!(fmt.channels(), 4, "RGBW must lay out 4 channels/pixel");
        let layout =
            CompiledLayout::linear_format(2, &[DeviceSpec { id: 0, universes: 1 }], fmt);
        let mut scratch = layout.make_scratch();
        // px0 (10,20,30) → GRB [20,10,30] + W=min=10 ; px1 (40,80,60) → GRB [80,40,60] + W=min=40
        let f = frame(&[PixelColor::rgb(10, 20, 30), PixelColor::rgb(40, 80, 60)]);
        layout.apply(&f, &mut scratch);
        assert_eq!(&scratch[0].data[0..8], &[20, 10, 30, 10, 80, 40, 60, 40]);
    }

    #[test]
    fn white_mode_none_zeros_the_white_channel() {
        let fmt = ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::None);
        let layout =
            CompiledLayout::linear_format(1, &[DeviceSpec { id: 0, universes: 1 }], fmt);
        let mut scratch = layout.make_scratch();
        layout.apply(&frame(&[PixelColor::rgb(10, 20, 30)]), &mut scratch);
        assert_eq!(&scratch[0].data[0..4], &[10, 20, 30, 0]);
    }

    #[test]
    fn linear_is_rgb_of_linear_format() {
        // linear() must be exactly linear_format(ColorFormat::Rgb(order)) — Frozen signature,
        // unchanged semantics.
        let a = CompiledLayout::linear(5, &[DeviceSpec { id: 1, universes: 1 }], RgbOrder::Bgr);
        let b = CompiledLayout::linear_format(
            5,
            &[DeviceSpec { id: 1, universes: 1 }],
            ColorFormat::Rgb(RgbOrder::Bgr),
        );
        assert_eq!(a.universe_count(), b.universe_count());
        let mut sa = a.make_scratch();
        let mut sb = b.make_scratch();
        let f = frame(&[PixelColor::rgb(1, 2, 3); 5]);
        a.apply(&f, &mut sa);
        b.apply(&f, &mut sb);
        assert_eq!(sa[0].data, sb[0].data, "linear must equal linear_format(Rgb)");
    }
}
