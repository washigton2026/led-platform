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

    /// **Onde começa o segmento deste nó** no buffer de pixels do destino (TD-016).
    ///
    /// # O defeito que isto corrige
    ///
    /// Os três construtores acima passam `0` ao `DdpDevice`, e esse `0` estava **escrito à
    /// mão**: não era um argumento que alguém se esquecesse de passar — era um parâmetro que
    /// esta API **não expunha**. Com um só alvo isso é invisível, porque aí zero é o valor
    /// correcto; foi assim que atravessou o GS4.1 até à primeira luz sem incomodar ninguém.
    ///
    /// Com N nós deixa de ser invisível. Cinco WLED a receber todos o intervalo a partir de
    /// zero acendem **os cinco a mesma coisa** em vez de cada um a sua parte do show. Não é
    /// palco escuro — é pior de diagnosticar, porque parece funcionar.
    ///
    /// É a mesma classe do `RgbOrder` do GS4.3 e do MTU do GS4.4: um campo que o fio suporta
    /// e que ninguém acima dele honrava. O `DdpDevice` já o aceita desde sempre
    /// (`offset_bytes` viaja no cabeçalho, big-endian) — **nenhuma lógica nova de protocolo
    /// entra aqui**, só deixa de haver um número fixo no caminho.
    ///
    /// Aditivo de propósito: nenhuma assinatura existente muda, e um alvo único continua a
    /// não escrever offset nenhum.
    #[must_use]
    pub fn with_pixel_offset(mut self, pixel_offset: u32) -> Self {
        // `get_mut` em vez de `lock`: em `&mut self` não há concorrência a arbitrar, e pedir
        // um lock aqui sugeriria que há.
        self.dev.get_mut().expect("mutex do DdpDevice").pixel_offset = pixel_offset;
        self
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

    /// **TD-016 — o offset de cada nó chega ao fio, e nós diferentes escrevem offsets
    /// diferentes.**
    ///
    /// O equivalente DDP do que o `wled_driver.rs` já faz para o `first_universe` do
    /// Art-Net. Sem isto, o campo de instância do protocolo validado em hardware era o único
    /// sem prova no fio.
    ///
    /// **O controlo negativo é a segunda metade, e sem ela o teste não valeria nada:** um
    /// teste que só afirmasse *"o offset chega"* passaria com os dois nós a zero — que é
    /// exactamente o defeito. É a diferença **entre** os dois que prova que cada nó recebe o
    /// seu segmento.
    #[test]
    fn ddp_o_offset_de_cada_no_chega_ao_fio_e_nos_diferentes_diferem() {
        use led_protocols::parse_ddp_packet;
        use std::net::UdpSocket;

        // Um só quadro e poucos pixels: o que se mede aqui é o ENDEREÇO, não a fragmentação
        // (essa já tem os seus testes). 4 px cabem num datagrama, portanto cada nó escreve
        // exactamente um — e o offset dele é o do segmento.
        let offsets_no_fio = |px_offset: u32| -> Vec<u32> {
            let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
            rx.set_read_timeout(Some(std::time::Duration::from_millis(300))).unwrap();
            let out =
                DdpOutput::new(rx.local_addr().unwrap(), 4).unwrap().with_pixel_offset(px_offset);
            play(&records(1, 4), &out, Speed::Max).unwrap();

            let mut buf = [0u8; 2048];
            let mut vistos = Vec::new();
            while let Ok((n, _)) = rx.recv_from(&mut buf) {
                let p = parse_ddp_packet(&buf[..n]).expect("datagrama DDP válido");
                vistos.push(p.offset_bytes);
            }
            vistos
        };

        // Nó 1 começa no pixel 0; nó 2 começa no pixel 720 (o tamanho de uma fita do rig).
        let no1 = offsets_no_fio(0);
        let no2 = offsets_no_fio(720);

        assert_eq!(no1, vec![0], "o primeiro nó começa no início do buffer");
        assert_eq!(
            no2,
            vec![720 * 3],
            "o offset viaja em BYTES: 720 px × 3 canais. Se aparecer 720 aqui, alguém \
             confundiu pixels com bytes e o segundo nó escreveria em cima do primeiro"
        );

        // **O controlo negativo.** Sem esta asserção, a implementação podia ignorar o
        // parâmetro e as duas listas acima seriam ambas `[0]` — o defeito do TD-016 intacto.
        assert_ne!(
            no1, no2,
            "dois nós com offsets diferentes TÊM de escrever offsets diferentes no fio; \
             iguais significa que os cinco robôs receberiam todos o mesmo segmento"
        );
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
