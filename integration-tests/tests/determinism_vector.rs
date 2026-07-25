//! Cross-platform determinism vectors.
//!
//! These golden hashes were recorded on the reference platform
//! (macOS arm64, rustc stable). Running this test on ANY other
//! machine/OS/architecture measures cross-platform determinism:
//!
//! - `intent_hash` uses only integer math (FNV-1a + SplitMix64) — it MUST be
//!   identical on every platform; a mismatch is a hard regression.
//! - The render hash exercises f32 trig (`sin`) in the Plasma kernel. IEEE 754
//!   `sin` is correctly-rounded-ish but not guaranteed identical across libm
//!   implementations; if this vector fails off-platform, that is a *finding*
//!   to record (mitigation: table-based or fixed-point trig in kernels).
//!
//! Regenerate goldens deliberately (never to silence a failure):
//! `cargo test -p integration-tests --test determinism_vector -- --nocapture`
//! prints the observed values on failure.

use led_core::compute_pixel_hash;
use led_pixel_engine::{ComputeEffect, Effect, Plasma, Vec3};
use led_sequencer::{ShowIntentGenerator, ShowStyle};

/// Golden: ShowIntent hash, seed 42, Drop section profile. Integer math only.
/// Recorded 2026-07-09, macOS arm64, rustc stable.
const GOLDEN_INTENT_HASH: u64 = 0x12ce2cfdf90ff176;

/// Golden: 64px Plasma render at 4 fixed timestamps, macOS arm64 reference.
/// Recorded 2026-07-09. f32 trig — see module docs before touching.
const GOLDEN_PLASMA_HASH: u64 = 0x1ed5508a56d0b0bc;

#[test]
fn show_intent_hash_matches_golden_on_all_platforms() {
    let generator = ShowIntentGenerator::new(42);
    let intent = generator
        .from_audio(0.8, true, 128.0, Some(led_core::MusicalSection::Drop), 10_000, 576)
        .expect("valid intent");

    assert_eq!(intent.style, ShowStyle::Drop);
    assert_eq!(
        intent.intent_hash, GOLDEN_INTENT_HASH,
        "intent hash is integer-only math and MUST be platform-independent; \
         got {:#018x}",
        intent.intent_hash
    );
}

#[test]
fn plasma_render_hash_matches_reference_platform() {
    let effect = ComputeEffect::new(Plasma { scale: 0.35, speed: 1.0 });
    let positions: Vec<Vec3> = (0..64)
        .map(|i| Vec3::new((i % 8) as f32, (i / 8) as f32, 0.0))
        .collect();

    let mut all_pixels = Vec::new();
    for t in [0u64, 333, 1_000, 5_000] {
        let mut frame = vec![led_core::PixelColor::default(); 64];
        effect.render(t, &positions, &mut frame);
        all_pixels.extend_from_slice(&frame);
    }
    let hash = compute_pixel_hash(&all_pixels);

    assert_eq!(
        hash, GOLDEN_PLASMA_HASH,
        "render hash differs from the macOS arm64 reference: got {hash:#018x}. \
         On the reference platform this is a regression; on another platform \
         it measures libm sin() divergence — record the finding either way."
    );
}

#[test]
fn same_seed_twice_is_bit_identical_locally() {
    // Local determinism (any platform): two runs in one process must agree.
    let run = || {
        let effect = ComputeEffect::new(Plasma { scale: 0.35, speed: 1.0 });
        let positions: Vec<Vec3> = (0..32).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        let mut frame = vec![led_core::PixelColor::default(); 32];
        effect.render(777, &positions, &mut frame);
        compute_pixel_hash(&frame)
    };
    assert_eq!(run(), run(), "same inputs, same process → identical pixels");
}
