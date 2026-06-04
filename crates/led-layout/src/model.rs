//! The logical pixel model. Everything upstream (effects, sequencer, AI) thinks in these
//! coordinates; nothing here knows about universes or channels.

use std::collections::HashMap;

/// A human-readable group name used to target props ("mega_tree", "matrix_a").
pub type GroupId = String;

/// One placed pixel in logical space (metres, right-handed, y up).
#[derive(Clone, Debug, PartialEq)]
pub struct PixelLogical {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub group: GroupId,
    pub label: Option<String>,
}

/// A finished layout: dense pixels (id == index) plus group membership.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub pixels: Vec<PixelLogical>,
    pub groups: HashMap<GroupId, Vec<u32>>,
}

impl Layout {
    pub fn len(&self) -> usize {
        self.pixels.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }
    /// Pixel ids in a group, or an empty slice.
    pub fn group(&self, name: &str) -> &[u32] {
        self.groups.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
}
