//! TD-030 (accept v2, §3.6): os efeitos REATIVOS (`BandPulse`, `BeatFlash`) chamam
//! `AudioShare::scalars()` em cada frame e não estão na `library()` do `no_alloc.rs`. O latch de
//! sequência e o registo do último snapshot válido (`thread_local` com init `const`) não podem
//! alocar no render.
//!
//! Binário próprio (um `#[global_allocator]` não se partilha entre binários) e réplica mínima do
//! contador POR THREAD do TD-023: só conta o que a thread marcada aloca enquanto está marcada.
//! Os testes deste binário serializam-se por `GATE`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use led_core::{AudioFeatures, PixelColor};
use led_pixel_engine::{AudioShare, Band, BandPulse, BeatFlash, Effect, Vec3};

struct Contador;

thread_local! {
    static MARCADA: Cell<bool> = const { Cell::new(false) };
    static ALOCACOES: Cell<usize> = const { Cell::new(0) };
}

fn contar() {
    // `try_with`: durante a destruição da thread o TLS pode já não existir — não conta.
    let _ = MARCADA.try_with(|m| {
        if m.get() {
            let _ = ALOCACOES.try_with(|a| a.set(a.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        contar();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        contar();
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        contar();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Contador = Contador;

static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Alocações feitas por ESTA thread durante `corpo`.
fn alocacoes_em(corpo: impl FnOnce()) -> usize {
    let antes = ALOCACOES.with(|a| a.get());
    MARCADA.with(|m| m.set(true));
    corpo();
    MARCADA.with(|m| m.set(false));
    ALOCACOES.with(|a| a.get()) - antes
}

fn share_publicado() -> Arc<AudioShare> {
    let share = Arc::new(AudioShare::new());
    share.publish(&AudioFeatures {
        sample_rate: 48_000,
        timestamp_ms: 1,
        rms: 0.5,
        beat: true,
        bass: 0.7,
        mid: 0.3,
        high: 0.1,
        spectrum: vec![0.0; 8],
        musical_section: None,
        instrument_class: None,
    });
    share
}

#[test]
fn efeitos_reativos_renderizam_sem_alocar() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let share = share_publicado();
    let efeitos: Vec<(&str, Box<dyn Effect>)> = vec![
        ("BandPulse", Box::new(BandPulse::new(PixelColor::rgb(255, 0, 0), Band::Bass, 1.0, share.clone()))),
        ("BeatFlash", Box::new(BeatFlash::new(PixelColor::rgb(0, 0, 255), 300, share.clone()))),
    ];
    const N: usize = 512;
    let positions = vec![Vec3::ZERO; N];
    let mut out = vec![PixelColor::default(); N];
    // Aquecimento fora da contagem (inicializações preguiçosas).
    for (_, fx) in &efeitos {
        for t in 0..50u64 {
            fx.render(t, &positions, &mut out);
        }
    }
    for (nome, fx) in &efeitos {
        let n = alocacoes_em(|| {
            for t in 0..2_000u64 {
                fx.render(t * 7, &positions, &mut out);
            }
        });
        assert_eq!(n, 0, "{nome} alocou {n} vez(es) em 2000 frames de {N} píxeis");
    }
    assert_eq!(share.fallbacks_do_leitor(), (0, 0));
}

/// Controlo negativo do próprio contador (KB-012): uma alocação plantada TEM de ser vista.
#[test]
fn o_contador_ve_uma_alocacao_plantada() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let n = alocacoes_em(|| {
        let v: Vec<u8> = Vec::with_capacity(64);
        std::hint::black_box(v);
    });
    assert!(n >= 1, "o contador não viu a alocação plantada");
}
