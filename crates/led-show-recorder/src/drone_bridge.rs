//! Drone Timeline Bridge — maps LED `MusicalSection` events to drone formation annotations.
//!
//! ## Purpose
//!
//! A LUMYX show has two layers:
//! 1. **LED layer**: pixels driven by `AudioFeatures` (section, beat, instrument).
//! 2. **Drone layer**: formations driven by `ShowIntent` / coreography.
//!
//! This module converts the LED timeline (stream of `SectionEvent`s) into a
//! `DroneAnnotation` list — a compact, human-readable mapping from section to
//! drone formation style. The drone software then uses these annotations to
//! select and schedule formations.
//!
//! ## Design (lumyx-ai-governor invariant)
//!
//! - No drone coordinates are produced here — only **intent labels**.
//! - The drone coreography engine converts labels to waypoints (never this bridge).
//! - Mapping is deterministic: same section stream → same annotations.
//! - Bridge is std-only, zero-dep — safe to use in LED and drone workspaces.
//!
//! ## Invariants
//! - Every `SectionEvent` maps to exactly one `DroneFormationHint`.
//! - `DroneAnnotation` timeline is sorted by `start_ms` (always).
//! - Minimum annotation duration is 1000ms (sections shorter than that are merged).

use led_core::MusicalSection;

// ── SectionEvent ──────────────────────────────────────────────────────────────

/// One section change event in the LED show timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionEvent {
    pub start_ms:  u64,
    pub end_ms:    u64,
    pub section:   MusicalSection,
    /// Average RMS energy during this section (used to select intensity of drone formation).
    pub avg_energy: f32,
}

// ── DroneFormationHint ────────────────────────────────────────────────────────

/// Hint for the drone coreography engine — intent only, no waypoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroneFormationHint {
    /// Tight grid — structured, calm.
    Grid,
    /// Ring / circle — open, welcoming.
    Ring,
    /// Radial burst — high energy, expansive.
    Burst,
    /// Descent / landing hold — outro, quiet.
    Descend,
    /// Rise / ascent — building energy.
    Rise,
    /// Wave pattern — melodic, flowing.
    Wave,
    /// Hold current formation — transition or silence.
    Hold,
}

impl DroneFormationHint {
    /// Derive a formation hint from a musical section + energy level.
    pub fn from_section(section: MusicalSection, energy: f32) -> Self {
        match section {
            MusicalSection::Intro   => Self::Rise,
            MusicalSection::Verse   => if energy > 0.4 { Self::Grid } else { Self::Wave },
            MusicalSection::Build   => Self::Rise,
            MusicalSection::Chorus  => if energy > 0.6 { Self::Burst } else { Self::Ring },
            MusicalSection::Drop    => Self::Burst,
            MusicalSection::Bridge  => Self::Wave,
            MusicalSection::Outro   => Self::Descend,
            MusicalSection::Unknown => Self::Hold,
        }
    }

    /// Human-readable label (for export, logging, drone coreography engine).
    pub fn label(self) -> &'static str {
        match self {
            Self::Grid    => "grid",
            Self::Ring    => "ring",
            Self::Burst   => "burst",
            Self::Descend => "descend",
            Self::Rise    => "rise",
            Self::Wave    => "wave",
            Self::Hold    => "hold",
        }
    }
}

// ── DroneAnnotation ───────────────────────────────────────────────────────────

/// One annotation in the drone timeline: a time range with a formation hint.
#[derive(Clone, Debug, PartialEq)]
pub struct DroneAnnotation {
    pub start_ms:  u64,
    pub end_ms:    u64,
    pub hint:      DroneFormationHint,
    pub section:   MusicalSection,
    pub avg_energy: f32,
}

impl DroneAnnotation {
    pub fn duration_ms(&self) -> u64 { self.end_ms.saturating_sub(self.start_ms) }

    /// JSON line for export to the drone coreography engine.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"start_ms":{s},"end_ms":{e},"formation":"{f}","section":"{sec:?}","energy":{en:.3}}}"#,
            s   = self.start_ms,
            e   = self.end_ms,
            f   = self.hint.label(),
            sec = self.section,
            en  = self.avg_energy,
        )
    }
}

// ── DroneBridge ───────────────────────────────────────────────────────────────

/// Converts a stream of `SectionEvent`s into a `DroneAnnotation` timeline.
pub struct DroneBridge {
    /// Minimum annotation duration (ms). Events shorter than this are merged with adjacent.
    pub min_duration_ms: u64,
}

impl Default for DroneBridge {
    fn default() -> Self { Self { min_duration_ms: 1_000 } }
}

impl DroneBridge {
    pub fn new(min_duration_ms: u64) -> Self { Self { min_duration_ms } }

    /// Build the annotation timeline from a sorted list of `SectionEvent`s.
    /// Returns annotations sorted by `start_ms`, merged if below `min_duration_ms`.
    pub fn build(&self, events: &[SectionEvent]) -> Vec<DroneAnnotation> {
        if events.is_empty() { return vec![]; }

        // Convert to annotations
        let mut annotations: Vec<DroneAnnotation> = events.iter().map(|e| {
            DroneAnnotation {
                start_ms:  e.start_ms,
                end_ms:    e.end_ms,
                hint:      DroneFormationHint::from_section(e.section, e.avg_energy),
                section:   e.section,
                avg_energy: e.avg_energy,
            }
        }).collect();

        // Sort by start time
        annotations.sort_by_key(|a| a.start_ms);

        // Merge short annotations into the previous
        let min = self.min_duration_ms;
        let mut merged: Vec<DroneAnnotation> = Vec::with_capacity(annotations.len());
        for ann in annotations {
            match merged.last_mut() {
                Some(prev) if ann.duration_ms() < min => {
                    // Merge: extend previous annotation
                    prev.end_ms = ann.end_ms;
                    // Use avg energy of longer segment
                    prev.avg_energy = (prev.avg_energy + ann.avg_energy) / 2.0;
                }
                _ => merged.push(ann),
            }
        }

        merged
    }

    /// Export the annotation timeline as a JSON array.
    pub fn export_json(&self, annotations: &[DroneAnnotation]) -> String {
        let lines: Vec<String> = annotations.iter().map(|a| a.to_json()).collect();
        format!("[{}]", lines.join(","))
    }

    /// Build a synchronised LED+Drone timeline from a `SectionEvent` stream.
    /// Returns (led_annotations, drone_annotations) — same time axis.
    pub fn build_synced(
        &self,
        events: &[SectionEvent],
    ) -> (Vec<DroneAnnotation>, Vec<DroneAnnotation>) {
        let annotations = self.build(events);
        // LED annotations are the same segments with the LED-appropriate hint
        let led_annotations = annotations.clone();
        // Drone annotations may use different formation hints for visual contrast
        let drone_annotations = annotations.iter().map(|a| {
            // For a chorus, drones might choose Ring while LEDs go Burst
            let drone_hint = match a.section {
                MusicalSection::Chorus => DroneFormationHint::Ring, // contrast with LED burst
                MusicalSection::Drop   => DroneFormationHint::Burst, // same high energy
                _                      => a.hint,
            };
            DroneAnnotation { hint: drone_hint, ..a.clone() }
        }).collect();
        (led_annotations, drone_annotations)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::MusicalSection;

    fn event(start: u64, end: u64, section: MusicalSection, energy: f32) -> SectionEvent {
        SectionEvent { start_ms: start, end_ms: end, section, avg_energy: energy }
    }

    // ── FormationHint ─────────────────────────────────────────────────────────

    #[test]
    fn drop_maps_to_burst() {
        assert_eq!(
            DroneFormationHint::from_section(MusicalSection::Drop, 0.9),
            DroneFormationHint::Burst
        );
    }

    #[test]
    fn intro_maps_to_rise() {
        assert_eq!(
            DroneFormationHint::from_section(MusicalSection::Intro, 0.2),
            DroneFormationHint::Rise
        );
    }

    #[test]
    fn outro_maps_to_descend() {
        assert_eq!(
            DroneFormationHint::from_section(MusicalSection::Outro, 0.1),
            DroneFormationHint::Descend
        );
    }

    #[test]
    fn low_energy_verse_maps_to_wave() {
        assert_eq!(
            DroneFormationHint::from_section(MusicalSection::Verse, 0.2),
            DroneFormationHint::Wave
        );
    }

    #[test]
    fn high_energy_verse_maps_to_grid() {
        assert_eq!(
            DroneFormationHint::from_section(MusicalSection::Verse, 0.7),
            DroneFormationHint::Grid
        );
    }

    #[test]
    fn all_sections_have_labels() {
        let sections = [
            MusicalSection::Intro, MusicalSection::Verse, MusicalSection::Chorus,
            MusicalSection::Build, MusicalSection::Bridge, MusicalSection::Drop,
            MusicalSection::Outro, MusicalSection::Unknown,
        ];
        for s in &sections {
            let hint = DroneFormationHint::from_section(*s, 0.5);
            let label = hint.label();
            assert!(!label.is_empty(), "all sections must have non-empty labels");
        }
    }

    // ── DroneBridge ───────────────────────────────────────────────────────────

    #[test]
    fn empty_events_returns_empty() {
        let b = DroneBridge::default();
        assert!(b.build(&[]).is_empty());
    }

    #[test]
    fn single_event_becomes_single_annotation() {
        let b = DroneBridge::default();
        let result = b.build(&[event(0, 5_000, MusicalSection::Verse, 0.5)]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].section, MusicalSection::Verse);
        assert_eq!(result[0].start_ms, 0);
        assert_eq!(result[0].end_ms, 5_000);
    }

    #[test]
    fn annotations_sorted_by_start_ms() {
        let b = DroneBridge::default();
        let events = vec![
            event(5_000, 10_000, MusicalSection::Chorus, 0.8),
            event(0,     5_000,  MusicalSection::Intro,  0.2),
        ];
        let result = b.build(&events);
        assert_eq!(result[0].start_ms, 0,     "first must be Intro");
        assert_eq!(result[1].start_ms, 5_000, "second must be Chorus");
    }

    #[test]
    fn short_events_merged_with_previous() {
        let b = DroneBridge::new(2_000); // 2s minimum
        let events = vec![
            event(0,     5_000,  MusicalSection::Verse,  0.5),
            event(5_000, 5_500,  MusicalSection::Build,  0.6), // only 500ms → merge
            event(5_500, 10_000, MusicalSection::Chorus, 0.8),
        ];
        let result = b.build(&events);
        assert_eq!(result.len(), 2, "short Build section must be merged");
        assert_eq!(result[0].end_ms, 5_500, "merged annotation extends to 5500");
    }

    #[test]
    fn long_events_not_merged() {
        let b = DroneBridge::new(1_000);
        let events = vec![
            event(0,      5_000, MusicalSection::Intro,  0.2),
            event(5_000, 10_000, MusicalSection::Chorus, 0.8),
            event(10_000, 15_000, MusicalSection::Outro,  0.1),
        ];
        let result = b.build(&events);
        assert_eq!(result.len(), 3, "no merging needed for long sections");
    }

    // ── JSON export ───────────────────────────────────────────────────────────

    #[test]
    fn annotation_json_contains_required_fields() {
        let ann = DroneAnnotation {
            start_ms: 1_000, end_ms: 5_000,
            hint: DroneFormationHint::Burst,
            section: MusicalSection::Drop,
            avg_energy: 0.85,
        };
        let json = ann.to_json();
        assert!(json.contains("\"start_ms\":1000"));
        assert!(json.contains("\"end_ms\":5000"));
        assert!(json.contains("\"formation\":\"burst\""));
        assert!(json.contains("\"energy\":0.850"));
    }

    #[test]
    fn export_json_is_valid_array() {
        let b = DroneBridge::default();
        let events = vec![
            event(0,     5_000, MusicalSection::Intro,  0.2),
            event(5_000, 10_000, MusicalSection::Chorus, 0.8),
        ];
        let annotations = b.build(&events);
        let json = b.export_json(&annotations);
        assert!(json.starts_with('[') && json.ends_with(']'), "must be JSON array");
        assert_eq!(json.matches("formation").count(), 2, "2 annotations");
    }

    // ── Synced timeline ───────────────────────────────────────────────────────

    #[test]
    fn synced_chorus_led_burst_drone_ring() {
        let b = DroneBridge::default();
        let events = vec![event(0, 10_000, MusicalSection::Chorus, 0.9)];
        let (led, drone) = b.build_synced(&events);
        assert_eq!(led[0].hint,   DroneFormationHint::Burst, "LED chorus → Burst");
        assert_eq!(drone[0].hint, DroneFormationHint::Ring,  "Drone chorus → Ring (contrast)");
    }

    #[test]
    fn synced_drop_both_burst() {
        let b = DroneBridge::default();
        let events = vec![event(0, 5_000, MusicalSection::Drop, 0.95)];
        let (led, drone) = b.build_synced(&events);
        assert_eq!(led[0].hint,   DroneFormationHint::Burst);
        assert_eq!(drone[0].hint, DroneFormationHint::Burst, "drop: both LED and drone go Burst");
    }

    // ── Full song structure ───────────────────────────────────────────────────

    #[test]
    fn full_song_structure_produces_correct_annotation_count() {
        let b = DroneBridge::new(1_000);
        let events = vec![
            event(0,      10_000,  MusicalSection::Intro,  0.1),
            event(10_000, 30_000,  MusicalSection::Verse,  0.4),
            event(30_000, 50_000,  MusicalSection::Build,  0.6),
            event(50_000, 80_000,  MusicalSection::Chorus, 0.85),
            event(80_000, 90_000,  MusicalSection::Bridge, 0.3),
            event(90_000, 120_000, MusicalSection::Chorus, 0.9),
            event(120_000, 130_000, MusicalSection::Outro, 0.1),
        ];
        let annotations = b.build(&events);
        assert_eq!(annotations.len(), 7, "all distinct long sections preserved");

        // Verify first and last
        assert_eq!(annotations.first().unwrap().section, MusicalSection::Intro);
        assert_eq!(annotations.last().unwrap().section,  MusicalSection::Outro);
    }
}
