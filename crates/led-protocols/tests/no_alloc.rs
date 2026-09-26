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
use std::cell::Cell;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use led_core::UniverseData;
use led_protocols::{DdpBackend, ProtocolBackend};

// ── O instrumento ────────────────────────────────────────────────────────────
//
// Este gate reprovava no Ubuntu — **4 alocações em 10 000 envios**, de forma
// não-determinística — e passava sempre no macOS. Ao longo de várias sessões a causa não
// foi encontrada, e a razão era o próprio instrumento: a mensagem dizia
// *"allocated 4 time(s)"* e nada mais. Um número sem proveniência não se pode
// investigar de outro sistema operativo, e mexer num orçamento a partir dele seria
// inventar a resposta. Por isso o contador passou a registar o **tamanho** de cada
// alocação da janela e se ela veio da **thread do teste** ou de outra — sem mudar o que
// a asserção afirmava, para que a próxima falha chegasse com prova em vez de enigma.
//
// **A falha chegou, e a regra de decisão estava escrita antes de haver dados**
// (run 34193510776, 2026-09-08):
//
//   assertion failed: DDP send path allocated 4 time(s) over 10000 frames
//     — tamanhos=[148, 608, 48, 96] bytes · fora da thread do teste=4 de 4
//
// **`4 de 4`.** Nenhuma veio da thread que corre o corpo do teste. E os tamanhos
// corroboram: são heterogéneos e pequenos, enquanto a falsificação de controlo — um
// `vec![7u8; 900]` plantado no laço — produzia `tamanhos=[900, 900, 900, 900]`. Uma
// alocação real do payload teria a forma do payload.
//
// **O defeito era do gate, não do caminho de envio.** `ALLOCS` incrementava em *toda*
// alocação do processo: o alocador é global e a janela media tudo o que qualquer thread
// fizesse lá dentro — o harness do `libtest` inclusive. No macOS essa contaminação media
// zero; no Linux, quatro.
//
// **A correcção atribui por thread**: só conta o que é alocado na thread do teste. Isto
// **fortalece** a asserção em vez de a afrouxar — a propriedade afirmada é *"o caminho de
// envio não aloca"*, e `send_universe` é síncrono, não gera threads: por construção,
// nada do caminho sob teste pode alocar noutra thread. O que sai da conta é ruído alheio.
//
// As alocações de fora continuam a ser **contadas e reportadas** — como contexto, nunca
// como falha. Foi esse número que resolveu a investigação, e apagá-lo cegaria a próxima.
//
// **O instrumento não pode alocar**, ou entraria em recursão dentro de si próprio: só
// atómicos e um `thread_local` com inicialização `const` (que não faz alocação preguiçosa).

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

/// A janela de medição está aberta? Fora dela nada é registado — o aquecimento tem de
/// poder alocar à vontade.
static MEDINDO: AtomicBool = AtomicBool::new(false);
/// Alocações da janela que **não** vieram da thread marcada.
static FORA_DA_THREAD: AtomicUsize = AtomicUsize::new(0);
/// Quantas amostras de tamanho já foram guardadas.
static N_AMOSTRAS: AtomicUsize = AtomicUsize::new(0);
const AMOSTRAS: usize = 8;
static TAMANHOS: [AtomicUsize; AMOSTRAS] = [const { AtomicUsize::new(0) }; AMOSTRAS];

thread_local! {
    /// Marca a thread que corre o corpo do teste. `const` para não alocar ao inicializar.
    static E_A_THREAD_DO_TESTE: Cell<bool> = const { Cell::new(false) };
}

/// Regista uma alocação da janela. Chamada de dentro do alocador: **sem alocar**.
///
/// Devolve `true` se a alocação é **atribuível à thread do teste** — e é só essa que conta
/// para a asserção. Sem esta atribuição o gate media o processo inteiro em vez do caminho
/// sob teste, e reprovava por ruído alheio (ver o cabeçalho do módulo).
fn registar(tamanho: usize) -> bool {
    // A atribuição é lida SEMPRE, mesmo fora da janela: é ela que decide se a alocação
    // entra no contador, e o `before`/`after` do teste é um delta sobre ele.
    //
    // `try_with`: durante a destruição de TLS um `with` normal entraria em pânico, e um
    // pânico dentro do alocador aborta o processo sem diagnóstico nenhum.
    let minha = E_A_THREAD_DO_TESTE.try_with(|c| c.get()).unwrap_or(false);

    if !MEDINDO.load(Ordering::Relaxed) {
        return minha;
    }

    if !minha {
        // Contada como CONTEXTO, nunca como falha. Foi este número — `4 de 4` — que
        // provou que o vermelho do Ubuntu era do instrumento e não do caminho de envio.
        FORA_DA_THREAD.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    // Só as da thread do teste ficam registadas por tamanho: quando o gate reprovar, são
    // estas as culpadas, e enchê-lo com ruído de outras threads escondia-as.
    let i = N_AMOSTRAS.fetch_add(1, Ordering::Relaxed);
    if i < AMOSTRAS {
        TAMANHOS[i].store(tamanho, Ordering::Relaxed);
    }
    true
}

/// Zera o diagnóstico. Os dois testes deste ficheiro partilham os estáticos; sem isto o
/// segundo herdaria as amostras do primeiro e o relatório mentiria sobre quem alocou.
fn reset_diagnostico() {
    FORA_DA_THREAD.store(0, Ordering::SeqCst);
    N_AMOSTRAS.store(0, Ordering::SeqCst);
    for t in TAMANHOS.iter() {
        t.store(0, Ordering::SeqCst);
    }
}

/// Serializa os testes deste binário: eles partilham a janela de medição e os estáticos do
/// diagnóstico. Mesmo mecanismo do `led-hal/tests/no_alloc.rs:37` — reuso, não um segundo.
static ALLOC_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if registar(l.size()) {
            ALLOCS.fetch_add(1, Ordering::SeqCst);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if registar(l.size()) {
            ALLOCS.fetch_add(1, Ordering::SeqCst);
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // No `realloc` o que interessa é o tamanho NOVO: é ele que diz para onde o buffer
        // cresceu, e um crescimento é a assinatura mais provável de um `Vec` escondido.
        if registar(n) {
            ALLOCS.fetch_add(1, Ordering::SeqCst);
        }
        System.realloc(p, l, n)
    }
}

/// O que a janela apanhou, em texto — para a mensagem de falha dizer **o quê**, não só
/// *quantos*.
fn diagnostico() -> String {
    let n = N_AMOSTRAS.load(Ordering::Relaxed);
    let vistas = n.min(AMOSTRAS);
    let mut tam = String::new();
    for (i, t) in TAMANHOS.iter().take(vistas).enumerate() {
        if i > 0 {
            tam.push_str(", ");
        }
        tam.push_str(&t.load(Ordering::Relaxed).to_string());
    }
    if n > AMOSTRAS {
        tam.push_str(", …");
    }
    format!(
        "tamanhos=[{tam}] bytes · na thread do teste={n} \
         · fora da thread, IGNORADAS={}",
        FORA_DA_THREAD.load(Ordering::Relaxed)
    )
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn ddp_backend_send_path_is_alloc_free() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());

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

    // A janela abre AQUI: o aquecimento acima pode alocar à vontade, e o instrumento só
    // regista o que acontecer daqui para a frente.
    E_A_THREAD_DO_TESTE.with(|c| c.set(true));
    reset_diagnostico();
    MEDINDO.store(true, Ordering::SeqCst);

    let before = ALLOCS.load(Ordering::SeqCst);
    for _ in 0..10_000 {
        backend.send_universe(&u).unwrap();
        while recv.recv(&mut sink).is_ok() {}
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    MEDINDO.store(false, Ordering::SeqCst);

    assert_eq!(
        before,
        after,
        "DDP send path allocated {} time(s) over 10000 frames — {}",
        after - before,
        diagnostico()
    );
}

/// **Controlo negativo do gate acima.** Sem ele, *"atribuir por thread"* e *"desligar o
/// contador"* são indistinguíveis: as duas deixariam o teste principal verde para sempre.
///
/// Planta na **própria thread do teste** a mesma forma que a falsificação de 2026-08-13d
/// plantou no caminho de envio — um `vec![7u8; 900]` — e exige que o instrumento a veja.
/// Se alguém enfraquecer a atribuição ao ponto de não contar nada, este teste reprova.
#[test]
fn o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste() {
    let _gate = ALLOC_GATE.lock().unwrap_or_else(|e| e.into_inner());

    E_A_THREAD_DO_TESTE.with(|c| c.set(true));
    reset_diagnostico();
    MEDINDO.store(true, Ordering::SeqCst);

    let before = ALLOCS.load(Ordering::SeqCst);
    let plantada = vec![7u8; 900];
    let after = ALLOCS.load(Ordering::SeqCst);

    MEDINDO.store(false, Ordering::SeqCst);

    // Depois de fechar a janela, para o optimizador não apagar a alocação e tornar este
    // controlo vacuoso — um gate que não exercita o que afirma é pior que gate nenhum.
    std::hint::black_box(&plantada);

    assert!(
        after > before,
        "o contador NAO viu uma alocação feita na própria thread do teste — a atribuição \
         por thread deixou de contar o que devia, e o gate principal tornou-se vacuoso. {}",
        diagnostico()
    );

    // Não basta contar: o diagnóstico tem de **nomear** a culpada, senão a próxima falha
    // real volta a chegar como enigma.
    let vistas = N_AMOSTRAS.load(Ordering::SeqCst).min(AMOSTRAS);
    let viu_o_tamanho = TAMANHOS
        .iter()
        .take(vistas)
        .any(|t| t.load(Ordering::Relaxed) == 900);
    assert!(
        viu_o_tamanho,
        "a alocação de 900 B foi contada mas não registada por tamanho — {}",
        diagnostico()
    );
}
