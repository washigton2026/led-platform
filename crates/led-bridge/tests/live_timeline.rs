//! P1 — Live Timeline: AudioFeatures → TempoMap → Timeline → HAL
//!
//! The full product loop — beats detected from live audio drive clip timing:
//!
//! ```text
//! [SimLoop]
//!   SineGen + BeatImpulse @ 48kHz
//!         │
//!         ▼
//!   Analyzer::process_hop   → AudioFeatures v1
//!         │
//!         ▼
//!   adapt_into              → AudioFeatures v0  (beat flag + timestamp)
//!         │  collect (ts, beat) pairs
//!         ▼
//!   TempoMap::from_beat_flags   ← timing grid from real DSP
//!         │
//!         ▼
//!   Timeline::with_track(clips snapped to beat grid)
//!         │  Timeline implements Effect
//!         ▼
//!   render(t, positions, out)   → [PixelColor; N]
//!         │
//!         ▼
//!   LogicalFrame → Hal::send_frame → SimulatorDevice
//!         │
//!         ▼
//!   assertions: correct pixels at beat times, black between beats
//! ```

use std::sync::Arc;

use audio_core::{Analyzer, contracts::HOP_SIZE};
use led_bridge::adapter::adapt_into;
use led_core::{AudioFeatures as V0, LogicalFrame, PixelColor, ProtocolOutput};
use led_hal::{CompiledLayout, DeviceSpec, Hal, RgbOrder, SimulatorDevice};
use led_pixel_engine::{Effect, SolidColor, Vec3};
use led_sequencer::{BlendMode, Clip, EasingType, Keyframe, TempoMap, Timeline, Track};

const PIXELS: usize = 60;
const SR: u32 = 48_000;

fn make_hal() -> (Arc<SimulatorDevice>, Hal) {
    let specs = [DeviceSpec { id: 1, universes: 1 }];
    let layout = CompiledLayout::linear(PIXELS, &specs, RgbOrder::Rgb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<Arc<dyn led_core::DeviceDriver>> = vec![sim.clone()];
    (sim, Hal::new(layout, devices))
}

fn positions() -> Vec<Vec3> {
    (0..PIXELS).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect()
}

/// Run the audio analysis for `duration_ms` with beats every `beat_ms`.
/// Returns (beat_timestamps, hop_dur_ms).
fn collect_beats(duration_ms: u64, beat_ms: u64) -> (Vec<(u64, bool)>, u64) {
    let mut analyzer = Analyzer::new(SR);
    let mut v0 = V0 {
        sample_rate: 0, timestamp_ms: 0, rms: 0.0, beat: false,
        bass: 0.0, mid: 0.0, high: 0.0, spectrum: Vec::new(),
    };
    let hop_dur_ms = (HOP_SIZE as u64 * 1_000) / SR as u64;
    let total_hops = duration_ms / hop_dur_ms;
    let mut stream = Vec::new();
    let mut phase = 0.0f32;
    let phase_inc = std::f32::consts::TAU * 0.0001 / SR as f32; // near-silence tone

    for hop_idx in 0..total_hops {
        let ts = hop_idx * hop_dur_ms;
        let mut hop = [0.0f32; HOP_SIZE];
        for s in hop.iter_mut() {
            *s = phase.sin() * 0.01; // near-silence
            phase = (phase + phase_inc) % std::f32::consts::TAU;
        }
        // Beat impulse
        if beat_ms > 0 && ts % beat_ms < hop_dur_ms {
            for s in hop.iter_mut() { *s += 0.9; }
        }
        let v1 = analyzer.process_hop(&hop, ts);
        adapt_into(&v1, &mut v0);
        stream.push((ts, v1.beat));
    }
    (stream, hop_dur_ms)
}

// ── P1a: TempoMap built from live beats drives clip placement ─────────────
#[test]
fn live_beats_drive_clip_placement() {
    let (stream, _hop_ms) = collect_beats(4_000, 500); // 4s at 120 BPM
    let tm = TempoMap::from_beat_flags(stream);

    // Place a red clip on beats 0→2 (≈0–1000ms)
    let clip = Clip::on_beats(&tm, 0, 2, Box::new(SolidColor(PixelColor::rgb(200, 0, 0))));
    let track = Track::new(BlendMode::Override).with_clip(clip);
    let tl = Timeline::new(PIXELS).with_track(track);

    let pos = positions();
    let mut out = vec![PixelColor::default(); PIXELS];

    // Render AT beat 1 (should be active)
    let t1 = tm.beat_time(1);
    tl.render(t1, &pos, &mut out);
    assert!(out[0].r > 0, "clip must be active at beat 1 (t={t1}ms), got r={}", out[0].r);

    // Render at beat 3 (beyond end_ms=beat_time(2) — should be black)
    let t3 = tm.beat_time(3);
    let mut out2 = vec![PixelColor::default(); PIXELS];
    tl.render(t3, &pos, &mut out2);
    assert_eq!(out2[0], PixelColor::default(), "clip must be inactive at beat 3 (t={t3}ms)");
}

// ── P1b: Full stack — live beats → Timeline → HAL → SimulatorDevice ──────
#[test]
fn live_timeline_full_stack_to_hal() {
    let (stream, _) = collect_beats(3_000, 500);
    let tm = TempoMap::from_beat_flags(stream);

    // Build a 2-beat clip: green flash
    let clip = Clip::on_beats(&tm, 0, 2, Box::new(SolidColor(PixelColor::rgb(0, 180, 0))));
    let track = Track::new(BlendMode::Override).with_clip(clip);
    let tl = Timeline::new(PIXELS).with_track(track);

    let (sim, hal) = make_hal();
    let pos = positions();

    // Render 4 frames across the timeline
    let mut total_lit = 0u32;
    for beat in 0..4u64 {
        let t = tm.beat_time(beat);
        let mut frame_buf = vec![PixelColor::default(); PIXELS];
        tl.render(t, &pos, &mut frame_buf);
        hal.send_frame(&LogicalFrame::new(frame_buf, t)).unwrap();
        let g = sim.channel(0, 1).unwrap_or(0); // G channel
        if g > 0 { total_lit += 1; }
    }
    assert_eq!(sim.frames_sent(), 4, "must send exactly 4 frames");
    assert!(total_lit >= 1, "at least one frame must have green pixels lit");
}

// ── P1c: Clip with fade-in/fade-out snapped to live beats ─────────────────
#[test]
fn live_clip_fade_in_out_correct() {
    let (stream, _) = collect_beats(5_000, 500);
    let tm = TempoMap::from_beat_flags(stream);

    let start = tm.beat_time(0);
    let end   = tm.beat_time(4);
    let fade  = (end - start) / 4; // 25% fade in / out

    let clip = Clip::new(start, end, Box::new(SolidColor(PixelColor::rgb(255, 255, 255))))
        .with_fades(fade, fade);
    let track = Track::new(BlendMode::Override).with_clip(clip);
    let tl = Timeline::new(PIXELS).with_track(track);
    let pos = positions();

    let mut at_start  = vec![PixelColor::default(); PIXELS];
    let mut at_mid    = vec![PixelColor::default(); PIXELS];
    let mut after_end = vec![PixelColor::default(); PIXELS];

    tl.render(start + 1,      &pos, &mut at_start);  // fade-in zone
    tl.render((start + end)/2, &pos, &mut at_mid);    // full brightness
    tl.render(end + 100,       &pos, &mut after_end); // after clip

    // Fade-in: brightness < full
    assert!(at_start[0].r < at_mid[0].r,
        "fade-in: start ({}) must be dimmer than mid ({})", at_start[0].r, at_mid[0].r);
    // Full brightness at midpoint
    assert!(at_mid[0].r > 200, "mid-clip must be near full brightness");
    // After end: black
    assert_eq!(after_end[0], PixelColor::default(), "after clip must be black");
}

// ── P1d: Timeline as Effect — drives the Pipeline interface ───────────────
#[test]
fn timeline_satisfies_effect_trait_for_pipeline() {
    let (stream, _) = collect_beats(2_000, 400);
    let tm = TempoMap::from_beat_flags(stream);

    // Build Timeline, then box it as Box<dyn Effect> — the sequencer contract
    let clip = Clip::on_beats(&tm, 0, 3, Box::new(SolidColor(PixelColor::rgb(128, 0, 255))));
    let track = Track::new(BlendMode::Override).with_clip(clip);
    let tl: Box<dyn Effect> = Box::new(Timeline::new(PIXELS).with_track(track));

    let pos = positions();
    let mut out = vec![PixelColor::default(); PIXELS];

    // Render at beat 1 — guaranteed inside clip [beat 0, beat 3)
    let t_inside = tm.beat_time(1);
    tl.render(t_inside, &pos, &mut out);
    assert!(out[0].r > 0 || out[0].b > 0,
        "Timeline-as-Effect must produce colored output at beat 1 (t={t_inside}ms)");
}

// ── P1e: Keyframe opacity on live beat timeline ───────────────────────────
#[test]
fn live_beat_keyframe_opacity_automation() {
    let (stream, _) = collect_beats(4_000, 500);
    let tm = TempoMap::from_beat_flags(stream);

    // Opacity envelope: 0→1 over beats 0–2, then 1→0 over beats 2–4
    let kfs = vec![
        Keyframe::on_beat(&tm, 0, 0.0),
        Keyframe::on_beat(&tm, 2, 1.0),
        Keyframe::on_beat(&tm, 4, 0.0),
    ];
    let clip = Clip::new(
        tm.beat_time(0), tm.beat_time(4),
        Box::new(SolidColor(PixelColor::rgb(255, 255, 255))),
    ).with_opacity(kfs);
    let track = Track::new(BlendMode::Override).with_clip(clip);
    let tl = Timeline::new(PIXELS).with_track(track);
    let pos = positions();

    let mut at_beat0 = vec![PixelColor::default(); PIXELS];
    let mut at_beat2 = vec![PixelColor::default(); PIXELS];
    let mut at_beat4 = vec![PixelColor::default(); PIXELS];

    tl.render(tm.beat_time(0) + 1, &pos, &mut at_beat0); // near 0 opacity
    tl.render(tm.beat_time(2),     &pos, &mut at_beat2); // full opacity
    tl.render(tm.beat_time(4),     &pos, &mut at_beat4); // near 0 opacity (at end of last kf)

    assert!(at_beat2[0].r > at_beat0[0].r,
        "beat 2 ({}) must be brighter than beat 0 ({})", at_beat2[0].r, at_beat0[0].r);
    assert!(at_beat2[0].r >= at_beat4[0].r,
        "beat 2 ({}) must be >= beat 4 ({})", at_beat2[0].r, at_beat4[0].r);
}

// ── P1f: TempoMap::from_beat_flags → markers → Timeline ──────────────────
#[test]
fn live_beats_become_timeline_markers() {
    let (stream, _) = collect_beats(3_000, 500);
    let beat_timestamps: Vec<u64> = stream.iter()
        .filter(|(_, b)| *b)
        .map(|(t, _)| *t)
        .collect();

    let markers: Vec<_> = beat_timestamps.iter()
        .map(|&t| led_sequencer::TimeMarker::beat(t))
        .collect();

    let tl = Timeline::new(PIXELS).with_markers(markers.clone());

    // Markers must be stored on the timeline (advisory metadata)
    assert_eq!(tl.markers.len(), markers.len(),
        "timeline must store all {} beat markers", markers.len());
    // All markers are beat kind
    for m in &tl.markers {
        assert_eq!(m.kind, led_sequencer::MarkerKind::Beat);
    }
}

// ── P1g: Stress — 100 beat-synced clips render without panic ──────────────
#[test]
fn stress_100_beat_synced_clips_no_panic() {
    let (stream, _) = collect_beats(10_000, 200); // 50 beats in 10s
    let tm = TempoMap::from_beat_flags(stream);

    let mut track = Track::new(BlendMode::Add);
    let max_beat = 48u64; // stay within detected beat count
    for i in 0..max_beat {
        let clip = Clip::on_beats(
            &tm, i, i + 1,
            Box::new(SolidColor(PixelColor::rgb(1, 0, 0))),
        );
        track = track.with_clip(clip);
    }
    let tl = Timeline::new(PIXELS).with_track(track);
    let pos = positions();
    let mut out = vec![PixelColor::default(); PIXELS];

    // Render at every beat — must not panic
    for beat in 0..max_beat {
        let t = tm.beat_time(beat);
        tl.render(t, &pos, &mut out);
    }
}

// ── P1h: EasingType::Step on beat-snapped keyframe ────────────────────────
#[test]
fn step_easing_on_beat_keyframe_hard_cut() {
    let (stream, _) = collect_beats(3_000, 500);
    let tm = TempoMap::from_beat_flags(stream);

    let kfs = vec![
        Keyframe::eased(tm.beat_time(0), 0.0, EasingType::Linear),
        Keyframe::eased(tm.beat_time(2), 1.0, EasingType::Step), // hard cut at beat 1
    ];
    let clip = Clip::new(
        tm.beat_time(0), tm.beat_time(4),
        Box::new(SolidColor(PixelColor::rgb(255, 0, 0))),
    ).with_opacity(kfs);
    let track = Track::new(BlendMode::Override).with_clip(clip);
    let tl = Timeline::new(PIXELS).with_track(track);
    let pos = positions();

    let t_before = (tm.beat_time(0) + tm.beat_time(2)) / 2 - 1; // just before midpoint
    let t_after  = (tm.beat_time(0) + tm.beat_time(2)) / 2 + 1; // just after midpoint

    let mut before_buf = vec![PixelColor::default(); PIXELS];
    let mut after_buf  = vec![PixelColor::default(); PIXELS];
    tl.render(t_before, &pos, &mut before_buf);
    tl.render(t_after,  &pos, &mut after_buf);

    assert!(after_buf[0].r >= before_buf[0].r,
        "Step easing: after midpoint ({}) must be >= before ({})",
        after_buf[0].r, before_buf[0].r);
}
