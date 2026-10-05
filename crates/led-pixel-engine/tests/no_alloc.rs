//! Prova que **renderizar** é livre de alocação — a contraparte, no lado do render, do gate
//! que o `led-hal` já tem no lado do envio.
//!
//! Por que este gate existe: a regra 1 do ADR-0021 ("efeito é função pura, sem estado
//! guardado") é fácil de escrever num doc e fácil de violar sem perceber — um `Vec` de
//! trabalho dentro de `render`, um `format!` num caminho de erro, uma coleção temporária
//! num efeito novo. Sem este arquivo a regra seria aspiração; com ele, é falsificável:
//! **qualquer** efeito da biblioteca que passe a alocar por frame quebra o build.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use led_core::PixelColor;
use led_pixel_engine::*;

// ── Contador POR THREAD (TD-023) ────────────────────────────────────────────────────────
//
// O contador antigo era global do processo: tudo o que QUALQUER thread alocasse dentro da
// janela entrava na asserção — as threads do libtest incluídas — e o gate reprovava sem o
// caminho quente ter alocado (TD-023: «7 time(s)», «232 vs 237», «180 vs 182»).
//
// Réplica mínima do padrão de `crates/led-protocols/tests/no_alloc.rs` (F7.2, 950a497):
// para a asserção só conta o que a thread marcada como a do teste aloca; o que as outras
// alocam dentro da janela fica em `FORA_DA_THREAD` — contexto, nunca falha. Replicado, e não
// partilhado, porque um `#[global_allocator]` não se partilha entre binários de teste e o
// workspace não tem crate de utilitários de teste.
//
// **O instrumento não pode alocar**: só atómicos e um `thread_local` com init `const`.

struct Counting;
/// Alocações da thread do teste — é este o número da asserção.
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// A janela de medição está aberta?
static MEDINDO: AtomicBool = AtomicBool::new(false);
/// Alocações da janela vindas de OUTRAS threads. O contador global antigo somava-as.
static FORA_DA_THREAD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Marca a thread que corre o corpo do teste. `const` para não alocar ao inicializar.
    static E_A_THREAD_DO_TESTE: Cell<bool> = const { Cell::new(false) };
}

/// Chamada de dentro do alocador: **sem alocar**. `true` se a alocação é da thread do teste.
fn registar() -> bool {
    // `try_with`: durante a destruição de TLS um `with` entraria em pânico, e um pânico
    // dentro do alocador aborta o processo sem diagnóstico.
    let minha = E_A_THREAD_DO_TESTE.try_with(|c| c.get()).unwrap_or(false);
    if !minha && MEDINDO.load(Ordering::Relaxed) {
        FORA_DA_THREAD.fetch_add(1, Ordering::Relaxed);
    }
    minha
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if registar() {
            ALLOCS.fetch_add(1, Ordering::SeqCst);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if registar() {
            ALLOCS.fetch_add(1, Ordering::SeqCst);
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if registar() {
            ALLOCS.fetch_add(1, Ordering::SeqCst);
        }
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Serializa os testes deste binário: partilham a janela e os estáticos acima.
static ALLOC_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Marca a thread atual como a do teste e abre a janela.
fn abrir_janela() {
    E_A_THREAD_DO_TESTE.with(|c| c.set(true));
    FORA_DA_THREAD.store(0, Ordering::SeqCst);
    MEDINDO.store(true, Ordering::SeqCst);
}

fn fechar_janela() {
    MEDINDO.store(false, Ordering::SeqCst);
}

/// Quantas alocações a thread de fundo tem de fazer DENTRO da janela antes de ela fechar.
const RUIDO_MINIMO: usize = 1_000;

/// Arnês do controlo negativo: corre `corpo` numa janela onde uma thread de fundo aloca em
/// ciclo, e só fecha a janela quando essa thread já alocou ≥ [`RUIDO_MINIMO`] vezes lá
/// dentro — espera ativa, sem `sleep`, portanto determinístico. `corpo` devolve quantas
/// iterações do caminho quente correu (tem de ser > 0). Devolve
/// `(Δ por thread, Δ que o contador global antigo teria visto)`.
fn medir_com_ruido_de_fundo(corpo: impl FnOnce() -> usize) -> (usize, usize) {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let parar = Arc::new(AtomicBool::new(false));
    let pronta = Arc::new(AtomicBool::new(false));
    let ruido = {
        let (parar, pronta) = (Arc::clone(&parar), Arc::clone(&pronta));
        std::thread::spawn(move || {
            pronta.store(true, Ordering::SeqCst);
            while !parar.load(Ordering::SeqCst) {
                std::hint::black_box(vec![0u8; 64]);
            }
        })
    };
    while !pronta.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }

    let prazo = Instant::now() + Duration::from_secs(60);
    abrir_janela();
    let antes = ALLOCS.load(Ordering::SeqCst);
    let iteracoes = corpo();
    while FORA_DA_THREAD.load(Ordering::SeqCst) < RUIDO_MINIMO && Instant::now() < prazo {
        std::thread::yield_now();
    }
    let depois = ALLOCS.load(Ordering::SeqCst);
    let fora = FORA_DA_THREAD.load(Ordering::SeqCst);
    fechar_janela();

    parar.store(true, Ordering::SeqCst);
    ruido.join().unwrap();
    // Um `corpo` que não corresse o caminho quente passaria vacuamente.
    assert!(iteracoes > 0, "o arnês não correu o caminho quente — controlo vacuoso");
    (depois - antes, depois - antes + fora)
}

/// As duas asserções do controlo negativo, iguais em todos os gates.
fn exigir_ruido_ignorado(por_thread: usize, global: usize, o_que: &str) {
    assert!(
        global >= RUIDO_MINIMO,
        "o arnês não pôs ruído na janela ({global} < {RUIDO_MINIMO}) — controlo vacuoso"
    );
    assert_eq!(
        por_thread, 0,
        "{o_que} alocou {por_thread} vez(es) na thread do teste com ruído de fundo \
         (o contador global antigo teria visto {global})"
    );
}

/// Controlo positivo: uma alocação plantada na thread do teste TEM de ser contada — por cada
/// uma das três entradas do alocador (`alloc`, `alloc_zeroed`, `realloc`). Sem isto, «atribuir
/// por thread» e «desligar o contador» seriam indistinguíveis.
fn exigir_que_ve_alocacao_plantada() {
    fn delta(o_que: &str, plantar: impl FnOnce() -> Vec<u8>) {
        abrir_janela();
        let antes = ALLOCS.load(Ordering::SeqCst);
        let plantada = plantar();
        let depois = ALLOCS.load(Ordering::SeqCst);
        fechar_janela();
        // Depois de fechar a janela, para o optimizador não apagar a alocação.
        std::hint::black_box(&plantada);
        assert!(
            depois > antes,
            "o contador NÃO viu um `{o_que}` feito na própria thread do teste — o gate tornou-se vacuoso"
        );
    }
    delta("alloc", || vec![7u8; 900]);
    delta("alloc_zeroed", || vec![0u8; 900]);
    let mut v: Vec<u8> = Vec::with_capacity(8);
    v.push(1);
    std::hint::black_box(&v);
    delta("realloc", move || {
        v.reserve_exact(4096);
        v
    });
}

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
        abrir_janela();
        let before = ALLOCS.load(Ordering::SeqCst);
        for t in 0..2_000u64 {
            fx.render(t * 7, &positions, &mut out);
        }
        let after = ALLOCS.load(Ordering::SeqCst);
        fechar_janela();
        assert_eq!(
            before,
            after,
            "{name} alocou {} vez(es) em 2000 frames de {N} pixels (outras threads, ignoradas: {})",
            after - before,
            FORA_DA_THREAD.load(Ordering::SeqCst)
        );
    }
}

/// Controlo negativo do TD-023: uma thread de fundo aloca em ciclo enquanto a biblioteca
/// inteira renderiza. O contador global antigo reprovaria; o por thread tem de ver zero.
#[test]
fn ruido_de_fundo_noutra_thread_nao_reprova_o_render() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());

    const N: usize = 512;
    let positions: Vec<Vec3> = (0..N)
        .map(|i| Vec3::new(i as f32 * 0.02, (i % 32) as f32 * 0.03, 0.0))
        .collect();
    let mut out = vec![PixelColor::default(); N];
    let effects = library();
    for (_, fx) in &effects {
        for t in 0..50u64 {
            fx.render(t, &positions, &mut out);
        }
    }
    let (por_thread, global) = medir_com_ruido_de_fundo(|| {
        let mut n = 0;
        for (_, fx) in &effects {
            for t in 0..200u64 {
                fx.render(t * 7, &positions, &mut out);
                n += 1;
            }
        }
        n
    });
    exigir_ruido_ignorado(por_thread, global, "a biblioteca de efeitos");
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

    abrir_janela();
    let before = ALLOCS.load(Ordering::SeqCst);
    for t in 0..100u64 {
        fx.render(t, &positions, &mut out);
    }
    let after = ALLOCS.load(Ordering::SeqCst);
    fechar_janela();

    assert!(
        after > before,
        "o gate não detectou um efeito que aloca — ele não estaria provando nada"
    );
}

#[test]
fn o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());
    exigir_que_ve_alocacao_plantada();
}
