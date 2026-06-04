//! Proves the whole Phase-1 chain: layout (logical) → mapper → HAL (mapped once) →
//! SimulatorDevice (physical). And that serpentine wiring order is honored.

use std::sync::Arc;

use led_core::{DeviceDriver, LogicalFrame, PixelColor, RgbOrder};
use led_hal::{Hal, ProtocolOutput, SimulatorDevice};
use led_layout::{LayoutBuilder, LayoutMapper};

#[test]
fn layout_maps_through_hal_to_device() {
    // 4 strands × 10 px = 40 pixels.
    let mut b = LayoutBuilder::new();
    b.add_mega_tree("mega_tree", 4, 10, 3.0, 0.6, 0.1);
    let layout = b.build();
    assert_eq!(layout.len(), 40);
    assert_eq!(layout.group("mega_tree").len(), 40);

    let compiled = LayoutMapper::compile_single_device(&layout, 1, 0, RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, compiled.device_universes(1));
    let devices: Vec<Arc<dyn DeviceDriver>> = vec![sim.clone()];
    let hal = Hal::new(compiled, devices);

    // Light logical pixel 5; everything else dark.
    let mut pixels = vec![PixelColor::default(); 40];
    pixels[5] = PixelColor::rgb(11, 22, 33);
    hal.send_frame(&LogicalFrame::new(pixels, 0)).unwrap();

    // Pixel 5, RGB order, base universe 0 → channels 15..18 of universe 0.
    assert_eq!(sim.channel(0, 15), Some(11));
    assert_eq!(sim.channel(0, 16), Some(22));
    assert_eq!(sim.channel(0, 17), Some(33));
    // A dark neighbor stays zero.
    assert_eq!(sim.channel(0, 0), Some(0));
}

#[test]
fn matrix_serpentine_id_follows_wiring_not_columns() {
    // 3×2 serpentine. Row 0 → x = 0,1,2 (ids 0,1,2). Row 1 reversed → x = 2,1,0 (ids 3,4,5).
    let mut b = LayoutBuilder::new();
    b.add_matrix("m", 3, 2, true, 1.0);
    let layout = b.build();
    assert_eq!(layout.len(), 6);
    assert_eq!(layout.pixels[0].x, 0.0);
    assert_eq!(layout.pixels[2].x, 2.0);
    assert_eq!(layout.pixels[3].x, 2.0, "row 1 starts at the far side (serpentine fold)");
    assert_eq!(layout.pixels[5].x, 0.0);
}
