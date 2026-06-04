//! Prop generators. They produce placed pixels in logical space, in **wiring order** (so
//! `id` follows the strand). No universe/channel knowledge here. See
//! `~/led-strip-platform-skill/led-layout/references/geometry.md` for the math.

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::model::{Layout, PixelLogical};

/// Builds a [`Layout`] from one or more props, assigning dense ids in add order.
#[derive(Default)]
pub struct LayoutBuilder {
    pixels: Vec<PixelLogical>,
    groups: HashMap<String, Vec<u32>>,
}

impl LayoutBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, x: f32, y: f32, z: f32, group: &str) -> u32 {
        let id = self.pixels.len() as u32;
        self.pixels.push(PixelLogical { id, x, y, z, group: group.to_string(), label: None });
        self.groups.entry(group.to_string()).or_default().push(id);
        id
    }

    /// A MegaTree: `strips` vertical strands on a cone, each `per_strip` pixels bottom→top.
    /// Vertical-strip wiring order (strand 0 fully, then strand 1, …).
    pub fn add_mega_tree(
        &mut self,
        group: &str,
        strips: u32,
        per_strip: u32,
        height: f32,
        bottom_r: f32,
        top_r: f32,
    ) -> &mut Self {
        for s in 0..strips {
            let theta = (s as f32 / strips as f32) * TAU;
            for p in 0..per_strip {
                let t = if per_strip > 1 { p as f32 / (per_strip - 1) as f32 } else { 0.0 };
                let r = bottom_r + (top_r - bottom_r) * t; // cone taper
                self.push(r * theta.cos(), t * height, r * theta.sin(), group);
            }
        }
        self
    }

    /// A `w`×`h` matrix at `pitch` metres. With `serpentine`, every other row runs in
    /// reverse — `id` follows the physical fold (wiring order), not the visual column.
    pub fn add_matrix(
        &mut self,
        group: &str,
        w: u32,
        h: u32,
        serpentine: bool,
        pitch: f32,
    ) -> &mut Self {
        for row in 0..h {
            for col in 0..w {
                let c = if serpentine && row % 2 == 1 { w - 1 - col } else { col };
                self.push(c as f32 * pitch, row as f32 * pitch, 0.0, group);
            }
        }
        self
    }

    pub fn build(self) -> Layout {
        Layout { pixels: self.pixels, groups: self.groups }
    }
}
