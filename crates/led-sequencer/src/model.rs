//! The timeline data model. **Non-destructive**: clips and keyframes describe the show;
//! composing a frame never mutates them, so undo/redo and re-rendering are trivial.

use led_pixel_engine::Effect;

use crate::tempo::TempoMap;

/// How a track's clips blend onto the accumulated frame (per channel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    /// Alpha-over: `out = src*a + out*(1-a)`. The default; supports crossfades.
    Override,
    /// Additive: `out = min(255, out + src*a)`.
    Add,
    /// Multiplicative, mixed by alpha: `out = lerp(out, out*src/255, a)`.
    Multiply,
}

/// One automation point for a scalar (e.g. opacity), value in 0..1.
#[derive(Clone, Copy, Debug)]
pub struct Keyframe {
    pub time_ms: u64,
    pub value: f32,
}

/// A scheduled effect: `[start_ms, end_ms)` with optional fades and opacity automation.
pub struct Clip {
    pub start_ms: u64,
    pub end_ms: u64,
    pub effect: Box<dyn Effect>,
    pub fade_in_ms: u64,
    pub fade_out_ms: u64,
    /// Opacity keyframes (0..1), linearly interpolated. Empty ⇒ constant 1.0.
    pub opacity: Vec<Keyframe>,
}

impl Clip {
    pub fn new(start_ms: u64, end_ms: u64, effect: Box<dyn Effect>) -> Self {
        debug_assert!(end_ms > start_ms, "clip end must be after start");
        Self { start_ms, end_ms, effect, fade_in_ms: 0, fade_out_ms: 0, opacity: Vec::new() }
    }
    pub fn with_fades(mut self, fade_in_ms: u64, fade_out_ms: u64) -> Self {
        self.fade_in_ms = fade_in_ms;
        self.fade_out_ms = fade_out_ms;
        self
    }
    pub fn with_opacity(mut self, kfs: Vec<Keyframe>) -> Self {
        self.opacity = kfs;
        self
    }

    /// A clip spanning beats `[start_beat, end_beat)` of the tempo grid (resolved to ms now).
    pub fn on_beats(tempo: &TempoMap, start_beat: u64, end_beat: u64, effect: Box<dyn Effect>) -> Self {
        Self::new(tempo.beat_time(start_beat), tempo.beat_time(end_beat), effect)
    }

    /// A clip whose start/end are snapped to the nearest beat of the tempo grid.
    pub fn snapped(tempo: &TempoMap, start_ms: u64, end_ms: u64, effect: Box<dyn Effect>) -> Self {
        Self::new(tempo.snap(start_ms), tempo.snap(end_ms), effect)
    }
}

impl Keyframe {
    /// An automation point placed on beat `beat` of the tempo grid.
    pub fn on_beat(tempo: &TempoMap, beat: u64, value: f32) -> Self {
        Self { time_ms: tempo.beat_time(beat), value }
    }
}

/// A layer of clips that share a blend mode. Tracks compose bottom→top.
pub struct Track {
    pub clips: Vec<Clip>,
    pub blend: BlendMode,
}

impl Track {
    pub fn new(blend: BlendMode) -> Self {
        Self { clips: Vec::new(), blend }
    }
    pub fn with_clip(mut self, clip: Clip) -> Self {
        self.clips.push(clip);
        self
    }
}
