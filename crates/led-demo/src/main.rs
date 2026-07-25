//! LUMYX LED Show Demo — production-grade render pipeline demonstration.
//!
//! Produces:
//! - `show.gif`        — visual preview (32×18 matrix, 20fps, 6s)
//! - `show.lumyx`      — binary replay file with `pixel_hash` integrity check
//! - `show_drone.json` — drone formation annotations derived from section timeline
//! - `show_report.json`— observability report (p50/p99, alerts, spans)
//!
//! ## Pipeline
//!
//! ```text
//! SectionDetector (simulated audio) → SectionClip (section-aware effects)
//!       ↓
//! AutoGpuPlasma (CPU fallback, GPU when available)
//!       ↓
//! ShowIntentGenerator → Timeline (beat-synced clips)
//!       ↓
//! ShowWriter (.lumyx) + GIF encoder + DroneBridge (.json)
//!       ↓
//! ReplayManifest integrity check + ObservabilityReport
//! ```
//!
//! Run: `cargo run -p led-demo --release`
//! GPU: `cargo run -p led-demo --release --features gpu`

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use std::time::Instant;

use led_core::{MusicalSection, PixelColor};
use led_hal::{
    AlertEngine, MetricsEmitter, ObservabilityReport, SpanCollector,
};
use led_layout::LayoutBuilder;
use led_pixel_engine::{
    AudioShare, AutoGpuPlasma, Effect, Pulse, Rainbow, SolidColor, Vec3, GPU_THRESHOLD_PIXELS,
};
use led_sequencer::{
    BlendMode, Clip, SectionClip,
    ShowIntentGenerator, TempoMap, Timeline, Track,
};
use led_show_recorder::{
    finalise_seekable, pixel_hash, verify_replay,
    DroneBridge, SectionEvent,
    ShowReader, ShowRecord, ShowWriter,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const W: usize = 32;
const H: usize = 18;
const BLOCK: usize = 12;
const FPS: u64 = 20;
const DURATION_MS: u64 = 6_000;
const BPM: f32 = 120.0;

// Simulated section timeline (replaces live audio for this offline demo).
// In production, SectionDetector produces this from AudioFeatures in real time.
const SECTION_TIMELINE: &[(u64, u64, MusicalSection, f32)] = &[
    (    0,  1_000, MusicalSection::Intro,  0.15),
    (1_000,  2_500, MusicalSection::Verse,  0.40),
    (2_500,  3_500, MusicalSection::Build,  0.65),
    (3_500,  5_000, MusicalSection::Chorus, 0.85),
    (5_000,  5_500, MusicalSection::Bridge, 0.30),
    (5_500,  6_000, MusicalSection::Outro,  0.10),
];

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let t_start = Instant::now();
    let n = W * H; // 576 pixels

    // ── 1. Layout ─────────────────────────────────────────────────────────────
    let mut b = LayoutBuilder::new();
    b.add_matrix("matrix", W as u32, H as u32, false, 1.0);
    let layout = b.build();
    let positions: Vec<Vec3> = layout.pixels.iter()
        .map(|p| Vec3::new(p.x, p.y, p.z))
        .collect();

    // ── 2. Observability setup ────────────────────────────────────────────────
    let metrics = Arc::new(MetricsEmitter::new("led-demo"));
    let spans   = SpanCollector::new(500);
    let alerts  = AlertEngine::lumyx_standard();

    // ── 3. AudioShare — simulated section labels ───────────────────────────────
    let audio_share = Arc::new(AudioShare::new());
    // Pre-publish a neutral AudioFeatures so effects start non-silent
    audio_share.publish(&led_core::AudioFeatures {
        sample_rate: 44_100,
        timestamp_ms: 0,
        rms: 0.3,
        beat: false,
        bass: 0.2,
        mid: 0.3,
        high: 0.1,
        spectrum: vec![],
        musical_section: Some(MusicalSection::Intro), instrument_class: None,
    });

    // ── 4. ShowIntent → initial timeline parameters ───────────────────────────
    let intent_gen = ShowIntentGenerator::new(42); // seed=42 → deterministic
    let initial_intent = intent_gen
        .from_audio(0.3, false, BPM, Some(MusicalSection::Intro), DURATION_MS, n)
        .expect("valid intent");
    println!("ShowIntent: style={:?} energy={:.2} bpm={:.0} hash={:#016x}",
        initial_intent.style, initial_intent.energy, initial_intent.tempo_bpm,
        initial_intent.intent_hash);

    // ── 5. AutoGpuPlasma (CPU or GPU based on pixel count) ────────────────────
    let plasma = AutoGpuPlasma::new(n, 0.55, 1.3);
    println!("GPU executor: {} (threshold={}px, n={}px)",
        if plasma.using_gpu { "ACTIVE" } else { "CPU fallback" },
        GPU_THRESHOLD_PIXELS, n);

    // ── 6. SectionClip — section-aware effects ────────────────────────────────
    let section_clip = SectionClip::new(
        // Default: Plasma via AutoGpuPlasma
        Box::new(AutoGpuPlasma::cpu_only(0.55, 1.3)),
        audio_share.clone(),
    )
    .with_section(MusicalSection::Intro,
        Box::new(Rainbow { speed_hz: 0.1, cycles_per_m: 0.3 }))
    .with_section(MusicalSection::Verse,
        Box::new(AutoGpuPlasma::cpu_only(0.4, 0.8)))
    .with_section(MusicalSection::Build,
        Box::new(Pulse { color: PixelColor::rgb(80, 120, 255), hz: 2.0 }))
    .with_section(MusicalSection::Chorus,
        Box::new(AutoGpuPlasma::cpu_only(0.8, 2.5)))
    .with_section(MusicalSection::Bridge,
        Box::new(SolidColor(PixelColor::rgb(20, 20, 60))))
    .with_section(MusicalSection::Outro,
        Box::new(Rainbow { speed_hz: 0.05, cycles_per_m: 0.1 }));

    // ── 7. Beat-synced flash track ─────────────────────────────────────────────
    let tempo = TempoMap::constant(BPM, 0);
    let mut beats = Track::new(BlendMode::Add);
    let beat_ms = (60_000.0 / BPM) as u64;
    let mut t = 0u64;
    while t < DURATION_MS {
        beats.clips.push(
            Clip::new(t, t + beat_ms / 2,
                Box::new(SolidColor(PixelColor::rgb(80, 80, 100))))
                .with_fades(0, beat_ms / 2),
        );
        t = tempo.beat_time((t / beat_ms) + 1);
    }

    // ── 8. Full timeline ───────────────────────────────────────────────────────
    let show = Timeline::new(n)
        .with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, DURATION_MS, Box::new(section_clip))),
        )
        .with_track(beats);

    // ── 9. Render loop — GIF + .lumyx ─────────────────────────────────────────
    let img_w = (W * BLOCK) as u16;
    let img_h = (H * BLOCK) as u16;
    let frame_count = DURATION_MS * FPS / 1000;
    let dt = 1000 / FPS;

    let mut gif_file = File::create("show.gif").expect("create show.gif");
    let mut encoder = gif::Encoder::new(&mut gif_file, img_w, img_h, &[]).expect("gif encoder");
    encoder.set_repeat(gif::Repeat::Infinite).expect("set repeat");

    let lumyx_file = File::create("show.lumyx").expect("create show.lumyx");
    let mut recorder = ShowWriter::new(BufWriter::new(lumyx_file), n as u32)
        .expect("show recorder");

    let mut logical = vec![PixelColor::default(); n];
    let mut rgb = vec![0u8; img_w as usize * img_h as usize * 3];
    let mut collected: Vec<ShowRecord> = Vec::with_capacity(frame_count as usize);

    // Track active section for DroneBridge
    let mut section_events: Vec<SectionEvent> = Vec::new();
    let mut current_section = MusicalSection::Intro;
    let mut section_start = 0u64;
    let mut section_energy_sum = 0.0f32;
    let mut section_hops = 0u64;

    for f in 0..frame_count {
        let time_ms = f * dt;

        // Update simulated audio section
        for &(start, end, section, energy) in SECTION_TIMELINE {
            if time_ms >= start && time_ms < end {
                if section != current_section {
                    // Flush previous section event
                    if section_hops > 0 {
                        section_events.push(SectionEvent {
                            start_ms:   section_start,
                            end_ms:     time_ms,
                            section:    current_section,
                            avg_energy: section_energy_sum / section_hops as f32,
                        });
                    }
                    current_section = section;
                    section_start = time_ms;
                    section_energy_sum = 0.0;
                    section_hops = 0;
                    // Update AudioShare with new section label
                    audio_share.publish(&led_core::AudioFeatures {
                        sample_rate:     44_100,
                        timestamp_ms:    time_ms,
                        rms:             energy,
                        beat:            time_ms % beat_ms < dt,
                        bass:            energy * 0.6,
                        mid:             energy * 0.8,
                        high:            energy * 0.3,
                        spectrum:        vec![],
                        musical_section: Some(section), instrument_class: None,
                    });
                }
                section_energy_sum += energy;
                section_hops += 1;
                break;
            }
        }

        // Render with span measurement
        let span = spans.start("show.render", 0, "layer=sequencer");
        show.render(time_ms, &positions, &mut logical);
        span.finish();

        // Record latency
        let t0 = Instant::now();
        let record = ShowRecord {
            timestamp_ms: time_ms,
            pixels:       logical.clone(),
            audio:        None,
        };
        recorder.write_frame(&record).expect("record frame");
        metrics.record_frame(t0.elapsed().as_micros() as u64);
        collected.push(record);

        // Encode GIF frame
        paint_frame(&logical, &mut rgb, img_w as usize);
        let mut gif_frame = gif::Frame::from_rgb_speed(img_w, img_h, &rgb, 10);
        gif_frame.delay = (100 / FPS) as u16;
        encoder.write_frame(&gif_frame).expect("write gif frame");
    }

    // Flush last section
    if section_hops > 0 {
        section_events.push(SectionEvent {
            start_ms:   section_start,
            end_ms:     DURATION_MS,
            section:    current_section,
            avg_energy: section_energy_sum / section_hops as f32,
        });
    }

    finalise_seekable(&mut recorder).expect("finalise lumyx header");
    let written_frames = recorder.frame_count();

    // ── 10. Replay integrity ──────────────────────────────────────────────────
    let replay_file = File::open("show.lumyx").expect("open show.lumyx for verify");
    let reader = ShowReader::new(std::io::BufReader::new(replay_file)).expect("reader");
    let manifest = match verify_replay(reader, pixel_hash(&collected)) {
        Ok(m)  => {
            println!("✅ Replay verified — aggregate_hash={:#018x}  frames={}", m.aggregate_hash, m.frame_count);
            m
        }
        Err((actual, expected)) => {
            eprintln!("❌ REPLAY INTEGRITY FAILED — actual={actual:#018x} expected={expected:#018x}");
            std::process::exit(1);
        }
    };

    // ── 11. DroneBridge — export drone annotations ────────────────────────────
    let bridge = DroneBridge::new(500); // min 500ms per section
    let (led_annotations, drone_annotations) = bridge.build_synced(&section_events);
    let drone_json = bridge.export_json(&drone_annotations);
    std::fs::write("show_drone.json", &drone_json).expect("write drone json");
    println!("✅ Drone annotations — {} segments → show_drone.json", drone_annotations.len());

    // ── 12. ObservabilityReport ───────────────────────────────────────────────
    let report = ObservabilityReport::collect(&metrics, &spans, &alerts, 1);
    let report_json = format!(
        r#"{{"show":{show},"replay":{replay},"drone_segments":{ds},"led_segments":{ls},"render_p99_us":{p99},"frames_total":{ft}}}"#,
        show    = report.to_json(),
        replay  = manifest.to_json(),
        ds      = drone_annotations.len(),
        ls      = led_annotations.len(),
        p99     = spans.p99_us("show.render"),
        ft      = written_frames,
    );
    std::fs::write("show_report.json", &report_json).expect("write report");

    let elapsed = t_start.elapsed();
    println!("\n══════════════════════════════════════════════════");
    println!("  LUMYX LED Show Demo — Build Complete");
    println!("══════════════════════════════════════════════════");
    println!("  show.gif          {}×{} matrix  {} frames  {BPM}BPM", W, H, written_frames);
    println!("  show.lumyx        {} frames  hash={:#018x}", written_frames, manifest.aggregate_hash);
    println!("  show_drone.json   {} drone segments", drone_annotations.len());
    println!("  show_report.json  {} alerts  p99={}µs", report.alerts.len(), spans.p99_us("show.render"));
    println!("  elapsed           {:.1}s", elapsed.as_secs_f32());
    println!("══════════════════════════════════════════════════");

    if !report.alerts.is_empty() {
        println!("\n⚠ ALERTS:");
        for a in &report.alerts { println!("  {a}"); }
    }
}

fn paint_frame(logical: &[PixelColor], rgb: &mut [u8], img_w: usize) {
    for (id, c) in logical.iter().enumerate() {
        let col = id % W;
        let row = id / W;
        for dy in 0..BLOCK {
            for dx in 0..BLOCK {
                let x = col * BLOCK + dx;
                let y = row * BLOCK + dy;
                let o = (y * img_w + x) * 3;
                rgb[o]     = c.r;
                rgb[o + 1] = c.g;
                rgb[o + 2] = c.b;
            }
        }
    }
}
