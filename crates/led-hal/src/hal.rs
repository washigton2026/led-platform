//! The HAL facade — the sole implementor of [`ProtocolOutput`].
//!
//! Owns the compiled mapping, the device drivers, and the pre-allocated scratch. Per frame
//! it applies the mapping **once**, then hands each device only the universes it owns.

use std::sync::{Arc, Mutex};

use led_core::{CompiledLayout, DeviceDriver, LogicalFrame, OutputError, ProtocolOutput, UniverseData};

pub struct Hal {
    layout: CompiledLayout,
    devices: Vec<Arc<dyn DeviceDriver>>,
    scratch: Mutex<Vec<UniverseData>>, // pre-sized once; reused every frame
}

impl Hal {
    pub fn new(layout: CompiledLayout, devices: Vec<Arc<dyn DeviceDriver>>) -> Self {
        let scratch = layout.make_scratch();
        Self { layout, devices, scratch: Mutex::new(scratch) }
    }

    pub fn layout(&self) -> &CompiledLayout {
        &self.layout
    }
}

impl ProtocolOutput for Hal {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        let mut scratch = self.scratch.lock().expect("scratch poisoned");

        // (1) Apply the ONE mapping, exactly once, into pre-allocated scratch.
        self.layout.apply(frame, &mut scratch);

        // (2) Fan out: each device receives only the universes it owns. No re-mapping.
        for dev in &self.devices {
            let range = self
                .layout
                .device_range(dev.id())
                .ok_or(OutputError::DeviceNotConnected(dev.id()))?;
            dev.send_physical(&scratch[range])?;
        }
        Ok(())
    }

    fn universe_count(&self) -> u16 {
        self.layout.universe_count()
    }
}
