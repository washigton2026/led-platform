//! Wires CPAL capture -> [`RingBuffer`] -> [`Analyzer`] -> [`tokio::sync::watch`].
//!
//! [`AudioPipeline::start_default_input`] opens the default input device and spawns a
//! dedicated analysis thread that pops [`HOP_SIZE`]-sample hops and publishes
//! [`AudioFeatures`] on a watch channel. Dropping the returned [`AudioPipeline`] stops both
//! the CPAL stream and the analysis thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::analyzer::Analyzer;
use crate::capture::{self, AudioCoreError, CaptureStream};
use crate::contracts::{AudioFeatures, HOP_SIZE};
use crate::ring_buffer::RingBuffer;

/// Ring buffer capacity in samples (power of two), comfortably larger than one hop so the
/// analysis thread tolerates normal scheduling jitter without dropping samples.
const RING_CAPACITY: usize = 1 << 14; // 16384 samples

/// Analysis thread poll interval while waiting for a full hop.
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Owns the CPAL capture stream and the background analysis thread. Drop to stop both.
pub struct AudioPipeline {
    _capture: CaptureStream,
    running: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl AudioPipeline {
    /// Start capturing from the default input device.
    ///
    /// Returns the pipeline (keep it alive for as long as you want to capture) and a
    /// [`watch::Receiver`] that always holds the latest [`AudioFeatures`] — `Default`
    /// (silent, `sample_rate: 0`) until the first full analysis window has been processed.
    pub fn start_default_input() -> Result<(Self, watch::Receiver<AudioFeatures>), AudioCoreError> {
        let ring = Arc::new(RingBuffer::new(RING_CAPACITY));
        let capture = capture::start_default_input(ring.clone())?;
        let sample_rate = capture.sample_rate();

        let (tx, rx) = watch::channel(AudioFeatures::default());
        let running = Arc::new(AtomicBool::new(true));

        let worker = {
            let running = running.clone();
            thread::spawn(move || {
                let mut analyzer = Analyzer::new(sample_rate);
                let mut hop = [0.0f32; HOP_SIZE];
                let start = Instant::now();
                while running.load(Ordering::Relaxed) {
                    if ring.pop_exact(&mut hop) {
                        let timestamp_ms = start.elapsed().as_millis() as u64;
                        let features = analyzer.process_hop(&hop, timestamp_ms);
                        if tx.send(features).is_err() {
                            break; // no receivers left
                        }
                    } else {
                        thread::sleep(POLL_INTERVAL);
                    }
                }
            })
        };

        Ok((Self { _capture: capture, running, worker: Some(worker) }, rx))
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
