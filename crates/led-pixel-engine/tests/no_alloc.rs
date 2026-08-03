//! Prova que **renderizar** é livre de alocação — a contraparte, no lado do render, do gate
//! que o `led-hal` já tem no lado do envio.
//!
//! Por que este gate existe: a regra 1 do ADR-0021 ("efeito é função pura, sem estado
//! guardado") é fácil de escrever num doc e fácil de violar sem perceber — um `Vec` de
//! trabalho dentro de `render`, um `format!` num caminho de erro, uma coleção temporária
//! num efeito novo. Sem este arquivo a regra seria aspiração; com ele, é falsificável:
//! **qualquer** efeito da biblioteca que passe a alocar por frame quebra o build.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use led_core::PixelColor;
use led_pixel_engine::*;

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

/// O contador é **global do processo** e o `cargo test` roda testes em threads paralelas —
/// duas medições simultâneas se contaminam. Mesmo hazard já documentado em
/// `led-hal/tests/no_alloc.rs` (uma execução limpa chegou a acusar 7 alocações fantasma que
/// sumiam ao rodar isolado). Todo teste deste arquivo segura este gate enquanto mede.
static ALLOC_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Cada efeito da biblioteca, com parâmetros plausíveis de show.
fn library() -> Vec<(&'static str, Box<dyn Effect>)> {
    vec![
        ("SolidColor", Box::new(SolidColor(PixelColor::rgb(20, 40, 60)))),
        ("Rainbow", Box::new(Rainbow { speed_hz: 0.3, cycles_per_m: 0.5 })),
        ("Pulse", Box::new(Pulse { color: PixelColor::rgb(255, 0, 0), hz: 1.5 })),
        (
            "Chase",
            Box::new(Chase {
                color: PixelColor::rgb(255, 255, 255),
                speed_m_s: 2.0,
                spacing_m: 0.8,
                tail_frac: 0.5,
            }),
        ),
        (
            "Twinkle",
            Box::new(Twinkle {
                color: PixelColor::rgb(255, 255, 200),
                base: 0.05,
                density: 0.2,
                rate_hz: 2.5,
                seed: 1,
            }),
        ),
        (
            "Fire",
            Box::new(Fire {
                speed_m_s: 1.2,
                cells_per_m: 8.0,
                cooling_per_m: 0.5,
                lateral: 4.0,
                seed: 2,
            }),
        ),
        (
            "ColorWash",
            Box::new(ColorWash {
                a: PixelColor::rgb(180, 0, 60),
                b: PixelColor::rgb(0, 60, 180),
                hz: 0.2,
            }),
        ),
        ("Strobe", Box::new(Strobe { color: PixelColor::rgb(255, 255, 255), hz: 9.0, duty: 0.15 })),
        (
            "Meteor",
            Box::new(Meteor {
                color: PixelColor::rgb(0, 220, 255),
                span_m: 6.0,
                speed_m_s: 3.0,
                tail_m: 0.9,
                sparkle: 0.35,
                seed: 3,
            }),
        ),
        (
            "Lightning",
            Box::new(Lightning {
                color: PixelColor::rgb(255, 255, 255),
                window_ms: 350,
                probability: 0.4,
                decay_ms: 90,
                seed: 4,
            }),
        ),
        (
            "Ripple",
            Box::new(Ripple {
                color: PixelColor::rgb(0, 255, 160),
                center: Vec3::new(3.0, 1.0, 0.0),
                speed_m_s: 1.8,
                wavelength_m: 0.6,
                falloff_per_m: 0.3,
            }),
        ),
    ]
}

#[test]
fn every_library_effect_renders_without_allocating() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());

    const N: usize = 512;
    let positions: Vec<Vec3> = (0..N)
        .map(|i| Vec3::new(i as f32 * 0.02, (i % 32) as f32 * 0.03, 0.0))
        .collect();
    let mut out = vec![PixelColor::default(); N];

    let effects = library();
    // Aquecimento fora da janela de medição: qualquer inicialização preguiçosa (TLS,
    // maquinaria de lock) acontece aqui, não durante a contagem.
    for (_, fx) in &effects {
        for t in 0..50u64 {
            fx.render(t, &positions, &mut out);
        }
    }

    for (name, fx) in &effects {
        let before = ALLOCS.load(Ordering::SeqCst);
        for t in 0..2_000u64 {
            fx.render(t * 7, &positions, &mut out);
        }
        let after = ALLOCS.load(Ordering::SeqCst);
        assert_eq!(
            before, after,
            "{name} alocou {} vez(es) em 2000 frames de {N} pixels",
            after - before
        );
    }
}

/// Controle negativo do próprio gate (KB-012): um efeito que **de propósito** aloca por
/// frame tem que ser pego. Sem isto, o teste acima poderia estar passando por não medir nada.
#[test]
fn negative_control_an_allocating_effect_is_caught() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());

    struct Allocates;
    impl Effect for Allocates {
        fn render(&self, _t: u64, positions: &[Vec3], out: &mut [PixelColor]) {
            // Exatamente o erro que o gate existe para pegar: buffer de trabalho por frame.
            let scratch: Vec<PixelColor> = vec![PixelColor::rgb(1, 2, 3); positions.len()];
            out.copy_from_slice(&scratch);
        }
    }

    let positions = vec![Vec3::ZERO; 64];
    let mut out = vec![PixelColor::default(); 64];
    let fx = Allocates;
    for _ in 0..10 {
        fx.render(0, &positions, &mut out);
    }

    let before = ALLOCS.load(Ordering::SeqCst);
    for t in 0..100u64 {
        fx.render(t, &positions, &mut out);
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert!(
        after > before,
        "o gate não detectou um efeito que aloca — ele não estaria provando nada"
    );
}
