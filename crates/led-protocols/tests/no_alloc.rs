//! Proves the DDP backend send path is allocation-free after warm-up.
//!
//! Regression guard for C2: `DdpBackend::send_universe` previously allocated a
//! `Vec<PixelColor>` **and** a packet `Vec<u8>` per universe per frame — a hot-path
//! allocation (the router runs under the HAL fan-out). The fix pre-allocates one packet
//! buffer and builds the packet from the mapped bytes via `build_ddp_packet_bytes`.
//!
//! A counting global allocator records every allocation on the whole process; after a
//! warm-up window, a large batch of `send_universe` calls must allocate exactly zero times.

use std::alloc::{GlobalAlloc, Layout, System};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicUsize, Ordering};

use led_core::UniverseData;
use led_protocols::{DdpBackend, ProtocolBackend};

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
fn ddp_backend_send_path_is_alloc_free() {
    // A localhost receiver so `send` has a live destination. Non-blocking + drained each
    // iteration so the kernel receive buffer never fills (which could block the sender).
    let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
    recv.set_nonblocking(true).unwrap();
    let addr = recv.local_addr().unwrap();
    let backend = DdpBackend::new(addr, 0).unwrap();

    // 300 pixels = 900 channel bytes → a single DDP fragment (< DDP_MAX_PAYLOAD).
    let u = UniverseData { universe: 0, data: vec![0x20u8; 900] };
    let mut sink = [0u8; 2048]; // stack drain buffer — no allocation

    // Warm-up: flush any one-time lazy init before measuring.
    for _ in 0..100 {
        backend.send_universe(&u).unwrap();
        while recv.recv(&mut sink).is_ok() {}
    }

    let before = ALLOCS.load(Ordering::SeqCst);
    for _ in 0..10_000 {
        backend.send_universe(&u).unwrap();
        while recv.recv(&mut sink).is_ok() {}
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert_eq!(
        before, after,
        "DDP send path allocated {} time(s) over 10000 frames",
        after - before
    );
}
