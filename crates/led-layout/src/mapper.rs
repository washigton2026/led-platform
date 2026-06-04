//! The LayoutMapper: assigns each logical pixel a physical destination, then compiles to a
//! [`CompiledLayout`] (the apply-once artifact the HAL consumes). RGB order is resolved
//! here — never in an effect.

use led_core::{CompiledLayout, DeviceId, PixelPhysical, RgbOrder, UNIVERSE_SIZE};

use crate::model::Layout;

pub struct LayoutMapper;

impl LayoutMapper {
    /// Assign every pixel to one device, packing 3 channels/pixel across consecutive
    /// universes starting at `base_universe`. Returns assignments indexed by logical id.
    pub fn single_device(
        layout: &Layout,
        device: DeviceId,
        base_universe: u16,
        order: RgbOrder,
    ) -> Vec<PixelPhysical> {
        let mut out = Vec::with_capacity(layout.pixels.len());
        let mut universe = base_universe;
        let mut channel: u16 = 0;
        for _ in &layout.pixels {
            if channel as usize + 3 > UNIVERSE_SIZE {
                universe += 1;
                channel = 0;
            }
            out.push(PixelPhysical { device, universe, channel, order });
            channel += 3;
        }
        out
    }

    /// Convenience: map onto a single device and compile in one step.
    pub fn compile_single_device(
        layout: &Layout,
        device: DeviceId,
        base_universe: u16,
        order: RgbOrder,
    ) -> CompiledLayout {
        let assignments = Self::single_device(layout, device, base_universe, order);
        CompiledLayout::compile(&assignments)
    }
}
