//! `RigBuilder` — instantiate N copies of a prop template (a "rig") with
//! automatic, conflict-free channel assignment.
//!
//! This is the feature the xLights workflow lacks: the user's real project
//! needed 12 hand-written Python scripts to clone one robot into five, and
//! still ended up with 2,701 overlapping channel pairs. Here, overlap-free
//! addressing is a property of the constructor, not a post-hoc check.
//!
//! ## Invariants
//! - **Conflict-free by construction**: strips are packed contiguously; a
//!   channel is owned by exactly one strip. `RigPlan::verify_no_overlap`
//!   proves it (and the tests call it).
//! - **One device per instance** by default (mirrors one controller per
//!   robot); multi-controller instances are a `device_stride` away.
//! - No pixel straddles a universe boundary (170 px per 510-channel universe).

use led_core::{DeviceId, PixelPhysical, RgbOrder};

/// One strip in the template, with its position relative to the rig origin.
#[derive(Clone, Debug)]
pub struct StripTemplate {
    pub name: String,
    pub pixels: u16,
    /// Position of the strip start relative to the rig origin.
    pub offset: (f32, f32, f32),
}

/// A reusable prop assembly (e.g. one LED robot: 86 strips of body parts).
#[derive(Clone, Debug)]
pub struct RigTemplate {
    pub name: String,
    pub strips: Vec<StripTemplate>,
}

impl RigTemplate {
    pub fn total_pixels(&self) -> u32 {
        self.strips.iter().map(|s| s.pixels as u32).sum()
    }
}

/// One placed strip of one instance: where it lives physically and in space.
#[derive(Clone, Debug)]
pub struct PlacedStrip {
    /// `"<template> <instance>/<strip>"`, e.g. `"robô 2/6pack 1"`.
    pub name: String,
    pub device: DeviceId,
    /// First universe this strip touches.
    pub universe: u16,
    /// 0-based channel within that universe.
    pub channel: u16,
    pub pixels: u16,
    pub world: (f32, f32, f32),
}

/// A fully-addressed rig of N instances.
#[derive(Clone, Debug)]
pub struct RigPlan {
    pub strips: Vec<PlacedStrip>,
    pub order: RgbOrder,
    pub px_per_universe: u16,
}

/// Pixels per 510-channel universe (3 channels each, no straddling).
pub const PX_PER_UNIVERSE_510: u16 = 170;

/// Instantiate `instances` copies of `template`.
///
/// - Instance `i` gets device id `first_device + i` (one controller per robot).
/// - Each instance's strips are packed contiguously from `first_universe`,
///   restarting at that universe for every instance (per-device numbering,
///   matching how WLED/xLights number each controller from 1).
/// - `spacing` is added to the rig origin per instance (line the robots up).
pub fn build_rig(
    template: &RigTemplate,
    instances: u16,
    first_device: DeviceId,
    first_universe: u16,
    order: RgbOrder,
    spacing: (f32, f32, f32),
) -> RigPlan {
    let px_per_universe = PX_PER_UNIVERSE_510;
    let mut strips = Vec::with_capacity(template.strips.len() * instances as usize);
    for i in 0..instances {
        let device = first_device + i;
        let origin = (
            spacing.0 * i as f32,
            spacing.1 * i as f32,
            spacing.2 * i as f32,
        );
        let mut px_cursor: u32 = 0; // pixel index within this instance
        for s in &template.strips {
            let universe = first_universe + (px_cursor / px_per_universe as u32) as u16;
            let channel = ((px_cursor % px_per_universe as u32) * 3) as u16;
            strips.push(PlacedStrip {
                name: format!("{} {}/{}", template.name, i + 1, s.name),
                device,
                universe,
                channel,
                pixels: s.pixels,
                world: (
                    origin.0 + s.offset.0,
                    origin.1 + s.offset.1,
                    origin.2 + s.offset.2,
                ),
            });
            px_cursor += s.pixels as u32;
        }
    }
    RigPlan { strips, order, px_per_universe }
}

impl RigPlan {
    /// Physical assignments, one per pixel, ready for `CompiledLayout::compile`.
    pub fn assignments(&self) -> Vec<PixelPhysical> {
        let ppu = self.px_per_universe as u32;
        let mut out = Vec::new();
        for s in &self.strips {
            // Global pixel index of the strip start within its device.
            let start_px =
                (s.universe as u32).saturating_sub(self.first_universe_of(s.device)) * ppu
                    + s.channel as u32 / 3;
            for p in 0..s.pixels as u32 {
                let px = start_px + p;
                out.push(PixelPhysical {
                    device: s.device,
                    universe: self.first_universe_of(s.device) as u16 + (px / ppu) as u16,
                    channel: ((px % ppu) * 3) as u16,
                    format: self.order.into(),
                });
            }
        }
        out
    }

    fn first_universe_of(&self, device: DeviceId) -> u32 {
        self.strips
            .iter()
            .filter(|s| s.device == device)
            .map(|s| s.universe as u32)
            .min()
            .unwrap_or(0)
    }

    /// Total pixels across all instances.
    pub fn total_pixels(&self) -> u32 {
        self.strips.iter().map(|s| s.pixels as u32).sum()
    }

    /// Universes needed per instance (ceiling of pixels / 170).
    pub fn universes_per_instance(&self, template_pixels: u32) -> u16 {
        template_pixels.div_ceil(self.px_per_universe as u32) as u16
    }

    /// Prove the invariant: no two strips overlap any (device, universe, channel).
    /// Returns the first overlapping pair, or `None` when clean.
    pub fn verify_no_overlap(&self) -> Option<(String, String)> {
        // (device, absolute channel range) per strip
        let mut ranges: Vec<(DeviceId, u32, u32, &str)> = self
            .strips
            .iter()
            .map(|s| {
                let base = self.first_universe_of(s.device);
                let abs = ((s.universe as u32 - base) * self.px_per_universe as u32 * 3)
                    + s.channel as u32;
                (s.device, abs, abs + s.pixels as u32 * 3, s.name.as_str())
            })
            .collect();
        ranges.sort();
        for w in ranges.windows(2) {
            let (da, _sa, ea, na) = w[0];
            let (db, sb, _eb, nb) = w[1];
            if da == db && sb < ea {
                return Some((na.to_string(), nb.to_string()));
            }
        }
        None
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A miniature of the user's robot: 4 body-part strips, 70 px total.
    fn mini_robot() -> RigTemplate {
        RigTemplate {
            name: "robô".into(),
            strips: vec![
                StripTemplate { name: "peito".into(), pixels: 20, offset: (0.0, 10.0, 0.0) },
                StripTemplate { name: "6pack".into(), pixels: 20, offset: (0.0, 5.0, 0.0) },
                StripTemplate { name: "perna E".into(), pixels: 15, offset: (-2.0, 0.0, 0.0) },
                StripTemplate { name: "perna D".into(), pixels: 15, offset: (2.0, 0.0, 0.0) },
            ],
        }
    }

    #[test]
    fn five_robots_no_overlap_by_construction() {
        let plan = build_rig(&mini_robot(), 5, 0, 1, RgbOrder::Grb, (600.0, 0.0, 0.0));
        assert_eq!(plan.strips.len(), 20, "4 strips × 5 instances");
        assert_eq!(plan.total_pixels(), 350);
        assert!(plan.verify_no_overlap().is_none(), "conflict-free by construction");
    }

    #[test]
    fn each_instance_gets_its_own_device() {
        let plan = build_rig(&mini_robot(), 5, 10, 1, RgbOrder::Rgb, (0.0, 0.0, 0.0));
        for (i, chunk) in plan.strips.chunks(4).enumerate() {
            assert!(chunk.iter().all(|s| s.device == 10 + i as u16),
                "instance {i} on device {}", 10 + i);
        }
    }

    #[test]
    fn strips_pack_contiguously_within_an_instance() {
        let plan = build_rig(&mini_robot(), 1, 0, 1, RgbOrder::Rgb, (0.0, 0.0, 0.0));
        // peito: px 0..20 → u1 ch0. 6pack: px 20..40 → u1 ch60. perna E: px 40 → ch120.
        assert_eq!((plan.strips[0].universe, plan.strips[0].channel), (1, 0));
        assert_eq!((plan.strips[1].universe, plan.strips[1].channel), (1, 60));
        assert_eq!((plan.strips[2].universe, plan.strips[2].channel), (1, 120));
        assert_eq!((plan.strips[3].universe, plan.strips[3].channel), (1, 165));
    }

    #[test]
    fn universe_rolls_over_at_170_pixels() {
        let big = RigTemplate {
            name: "torre".into(),
            strips: vec![
                StripTemplate { name: "a".into(), pixels: 170, offset: (0.0, 0.0, 0.0) },
                StripTemplate { name: "b".into(), pixels: 10, offset: (0.0, 1.0, 0.0) },
            ],
        };
        let plan = build_rig(&big, 1, 0, 1, RgbOrder::Rgb, (0.0, 0.0, 0.0));
        assert_eq!(plan.strips[0].universe, 1);
        assert_eq!((plan.strips[1].universe, plan.strips[1].channel), (2, 0),
            "pixel 170 starts universe 2");
    }

    #[test]
    fn world_positions_offset_per_instance() {
        let plan = build_rig(&mini_robot(), 3, 0, 1, RgbOrder::Rgb, (600.0, 0.0, 0.0));
        assert_eq!(plan.strips[0].world, (0.0, 10.0, 0.0), "robot 1 peito");
        assert_eq!(plan.strips[4].world, (600.0, 10.0, 0.0), "robot 2 peito");
        assert_eq!(plan.strips[8].world, (1200.0, 10.0, 0.0), "robot 3 peito");
    }

    #[test]
    fn assignments_compile_into_a_layout() {
        let plan = build_rig(&mini_robot(), 2, 0, 1, RgbOrder::Grb, (600.0, 0.0, 0.0));
        let assigns = plan.assignments();
        assert_eq!(assigns.len(), 140, "70 px × 2 instances");
        let layout = led_core::CompiledLayout::compile(&assigns);
        assert_eq!(layout.device_universes(0).len(), 1, "70px fits one universe");
        assert_eq!(layout.device_universes(1).len(), 1);
        // no pixel straddles: every channel is ≤ 507
        assert!(assigns.iter().all(|a| a.channel <= 507));
    }

    #[test]
    fn user_scale_rig_86_strips_5_robots() {
        // The real robot: 86 strips × ~20px ≈ 1,240 px per robot.
        let strips: Vec<StripTemplate> = (0..86)
            .map(|i| StripTemplate {
                name: format!("strip {i}"),
                pixels: 20,
                offset: (0.0, i as f32, 0.0),
            })
            .collect();
        let template = RigTemplate { name: "robô".into(), strips };
        assert_eq!(template.total_pixels(), 1720);

        let plan = build_rig(&template, 5, 0, 1, RgbOrder::Grb, (600.0, 0.0, 0.0));
        assert_eq!(plan.total_pixels(), 8600);
        assert!(plan.verify_no_overlap().is_none(),
            "the layout that took 12 Python scripts is one call, conflict-free");
        assert_eq!(plan.universes_per_instance(1720), 11, "1720px → 11 universes");
    }
}
