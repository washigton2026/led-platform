//! Deterministic distributed replay — verify that a `.lumyx` show file
//! produces identical pixel output on any machine, OS, or cluster node.
//!
//! ## Invariants (lumyx-regression-guardian)
//! - `replay_hash` is identical for the same `.lumyx` file on any platform.
//! - `verify_replay` checks that replayed output matches the recorded hash.
//! - `cross_node_verify` checks that two nodes replaying the same file agree.
//! - All hashing uses FNV-1a 64-bit (same as `led-core::compute_pixel_hash`).

use crate::{pixel_hash, ShowReader, ShowRecord, ShowWriter};
use std::io::{Cursor, Read, Seek};

// ── ReplayManifest ────────────────────────────────────────────────────────────

/// A complete replay manifest: per-frame hashes + aggregate hash.
/// Two nodes with identical `ReplayManifest` are guaranteed to be in sync.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayManifest {
    /// Number of frames in the recording.
    pub frame_count: usize,
    /// FNV-1a hash of all pixel data, all frames concatenated.
    pub aggregate_hash: u64,
    /// Per-frame pixel hashes (for frame-level diff).
    pub frame_hashes: Vec<u64>,
    /// Pixel count per frame (must be constant across the file).
    pub pixel_count: u32,
}

impl ReplayManifest {
    /// Build a manifest from a slice of `ShowRecord`s.
    pub fn from_records(records: &[ShowRecord]) -> Self {
        let frame_hashes: Vec<u64> = records
            .iter()
            .map(|r| pixel_hash(std::slice::from_ref(r)))
            .collect();

        let aggregate_hash = pixel_hash(records);
        let pixel_count = records.first().map(|r| r.pixels.len() as u32).unwrap_or(0);

        Self {
            frame_count: records.len(),
            aggregate_hash,
            frame_hashes,
            pixel_count,
        }
    }

    /// Emit a JSON summary (one line, for log aggregators).
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"frames":{fc},"pixels":{pc},"hash":"{hash:#018x}"}}"#,
            fc   = self.frame_count,
            pc   = self.pixel_count,
            hash = self.aggregate_hash,
        )
    }

    /// Compare this manifest against another — returns the first divergent frame index.
    pub fn diff(&self, other: &ReplayManifest) -> Option<usize> {
        if self.aggregate_hash == other.aggregate_hash { return None; }
        self.frame_hashes
            .iter()
            .zip(&other.frame_hashes)
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
    }
}

// ── verify_replay ─────────────────────────────────────────────────────────────

/// Verify that replaying a `.lumyx` file produces `expected_hash`.
/// Returns `Ok(manifest)` if hash matches, `Err(actual_hash)` if not.
pub fn verify_replay<R: Read + Seek>(
    reader: ShowReader<R>,
    expected_hash: u64,
) -> Result<ReplayManifest, (u64, u64)> {
    let records = reader.collect_all().map_err(|_| (0u64, expected_hash))?;
    let manifest = ReplayManifest::from_records(&records);
    if manifest.aggregate_hash == expected_hash {
        Ok(manifest)
    } else {
        Err((manifest.aggregate_hash, expected_hash))
    }
}

/// Record a show to a `Vec<u8>` buffer and return the manifest.
/// Used to establish a baseline for cross-node comparison.
pub fn record_and_manifest<F>(pixel_count: u32, f: F) -> (Vec<u8>, ReplayManifest)
where
    F: FnOnce(&mut ShowWriter<&mut Cursor<Vec<u8>>>),
{
    let mut backing = Cursor::new(Vec::<u8>::new());
    {
        let mut writer = ShowWriter::new(&mut backing, pixel_count).expect("writer init failed");
        f(&mut writer);
        crate::finalise_seekable(&mut writer).ok();
    }
    let data = backing.into_inner();
    let reader = ShowReader::new(Cursor::new(data.clone())).expect("reader init failed");
    let records = reader.collect_all().expect("read failed");
    let manifest = ReplayManifest::from_records(&records);
    (data, manifest)
}

/// Simulate two nodes replaying the same data and check agreement.
/// Returns `None` (no divergence) or `Some(frame_idx)` of first disagreement.
pub fn cross_node_verify(data_node_a: &[u8], data_node_b: &[u8]) -> Option<usize> {
    let records_a: Vec<ShowRecord> = ShowReader::new(Cursor::new(data_node_a))
        .and_then(|r| r.collect_all())
        .unwrap_or_default();
    let records_b: Vec<ShowRecord> = ShowReader::new(Cursor::new(data_node_b))
        .and_then(|r| r.collect_all())
        .unwrap_or_default();

    let m_a = ReplayManifest::from_records(&records_a);
    let m_b = ReplayManifest::from_records(&records_b);
    m_a.diff(&m_b)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ShowRecord;
    use led_core::PixelColor;

    fn px(r: u8, g: u8, b: u8) -> PixelColor { PixelColor::rgb(r, g, b) }

    fn record(t: u64, n: usize) -> ShowRecord {
        ShowRecord {
            timestamp_ms: t,
            pixels: (0..n).map(|i| px((t + i as u64) as u8, i as u8, 0)).collect(),
            audio: None,
        }
    }

    // ── ReplayManifest ────────────────────────────────────────────────────────

    #[test]
    fn manifest_from_empty_records() {
        let m = ReplayManifest::from_records(&[]);
        assert_eq!(m.frame_count, 0);
        assert_eq!(m.pixel_count, 0);
    }

    #[test]
    fn manifest_aggregate_hash_deterministic() {
        let records = vec![record(0, 4), record(33, 4)];
        let m1 = ReplayManifest::from_records(&records);
        let m2 = ReplayManifest::from_records(&records);
        assert_eq!(m1.aggregate_hash, m2.aggregate_hash, "must be deterministic");
    }

    #[test]
    fn manifest_frame_hashes_count_matches_records() {
        let records: Vec<_> = (0..10).map(|i| record(i * 33, 8)).collect();
        let m = ReplayManifest::from_records(&records);
        assert_eq!(m.frame_hashes.len(), 10);
        assert_eq!(m.frame_count, 10);
    }

    #[test]
    fn manifest_diff_identical_returns_none() {
        let records = vec![record(0, 4), record(33, 4)];
        let m = ReplayManifest::from_records(&records);
        assert!(m.diff(&m.clone()).is_none(), "identical manifests have no diff");
    }

    #[test]
    fn manifest_diff_detects_first_divergent_frame() {
        let records_a = vec![record(0, 4), record(33, 4), record(66, 4)];
        let mut records_b = records_a.clone();
        records_b[1].pixels[0] = px(255, 0, 0); // corrupt frame 1

        let m_a = ReplayManifest::from_records(&records_a);
        let m_b = ReplayManifest::from_records(&records_b);
        assert_eq!(m_a.diff(&m_b), Some(1), "divergence at frame 1");
    }

    // ── verify_replay ─────────────────────────────────────────────────────────

    #[test]
    fn verify_replay_matching_hash_returns_ok() {
        let (data, manifest) = record_and_manifest(4, |w| {
            for i in 0..5u64 { w.write_frame(&record(i * 33, 4)).unwrap(); }
        });
        let reader = crate::ShowReader::new(std::io::Cursor::new(data)).unwrap();
        let result = verify_replay(reader, manifest.aggregate_hash);
        assert!(result.is_ok(), "matching hash must return Ok");
    }

    #[test]
    fn verify_replay_wrong_hash_returns_err() {
        let (data, _) = record_and_manifest(4, |w| {
            w.write_frame(&record(0, 4)).unwrap();
        });
        let reader = crate::ShowReader::new(std::io::Cursor::new(data)).unwrap();
        let result = verify_replay(reader, 0xDEADBEEF);
        assert!(result.is_err(), "wrong hash must return Err");
        let (actual, expected) = result.unwrap_err();
        assert_ne!(actual, expected);
    }

    // ── cross_node_verify ─────────────────────────────────────────────────────

    #[test]
    fn cross_node_verify_identical_files_no_divergence() {
        let (data, _) = record_and_manifest(8, |w| {
            for i in 0..10u64 { w.write_frame(&record(i * 33, 8)).unwrap(); }
        });
        // Same data on both nodes → no divergence
        assert!(cross_node_verify(&data, &data).is_none(),
            "identical data must have no cross-node divergence");
    }

    #[test]
    fn cross_node_verify_detects_divergence() {
        let (data_a, _) = record_and_manifest(4, |w| {
            for i in 0..5u64 { w.write_frame(&record(i, 4)).unwrap(); }
        });
        // Node B has a different pixel at frame 2
        let mut records_b: Vec<ShowRecord> = (0..5u64)
            .map(|i| record(i, 4))
            .collect();
        records_b[2].pixels[0] = px(200, 100, 50);
        let (data_b, _) = record_and_manifest(4, |w| {
            for r in &records_b { w.write_frame(r).unwrap(); }
        });

        let divergence = cross_node_verify(&data_a, &data_b);
        assert!(divergence.is_some(), "must detect cross-node divergence");
    }

    // ── JSON manifest ─────────────────────────────────────────────────────────

    #[test]
    fn manifest_json_contains_key_fields() {
        let records = vec![record(0, 4)];
        let m = ReplayManifest::from_records(&records);
        let json = m.to_json();
        assert!(json.contains("\"frames\":1"));
        assert!(json.contains("\"pixels\":4"));
        assert!(json.contains("\"hash\":\"0x"),
            "hash must be a quoted string — bare 0x literal is invalid JSON");
        assert!(json.starts_with('{') && json.ends_with('}'));
    }

    // ── Stress: 1000 frames identical on simulated two nodes ─────────────────

    #[test]
    fn distributed_replay_1000_frames_no_divergence() {
        let (data, manifest) = record_and_manifest(16, |w| {
            for i in 0u64..1000 {
                let pixels = (0..16).map(|j| px((i * j as u64 % 256) as u8, j as u8, 0)).collect();
                w.write_frame(&ShowRecord { timestamp_ms: i * 33, pixels, audio: None }).unwrap();
            }
        });

        // Node A
        let r_a = crate::ShowReader::new(std::io::Cursor::new(data.clone())).unwrap();
        let res_a = verify_replay(r_a, manifest.aggregate_hash).expect("node A must verify");

        // Node B (same data — simulates identical file on a second machine)
        let r_b = crate::ShowReader::new(std::io::Cursor::new(data.clone())).unwrap();
        let res_b = verify_replay(r_b, manifest.aggregate_hash).expect("node B must verify");

        assert_eq!(res_a.aggregate_hash, res_b.aggregate_hash,
            "cross-node hash must be identical");
        assert!(cross_node_verify(&data, &data).is_none(),
            "identical files must have zero frame-level divergence");
    }
}
