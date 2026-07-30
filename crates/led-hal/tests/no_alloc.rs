//! Proves the frame hot path is allocation-free. A counting global allocator records every
//! allocation; after a warm-up frame, 1000 more frames must allocate zero times.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use led_hal::*;

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

/// The allocation counter is **process-global**, and `cargo test` runs tests in parallel
/// threads — two tests measuring `ALLOCS` at the same time contaminate each other's window
/// (observed: a clean run reported 7 phantom allocations that vanished when run alone).
/// Every test in this file must hold this gate while measuring.
static ALLOC_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn zero_allocations_on_hot_path() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let specs = [DeviceSpec { id: 1, universes: 2 }];
    let layout = CompiledLayout::linear(300, &specs, RgbOrder::Grb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<std::sync::Arc<dyn DeviceDriver>> = vec![sim];
    let hal = Hal::new(layout, devices);
    let frame = LogicalFrame::new(vec![PixelColor::rgb(10, 20, 30); 300], 0);

    // Warm-up: flush all one-time lazy init (TLS, lock machinery) before measuring.
    for _ in 0..100 {
        hal.send_frame(&frame).unwrap();
    }

    // Measure a large window. If the hot path allocated *per frame*, this would grow with
    // the frame count; a steady-state alloc-free path shows zero growth.
    let before = ALLOCS.load(Ordering::SeqCst);
    for _ in 0..10_000 {
        hal.send_frame(&frame).unwrap();
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert_eq!(before, after, "hot path allocated {} time(s) over 10000 frames", after - before);
}

/// Same proof, but with per-output calibration active (ADR-0019): the LUT and the calibrated
/// output buffer are built at startup, so the frame path must still allocate nothing.
#[test]
fn zero_allocations_on_hot_path_with_calibration() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let specs = [DeviceSpec { id: 1, universes: 2 }];
    let layout = CompiledLayout::linear(300, &specs, RgbOrder::Grb);
    let sim = SimulatorDevice::new(1, layout.device_universes(1));
    let devices: Vec<std::sync::Arc<dyn DeviceDriver>> = vec![sim];

    let mut cal = Calibration::new();
    cal.set(1, 2.2, 0.8); // gamma + brightness folded into one LUT at startup
    let hal = Hal::new(layout, devices).with_calibration(cal);
    let frame = LogicalFrame::new(vec![PixelColor::rgb(10, 20, 30); 300], 0);

    for _ in 0..100 {
        hal.send_frame(&frame).unwrap();
    }

    let before = ALLOCS.load(Ordering::SeqCst);
    for _ in 0..10_000 {
        hal.send_frame(&frame).unwrap();
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert_eq!(
        before, after,
        "calibrated hot path allocated {} time(s) over 10000 frames",
        after - before
    );
}
