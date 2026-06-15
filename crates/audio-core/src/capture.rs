//! CPAL audio capture: opens an input device, downmixes to mono, and pushes samples into a
//! [`RingBuffer`]. The CPAL callback runs on the platform's realtime audio thread, so it
//! must not allocate or block: downmixing writes into a small fixed-size stack buffer and
//! flushes it to the ring buffer in chunks.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::ring_buffer::RingBuffer;

/// Stack buffer size used while downmixing multi-channel frames before flushing to the
/// ring buffer.
const DOWNMIX_CHUNK: usize = 256;

#[derive(Debug)]
pub enum AudioCoreError {
    NoInputDevice,
    NoSupportedConfig(String),
    UnsupportedSampleFormat(SampleFormat),
    BuildStream(String),
    PlayStream(String),
}

impl std::fmt::Display for AudioCoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioCoreError::NoInputDevice => write!(f, "no default audio input device"),
            AudioCoreError::NoSupportedConfig(e) => write!(f, "no supported input config: {e}"),
            AudioCoreError::UnsupportedSampleFormat(fmt) => write!(f, "unsupported input sample format: {fmt:?}"),
            AudioCoreError::BuildStream(e) => write!(f, "failed to build input stream: {e}"),
            AudioCoreError::PlayStream(e) => write!(f, "failed to start input stream: {e}"),
        }
    }
}

impl std::error::Error for AudioCoreError {}

/// A live CPAL input stream feeding a [`RingBuffer`]. Drop to stop capturing.
pub struct CaptureStream {
    // Held only to keep the stream alive (and stop it on Drop) — never read directly.
    _stream: Stream,
    sample_rate: u32,
}

impl CaptureStream {
    /// The device's actual sample rate (read from its config — invariant 7: never
    /// hardcoded).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Open the default input device and start streaming mono-downmixed samples into `ring`.
pub fn start_default_input(ring: Arc<RingBuffer>) -> Result<CaptureStream, AudioCoreError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(AudioCoreError::NoInputDevice)?;
    let config = device.default_input_config().map_err(|e| AudioCoreError::NoSupportedConfig(e.to_string()))?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let err_fn = |e: cpal::StreamError| eprintln!("audio-core: input stream error: {e}");

    let stream = match sample_format {
        SampleFormat::F32 => {
            let ring = ring.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| push_downmixed_f32(&ring, data, channels),
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let ring = ring.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| push_downmixed_i16(&ring, data, channels),
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let ring = ring.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| push_downmixed_u16(&ring, data, channels),
                err_fn,
                None,
            )
        }
        other => return Err(AudioCoreError::UnsupportedSampleFormat(other)),
    }
    .map_err(|e| AudioCoreError::BuildStream(e.to_string()))?;

    stream.play().map_err(|e| AudioCoreError::PlayStream(e.to_string()))?;

    Ok(CaptureStream { _stream: stream, sample_rate })
}

/// Downmix interleaved `f32` frames to mono and push to `ring`, flushing in
/// [`DOWNMIX_CHUNK`]-sized batches (no per-sample allocation, no growth).
fn push_downmixed_f32(ring: &RingBuffer, data: &[f32], channels: usize) {
    let ch = channels.max(1);
    let mut chunk = [0.0f32; DOWNMIX_CHUNK];
    let mut filled = 0;
    for frame in data.chunks_exact(ch) {
        chunk[filled] = frame.iter().sum::<f32>() / ch as f32;
        filled += 1;
        if filled == DOWNMIX_CHUNK {
            ring.push_slice(&chunk);
            filled = 0;
        }
    }
    if filled > 0 {
        ring.push_slice(&chunk[..filled]);
    }
}

/// `i16` samples are full-scale at `i16::MAX`, centered at zero.
fn push_downmixed_i16(ring: &RingBuffer, data: &[i16], channels: usize) {
    let ch = channels.max(1);
    let mut chunk = [0.0f32; DOWNMIX_CHUNK];
    let mut filled = 0;
    for frame in data.chunks_exact(ch) {
        let sum: f32 = frame.iter().map(|&s| s as f32 / i16::MAX as f32).sum();
        chunk[filled] = sum / ch as f32;
        filled += 1;
        if filled == DOWNMIX_CHUNK {
            ring.push_slice(&chunk);
            filled = 0;
        }
    }
    if filled > 0 {
        ring.push_slice(&chunk[..filled]);
    }
}

/// `u16` samples are unsigned, centered at `32768`.
fn push_downmixed_u16(ring: &RingBuffer, data: &[u16], channels: usize) {
    let ch = channels.max(1);
    let mut chunk = [0.0f32; DOWNMIX_CHUNK];
    let mut filled = 0;
    for frame in data.chunks_exact(ch) {
        let sum: f32 = frame.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum();
        chunk[filled] = sum / ch as f32;
        filled += 1;
        if filled == DOWNMIX_CHUNK {
            ring.push_slice(&chunk);
            filled = 0;
        }
    }
    if filled > 0 {
        ring.push_slice(&chunk[..filled]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_f32_averages_channels() {
        let ring = RingBuffer::new(16);
        // 2 channels, 3 frames: (1,3) (2,4) (0,0) -> mono (2,3,0)
        push_downmixed_f32(&ring, &[1.0, 3.0, 2.0, 4.0, 0.0, 0.0], 2);
        let mut out = [0.0f32; 3];
        assert!(ring.pop_exact(&mut out));
        assert_eq!(out, [2.0, 3.0, 0.0]);
    }

    #[test]
    fn downmix_i16_normalizes_to_unit_range() {
        let ring = RingBuffer::new(16);
        push_downmixed_i16(&ring, &[i16::MAX, i16::MIN], 1);
        let mut out = [0.0f32; 2];
        assert!(ring.pop_exact(&mut out));
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert!((out[1] - (-1.0)).abs() < 1e-3);
    }

    #[test]
    fn downmix_u16_centers_at_zero() {
        let ring = RingBuffer::new(16);
        push_downmixed_u16(&ring, &[32768u16, 0, 65535], 1);
        let mut out = [0.0f32; 3];
        assert!(ring.pop_exact(&mut out));
        assert!((out[0] - 0.0).abs() < 1e-6);
        assert!((out[1] - (-1.0)).abs() < 1e-6);
        assert!((out[2] - 1.0).abs() < 1e-3);
    }
}

// ─── Mock capture source ──────────────────────────────────────────────────────

/// A hardware-free audio source that feeds a pre-built sample slice into a
/// [`RingBuffer`], using the same downmix and chunking code as the real CPAL
/// path. Use in tests to exercise the full capture→ring→analyzer pipeline
/// without requiring an audio device.
///
/// ```
/// use audio_core::{RingBuffer, MockCaptureSource};
/// use std::sync::Arc;
///
/// let ring = Arc::new(RingBuffer::new(8192));
/// let mock = MockCaptureSource::new(44_100, vec![0.0; 4096]);
/// mock.play(ring);
/// ```
pub struct MockCaptureSource {
    pub sample_rate: u32,
    /// Mono F32 samples to feed. Multi-channel: provide interleaved, set `channels`.
    pub samples: Vec<f32>,
    /// Channel count for the samples slice (default 1 = already mono).
    pub channels: usize,
}

impl MockCaptureSource {
    /// Create a mono mock source.
    pub fn new(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self { sample_rate, samples, channels: 1 }
    }

    /// Create a multi-channel mock source (samples are interleaved).
    pub fn stereo(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self { sample_rate, samples, channels: 2 }
    }

    /// Push all samples into `ring` synchronously, exactly as the CPAL callback would.
    /// The ring is filled from position 0; call once per simulated "capture burst".
    pub fn play(&self, ring: Arc<RingBuffer>) {
        push_downmixed_f32(&ring, &self.samples, self.channels);
    }

    /// Push samples and then drain via `Analyzer::process_hop` until the ring is empty.
    /// Returns a list of all `AudioFeatures` produced (one per hop).
    pub fn analyze_all(&self) -> Vec<crate::contracts::AudioFeatures> {
        use crate::{Analyzer, contracts::HOP_SIZE, ring_buffer::RingBuffer};
        // RingBuffer requires a power-of-two capacity.
        let raw = self.samples.len() + HOP_SIZE * 4;
        let cap = raw.next_power_of_two();
        let ring = Arc::new(RingBuffer::new(cap));
        push_downmixed_f32(&ring, &self.samples, self.channels);

        let mut analyzer = Analyzer::new(self.sample_rate);
        let mut results  = Vec::new();
        let mut hop_buf  = [0.0f32; HOP_SIZE];
        let mut ts       = 0u64;
        let hop_ms       = (HOP_SIZE as u64 * 1_000) / self.sample_rate as u64;

        while ring.pop_exact(&mut hop_buf) {
            results.push(analyzer.process_hop(&hop_buf, ts));
            ts += hop_ms;
        }
        results
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use crate::contracts::{AudioFeatures, HOP_SIZE, SPECTRUM_LEN};
    use std::f32::consts::TAU;

    // ── CONTRACT: MockCaptureSource feeds samples into ring ───────────────
    #[test]
    fn mock_play_fills_ring() {
        let ring = Arc::new(crate::ring_buffer::RingBuffer::new(1024));
        let samples = vec![0.5f32; 512];
        MockCaptureSource::new(44_100, samples).play(ring.clone());
        let mut out = [0.0f32; 256];
        assert!(ring.pop_exact(&mut out), "ring must have samples after play()");
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6),
            "all samples must be 0.5");
    }

    // ── CONTRACT: analyze_all produces AudioFeatures with correct sample_rate ─
    #[test]
    fn mock_analyze_all_sample_rate_explicit() {
        // 2 seconds of silence at 44100 Hz → should produce ~2s/5.3ms ≈ 376 hops
        let samples = vec![0.0f32; 44_100 * 2];
        let results = MockCaptureSource::new(44_100, samples).analyze_all();
        assert!(!results.is_empty(), "must produce at least 1 AudioFeatures");
        for f in &results {
            assert_eq!(f.sample_rate, 44_100,
                "sample_rate must be explicit (invariant 7), not hardcoded");
        }
    }

    // ── CONTRACT: silence produces zero RMS ───────────────────────────────
    #[test]
    fn mock_silence_produces_zero_rms() {
        let samples = vec![0.0f32; 44_100]; // 1s of silence
        let results = MockCaptureSource::new(44_100, samples).analyze_all();
        for f in &results[1..] { // skip first hop (warmup)
            assert!((f.rms - 0.0).abs() < 1e-6,
                "silence must produce rms=0, got {}", f.rms);
        }
    }

    // ── CONTRACT: bass tone produces bass energy ───────────────────────────
    #[test]
    fn mock_bass_tone_produces_bass_energy() {
        // 100 Hz sine at 48kHz — falls in bass band (20–250 Hz)
        let sr = 48_000u32;
        let hz = 100.0f32;
        let samples: Vec<f32> = (0..sr as usize * 2) // 2 seconds
            .map(|i| (TAU * hz * i as f32 / sr as f32).sin() * 0.8)
            .collect();
        let results = MockCaptureSource::new(sr, samples).analyze_all();
        // After warmup, bass energy must be non-zero
        let avg_bass = results[10..].iter().map(|f| f.bass_energy).sum::<f32>()
            / (results.len() - 10) as f32;
        assert!(avg_bass > 0.01,
            "100Hz sine must produce bass energy (avg={avg_bass:.4})");
    }

    // ── CONTRACT: beat impulse detected ───────────────────────────────────
    #[test]
    fn mock_beat_impulses_detected() {
        let sr = 48_000u32;
        let hop = HOP_SIZE;
        let hop_ms = (hop as u64 * 1_000) / sr as u64;
        // Generate: 500ms silence, then alternating click/silence every 500ms
        let mut samples = vec![0.0f32; sr as usize]; // 1s silence
        // Add 4 clicks: broadband impulses spaced ~500ms apart
        for k in 0..4 {
            let offset = sr as usize + k * (sr as usize / 2);
            if offset + hop < samples.len() + sr as usize * 3 {
                // extend if needed
            }
        }
        // Simpler: just put impulses in the buffer directly
        let total = sr as usize * 4; // 4 seconds
        let mut s = vec![0.0f32; total];
        let interval = sr as usize / 2; // every 500ms
        for k in 0..8 {
            let start = sr as usize + k * interval; // start after 1s warmup
            for i in start..(start + hop).min(total) {
                s[i] = 0.9; // broadband click
            }
        }
        let results = MockCaptureSource::new(sr, s).analyze_all();
        let total_beats: u32 = results.iter().map(|f| f.beat as u32).sum();
        assert!(total_beats >= 2,
            "must detect ≥2 beat clicks in 4s (got {total_beats})");
    }

    // ── CONTRACT: stereo source is downmixed to mono ──────────────────────
    #[test]
    fn mock_stereo_downmixed_to_mono() {
        // Stereo: left=1.0, right=-1.0 → mono average = 0.0
        let interleaved: Vec<f32> = (0..512).flat_map(|_| [1.0f32, -1.0f32]).collect();
        let ring = Arc::new(crate::ring_buffer::RingBuffer::new(1024));
        MockCaptureSource::stereo(48_000, interleaved).play(ring.clone());
        let mut out = [0.0f32; 256];
        if ring.pop_exact(&mut out) {
            for s in out {
                assert!((s - 0.0).abs() < 1e-6,
                    "stereo (1, -1) must downmix to mono 0.0, got {s}");
            }
        }
    }

    // ── CONTRACT: spectrum len invariant preserved through mock ──────────
    #[test]
    fn mock_spectrum_len_invariant() {
        let samples: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let results = MockCaptureSource::new(48_000, samples).analyze_all();
        for f in &results {
            assert_eq!(f.spectrum.len(), SPECTRUM_LEN,
                "spectrum len must be SPECTRUM_LEN={SPECTRUM_LEN} through mock");
        }
    }

    // ── STRESS: 60 seconds of synthetic audio via mock ───────────────────
    #[test]
    fn mock_60s_stress_no_panic() {
        let sr = 48_000u32;
        let samples: Vec<f32> = (0..sr as usize * 60)
            .map(|i| (i as f32 * 0.01).sin() * 0.3)
            .collect();
        let results = MockCaptureSource::new(sr, samples).analyze_all();
        assert!(results.len() > 11_000, "60s @ 48kHz must yield >11000 hops");
    }
}

#[cfg(test)]
mod mock_adversarial_tests {
    use super::*;
    use crate::{RingBuffer, contracts::{HOP_SIZE, FFT_SIZE, SPECTRUM_LEN}};
    use std::sync::Arc;
    use std::f32::consts::TAU;

    fn sine_samples(hz: f32, sr: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * hz * i as f32 / sr as f32).sin() * 0.5).collect()
    }

    // ── CONTRACT: MockCaptureSource fills ring with correct sample count ──
    #[test]
    fn mock_fills_ring_correct_sample_count() {
        let samples = vec![0.5f32; 1024];
        let ring = Arc::new(RingBuffer::new(4096));
        let mock = MockCaptureSource::new(48_000, samples.clone());
        mock.play(ring.clone());
        // Read out what was pushed
        let mut hop = [0.0f32; HOP_SIZE];
        let mut total = 0usize;
        while ring.pop_exact(&mut hop) { total += HOP_SIZE; }
        assert!(total >= HOP_SIZE, "ring must have at least one hop's worth of samples");
    }

    // ── CONTRACT: stereo downmix averages channels ─────────────────────────
    #[test]
    fn mock_stereo_downmix_to_mono() {
        // Stereo: L=1.0, R=0.0 → mono=0.5
        let stereo: Vec<f32> = (0..512).flat_map(|_| [1.0f32, 0.0f32]).collect();
        let ring = Arc::new(RingBuffer::new(4096));
        let mock = MockCaptureSource::stereo(48_000, stereo);
        mock.play(ring.clone());
        let mut hop = [0.0f32; HOP_SIZE];
        ring.pop_exact(&mut hop);
        for s in &hop {
            assert!((s - 0.5).abs() < 1e-5, "stereo (1,0) must downmix to 0.5, got {s}");
        }
    }

    // ── CONTRACT: analyze_all returns AudioFeatures with correct sample_rate ─
    #[test]
    fn mock_analyze_all_sample_rate_correct() {
        let samples = sine_samples(440.0, 48_000, FFT_SIZE * 4);
        let mock = MockCaptureSource::new(48_000, samples);
        let results = mock.analyze_all();
        assert!(!results.is_empty(), "analyze_all must produce at least one AudioFeatures");
        for af in &results {
            assert_eq!(af.sample_rate, 48_000, "sample_rate must be 48000 in every frame");
        }
    }

    // ── CONTRACT: analyze_all timestamps are monotonically increasing ──────
    #[test]
    fn mock_analyze_all_timestamps_monotone() {
        let samples = sine_samples(220.0, 48_000, HOP_SIZE * 20);
        let mock = MockCaptureSource::new(48_000, samples);
        let results = mock.analyze_all();
        let mut prev = 0u64;
        for af in &results {
            assert!(af.timestamp_ms >= prev,
                "timestamp must not go backwards: {} < {}", af.timestamp_ms, prev);
            prev = af.timestamp_ms;
        }
    }

    // ── CONTRACT: bass tone → bass_energy is the dominant band ───────────
    #[test]
    fn mock_bass_tone_dominant_in_bass_band() {
        // 100 Hz = bass (20-250 Hz)
        let samples = sine_samples(100.0, 48_000, FFT_SIZE * 8);
        let mock = MockCaptureSource::new(48_000, samples);
        let results = mock.analyze_all();
        // Skip first few hops (window warmup), then check steady state
        let steady: Vec<_> = results.iter().skip(4).collect();
        assert!(!steady.is_empty());
        let avg_bass: f32 = steady.iter().map(|a| a.bass_energy).sum::<f32>() / steady.len() as f32;
        let avg_high: f32 = steady.iter().map(|a| a.high_energy).sum::<f32>() / steady.len() as f32;
        assert!(avg_bass > avg_high,
            "100Hz tone: bass_energy ({avg_bass:.4}) must exceed high_energy ({avg_high:.4})");
    }

    // ── CONTRACT: silence → RMS ≈ 0 ──────────────────────────────────────
    #[test]
    fn mock_silence_gives_near_zero_rms() {
        let samples = vec![0.0f32; HOP_SIZE * 10];
        let mock = MockCaptureSource::new(48_000, samples);
        let results = mock.analyze_all();
        for af in &results {
            assert!(af.rms < 1e-6, "silence must produce near-zero RMS (got {})", af.rms);
        }
    }

    // ── FUZZ: empty samples → analyze_all returns empty vec ──────────────
    #[test]
    fn mock_empty_samples_analyze_all_empty() {
        let mock = MockCaptureSource::new(48_000, vec![]);
        let results = mock.analyze_all();
        assert!(results.is_empty(), "empty input must produce no AudioFeatures");
    }

    // ── FUZZ: NaN samples → no panic ─────────────────────────────────────
    #[test]
    fn mock_nan_samples_no_panic() {
        let samples = vec![f32::NAN; HOP_SIZE * 2];
        let ring = Arc::new(RingBuffer::new(HOP_SIZE * 8));
        let mock = MockCaptureSource::new(48_000, samples);
        mock.play(ring); // must not panic
    }

    // ── STRESS: 10 seconds of synthetic audio via MockCaptureSource ───────
    #[test]
    fn mock_10s_stress_no_panic() {
        let sr = 48_000u32;
        let n = sr as usize * 10; // 10 seconds
        let samples = sine_samples(440.0, sr, n);
        let mock = MockCaptureSource::new(sr, samples);
        let results = mock.analyze_all();
        assert!(results.len() > 1_000, "10s at 48kHz must produce >1000 hops (got {})", results.len());
    }

    // ── REAL-TIME: analyze_all must process 1s of audio in < 1s wall-clock ─
    #[test]
    fn mock_analyze_all_realtime_speed() {
        use std::time::Instant;
        let sr = 48_000u32;
        let samples = sine_samples(440.0, sr, sr as usize); // 1s
        let mock = MockCaptureSource::new(sr, samples);
        let t0 = Instant::now();
        let results = mock.analyze_all();
        let elapsed = t0.elapsed();
        assert!(!results.is_empty());
        assert!(elapsed.as_secs_f64() < 1.0,
            "1s audio must analyze faster than real-time (took {:.3}s)", elapsed.as_secs_f64());
    }
}
