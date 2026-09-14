//! **O custo de `Stage::on_tick`, medido — porque hoje nada o mede.**
//!
//! Que falsidade este ficheiro impede? Esta: *«o gate de alocação do daemon cobre o laço»*.
//! Não cobre. O `custo_do_fanout.rs` chama `mgr.send(&frame)` **directamente** (`:133`,
//! `:141`, `:156`) — mede o `OutputManager`, e **nunca** o `Stage::on_tick`. Tudo o que
//! for acrescentado ao tick é, hoje, **invisível** a todos os gates do repositório: passaria
//! verde sem exercitar nada, que é KB-012 na forma mais cara.
//!
//! Este ficheiro é o **instrumento**, entregue ANTES da funcionalidade que o vai precisar
//! (D4/preview, ADR-0015). É a mesma ordem que resolveu a F7.2: primeiro o que mede, depois
//! o veredito. Enquanto o preview não existir, ele fixa a **linha de base** do tick — e é
//! essa linha que torna o custo do preview mensurável no dia em que ele chegar.
//!
//! **Zero linhas de produção.** `stage.rs` e `output.rs` não são tocados.
//!
//! ## Porque a atribuição é por thread
//!
//! O alocador é **global ao processo**: a janela mede o que qualquer thread fizer lá dentro,
//! incluindo o harness do `libtest` e — aqui — a thread que drena o socket. É exactamente a
//! contaminação que a F7.2 diagnosticou no Ubuntu (`fora da thread do teste = 4 de 4`, com o
//! caminho de envio limpo). A conta fica restrita à thread que tica; sem isso este gate
//! nasceria com o flake já lá dentro.
//!
//! O `thread_local` é de inicialização **`const`** de propósito: um `lazy` alocaria dentro do
//! próprio alocador e entraria em recursão.

use led_core::PixelColor;
use led_daemon::State;
use led_daemon_bin::{profile_by_name, Stage, StageTick};
use led_show_recorder::{ShowRecord, ShowWriter};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

// ── O contador ───────────────────────────────────────────────────────────────

struct Contador;
static ALOCACOES: AtomicUsize = AtomicUsize::new(0);
static MEDINDO: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Só a thread que chama `on_tick` conta. Ver a nota de módulo.
    static ESTA_THREAD_CONTA: Cell<bool> = const { Cell::new(false) };
}

/// `try_with` e não `with`: durante o teardown de uma thread o TLS já não está acessível, e
/// um pânico dentro do alocador global não teria onde ser apanhado.
fn conta_aqui() -> bool {
    ESTA_THREAD_CONTA.try_with(|f| f.get()).unwrap_or(false)
}

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if MEDINDO.load(Ordering::Relaxed) && conta_aqui() {
            ALOCACOES.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if MEDINDO.load(Ordering::Relaxed) && conta_aqui() {
            ALOCACOES.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if MEDINDO.load(Ordering::Relaxed) && conta_aqui() {
            ALOCACOES.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Contador = Contador;

// ── Parâmetros ───────────────────────────────────────────────────────────────

/// O `max_pixels` do preset: **um** alvo com offset 0 é o caminho rápido que o
/// `custo_do_fanout.rs:214` fixa em zero alocações no `send`. Assim, o que este ficheiro
/// medir a mais que zero vem do **resto** do tick, não do fan-out.
const PX: u32 = 1500;
/// 40 Hz — a cadência real do daemon, não um número de conveniência.
const PASSO_MS: u64 = 25;
const AQUECIMENTO: usize = 50;
const MEDIDOS: usize = 200;
/// Folga sobre os ticks precisos, para o show nunca acabar a meio da medição.
const FRAMES: u32 = (AQUECIMENTO + MEDIDOS) as u32 + 100;

/// O orçamento do próprio tick a 40 Hz. **Não é um número inventado**: se `on_tick` não
/// couber no seu próprio período, isso é defeito, não ruído. Generoso de propósito — a
/// lição do TD-006 é exacto nas alocações, folgado no relógio.
const ORCAMENTO_US: u128 = (PASSO_MS * 1000) as u128;

fn escrever(nome: &str) -> String {
    // O pid no nome pela razão que o `e2e_output.rs:44-52` documenta: duas execuções
    // concorrentes de `cargo test` escreveriam o MESMO ficheiro em `temp_dir()`.
    let path = std::env::temp_dir().join(format!("{}-{nome}", std::process::id()));
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ShowWriter::new(f, PX).unwrap();
    for i in 0..FRAMES {
        w.write_frame(&ShowRecord {
            timestamp_ms: i as u64 * PASSO_MS,
            pixels: vec![PixelColor { r: 10, g: 20, b: 30 }; PX as usize],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path.to_str().unwrap().to_string()
}

/// Uma medição do laço: alocações por tick, tick a tick, e o tempo da janela.
struct Medida {
    por_tick: Vec<usize>,
    media_us: u128,
}

fn medir() -> Medida {
    let path = escrever("on_tick.lumyx");
    let sock = UdpSocket::bind("127.0.0.1:0").expect("socket de captura");
    let alvo = sock.local_addr().unwrap().to_string();

    // **O dreno é obrigatório e não é decoração.** 250 ticks × 1500 px em DDP são ~1000
    // datagramas; sem ninguém a ler, o buffer do receptor enche e o `send` passa a falhar —
    // o tick mudava de caminho a meio da medição e o número não descreveria nada. A thread
    // do dreno NÃO conta para o contador (atribuição por thread), que é precisamente a razão
    // de ela existir aqui em vez de contaminar tudo.
    let parar = std::sync::Arc::new(AtomicBool::new(false));
    let dreno = {
        let s = sock.try_clone().unwrap();
        let parar = parar.clone();
        s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !parar.load(Ordering::Relaxed) {
                let _ = s.recv(&mut buf);
            }
        })
    };

    let profile = profile_by_name("esp32-poe-wled-ddp").expect("preset do catálogo");
    let mut stage = Stage::open(&path, &[alvo], &profile).expect("abrir palco");

    let mut pos = 0u64;
    let mut agora = 0u64;
    let tick = |s: &mut Stage, pos: &mut u64, agora: &mut u64| -> StageTick {
        let r = s.on_tick(State::Playing, *pos, *agora);
        *pos += PASSO_MS;
        *agora += PASSO_MS;
        r
    };

    // Aquecimento fora da janela: o primeiro tick abre buffers e lê o ficheiro pela primeira
    // vez, e isso não é o custo de regime.
    for _ in 0..AQUECIMENTO {
        let r = tick(&mut stage, &mut pos, &mut agora);
        assert!(
            matches!(r, StageTick::Sent { .. }),
            "o aquecimento tem de enviar de verdade, senão a medição não exercita o caminho: {r:?}"
        );
    }

    // **Alocado ANTES da janela**, senão o próprio registo entrava na conta.
    let mut por_tick = vec![0usize; MEDIDOS];

    ESTA_THREAD_CONTA.with(|f| f.set(true));
    MEDINDO.store(true, Ordering::Relaxed);
    let t0 = Instant::now();
    let mut anterior = ALOCACOES.load(Ordering::Relaxed);

    for slot in por_tick.iter_mut() {
        let _ = stage.on_tick(State::Playing, pos, agora);
        pos += PASSO_MS;
        agora += PASSO_MS;
        let atual = ALOCACOES.load(Ordering::Relaxed);
        *slot = atual - anterior;
        anterior = atual;
    }

    let decorrido = t0.elapsed();
    MEDINDO.store(false, Ordering::Relaxed);
    ESTA_THREAD_CONTA.with(|f| f.set(false));

    parar.store(true, Ordering::Relaxed);
    let _ = dreno.join();
    let _ = std::fs::remove_file(&path);

    Medida { por_tick, media_us: decorrido.as_micros() / MEDIDOS as u128 }
}

#[test]
fn o_custo_de_alocacao_do_on_tick_e_estavel_e_conhecido() {
    let m = medir();

    // Premissa: a janela mediu mesmo alguma coisa. Uma janela vazia e "sem regressão" seriam
    // indistinguíveis — KB-012, e este repositório já foi mordido por isso (Miri N=0).
    assert_eq!(m.por_tick.len(), MEDIDOS, "a janela não mediu os ticks que diz");

    let primeiro = m.por_tick[0];
    let iguais = m.por_tick.iter().all(|&n| n == primeiro);
    assert!(
        iguais,
        "o custo do tick NÃO é estável — mín {:?}, máx {:?}, primeiros 8: {:?}. \
         Um tick que aloca de forma variável não tem linha de base, e sem linha de base o \
         custo do preview (ADR-0015) não é mensurável.",
        m.por_tick.iter().min(),
        m.por_tick.iter().max(),
        &m.por_tick[..8.min(m.por_tick.len())]
    );

    // **A linha de base, afirmada como valor exacto e não como limite.**
    //
    // Estabilidade sozinha NÃO chega, e é a armadilha deste ficheiro: uma alocação plantada
    // por tick mantém o custo perfeitamente constante — só o *valor* sobe. Um gate que só
    // afirmasse "é sempre igual" passaria com o defeito lá dentro. É por isso que o controlo
    // negativo ataca este número, e não a estabilidade.
    const BASELINE: usize = 4;
    assert_eq!(
        primeiro, BASELINE,
        "a linha de base de alocações por tick mudou: era {BASELINE}, é {primeiro}. \
         Se isto reprovou depois de acrescentar o preview (ADR-0015), o preview está a \
         alocar no hot-path e viola o invariante #3 da constituição."
    );

    // Relógio: limite generoso e derivado, não escolhido. O tick a 40 Hz tem 25 ms; se
    // `on_tick` não couber no próprio período, é defeito e não carga da máquina (TD-006).
    assert!(
        m.media_us < ORCAMENTO_US,
        "on_tick custou {} µs por tick, acima do orçamento de {} µs (o período de 40 Hz)",
        m.media_us,
        ORCAMENTO_US
    );

    println!(
        "on_tick: {} alocações/tick · {} µs/tick · {} ticks medidos",
        primeiro, m.media_us, MEDIDOS
    );
}
