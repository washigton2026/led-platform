//! The timeline data model. **Non-destructive**: clips and keyframes describe the show;
//! composing a frame never mutates them, so undo/redo and re-rendering are trivial.

use led_pixel_engine::Effect;

use crate::tempo::TempoMap;

// ─── EasingType ──────────────────────────────────────────────────────────────

/// Curve applied to the interpolation factor when moving *toward* a keyframe.
///
/// Convention: the easing field lives on the **destination** keyframe — it describes how
/// you *arrive* at that value, not how you leave the previous one. Same convention as most
/// DAWs (e.g. After Effects graph editor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EasingType {
    /// Constant rate of change. Default.
    Linear,
    /// Slow start, fast end (cubic `t³`).
    EaseIn,
    /// Fast start, slow end (cubic `1-(1-t)³`).
    EaseOut,
    /// Slow start and end, fast middle (cubic S-curve).
    EaseInOut,
    /// Instant jump: 0 below the midpoint, 1 at/above. "Hard cut" between keyframes.
    Step,
}

impl Default for EasingType {
    fn default() -> Self {
        Self::Linear
    }
}

impl EasingType {
    /// Map raw interpolation factor `t ∈ [0,1]` through this easing curve.
    #[inline]
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear    => t,
            Self::EaseIn    => t * t * t,
            Self::EaseOut   => 1.0 - (1.0 - t).powi(3),
            Self::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0_f32).powi(3) * 0.5
                }
            }
            Self::Step      => if t >= 0.5 { 1.0 } else { 0.0 },
        }
    }
}

// ─── Keyframe ────────────────────────────────────────────────────────────────

/// One automation point for a scalar (e.g. opacity), value in 0..1.
///
/// `easing` is applied when interpolating **from the previous keyframe toward this one**.
#[derive(Clone, Copy, Debug)]
pub struct Keyframe {
    pub time_ms: u64,
    pub value:   f32,
    pub easing:  EasingType,
}

impl Keyframe {
    /// Linear keyframe (most common). Equivalent to `eased(.., Linear)`.
    pub fn new(time_ms: u64, value: f32) -> Self {
        Self { time_ms, value, easing: EasingType::Linear }
    }

    /// Keyframe with an explicit easing curve arriving at `value`.
    pub fn eased(time_ms: u64, value: f32, easing: EasingType) -> Self {
        Self { time_ms, value, easing }
    }

    /// Automation point placed on beat `beat` of the tempo grid (linear easing).
    pub fn on_beat(tempo: &TempoMap, beat: u64, value: f32) -> Self {
        Self::new(tempo.beat_time(beat), value)
    }
}

// ─── BlendMode ───────────────────────────────────────────────────────────────

/// How a track's clips blend onto the accumulated frame (per channel, in linear space).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    /// Alpha-over: `out = src·a + out·(1-a)`. Supports clean crossfades.
    Override,
    /// Additive: `out = min(255, out + src·a)`. Good for glow / flash layers.
    Add,
    /// Multiplicative, mixed by alpha: `out = lerp(out, out·src/255, a)`. Masks / tints.
    Multiply,
}

// ─── Clip ────────────────────────────────────────────────────────────────────

/// A scheduled effect on the timeline: `[start_ms, end_ms)` with optional fades and
/// opacity automation. The effect is evaluated at *clip-local* time (so it always starts
/// at t=0 when the clip begins), while opacity keyframes are at *timeline* time.
pub struct Clip {
    pub start_ms:    u64,
    pub end_ms:      u64,
    pub effect:      Box<dyn Effect>,
    pub fade_in_ms:  u64,
    pub fade_out_ms: u64,
    /// Opacity keyframes (0..1). Empty ⇒ constant 1.0.
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

    /// A clip spanning beats `[start_beat, end_beat)`, timings resolved to ms at build time.
    pub fn on_beats(tempo: &TempoMap, start_beat: u64, end_beat: u64, effect: Box<dyn Effect>) -> Self {
        Self::new(tempo.beat_time(start_beat), tempo.beat_time(end_beat), effect)
    }

    /// A clip whose start/end are snapped to the nearest beat of the tempo grid.
    pub fn snapped(tempo: &TempoMap, start_ms: u64, end_ms: u64, effect: Box<dyn Effect>) -> Self {
        Self::new(tempo.snap(start_ms), tempo.snap(end_ms), effect)
    }
}

// ─── Track ───────────────────────────────────────────────────────────────────

/// A layer of clips that share a blend mode. Tracks compose bottom → top.
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

// ─── TimeMarker ──────────────────────────────────────────────────────────────

/// What kind of event a marker flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    /// A metronome beat (from a TempoMap or AudioFeatures beat flag).
    Beat,
    /// A structural section boundary (verse, chorus, bridge, …).
    Section,
    /// A transient onset detected by led-audio.
    Onset,
    /// User-defined cue point.
    Cue,
}

/// A labelled timestamp on the timeline — beats/sections from led-audio, or manual cues.
///
/// Markers are **advisory metadata** for the editor / AI designer: they drive snap
/// operations and clip placement, but are never consulted on the render hot-path.
#[derive(Clone, Debug)]
pub struct TimeMarker {
    pub time_ms: u64,
    pub kind:    MarkerKind,
    /// Human-readable label (e.g. "Chorus", "Beat 8"). `None` for unlabelled beats.
    pub label: Option<String>,
}

impl TimeMarker {
    pub fn beat(time_ms: u64) -> Self {
        Self { time_ms, kind: MarkerKind::Beat, label: None }
    }

    pub fn section(time_ms: u64, label: impl Into<String>) -> Self {
        Self { time_ms, kind: MarkerKind::Section, label: Some(label.into()) }
    }

    pub fn cue(time_ms: u64, label: impl Into<String>) -> Self {
        Self { time_ms, kind: MarkerKind::Cue, label: Some(label.into()) }
    }

    pub fn onset(time_ms: u64) -> Self {
        Self { time_ms, kind: MarkerKind::Onset, label: None }
    }
}
