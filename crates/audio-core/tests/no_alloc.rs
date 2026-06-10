//! Proves the analysis hot path is allocation-free (lumyx-system-architect invariant 3 /
//! CLAUDE.md "no allocation on the hot path"). A counting global allocator records every
//! allocation; after warm-up, 1000 more hops — `Analyzer::process_hop` AND the
//! `tokio::sync::watch` send — must allocate zero times.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use audio_core::{Analyzer, AudioFeatures, HOP_SIZE};
use tokio::sync::watch;

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
fn zero_allocations_on_hot_path() {
    let mut analyzer = Analyzer::new(48_000);
    let (tx, _rx) = watch::channel(AudioFeatures::default());
    let hop = [0.1f32; HOP_SIZE];

    // Warm-up: flush all one-time lazy init (TLS, lock machinery) before measuring.
    for t in 0..200u64 {
        let features = analyzer.process_hop(&hop, t);
        tx.send(features).unwrap();
    }

    let before = ALLOCS.load(Ordering::SeqCst);
    for t in 200..1200u64 {
        let features = analyzer.process_hop(&hop, t);
        tx.send(features).unwrap();
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert_eq!(before, after, "hot path allocated {} time(s) over 1000 hops", after - before);
}
