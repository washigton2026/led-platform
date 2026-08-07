//! `led-player` — plays a `.lumyx` recording through any [`ProtocolOutput`].
//!
//! The player is the product half of deterministic replay: `led-show-recorder`
//! proves two nodes render identical pixels; the player puts those pixels back
//! on real hardware (or the simulator) with the recording's own timing.
//!
//! ## Invariants
//! - The player never fabricates pixels: what was recorded is what is sent.
//!   (`verify` recomputes the [`ReplayManifest`] hash before playback.)
//! - Pacing follows the recorded `timestamp_ms` deltas; `Speed::Max` plays
//!   without sleeping (for tests and integrity re-checks).
//! - The player is NOT a hot path: sleeping between frames is deliberate.

use std::time::Duration;

pub mod stream;
/// Playback em fluxo — **caminho de bancada, não de palco** (TD-013).
///
/// `play_streaming_unverified` emite o 1º quadro sem verificar assinatura nem integridade.
/// O caminho autenticado é o do binário (`--verify-key` → `verify_manifest_pinned` → tocar).
pub use stream::{
    play_streaming_unverified, Pacing, PacingPolicy, StreamError, StreamReport,
};

use led_core::{LogicalFrame, OutputError, PixelPhysical, ProtocolOutput, RgbOrder};
use led_show_recorder::replay::ReplayManifest;
use led_show_recorder::ShowRecord;

/// Playback pacing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    /// Follow the recorded timestamps (1.0 = real time; 2.0 = twice as fast).
    Factor(f32),
    /// No sleeping — as fast as the output accepts frames.
    Max,
}

/// Summary of one playback run.
#[derive(Debug)]
pub struct PlaybackReport {
    pub frames_played: u64,
    pub frames_failed: u64,
    pub duration_ms: u64,
    pub pixel_count: u32,
    pub manifest_hash: u64,
}

/// Inspection summary of a recording (the "timeline viewer" in text form).
#[derive(Debug)]
pub struct ShowInfo {
    pub frame_count: usize,
    pub pixel_count: u32,
    pub duration_ms: u64,
    pub avg_frame_interval_ms: f64,
    pub beats: u64,
    pub manifest_hash: u64,
}

impl ShowInfo {
    pub fn from_records(records: &[ShowRecord]) -> Self {
        let manifest = ReplayManifest::from_records(records);
        let duration_ms = match (records.first(), records.last()) {
            (Some(a), Some(b)) => b.timestamp_ms.saturating_sub(a.timestamp_ms),
            _ => 0,
        };
        let beats = records
            .iter()
            .filter(|r| r.audio.as_ref().is_some_and(|a| a.beat))
            .count() as u64;
        let avg = if records.len() > 1 {
            duration_ms as f64 / (records.len() - 1) as f64
        } else {
            0.0
        };
        Self {
            frame_count: records.len(),
            pixel_count: manifest.pixel_count,
            duration_ms,
            avg_frame_interval_ms: avg,
            beats,
            manifest_hash: manifest.aggregate_hash,
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            r#"{{"frames":{},"pixels":{},"duration_ms":{},"avg_interval_ms":{:.2},"beats":{},"hash":"{:#018x}"}}"#,
            self.frame_count,
            self.pixel_count,
            self.duration_ms,
            self.avg_frame_interval_ms,
            self.beats,
            self.manifest_hash,
        )
    }
}

/// Play `records` through `output`, pacing by recorded timestamps.
pub fn play(
    records: &[ShowRecord],
    output: &dyn ProtocolOutput,
    speed: Speed,
) -> Result<PlaybackReport, OutputError> {
    play_instrumented(records, output, speed, None)
}

/// [`play`] with an optional [`MetricsEmitter`](led_hal::MetricsEmitter):
/// every send is timed (`record_frame`) and every failure counted
/// (`record_drop`) — the numbers behind the Prometheus SLOs.
pub fn play_instrumented(
    records: &[ShowRecord],
    output: &dyn ProtocolOutput,
    speed: Speed,
    metrics: Option<&led_hal::MetricsEmitter>,
) -> Result<PlaybackReport, OutputError> {
    let manifest = ReplayManifest::from_records(records);
    let mut played = 0u64;
    let mut failed = 0u64;
    let mut prev_ts: Option<u64> = None;

    for r in records {
        if let (Speed::Factor(f), Some(prev)) = (speed, prev_ts) {
            let gap = r.timestamp_ms.saturating_sub(prev);
            if gap > 0 && f > 0.0 {
                std::thread::sleep(Duration::from_micros((gap as f64 * 1000.0 / f as f64) as u64));
            }
        }
        prev_ts = Some(r.timestamp_ms);
        let frame = LogicalFrame::new(r.pixels.clone(), r.timestamp_ms);
        let t0 = std::time::Instant::now();
        match output.send_frame(&frame) {
            Ok(()) => {
                played += 1;
                if let Some(m) = metrics {
                    m.record_frame(t0.elapsed().as_micros() as u64);
                }
            }
            Err(_) => {
                failed += 1; // keep playing: partial show beats a black stage
                if let Some(m) = metrics {
                    m.record_drop();
                }
            }
        }
    }

    let duration_ms = match (records.first(), records.last()) {
        (Some(a), Some(b)) => b.timestamp_ms.saturating_sub(a.timestamp_ms),
        _ => 0,
    };
    Ok(PlaybackReport {
        frames_played: played,
        frames_failed: failed,
        duration_ms,
        pixel_count: manifest.pixel_count,
        manifest_hash: manifest.aggregate_hash,
    })
}

/// Pixel-native DDP output: no universe mapping at all — DDP addresses pixels
/// by byte offset and auto-fragments at 487 px per datagram (vs 170 px per
/// ArtNet universe: ~3× fewer packets for the same rig, the capacity path for
/// WLED controllers).
pub struct DdpOutput {
    dev: std::sync::Mutex<led_protocols::DdpDevice>,
    universes_equiv: u16,
}

impl DdpOutput {
    pub fn new(addr: std::net::SocketAddr, pixel_count: usize) -> std::io::Result<Self> {
        Ok(Self {
            dev: std::sync::Mutex::new(led_protocols::DdpDevice::new(addr, 0)?),
            universes_equiv: pixel_count.div_ceil(170) as u16,
        })
    }

    /// Saída DDP pixel-nativa com um [`ColorFormat`] explícito — é assim que um preset RGBW
    /// (ADR-0011/0018) chega ao fio por DDP. O branco é derivado pelo mesmo contrato usado
    /// no mapper; não há segunda implementação.
    pub fn with_format(
        addr: std::net::SocketAddr,
        pixel_count: usize,
        format: led_core::ColorFormat,
    ) -> std::io::Result<Self> {
        Ok(Self {
            dev: std::sync::Mutex::new(led_protocols::DdpDevice::with_format(addr, 0, format)?),
            universes_equiv: pixel_count.div_ceil(170) as u16,
        })
    }

    /// Saída DDP com **os limites declarados pelo hardware**: quantos pixels cabem num
    /// datagrama (derivado do MTU) e quantos num universo equivalente.
    ///
    /// O DDP não tem universos; `universes_equiv` existe só para o `universe_count()` do
    /// `ProtocolOutput`. Recebê-lo em vez de o assumir 170 é o que impede este número de ser
    /// mais um valor físico escrito à mão.
    pub fn with_limits(
        addr: std::net::SocketAddr,
        pixel_count: usize,
        format: led_core::ColorFormat,
        max_pixels_per_datagram: usize,
        pixels_per_universe: u16,
    ) -> std::io::Result<Self> {
        let mut dev = led_protocols::DdpDevice::with_format(addr, 0, format)?;
        dev.set_max_pixels(max_pixels_per_datagram);
        Ok(Self {
            dev: std::sync::Mutex::new(dev),
            universes_equiv: pixel_count.div_ceil(pixels_per_universe.max(1) as usize) as u16,
        })
    }
}

impl ProtocolOutput for DdpOutput {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        self.dev
            .lock()
            .unwrap()
            .send_pixels(&frame.pixels)
            .map_err(|e| OutputError::Transport(format!("ddp send: {e}")))
    }

    fn universe_count(&self) -> u16 {
        self.universes_equiv
    }
}

/// Linear physical assignments for playing to real hardware: `pixel_count`
/// pixels on one device, 170 px per universe (510 channels — no pixel straddles
/// a universe boundary), universes numbered from `first_universe` (WLED/xLights
/// rigs usually start at 1; `CompiledLayout::linear` starts at 0).
pub fn linear_assignments(
    pixel_count: usize,
    device: led_core::DeviceId,
    first_universe: u16,
    order: RgbOrder,
) -> Vec<PixelPhysical> {
    const PX_PER_UNIVERSE: usize = 170; // 510 / 3
    (0..pixel_count)
        .map(|i| PixelPhysical {
            device,
            universe: first_universe + (i / PX_PER_UNIVERSE) as u16,
            channel: ((i % PX_PER_UNIVERSE) * 3) as u16,
            format: order.into(),
        })
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::{CompiledLayout, PixelColor};
    use led_hal::{Hal, SimulatorDevice};

    fn records(n: u64, px: usize) -> Vec<ShowRecord> {
        (0..n)
            .map(|i| ShowRecord {
                timestamp_ms: i * 33,
                pixels: (0..px).map(|j| PixelColor::rgb((i + j as u64) as u8, 0, 0)).collect(),
                audio: None,
            })
            .collect()
    }

    fn sim_output(px: usize) -> (Hal, std::sync::Arc<SimulatorDevice>) {
        let assigns = linear_assignments(px, 30, 1, RgbOrder::Rgb);
        let layout = CompiledLayout::compile(&assigns);
        let sim = SimulatorDevice::new(30, layout.device_universes(30));
        (Hal::new(layout, vec![sim.clone()]), sim)
    }

    #[test]
    fn plays_every_recorded_frame_to_the_output() {
        let recs = records(50, 8);
        let (hal, sim) = sim_output(8);
        let report = play(&recs, &hal, Speed::Max).unwrap();
        assert_eq!(report.frames_played, 50);
        assert_eq!(report.frames_failed, 0);
        assert_eq!(sim.frames_sent(), 50, "all frames reach the device");
        assert_eq!(report.duration_ms, 49 * 33);
    }

    #[test]
    fn playback_hash_matches_recording_manifest() {
        let recs = records(20, 4);
        let manifest = ReplayManifest::from_records(&recs);
        let (hal, _sim) = sim_output(4);
        let report = play(&recs, &hal, Speed::Max).unwrap();
        assert_eq!(report.manifest_hash, manifest.aggregate_hash,
            "player must not alter pixels");
    }

    #[test]
    fn speed_factor_paces_playback() {
        let recs = records(4, 2); // 3 gaps × 33ms = 99ms at 1×
        let (hal, _sim) = sim_output(2);
        let t0 = std::time::Instant::now();
        play(&recs, &hal, Speed::Factor(10.0)).unwrap(); // 10× → ~9.9ms
        let elapsed = t0.elapsed().as_millis();
        assert!(elapsed >= 8, "10x speed still sleeps ~9.9ms, got {elapsed}ms");
        assert!(elapsed < 99, "10x must be faster than real time, got {elapsed}ms");
    }

    #[test]
    fn empty_recording_is_a_clean_noop() {
        let (hal, sim) = sim_output(4);
        let report = play(&[], &hal, Speed::Max).unwrap();
        assert_eq!(report.frames_played, 0);
        assert_eq!(sim.frames_sent(), 0);
    }

    #[test]
    fn show_info_summarises_the_timeline() {
        let snap = |beat| led_show_recorder::AudioSnapshot {
            sample_rate: 48_000, rms: 0.5, beat, bpm: 120.0,
            bass_energy: 0.1, mid_energy: 0.1, high_energy: 0.1,
        };
        let mut recs = records(10, 4);
        recs[3].audio = Some(snap(true));
        recs[7].audio = Some(snap(true));
        let info = ShowInfo::from_records(&recs);
        assert_eq!(info.frame_count, 10);
        assert_eq!(info.pixel_count, 4);
        assert_eq!(info.duration_ms, 9 * 33);
        assert_eq!(info.beats, 2);
        assert!(info.to_json().contains("\"beats\":2"));
    }

    #[test]
    fn ddp_output_sends_pixel_native_datagrams() {
        use led_protocols::parse_ddp_packet;
        use std::net::UdpSocket;

        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_millis(300))).unwrap();
        // 600 px > 487 → exactly 2 DDP fragments per frame.
        let out = DdpOutput::new(rx.local_addr().unwrap(), 600).unwrap();
        let recs = records(3, 600);
        let report = play(&recs, &out, Speed::Max).unwrap();
        assert_eq!(report.frames_played, 3);

        let mut buf = [0u8; 2048];
        let mut fragments = 0;
        while let Ok((n, _)) = rx.recv_from(&mut buf) {
            assert!(parse_ddp_packet(&buf[..n]).is_some(), "every datagram is valid DDP");
            fragments += 1;
        }
        assert_eq!(fragments, 6, "3 frames × 2 fragments (600px @ 487/packet)");
    }

    #[test]
    fn linear_assignments_never_straddle_universes() {
        // 512 px: universes 1..=4, pixel 170 starts universe 2 channel 0.
        let a = linear_assignments(512, 1, 1, RgbOrder::Rgb);
        assert_eq!((a[169].universe, a[169].channel), (1, 507));
        assert_eq!((a[170].universe, a[170].channel), (2, 0));
        assert!(a.iter().all(|p| p.channel <= 507), "3 channels always fit");
        assert_eq!(a.last().unwrap().universe, 4);
    }
}
