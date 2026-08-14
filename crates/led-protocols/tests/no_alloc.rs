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
// Este gate reprova no Ubuntu — **4 alocações em 10 000 envios**, de forma
// não-determinística — e passa sempre no macOS. Ao longo de várias sessões a causa não
// foi encontrada, e a razão é o próprio instrumento: a mensagem dizia
// *"allocated 4 time(s)"* e nada mais. Um número sem proveniência não se pode
// investigar de outro sistema operativo, e mexer num orçamento a partir dele seria
// inventar a resposta.
//
// Por isso o contador passa a **registar o que apanhou**, sem mudar o que afirma:
//
//   * o **tamanho** de cada uma das primeiras alocações da janela — 48 bytes e 1 KiB
//     apontam para culpados diferentes;
//   * se a alocação veio da **thread do teste** ou de outra. O alocador é global ao
//     processo, portanto a janela mede tudo o que lá acontece: o harness do `libtest`
//     também corre. Aqui essa hipótese mede **0** (verificado com uma janela de controlo
//     em que o teste não chama nada do caminho DDP), mas isso é uma medição de macOS —
//     e é exactamente o Linux que falha.
//
// Nada disto altera a asserção: o gate continua a exigir **zero**. O que muda é que a
// próxima falha na CI chega com a prova em vez de com um enigma.
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
fn registar(tamanho: usize) {
    if !MEDINDO.load(Ordering::Relaxed) {
        return;
    }
    // `try_with`: durante a destruição de TLS um `with` normal entraria em pânico, e um
    // pânico dentro do alocador aborta o processo sem diagnóstico nenhum.
    let minha = E_A_THREAD_DO_TESTE.try_with(|c| c.get()).unwrap_or(false);
    if !minha {
        FORA_DA_THREAD.fetch_add(1, Ordering::Relaxed);
    }
    let i = N_AMOSTRAS.fetch_add(1, Ordering::Relaxed);
    if i < AMOSTRAS {
        TAMANHOS[i].store(tamanho, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        registar(l.size());
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        registar(l.size());
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        // No `realloc` o que interessa é o tamanho NOVO: é ele que diz para onde o buffer
        // cresceu, e um crescimento é a assinatura mais provável de um `Vec` escondido.
        registar(n);
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
        "tamanhos=[{tam}] bytes · fora da thread do teste={} de {n}",
        FORA_DA_THREAD.load(Ordering::Relaxed)
    )
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

    // A janela abre AQUI: o aquecimento acima pode alocar à vontade, e o instrumento só
    // regista o que acontecer daqui para a frente.
    E_A_THREAD_DO_TESTE.with(|c| c.set(true));
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
