//! # led-show-recorder — deterministic show export and replay
//!
//! Records a running show (sequence of [`LogicalFrame`]s and associated
//! [`AudioFeatures`]) to a compact binary file. The file can be played back
//! offline to produce a byte-for-byte identical output — enabling:
//!
//! - **Regression tests**: compare hashes of replay output before/after a code change.
//! - **Postmortem debugging**: replay a captured show to reproduce a visual artifact.
//! - **Offline rendering**: generate `show.gif` or similar without live audio.
//!
//! ## File format (`.lumyx` — custom, std-only, no dependencies)
//!
//! ```text
//! Header (16 bytes):
//!   [0..4]   magic      "LUMX"
//!   [4..8]   version    u32 LE = 1
//!   [8..12]  pixel_count u32 LE  (pixels per LogicalFrame)
//!   [12..16] frame_count u32 LE
//!
//! For each frame:
//!   [0..8]   timestamp_ms  u64 LE
//!   [3*N]    pixel data     N × (r, g, b) bytes
//!   [4]      has_audio      0x00 | 0x01
//!   if has_audio == 0x01:
//!     [4]    sample_rate    u32 LE
//!     [4]    rms            f32 LE
//!     [1]    beat           0x00 | 0x01
//!     [4]    bpm            f32 LE
//!     [4]    bass_energy    f32 LE
//!     [4]    mid_energy     f32 LE
//!     [4]    high_energy    f32 LE
//! ```
//!
//! ## Invariants (lumyx-regression-guardian)
//! - `pixel_count` is fixed per file (set at write time, validated on read).
//! - `frame_count` in the header is updated when the file is finalised.
//! - Replay produces **identical** `LogicalFrame` values — no lossy conversion.
//! - Audio fields are written/read as little-endian bytes (no `f32` NaN/Inf accepted).
//! - The magic bytes `"LUMX"` detect truncated or wrong-type files early.

use std::io::{self, Read, Write};

pub mod drone_bridge;
pub mod replay;
pub mod signing;

pub use drone_bridge::{
    DroneBridge, DroneAnnotation, DroneFormationHint, SectionEvent,
};
pub use replay::{cross_node_verify, record_and_manifest, verify_replay, ReplayManifest};

use led_core::{LogicalFrame, PixelColor};

// ── Format constants ──────────────────────────────────────────────────────────

const MAGIC: &[u8; 4] = b"LUMX";
const VERSION: u32 = 1;
#[allow(dead_code)] const HEADER_LEN: usize = 16;

// ── AudioSnapshot ─────────────────────────────────────────────────────────────

/// Minimal audio state captured per frame (avoids depending on `audio-core`).
/// Mirrors the fields most useful for replay and regression testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSnapshot {
    pub sample_rate:  u32,
    pub rms:          f32,
    pub beat:         bool,
    pub bpm:          f32,
    pub bass_energy:  f32,
    pub mid_energy:   f32,
    pub high_energy:  f32,
}

impl AudioSnapshot {
    /// Size of the serialised form (all fields present).
    #[allow(dead_code)] const SERIAL_LEN: usize = 4 + 4 + 1 + 4 + 4 + 4 + 4; // = 25 bytes

    fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&self.sample_rate.to_le_bytes())?;
        w.write_all(&self.rms.to_le_bytes())?;
        w.write_all(&[self.beat as u8])?;
        w.write_all(&self.bpm.to_le_bytes())?;
        w.write_all(&self.bass_energy.to_le_bytes())?;
        w.write_all(&self.mid_energy.to_le_bytes())?;
        w.write_all(&self.high_energy.to_le_bytes())?;
        Ok(())
    }

    fn read_from<R: Read>(r: &mut R) -> io::Result<Self> {
        let sample_rate = read_u32_le(r)?;
        let rms         = read_f32_le(r)?;
        let mut beat_buf = [0u8; 1];
        r.read_exact(&mut beat_buf)?;
        let beat = beat_buf[0] != 0;
        let bpm         = read_f32_le(r)?;
        let bass_energy = read_f32_le(r)?;
        let mid_energy  = read_f32_le(r)?;
        let high_energy = read_f32_le(r)?;
        Ok(Self { sample_rate, rms, beat, bpm, bass_energy, mid_energy, high_energy })
    }
}

// ── ShowRecord (one frame in the file) ────────────────────────────────────────

/// One recorded frame: pixel data + optional audio snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ShowRecord {
    pub timestamp_ms: u64,
    pub pixels:       Vec<PixelColor>,
    pub audio:        Option<AudioSnapshot>,
}

// ── ShowWriter ────────────────────────────────────────────────────────────────

/// Streams `ShowRecord`s to a [`Write`] sink (file, cursor, …).
///
/// Call [`finalise`] to update the `frame_count` header field. For
/// [`io::Cursor`] sinks you can seek back; for non-seekable sinks the
/// header `frame_count` stays 0 (the reader ignores it and reads until EOF).
///
/// [`finalise`]: ShowWriter::finalise
pub struct ShowWriter<W: Write> {
    inner:       W,
    pixel_count: u32,
    frame_count: u32,
    header_written: bool,
}

impl<W: Write> ShowWriter<W> {
    /// Create a new writer. The file header is written immediately.
    pub fn new(mut inner: W, pixel_count: u32) -> io::Result<Self> {
        // Header (frame_count placeholder = 0; updated in finalise)
        inner.write_all(MAGIC)?;
        inner.write_all(&VERSION.to_le_bytes())?;
        inner.write_all(&pixel_count.to_le_bytes())?;
        inner.write_all(&0u32.to_le_bytes())?; // frame_count placeholder
        Ok(Self { inner, pixel_count, frame_count: 0, header_written: true })
    }

    /// Write one frame. `frame.pixels.len()` must equal `pixel_count`.
    pub fn write_frame(&mut self, record: &ShowRecord) -> io::Result<()> {
        assert!(
            self.header_written,
            "ShowWriter: header not yet written (internal error)"
        );
        if record.pixels.len() as u32 != self.pixel_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "pixel count mismatch: expected {}, got {}",
                    self.pixel_count, record.pixels.len()
                ),
            ));
        }

        // Timestamp
        self.inner.write_all(&record.timestamp_ms.to_le_bytes())?;

        // Pixel data
        for px in &record.pixels {
            self.inner.write_all(&[px.r, px.g, px.b])?;
        }

        // Audio
        if let Some(audio) = &record.audio {
            self.inner.write_all(&[0x01])?;
            audio.write_to(&mut self.inner)?;
        } else {
            self.inner.write_all(&[0x00])?;
        }

        self.frame_count += 1;
        Ok(())
    }

    /// Convenience: write a `LogicalFrame` with an optional audio snapshot.
    pub fn write_logical_frame(
        &mut self,
        frame: &LogicalFrame,
        audio: Option<AudioSnapshot>,
    ) -> io::Result<()> {
        self.write_frame(&ShowRecord {
            timestamp_ms: frame.timestamp_ms,
            pixels:       frame.pixels.clone(),
            audio,
        })
    }

    /// Flush the underlying writer. Does NOT seek back to update `frame_count`.
    pub fn flush(&mut self) -> io::Result<()> { self.inner.flush() }

    /// Total frames written so far.
    pub fn frame_count(&self) -> u32 { self.frame_count }
}

/// Update the `frame_count` field in the header of a seekable writer.
pub fn finalise_seekable<W: Write + io::Seek>(
    writer: &mut ShowWriter<W>,
) -> io::Result<()> {
    let fc = writer.frame_count;
    writer.inner.seek(io::SeekFrom::Start(12))?;
    writer.inner.write_all(&fc.to_le_bytes())?;
    writer.inner.seek(io::SeekFrom::End(0))?;
    writer.flush()
}

// ── ShowReader ────────────────────────────────────────────────────────────────

/// Reads a `.lumyx` file frame by frame.
pub struct ShowReader<R: Read> {
    inner:       R,
    pub pixel_count:  u32,
    /// Frame count from header (may be 0 for non-seekable writers).
    pub frame_count_hint: u32,
    frames_read: u32,
}

/// Error type for show file reading.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    BadMagic,
    UnsupportedVersion(u32),
    PixelCountMismatch { expected: u32, got: u32 },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "I/O error: {e}"),
            ReadError::BadMagic => write!(f, "not a LUMYX show file (bad magic)"),
            ReadError::UnsupportedVersion(v) => write!(f, "unsupported version {v}"),
            ReadError::PixelCountMismatch { expected, got } =>
                write!(f, "pixel count mismatch: file={expected} caller={got}"),
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self { ReadError::Io(e) }
}

impl ShowReader<std::fs::File> {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, ReadError> {
        let f = std::fs::File::open(path).map_err(ReadError::Io)?;
        ShowReader::new(f)
    }
}

impl<R: Read> ShowReader<R> {
    pub fn new(mut inner: R) -> Result<Self, ReadError> {
        let mut magic = [0u8; 4];
        inner.read_exact(&mut magic).map_err(ReadError::Io)?;
        if &magic != MAGIC { return Err(ReadError::BadMagic); }

        let version = read_u32_le(&mut inner).map_err(ReadError::Io)?;
        if version != VERSION { return Err(ReadError::UnsupportedVersion(version)); }

        let pixel_count      = read_u32_le(&mut inner).map_err(ReadError::Io)?;
        let frame_count_hint = read_u32_le(&mut inner).map_err(ReadError::Io)?;

        Ok(Self { inner, pixel_count, frame_count_hint, frames_read: 0 })
    }

    /// Read the next frame. Returns `None` at EOF.
    pub fn next_frame(&mut self) -> Result<Option<ShowRecord>, ReadError> {
        let timestamp_ms = match read_u64_le(&mut self.inner) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(ReadError::Io(e)),
        };

        let n = self.pixel_count as usize;
        let mut pixels = Vec::with_capacity(n);
        for _ in 0..n {
            let mut rgb = [0u8; 3];
            self.inner.read_exact(&mut rgb).map_err(ReadError::Io)?;
            pixels.push(PixelColor::rgb(rgb[0], rgb[1], rgb[2]));
        }

        let mut flag = [0u8; 1];
        self.inner.read_exact(&mut flag).map_err(ReadError::Io)?;
        let audio = if flag[0] == 0x01 {
            Some(AudioSnapshot::read_from(&mut self.inner).map_err(ReadError::Io)?)
        } else {
            None
        };

        self.frames_read += 1;
        Ok(Some(ShowRecord { timestamp_ms, pixels, audio }))
    }

    /// Collect all frames (convenience for small files / tests).
    pub fn collect_all(mut self) -> Result<Vec<ShowRecord>, ReadError> {
        let mut out = Vec::new();
        while let Some(rec) = self.next_frame()? {
            out.push(rec);
        }
        Ok(out)
    }

    pub fn frames_read(&self) -> u32 { self.frames_read }
}

// ── Hash helper ───────────────────────────────────────────────────────────────

/// Compute a simple FNV-1a 64-bit hash over the pixel data of a slice of records.
/// Identical input → identical hash — used by the regression-guardian for replay comparison.
pub fn pixel_hash(records: &[ShowRecord]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME:  u64 = 0x00000100000001b3;
    let mut h = FNV_OFFSET;
    for rec in records {
        for px in &rec.pixels {
            h ^= px.r as u64; h = h.wrapping_mul(FNV_PRIME);
            h ^= px.g as u64; h = h.wrapping_mul(FNV_PRIME);
            h ^= px.b as u64; h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn read_u32_le<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4]; r.read_exact(&mut b)?; Ok(u32::from_le_bytes(b))
}
fn read_u64_le<R: Read>(r: &mut R) -> io::Result<u64> {
    let mut b = [0u8; 8]; r.read_exact(&mut b)?; Ok(u64::from_le_bytes(b))
}
fn read_f32_le<R: Read>(r: &mut R) -> io::Result<f32> {
    let mut b = [0u8; 4]; r.read_exact(&mut b)?; Ok(f32::from_le_bytes(b))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn px(r: u8, g: u8, b: u8) -> PixelColor { PixelColor::rgb(r, g, b) }

    fn sample_record(t: u64, pixels: Vec<PixelColor>, with_audio: bool) -> ShowRecord {
        ShowRecord {
            timestamp_ms: t,
            pixels,
            audio: if with_audio {
                Some(AudioSnapshot {
                    sample_rate: 44100,
                    rms: 0.5,
                    beat: true,
                    bpm: 120.0,
                    bass_energy: 0.3,
                    mid_energy: 0.4,
                    high_energy: 0.2,
                })
            } else {
                None
            },
        }
    }

    // ── Round-trip ─────────────────────────────────────────────────────────────

    #[test]
    fn write_then_read_round_trip() {
        let pixels = vec![px(0xFF, 0, 0), px(0, 0xFF, 0), px(0, 0, 0xFF)];
        let n = pixels.len() as u32;

        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, n).unwrap();
        writer.write_frame(&sample_record(1000, pixels.clone(), true)).unwrap();
        writer.write_frame(&sample_record(2000, pixels.clone(), false)).unwrap();
        writer.flush().unwrap();

        buf.set_position(0);
        let mut reader = ShowReader::new(buf).unwrap();
        assert_eq!(reader.pixel_count, n);

        let rec0 = reader.next_frame().unwrap().unwrap();
        assert_eq!(rec0.timestamp_ms, 1000);
        assert_eq!(rec0.pixels, pixels);
        assert!(rec0.audio.is_some());
        let audio = rec0.audio.unwrap();
        assert_eq!(audio.sample_rate, 44100);
        assert!((audio.rms - 0.5).abs() < 1e-6);
        assert!(audio.beat);
        assert!((audio.bpm - 120.0).abs() < 1e-4);

        let rec1 = reader.next_frame().unwrap().unwrap();
        assert_eq!(rec1.timestamp_ms, 2000);
        assert!(rec1.audio.is_none());

        assert!(reader.next_frame().unwrap().is_none(), "EOF after 2 frames");
    }

    // ── Magic / format ─────────────────────────────────────────────────────────

    #[test]
    fn bad_magic_returns_error() {
        let data = b"NOTLUMX\x01\x00\x00\x00"; // wrong magic
        let result = ShowReader::new(Cursor::new(data.as_ref()));
        assert!(matches!(result, Err(ReadError::BadMagic)));
    }

    #[test]
    fn header_magic_bytes_are_lumx() {
        let mut buf = Cursor::new(Vec::<u8>::new());
        ShowWriter::new(&mut buf, 1).unwrap();
        let bytes = buf.into_inner();
        assert_eq!(&bytes[..4], b"LUMX", "magic must be LUMX");
    }

    #[test]
    fn header_version_is_1() {
        let mut buf = Cursor::new(Vec::<u8>::new());
        ShowWriter::new(&mut buf, 1).unwrap();
        let bytes = buf.into_inner();
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(version, 1);
    }

    #[test]
    fn header_pixel_count_correct() {
        let mut buf = Cursor::new(Vec::<u8>::new());
        ShowWriter::new(&mut buf, 42).unwrap();
        let bytes = buf.into_inner();
        let pc = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(pc, 42);
    }

    // ── Pixel count mismatch ──────────────────────────────────────────────────

    #[test]
    fn wrong_pixel_count_returns_error() {
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, 3).unwrap();
        // Write 2 pixels instead of 3
        let result = writer.write_frame(&ShowRecord {
            timestamp_ms: 0,
            pixels: vec![px(0, 0, 0), px(1, 1, 1)], // 2 ≠ 3
            audio: None,
        });
        assert!(result.is_err(), "wrong pixel count must error");
    }

    // ── Determinism / hash ────────────────────────────────────────────────────

    #[test]
    fn replay_hash_is_deterministic() {
        let pixels = vec![px(10, 20, 30), px(40, 50, 60)];
        let records = vec![
            sample_record(0,    pixels.clone(), false),
            sample_record(1000, pixels.clone(), true),
        ];
        let h1 = pixel_hash(&records);
        let h2 = pixel_hash(&records);
        assert_eq!(h1, h2, "hash must be deterministic");
    }

    #[test]
    fn different_pixels_produce_different_hash() {
        let a = vec![sample_record(0, vec![px(1, 2, 3)], false)];
        let b = vec![sample_record(0, vec![px(3, 2, 1)], false)];
        assert_ne!(pixel_hash(&a), pixel_hash(&b));
    }

    #[test]
    fn identical_replay_produces_identical_hash() {
        let n = 8u32;
        let frames: Vec<ShowRecord> = (0..5).map(|i| {
            ShowRecord {
                timestamp_ms: i * 100,
                pixels: (0..n).map(|j| px(((i * j as u64) % 256) as u8, j as u8, (i % 256) as u8)).collect(),
                audio: None,
            }
        }).collect();

        // Write + read back
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, n).unwrap();
        for f in &frames { writer.write_frame(f).unwrap(); }
        writer.flush().unwrap();

        buf.set_position(0);
        let replayed = ShowReader::new(buf).unwrap().collect_all().unwrap();

        assert_eq!(pixel_hash(&frames), pixel_hash(&replayed),
            "replay must produce identical hash");
    }

    // ── Finalise ──────────────────────────────────────────────────────────────

    #[test]
    fn finalise_updates_frame_count_in_header() {
        let n = 2u32;
        let pixels = vec![px(0, 0, 0), px(1, 1, 1)];
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, n).unwrap();
        writer.write_frame(&sample_record(0, pixels.clone(), false)).unwrap();
        writer.write_frame(&sample_record(1, pixels.clone(), false)).unwrap();
        finalise_seekable(&mut writer).unwrap();

        let bytes = buf.into_inner();
        let fc = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        assert_eq!(fc, 2, "frame_count in header must be updated to 2");
    }

    // ── Collect all ───────────────────────────────────────────────────────────

    #[test]
    fn collect_all_returns_all_frames() {
        let pixels = vec![px(0xFF, 0xFF, 0xFF)];
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, 1).unwrap();
        for t in 0..10u64 {
            writer.write_frame(&sample_record(t * 33, pixels.clone(), false)).unwrap();
        }
        writer.flush().unwrap();

        buf.set_position(0);
        let all = ShowReader::new(buf).unwrap().collect_all().unwrap();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].timestamp_ms, 0);
        assert_eq!(all[9].timestamp_ms, 9 * 33);
    }

    // ── Audio snapshot ────────────────────────────────────────────────────────

    #[test]
    fn audio_snapshot_round_trips_all_fields() {
        let snap = AudioSnapshot {
            sample_rate: 48000,
            rms: 0.123,
            beat: false,
            bpm: 140.5,
            bass_energy: 0.9,
            mid_energy:  0.5,
            high_energy: 0.1,
        };
        let pixels = vec![px(0, 0, 0)];
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, 1).unwrap();
        writer.write_frame(&ShowRecord { timestamp_ms: 0, pixels: pixels.clone(), audio: Some(snap) }).unwrap();
        writer.flush().unwrap();

        buf.set_position(0);
        let rec = ShowReader::new(buf).unwrap().next_frame().unwrap().unwrap();
        let a = rec.audio.unwrap();
        assert_eq!(a.sample_rate, 48000);
        assert!((a.rms - 0.123).abs() < 1e-5);
        assert!(!a.beat);
        assert!((a.bpm - 140.5).abs() < 1e-3);
        assert!((a.bass_energy - 0.9).abs() < 1e-5);
    }

    // ── Stress ────────────────────────────────────────────────────────────────

    #[test]
    fn stress_1000_frames_round_trip() {
        let n = 16u32;
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut writer = ShowWriter::new(&mut buf, n).unwrap();

        for i in 0u64..1000 {
            let pixels = (0..n).map(|j| px((i % 256) as u8, j as u8, 0)).collect();
            writer.write_frame(&ShowRecord {
                timestamp_ms: i * 33,
                pixels,
                audio: if i % 10 == 0 { Some(AudioSnapshot {
                    sample_rate: 44100, rms: 0.1 * (i % 10) as f32,
                    beat: i % 20 == 0, bpm: 120.0,
                    bass_energy: 0.2, mid_energy: 0.3, high_energy: 0.1,
                }) } else { None },
            }).unwrap();
        }
        writer.flush().unwrap();

        buf.set_position(0);
        let all = ShowReader::new(buf).unwrap().collect_all().unwrap();
        assert_eq!(all.len(), 1000);
        assert_eq!(all[500].timestamp_ms, 500 * 33);
    }
}
