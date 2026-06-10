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
