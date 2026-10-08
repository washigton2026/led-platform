//! Perf gate: composing a timeline frame is allocation-free (pre-sized clip scratch,
//! in-place blend). Counting allocator (per-thread, TD-023), zero growth across 10k renders
//! after warm-up.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use led_core::PixelColor;
use led_pixel_engine::{Effect, SolidColor, Vec3};
use led_sequencer::{BlendMode, Clip, Timeline, Track};

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
/// A janela de medição está aberta?
static MEDINDO: AtomicBool = AtomicBool::new(false);
/// Alocações da janela vindas de OUTRAS threads. O contador global antigo somava-as.
static FORA_DA_THREAD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Marca a thread que corre o corpo do teste. `const` para não alocar ao inicializar.
    static E_A_THREAD_DO_TESTE: Cell<bool> = const { Cell::new(false) };
    /// Alocações contadas NESTA thread enquanto marcada — é este o número da asserção (TD-023,
    /// R7.2). Nenhuma outra thread escreve aqui: a identidade é a do próprio `thread_local`.
    static ALLOCS_DESTA_THREAD: Cell<usize> = const { Cell::new(0) };
}

// O contador era um `AtomicUsize` global que qualquer thread MARCADA incrementava, e a marca
// nunca era desligada: uma thread de outro teste do mesmo binário, já fora da sua janela mas
// ainda marcada, entrava na contagem da janela alheia. R6.1 (run 37735453730): 10/10 falhas em
// macos-26 vieram de OUTRA thread marcada, nenhuma da thread dona. Agora cada thread conta só
// para si, e quem lê é a própria thread do teste.

/// Soma uma alocação ao contador desta thread. Sem alocar; `try_with` pela mesma razão que
/// em `registar` (destruição de TLS).
fn contar_nesta_thread() {
    let _ = ALLOCS_DESTA_THREAD.try_with(|c| c.set(c.get() + 1));
}

/// Alocações contadas até agora na thread que chama.
fn allocs_desta_thread() -> usize {
    ALLOCS_DESTA_THREAD.with(|c| c.get())
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
            contar_nesta_thread();
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if registar() {
            contar_nesta_thread();
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if registar() {
            contar_nesta_thread();
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
    // Desmarcar: fora da janela esta thread deixa de ser «a do teste» (TD-023, R7.2).
    E_A_THREAD_DO_TESTE.with(|c| c.set(false));
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
    let antes = allocs_desta_thread();
    let iteracoes = corpo();
    while FORA_DA_THREAD.load(Ordering::SeqCst) < RUIDO_MINIMO && Instant::now() < prazo {
        std::thread::yield_now();
    }
    let depois = allocs_desta_thread();
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
        let antes = allocs_desta_thread();
        let plantada = plantar();
        let depois = allocs_desta_thread();
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

const N: usize = 300;

fn timeline_de_teste() -> Timeline {
    Timeline::new(N)
        .with_track(
            Track::new(BlendMode::Override)
                .with_clip(Clip::new(0, 10_000, Box::new(SolidColor(PixelColor::rgb(255, 0, 0)))).with_fades(100, 100)),
        )
        .with_track(
            Track::new(BlendMode::Add)
                .with_clip(Clip::new(0, 10_000, Box::new(SolidColor(PixelColor::rgb(0, 255, 0))))),
        )
}

#[test]
fn timeline_render_is_allocation_free() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let timeline = timeline_de_teste();
    let positions = vec![Vec3::ZERO; N];
    let mut out = vec![PixelColor::default(); N];

    for _ in 0..100 {
        timeline.render(500, &positions, &mut out); // warm-up flushes lazy init
    }
    abrir_janela();
    let before = allocs_desta_thread();
    for t in 0..10_000u64 {
        timeline.render(500 + (t % 7), &positions, &mut out);
    }
    let after = allocs_desta_thread();
    fechar_janela();

    assert_eq!(
        before,
        after,
        "timeline render allocated {} time(s) over 10000 frames (outras threads, ignoradas: {})",
        after - before,
        FORA_DA_THREAD.load(Ordering::SeqCst)
    );
}

/// Controlo negativo do TD-023 (o flake «180 vs 182» do PR #20 era deste ficheiro).
#[test]
fn ruido_de_fundo_noutra_thread_nao_reprova_o_render() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());
    let timeline = timeline_de_teste();
    let positions = vec![Vec3::ZERO; N];
    let mut out = vec![PixelColor::default(); N];
    for _ in 0..100 {
        timeline.render(500, &positions, &mut out);
    }
    let (por_thread, global) = medir_com_ruido_de_fundo(|| {
        let mut n = 0;
        for t in 0..1_000u64 {
            timeline.render(500 + (t % 7), &positions, &mut out);
            n += 1;
        }
        n
    });
    exigir_ruido_ignorado(por_thread, global, "timeline render");
}

#[test]
fn o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());
    exigir_que_ve_alocacao_plantada();
}

/// TD-023 (R7.2) — o caso exato das falhas do R6.1: uma thread MARCADA que não é a do teste
/// aloca DENTRO da janela. Com o contador global antigo, as alocações dela entravam na contagem
/// desta thread; agora não podem. Determinístico: sincronização só por atómicos (um `Barrier`
/// ou `Mutex` podia alocar na thread do teste e sujar a própria medição).
#[test]
fn outra_thread_marcada_nao_entra_na_contagem_desta() {
    use std::sync::Arc;
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());

    const N: usize = 1_000;
    let janela_aberta = Arc::new(AtomicBool::new(false));
    let terminou = Arc::new(AtomicBool::new(false));
    let outra = {
        let (janela_aberta, terminou) = (Arc::clone(&janela_aberta), Arc::clone(&terminou));
        std::thread::spawn(move || {
            E_A_THREAD_DO_TESTE.with(|c| c.set(true));
            while !janela_aberta.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            let antes = allocs_desta_thread();
            for _ in 0..N {
                std::hint::black_box(vec![0u8; 64]);
            }
            let vistas = allocs_desta_thread() - antes;
            terminou.store(true, Ordering::SeqCst);
            vistas
        })
    };

    abrir_janela();
    let antes = allocs_desta_thread();
    janela_aberta.store(true, Ordering::SeqCst);
    while !terminou.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let depois = allocs_desta_thread();
    fechar_janela();

    let vistas_na_outra = outra.join().unwrap();
    // Premissa: a outra thread estava mesmo marcada e o instrumento contou-a — para ELA.
    assert!(
        vistas_na_outra >= N,
        "premissa falhou: a thread marcada só contou {vistas_na_outra} de {N} alocações — controlo vacuoso"
    );
    assert_eq!(
        depois - antes,
        0,
        "{} alocação(ões) de OUTRA thread marcada entraram na contagem desta thread (TD-023)",
        depois - antes
    );
}
