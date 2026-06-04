//! Perf gate: composing a timeline frame is allocation-free (pre-sized clip scratch,
//! in-place blend). Counting allocator, zero growth across 10k renders after warm-up.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use led_core::PixelColor;
use led_pixel_engine::{Effect, SolidColor, Vec3};
use led_sequencer::{BlendMode, Clip, Timeline, Track};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static A: Counting = Counting;

#[test]
fn timeline_render_is_allocation_free() {
    const N: usize = 300;
    let timeline = Timeline::new(N)
        .with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, 10_000, Box::new(SolidColor(PixelColor::rgb(255, 0, 0)))).with_fades(100, 100)),
        )
        .with_track(
            Track::new(BlendMode::Add)
                .with_clip(Clip::new(0, 10_000, Box::new(SolidColor(PixelColor::rgb(0, 255, 0))))),
        );

    let positions = vec![Vec3::ZERO; N];
    let mut out = vec![PixelColor::default(); N];

    for _ in 0..100 {
        timeline.render(500, &positions, &mut out); // warm-up flushes lazy init
    }
    let before = ALLOCS.load(Ordering::SeqCst);
    for t in 0..10_000u64 {
        timeline.render(500 + (t % 7), &positions, &mut out);
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert_eq!(before, after, "timeline render allocated {} time(s) over 10000 frames", after - before);
}
