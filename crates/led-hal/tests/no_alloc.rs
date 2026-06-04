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

#[test]
fn zero_allocations_on_hot_path() {
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
