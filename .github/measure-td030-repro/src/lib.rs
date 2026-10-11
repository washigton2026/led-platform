//! Minimal reproduction attempt: can `ArcSwap::load()` on one instance return a value that was
//! only ever stored into a DIFFERENT, independent `ArcSwap` instance in the same process?
//!
//! Only `arc-swap` 1.9.1 and std. Every test owns its own `ArcSwap<Snap>`; no instance is shared
//! between tests. The test harness runs the tests in parallel (default libtest threads), which
//! is the condition under which the original observation happened.
//!
//! `Snap` mirrors a small `Copy` struct of scalars (u32, u64, f32, bool, f32, f32, f32, Option<u8>).
//! Each store also allocates a small `Vec<f32>` (8 elements) next to the `Arc`, mimicking the
//! allocation pattern of the original publisher.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snap {
    pub tag: u32,
    pub seq: u64,
    pub a: f32,
    pub flag: bool,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub opt: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::Snap;
    use arc_swap::ArcSwap;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    const NOISE_TAG: u32 = 48_000;

    fn snap(tag: u32, seq: u64, flag: bool, a: f32, b: f32) -> Snap {
        Snap { tag, seq, a, flag, b, c: 0.3, d: 0.1, opt: None }
    }

    fn new_cell() -> ArcSwap<Snap> {
        ArcSwap::from_pointee(Snap::default())
    }

    /// Store plus a side allocation of 8 floats, like the original publisher.
    fn store(cell: &ArcSwap<Snap>, side: &Mutex<Vec<f32>>, s: Snap) {
        let fresh = vec![0.0f32; 8];
        cell.store(Arc::new(s));
        let mut g = side.lock().unwrap();
        if g.len() != fresh.len() {
            g.resize(fresh.len(), 0.0);
        }
        g.copy_from_slice(&fresh);
    }

    fn read(cell: &ArcSwap<Snap>) -> Snap {
        *cell.load().as_ref()
    }

    /// Identity check: one writer stores `tag = TAG` and `seq = i` (i = 1..=n) into its OWN cell;
    /// one reader loads in a tight loop and asserts that every snapshot is either the initial
    /// default or carries this cell's TAG and a coherent flag. Returns (violations, first, reads).
    fn identity_run(tag: u32, n: u64, mark_all: bool) -> (u32, Option<(Snap, u64)>, u64) {
        let cell = Arc::new(new_cell());
        let side = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicU64::new(0));
        let viol = Arc::new(AtomicU32::new(0));

        let (wc, ws, wd) = (cell.clone(), side.clone(), done.clone());
        let writer = thread::spawn(move || {
            for i in 1..=n {
                let s = if mark_all {
                    Snap { tag, seq: i, a: i as f32, flag: i % 2 == 1, b: i as f32, c: 0.3, d: 0.1, opt: None }
                } else {
                    snap(tag, i, i % 2 == 1, 0.0, 0.0)
                };
                store(&wc, &ws, s);
                wd.store(i, Ordering::SeqCst);
            }
        });

        let (rc, rs, rd, rv) = (cell.clone(), stop.clone(), done.clone(), viol.clone());
        let reader = thread::spawn(move || {
            let mut reads = 0u64;
            let mut first: Option<(Snap, u64)> = None;
            while !rs.load(Ordering::Relaxed) {
                reads += 1;
                let s = read(&rc);
                if s == Snap::default() {
                    continue;
                }
                let ok = s.tag == tag && s.flag == (s.seq % 2 == 1);
                if !ok {
                    rv.fetch_add(1, Ordering::Relaxed);
                    if first.is_none() {
                        first = Some((s, rd.load(Ordering::SeqCst)));
                    }
                }
            }
            (first, reads)
        });

        writer.join().unwrap();
        stop.store(true, Ordering::Relaxed);
        let (first, reads) = reader.join().unwrap();
        (viol.load(Ordering::Relaxed), first, reads)
    }

    fn assert_identity(name: &str, tag: u32, mark_all: bool) {
        let (v, first, reads) = identity_run(tag, 10_000, mark_all);
        if let Some((s, done)) = first {
            eprintln!(
                "REPRO_VIOLATION test={name} own_tag={tag} seen_tag={} seq={} flag={} a={} b={} last_store_done={done}",
                s.tag, s.seq, s.flag, s.a, s.b
            );
        }
        assert_eq!(v, 0, "{name}: {v} foreign/incoherent snapshots in {reads} reads; first={first:?}");
    }

    // ── the two identity tests (the property under test) ─────────────────────
    #[test]
    fn identity_parity_tag_1001() {
        assert_identity("identity_parity_tag_1001", 1001, false);
    }

    #[test]
    fn identity_all_fields_marked_tag_1002() {
        assert_identity("identity_all_fields_marked_tag_1002", 1002, true);
    }

    // ── background tests: independent instances, same shapes as the original module ──
    #[test]
    fn noise_8_writers_8_readers() {
        let cell = Arc::new(new_cell());
        let side = Arc::new(Mutex::new(Vec::new()));
        let mut hs = Vec::new();
        for i in 0..8u64 {
            let (c, s) = (cell.clone(), side.clone());
            hs.push(thread::spawn(move || {
                for j in 0..1_000u64 {
                    store(&c, &s, snap(NOISE_TAG, i * 1000 + j, j % 3 == 0, 0.5, j as f32 * 0.001));
                }
            }));
        }
        for _ in 0..8 {
            let c = cell.clone();
            hs.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    let s = read(&c);
                    assert!(s.a >= 0.0);
                }
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
    }

    #[test]
    fn noise_2_threads_32() {
        let cell = Arc::new(new_cell());
        let side = Arc::new(Mutex::new(Vec::new()));
        let (c1, s1, c2) = (cell.clone(), side.clone(), cell.clone());
        let w = thread::spawn(move || {
            for j in 0..32u64 {
                store(&c1, &s1, snap(NOISE_TAG, j, j % 2 == 0, 0.5, j as f32 * 0.01));
            }
        });
        let r = thread::spawn(move || {
            for _ in 0..32 {
                assert!(read(&c2).a >= 0.0);
            }
        });
        w.join().unwrap();
        r.join().unwrap();
    }

    #[test]
    fn noise_100k_single_thread() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        for i in 0..100_000u64 {
            store(&cell, &side, snap(NOISE_TAG, i, i % 4 == 0, 0.5, 1.0));
        }
        assert_eq!(side.lock().unwrap().len(), 8);
    }

    #[test]
    fn noise_single_store_a() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(NOISE_TAG, 100, false, 0.8, 0.5));
        assert_eq!(read(&cell).seq, 100);
    }

    #[test]
    fn noise_single_store_b() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(44_100, 0, false, 0.0, 0.0));
        assert_eq!(read(&cell).tag, 44_100);
    }

    #[test]
    fn noise_single_store_c() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(NOISE_TAG, 1, false, f32::NAN, f32::INFINITY));
        assert!(read(&cell).b.is_infinite());
    }

    #[test]
    fn noise_store_then_two_loads() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(NOISE_TAG, 42, true, 0.5, 0.0));
        let a = read(&cell);
        let b = read(&cell);
        assert_eq!(a, b);
    }

    #[test]
    fn noise_store_then_load_d() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(NOISE_TAG, 1, false, 1.0, 9999.0));
        assert_eq!(read(&cell).seq, 1);
    }

    #[test]
    fn noise_store_then_load_e() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(NOISE_TAG, 1, false, 1.0, 1.0));
        assert_eq!(read(&cell).seq, 1);
    }

    #[test]
    fn noise_store_then_1000_loads() {
        let cell = new_cell();
        let side = Mutex::new(Vec::new());
        store(&cell, &side, snap(NOISE_TAG, 1, true, 0.5, 0.5));
        for _ in 0..1_000 {
            assert_eq!(read(&cell).seq, 1);
        }
    }
}
