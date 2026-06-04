//! The render→send pipeline: a render thread (producer) and a send thread (consumer),
//! decoupled by the lock-free triple buffer. The render thread never blocks on the send
//! thread, and they never share a mutable buffer (master §4).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use led_core::{LogicalFrame, PixelColor, ProtocolOutput};

use crate::effect::{Effect, Vec3};
use crate::triple::triple_buffer;

/// Owns the two running threads; stops and joins them on `stop()` or drop.
pub struct PipelineHandle {
    stop: Arc<AtomicBool>,
    render: Option<JoinHandle<()>>,
    send: Option<JoinHandle<()>>,
}

impl PipelineHandle {
    pub fn stop(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.render.take() {
            let _ = h.join();
        }
        if let Some(h) = self.send.take() {
            let _ = h.join();
        }
    }
}

impl Drop for PipelineHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Spawn a render thread driving `effect` at `fps`, handing frames to `out` via a triple
/// buffer drained by a send thread. `positions` are the logical-space pixel coordinates;
/// `positions.len()` is the pixel count.
pub fn spawn(
    effect: Box<dyn Effect>,
    positions: Vec<Vec3>,
    out: Arc<dyn ProtocolOutput>,
    fps: u32,
) -> PipelineHandle {
    let n = positions.len();
    let blank = || LogicalFrame::new(vec![PixelColor::default(); n], 0);
    let (mut producer, mut consumer) = triple_buffer(blank(), blank(), blank());

    let stop = Arc::new(AtomicBool::new(false));
    let interval = Duration::from_secs_f64(1.0 / fps.max(1) as f64);

    // Render thread (producer): pure, allocation-free into the owned buffer.
    let stop_r = stop.clone();
    let render = std::thread::spawn(move || {
        let start = Instant::now();
        while !stop_r.load(Ordering::Acquire) {
            let t = start.elapsed().as_millis() as u64;
            {
                let frame = producer.input();
                effect.render(t, &positions, &mut frame.pixels);
                frame.timestamp_ms = t;
            }
            producer.publish();
            std::thread::sleep(interval);
        }
    });

    // Send thread (consumer): forwards the latest published frame to the output edge.
    let stop_s = stop.clone();
    let send = std::thread::spawn(move || {
        while !stop_s.load(Ordering::Acquire) {
            if consumer.update() {
                let _ = out.send_frame(consumer.output());
            } else {
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        // Final drain so the last rendered frame reaches the device.
        if consumer.update() {
            let _ = out.send_frame(consumer.output());
        }
    });

    PipelineHandle { stop, render: Some(render), send: Some(send) }
}
