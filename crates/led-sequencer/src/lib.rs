//! # led-sequencer — arranging effects in time
//!
//! A non-destructive [`Timeline`] of [`Track`]s and [`Clip`]s that composes into a
//! `LogicalFrame` at any time `t`. The timeline **is an [`Effect`](led_pixel_engine::Effect)**,
//! so the render→send pipeline drives it like any other effect — the sequencer sits one
//! layer above the engine and produces frames in logical space.
//!
//! - [`model`] — `Timeline`/`Track`/`Clip`/`Keyframe`/`BlendMode`/`EasingType`/`TimeMarker`
//! - [`timeline`] — the composition algorithm: fades, opacity keyframes (with easing),
//!   blend modes, marker metadata.

pub mod model;
pub mod tempo;
pub mod timeline;

pub use model::{BlendMode, Clip, EasingType, Keyframe, MarkerKind, TimeMarker, Track};
pub use tempo::TempoMap;
pub use timeline::Timeline;

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::PixelColor;
    use led_pixel_engine::{Effect, SolidColor, Vec3};

    const N: usize = 4;

    fn render_at(tl: &Timeline, t: u64) -> Vec<PixelColor> {
        let mut out = vec![PixelColor::default(); N];
        tl.render(t, &[Vec3::ZERO; N], &mut out);
        out
    }

    fn solid(c: PixelColor) -> Box<dyn Effect> {
        Box::new(SolidColor(c))
    }

    // ── Scheduling ───────────────────────────────────────────────────────────

    #[test]
    fn clips_schedule_in_time_with_gaps() {
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, 100, solid(PixelColor::rgb(255, 0, 0))))
                .with_clip(Clip::new(200, 300, solid(PixelColor::rgb(0, 255, 0)))),
        );
        assert_eq!(render_at(&tl,  50)[0], PixelColor::rgb(255, 0, 0), "inside clip 1");
        assert_eq!(render_at(&tl, 150)[0], PixelColor::rgb(0, 0, 0),   "gap is black");
        assert_eq!(render_at(&tl, 250)[0], PixelColor::rgb(0, 255, 0), "inside clip 2");
        assert_eq!(render_at(&tl, 999)[0], PixelColor::rgb(0, 0, 0),   "after the end is black");
    }

    #[test]
    fn crossfade_between_overlapping_clips() {
        // Red fading out; green fading in. At the midpoint both are ~half.
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, 1000, solid(PixelColor::rgb(255, 0, 0))).with_fades(0, 1000))
                .with_clip(Clip::new(0, 1000, solid(PixelColor::rgb(0, 255, 0))).with_fades(1000, 0)),
        );
        let mid = render_at(&tl, 500)[0];
        assert!(mid.r > 0 && mid.g > 0, "both colors present mid-crossfade: {mid:?}");
        assert!(mid.r < 200 && mid.g < 200, "neither at full intensity: {mid:?}");
    }

    // ── Blend modes ──────────────────────────────────────────────────────────

    #[test]
    fn add_blend_sums_tracks() {
        let tl = Timeline::new(N)
            .with_track(Track::new(BlendMode::Override).with_clip(Clip::new(0, 100, solid(PixelColor::rgb(255, 0, 0)))))
            .with_track(Track::new(BlendMode::Add)     .with_clip(Clip::new(0, 100, solid(PixelColor::rgb(0, 255, 0)))));
        assert_eq!(render_at(&tl, 50)[0], PixelColor::rgb(255, 255, 0), "red + green = yellow");
    }

    #[test]
    fn multiply_blend_masks() {
        let tl = Timeline::new(N)
            .with_track(Track::new(BlendMode::Override) .with_clip(Clip::new(0, 100, solid(PixelColor::rgb(255, 255, 255)))))
            .with_track(Track::new(BlendMode::Multiply) .with_clip(Clip::new(0, 100, solid(PixelColor::rgb(255, 0, 0)))));
        assert_eq!(render_at(&tl, 50)[0], PixelColor::rgb(255, 0, 0), "white × red = red");
    }

    // ── Opacity keyframes ────────────────────────────────────────────────────

    #[test]
    fn opacity_keyframes_interpolate_linear() {
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override).with_clip(
                Clip::new(0, 1000, solid(PixelColor::rgb(200, 0, 0))).with_opacity(vec![
                    Keyframe::new(0, 0.0),
                    Keyframe::new(1000, 1.0),
                ]),
            ),
        );
        assert_eq!(render_at(&tl, 500)[0], PixelColor::rgb(100, 0, 0), "opacity 0.5 at midpoint");
    }

    // ── EasingType ───────────────────────────────────────────────────────────

    #[test]
    fn easing_step_hard_cuts_at_midpoint() {
        // Opacity 0→1 with Step easing on destination keyframe at 1000 ms.
        // raw factor at t=499 → f=0.499 → Step.apply < 0.5 → 0.0 → black
        // raw factor at t=501 → f=0.501 → Step.apply >= 0.5 → 1.0 → full
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override).with_clip(
                Clip::new(0, 1000, solid(PixelColor::rgb(200, 0, 0))).with_opacity(vec![
                    Keyframe::new(0, 0.0),
                    Keyframe::eased(1000, 1.0, EasingType::Step),
                ]),
            ),
        );
        assert_eq!(render_at(&tl, 499)[0].r, 0,   "Step: before midpoint → 0 opacity");
        assert_eq!(render_at(&tl, 501)[0].r, 200, "Step: past midpoint → full opacity");
    }

    #[test]
    fn easing_ease_in_out_is_s_shaped() {
        let e = EasingType::EaseInOut;
        assert!(e.apply(0.1)  < 0.1, "slow start");
        assert!((e.apply(0.5) - 0.5).abs() < 1e-5, "symmetric midpoint");
        assert!(e.apply(0.9)  > 0.9, "slow end (mirror of start)");
    }

    #[test]
    fn easing_ease_in_is_slow_at_start() {
        let e = EasingType::EaseIn;
        assert!(e.apply(0.0) < 0.001);
        assert!(e.apply(0.5) < 0.5,   "cubic: midpoint below linear");
        assert!((e.apply(1.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn easing_ease_out_is_slow_at_end() {
        let e = EasingType::EaseOut;
        assert!(e.apply(0.5) > 0.5, "cubic ease-out: midpoint above linear");
        assert!((e.apply(1.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn easing_applied_to_opacity_keyframe_changes_midpoint_value() {
        // EaseIn: f(0.5) ≈ 0.125 (t^3). Opacity at midpoint should be ~0.125, not 0.5.
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override).with_clip(
                Clip::new(0, 1000, solid(PixelColor::rgb(200, 0, 0))).with_opacity(vec![
                    Keyframe::new(0, 0.0),
                    Keyframe::eased(1000, 1.0, EasingType::EaseIn),
                ]),
            ),
        );
        let mid = render_at(&tl, 500)[0];
        // Linear midpoint would be rgb(100,0,0). EaseIn makes it dimmer.
        assert!(mid.r < 100, "EaseIn opacity at midpoint is below linear: {}", mid.r);
        assert!(mid.r > 0,   "but not zero (EaseIn(0.5) = 0.125 → rgb(25,0,0) approx)");
    }

    // ── Beat-sync ────────────────────────────────────────────────────────────

    #[test]
    fn clips_snap_to_the_beat_grid() {
        let tempo = TempoMap::constant(120.0, 0); // 500 ms / beat

        // A clip on beats 2..4 ⇒ 1000..2000 ms.
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::on_beats(&tempo, 2, 4, solid(PixelColor::rgb(255, 0, 0)))),
        );
        assert_eq!(render_at(&tl,  500)[0], PixelColor::rgb(0, 0, 0),   "before beat 2");
        assert_eq!(render_at(&tl, 1500)[0], PixelColor::rgb(255, 0, 0), "inside beats 2..4");
        assert_eq!(render_at(&tl, 2500)[0], PixelColor::rgb(0, 0, 0),   "after beat 4");

        // A loosely-timed clip snapped to the grid: 480..1010 ⇒ 500..1000 ms.
        let tl2 = Timeline::new(N).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::snapped(&tempo, 480, 1010, solid(PixelColor::rgb(0, 255, 0)))),
        );
        assert_eq!(render_at(&tl2,  400)[0], PixelColor::rgb(0, 0, 0),   "before snapped start (500)");
        assert_eq!(render_at(&tl2,  700)[0], PixelColor::rgb(0, 255, 0), "inside snapped clip");
        assert_eq!(render_at(&tl2, 1000)[0], PixelColor::rgb(0, 0, 0),   "at snapped end (1000, exclusive)");
    }

    #[test]
    fn keyframes_can_be_placed_on_beats() {
        let tempo = TempoMap::constant(120.0, 0); // beat 0 = 0 ms, beat 2 = 1000 ms
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override).with_clip(
                Clip::new(0, 2000, solid(PixelColor::rgb(200, 0, 0))).with_opacity(vec![
                    Keyframe::on_beat(&tempo, 0, 0.0),
                    Keyframe::on_beat(&tempo, 2, 1.0),
                ]),
            ),
        );
        assert_eq!(render_at(&tl, 500)[0], PixelColor::rgb(100, 0, 0), "opacity 0.5 a quarter through");
    }

    #[test]
    fn beat_grid_from_detected_beats_drives_clips() {
        let tempo = TempoMap::from_beat_flags([
            (0u64, true), (250, false), (500, true), (1000, true), (1500, true),
        ]);
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::on_beats(&tempo, 1, 3, solid(PixelColor::rgb(10, 20, 30)))),
        );
        assert_eq!(render_at(&tl,  400)[0], PixelColor::rgb(0, 0, 0));
        assert_eq!(render_at(&tl,  800)[0], PixelColor::rgb(10, 20, 30), "active across detected beats");
        assert_eq!(render_at(&tl, 1600)[0], PixelColor::rgb(0, 0, 0));
    }

    // ── TimeMarker ───────────────────────────────────────────────────────────

    #[test]
    fn time_markers_store_metadata_and_are_not_rendered() {
        let markers = vec![
            TimeMarker::beat(0),
            TimeMarker::beat(500),
            TimeMarker::section(1000, "Chorus"),
            TimeMarker::cue(1500, "FX trigger"),
        ];
        let tl = Timeline::new(N)
            .with_duration(4000)
            .with_markers(markers)
            .with_track(Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, 500, solid(PixelColor::rgb(99, 0, 0)))));

        // Markers don't affect rendering.
        assert_eq!(render_at(&tl, 250)[0], PixelColor::rgb(99, 0, 0), "clip active inside marker range");
        assert_eq!(render_at(&tl, 750)[0], PixelColor::rgb(0, 0, 0),  "gap is black despite markers");

        // Metadata is accessible.
        assert_eq!(tl.markers.len(), 4);
        assert_eq!(tl.duration_ms, Some(4000));
        assert_eq!(tl.markers[2].label.as_deref(), Some("Chorus"));
        assert_eq!(tl.markers[2].kind, MarkerKind::Section);
    }

    #[test]
    fn time_markers_built_from_tempo_map() {
        let tempo = TempoMap::constant(120.0, 0); // 500 ms/beat
        let beat_markers: Vec<TimeMarker> = (0u64..4)
            .map(|b| TimeMarker::beat(tempo.beat_time(b)))
            .collect();
        assert_eq!(beat_markers[2].time_ms, 1000, "beat 2 at 1000 ms @ 120 BPM");
        assert_eq!(beat_markers[3].kind, MarkerKind::Beat);
    }

    // ── Non-destructive guarantee ────────────────────────────────────────────

    #[test]
    fn rendering_is_non_destructive() {
        let tl = Timeline::new(N).with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, 1000, solid(PixelColor::rgb(123, 45, 67))).with_fades(200, 200)),
        );
        let first = render_at(&tl, 500);
        let _ = render_at(&tl, 900);
        let _ = render_at(&tl, 100);
        let again = render_at(&tl, 500);
        assert_eq!(first, again, "same time ⇒ same frame; timeline unchanged");
    }
}
