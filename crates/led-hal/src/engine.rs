//! A stand-in for the Core Engine, here only to *demonstrate the boundary*: it holds a
//! `Arc<dyn ProtocolOutput>` and nothing else. There is no field, method, or path from a
//! `Core` to a [`led_core::DeviceDriver`], a socket, or a controller IP. Swapping the HAL
//! for a fake `ProtocolOutput` in a test requires touching nothing here — that is the point.

use std::sync::Arc;

use led_core::{LogicalFrame, OutputError, ProtocolOutput};

pub struct Core {
    out: Arc<dyn ProtocolOutput>,
}

impl Core {
    pub fn new(out: Arc<dyn ProtocolOutput>) -> Self {
        Self { out }
    }

    /// In the real engine, this renders effects into a `LogicalFrame` first. The slice just
    /// forwards an externally-built frame to the output edge.
    pub fn render_and_send(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        self.out.send_frame(frame)
    }

    pub fn universe_count(&self) -> u16 {
        self.out.universe_count()
    }
}
