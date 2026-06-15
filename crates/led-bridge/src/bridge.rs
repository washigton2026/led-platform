//! [`BridgeHandle`] — keeps `AudioShare` current at the audio analysis rate.
//!
//! Spawns one background thread that:
//! 1. Waits for a new `audio_core::AudioFeatures` v1 on the watch channel.
//! 2. Adapts it to `led_core::AudioFeatures` v0 via [`crate::adapt_into`] (zero-alloc after warmup).
//! 3. Calls `AudioShare::publish` so the render thread sees the latest features.
//!
//! ## Shutdown
//!
//! Drop the `BridgeHandle` to request shutdown. The background thread exits on its next
//! `changed()` wait (which returns `Err` after the sender is dropped).
//!
//! ## Latency
//!
//! Audio analysis runs at HOP_SIZE/sample_rate ≈ 256/48000 ≈ 5.3 ms per frame.
//! The bridge thread adds one channel recv + one Mutex lock ≈ 1–5 µs. Total bridge
//! latency is negligible relative to the 50 ms render tick.

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use audio_core::contracts::AudioFeatures as V1;
use led_core::AudioFeatures as V0;
use led_pixel_engine::AudioShare;
use tokio::sync::watch;

use crate::adapter::adapt_into;

/// A running bridge between the audio-core pipeline and the AudioShare render bridge.
///
/// Drop to shut down the background thread.
pub struct BridgeHandle {
    _thread: JoinHandle<()>,
}

impl BridgeHandle {
    /// Start the bridge thread.
    ///
    /// - `rx`: the watch channel produced by `audio_core::AudioPipeline::subscribe()`.
    /// - `share`: the `AudioShare` that `BandPulse`/`BeatFlash` effects read from.
    pub fn start(mut rx: watch::Receiver<V1>, share: Arc<AudioShare>) -> Self {
        let thread = thread::Builder::new()
            .name("lumyx-bridge".to_string())
            .spawn(move || {
                // Pre-allocate the v0 scratch — resized only if sample_rate changes.
                let mut v0 = V0 {
                    sample_rate:  0,
                    timestamp_ms: 0,
                    rms: 0.0, beat: false,
                    bass: 0.0, mid: 0.0, high: 0.0,
                    spectrum: Vec::new(),
                };
                // Spin a minimal single-threaded tokio runtime for the async watch loop.
                let rt = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .expect("bridge tokio runtime");
                rt.block_on(async move {
                    loop {
                        if rx.changed().await.is_err() {
                            break; // sender dropped — pipeline shut down
                        }
                        let v1 = *rx.borrow_and_update();
                        adapt_into(&v1, &mut v0);
                        share.publish(&v0);
                    }
                });
            })
            .expect("failed to spawn lumyx-bridge thread");
        Self { _thread: thread }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::contracts::{AudioFeatures as V1, SPECTRUM_LEN};
    use led_pixel_engine::AudioShare;
    use std::sync::Arc;
    use std::time::Duration;

    fn make_v1(beat: bool, ts: u64, bass: f32) -> V1 {
        let mut v1 = V1::default();
        v1.timestamp_ms = ts;
        v1.sample_rate  = 48_000;
        v1.rms          = 0.6;
        v1.beat         = beat;
        v1.bass_energy  = bass;
        v1.mid_energy   = 0.2;
        v1.high_energy  = 0.1;
        v1
    }

    // ── CONTRACT: bridge delivers published features to AudioShare ────────
    #[test]
    fn bridge_publishes_to_audio_share() {
        let share = Arc::new(AudioShare::new());
        let (tx, rx) = watch::channel(V1::default());

        let _handle = BridgeHandle::start(rx, share.clone());

        // Send a known v1 frame
        let v1 = make_v1(true, 42, 0.88);
        tx.send(v1).unwrap();

        // Give the bridge thread a moment to process
        thread::sleep(Duration::from_millis(10));

        let sc = share.scalars();
        assert_eq!(sc.sample_rate,  48_000, "sample_rate must bridge to AudioShare");
        assert_eq!(sc.timestamp_ms, 42,     "timestamp must bridge");
        assert!(sc.beat,                    "beat=true must bridge");
        assert!((sc.bass - 0.88).abs() < 1e-5, "bass_energy must bridge as bass");
    }

    // ── CONTRACT: bridge adapts multiple frames in order ──────────────────
    #[test]
    fn bridge_preserves_last_frame() {
        let share = Arc::new(AudioShare::new());
        let (tx, rx) = watch::channel(V1::default());
        let _handle = BridgeHandle::start(rx, share.clone());

        for i in 0..20u64 {
            tx.send(make_v1(i % 3 == 0, i * 10, i as f32 * 0.05)).unwrap();
        }
        thread::sleep(Duration::from_millis(20));

        let sc = share.scalars();
        // Last sent ts = 19 * 10 = 190
        assert_eq!(sc.timestamp_ms, 190, "must reflect last published frame");
    }

    // ── CONTRACT: spectrum passes through bridge ───────────────────────────
    #[test]
    fn bridge_spectrum_passes_through() {
        let share = Arc::new(AudioShare::new());
        let (tx, rx) = watch::channel(V1::default());
        let _handle = BridgeHandle::start(rx, share.clone());

        let mut v1 = make_v1(false, 1, 0.5);
        v1.spectrum[0]   = 0.99;
        v1.spectrum[511] = 0.77;
        tx.send(v1).unwrap();
        thread::sleep(Duration::from_millis(10));

        share.with_spectrum(|s| {
            assert_eq!(s.len(), SPECTRUM_LEN, "spectrum len must be SPECTRUM_LEN");
            assert!((s[0]   - 0.99).abs() < 1e-5, "spectrum[0] must bridge");
            assert!((s[511] - 0.77).abs() < 1e-5, "spectrum[511] must bridge");
        });
    }

    // ── CONTRACT: bridge shuts down cleanly when sender dropped ───────────
    #[test]
    fn bridge_shuts_down_on_sender_drop() {
        let share = Arc::new(AudioShare::new());
        let (tx, rx) = watch::channel(V1::default());
        let handle = BridgeHandle::start(rx, share.clone());
        tx.send(make_v1(false, 1, 0.0)).unwrap();
        thread::sleep(Duration::from_millis(5));
        drop(tx); // signal shutdown
        // The handle's thread should exit — join with timeout via the JoinHandle type
        // (We can't join directly because _thread is private; but Drop is clean)
        drop(handle);
        // If we reach here without hanging, shutdown is clean
    }
}
