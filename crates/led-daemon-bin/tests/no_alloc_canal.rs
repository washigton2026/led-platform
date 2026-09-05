//! `control-protocol.md:118` — **o `no_alloc` da saída permanece verde com o canal de
//! controlo activo.**
//!
//! # A dívida que este ficheiro fecha
//!
//! Aquela linha promete um gate que **não existia**. Auditei os cinco `no_alloc.rs` do
//! repositório (`audio-core`, `led-hal`, `led-pixel-engine`, `led-protocols`,
//! `led-sequencer`) e **nenhum** menciona `OutputManager`, `ControlPlane` ou
//! `run_with_control`: a promessa estava escrita e sem consumidor.
//!
//! # Porquê um binário próprio, e não mais um `#[test]` no `custo_do_fanout.rs`
//!
//! Aquele ficheiro declara, no seu cabeçalho, ter **um só `#[test]` de propósito** — um
//! vizinho a alocar em paralelo tornaria a medição ruído. Acrescentar-lhe um segundo teste
//! violaria a razão pela qual ele é como é. Em Rust cada ficheiro de teste é um binário
//! próprio, portanto isto **é** o isolamento, não um caminho paralelo: o contador e a
//! disciplina do `MEDINDO` são reutilizados tal como estão.
//!
//! # A propriedade, dita sem folga
//!
//! *Exercitar a superfície de controlo não acrescenta uma única alocação ao caminho de
//! envio.* Não é «o blackout é grátis» — não é, e este ficheiro **mede** o que ele custa
//! quando accionado, em vez de o esconder.

use led_core::{LogicalFrame, PixelColor};
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use led_hardware_profile::{Calibration as ProfileCalibration, HardwareProfile};
use std::alloc::{GlobalAlloc, Layout, System};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ── O contador ───────────────────────────────────────────────────────────────
//
// Cópia deliberada do `custo_do_fanout.rs`: cada binário de teste tem de instalar o SEU
// `#[global_allocator]` — não há como partilhá-lo entre binários. A disciplina do `MEDINDO`
// vem de lá, e existe para que o arranque possa alocar à vontade sem contaminar a janela.

struct Contador;
static ALOCACOES: AtomicUsize = AtomicUsize::new(0);
static MEDINDO: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if MEDINDO.load(Ordering::Relaxed) {
            ALOCACOES.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if MEDINDO.load(Ordering::Relaxed) {
            ALOCACOES.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if MEDINDO.load(Ordering::Relaxed) {
            ALOCACOES.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Contador = Contador;

const PX: usize = 1500;
const FRAMES: usize = 200;

/// Identidade na calibração: γ=1.0 e brilho=1.0 **não** constroem LUT nenhuma, portanto o
/// caminho rápido é o caminho rápido de verdade (ADR-0019 Emenda 1). Sem isto, mediríamos a
/// alocação da calibração e concluiríamos que o canal de controlo a causou.
fn perfil() -> HardwareProfile {
    let mut p = profile_by_name("esp32-poe-wled-ddp").expect("preset do catálogo");
    p.calibration = ProfileCalibration { gamma: 1.0, brightness: 1.0 };
    p
}

fn abrir() -> (OutputManager, UdpSocket) {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_nonblocking(true).unwrap();
    let cfg = OutputConfig::resolve(&perfil(), &s.local_addr().unwrap().to_string(), PX)
        .expect("resolver");
    (OutputManager::open(cfg).expect("abrir saída"), s)
}

/// Corre `f` com a janela de medição aberta e devolve quantas alocações aconteceram.
fn medir(f: impl FnOnce()) -> usize {
    ALOCACOES.store(0, Ordering::Relaxed);
    MEDINDO.store(true, Ordering::Relaxed);
    f();
    MEDINDO.store(false, Ordering::Relaxed);
    ALOCACOES.load(Ordering::Relaxed)
}

/// **Um só `#[test]`**, pela mesma razão do `custo_do_fanout.rs`: o contador é global ao
/// processo, e um teste vizinho a alocar em paralelo tornaria estas contagens ruído.
#[test]
fn o_canal_de_controlo_nao_acrescenta_alocacoes_ao_caminho_de_envio() {
    let (om, _sock) = abrir();
    let frame = LogicalFrame::new(vec![PixelColor { r: 200, g: 100, b: 50 }; PX], 0);

    // Aquecimento FORA da janela: o primeiro envio pode alocar buffers do socket e do driver,
    // e isso é arranque, não hot path. Medir sem aquecer contaria o arranque como defeito.
    for _ in 0..20 {
        om.send(&frame).unwrap();
    }

    // ── A · A REFERÊNCIA ────────────────────────────────────────────────────
    // Caminho rápido (um alvo, offset 0, sem calibração) sem tocar no controlo.
    let so_envio = medir(|| {
        for _ in 0..FRAMES {
            om.send(&frame).unwrap();
        }
    });
    assert_eq!(
        so_envio, 0,
        "premissa deste ficheiro: o caminho rápido é livre de alocação. \
         Se isto falhar, o defeito é anterior ao canal de controlo e as outras \
         asserções deixam de significar o que dizem — {so_envio} alocações em {FRAMES} frames"
    );

    // ── B · O GATE ──────────────────────────────────────────────────────────
    // A MESMA carga, com a superfície de controlo exercitada **entre** os envios: o estado é
    // lido, accionado e levantado a cada frame. É isto que a linha 118 chama «canal de
    // controlo activo» — comandos a chegar e a ser aplicados, não o palco apagado.
    let com_controlo = medir(|| {
        for _ in 0..FRAMES {
            let _ = om.blackout_activo();
            om.blackout_accionar();
            om.blackout_levantar();
            om.send(&frame).unwrap();
        }
    });
    assert_eq!(
        com_controlo, 0,
        "o canal de controlo NÃO pode acrescentar alocações ao envio — \
         {com_controlo} em {FRAMES} frames, contra {so_envio} sem controlo"
    );
    assert!(!om.blackout_activo(), "o ciclo accionar/levantar tem de acabar desligado");

    // ── C · O CUSTO DECLARADO, FIXADO POR TESTE ─────────────────────────────
    // Com o blackout ACCIONADO a máscara custa **uma alocação por nó e por frame**, porque
    // `LogicalFrame` possui o `Vec` e não há construtor por empréstimo — dar-lhe um seria
    // mexer no `led-core`, que é contrato canónico. Isto foi declarado no commit do D6; aqui
    // deixa de ser prosa e passa a ser número guardado.
    //
    // O critério de reversão do ADR-0017 diz que uma colisão com o gate de alocação do
    // hot-path exige decisão nova. Esta asserção é o detector dessa colisão: se alguém
    // tornar a máscara mais cara, o número sobe e o teste reprova.
    om.blackout_accionar();
    let com_blackout = medir(|| {
        for _ in 0..FRAMES {
            om.send(&frame).unwrap();
        }
    });
    om.blackout_levantar();
    assert_eq!(
        com_blackout, FRAMES,
        "com o blackout accionado o custo é UMA alocação por frame (um alvo), \
         medido e declarado — não zero, e não mais que isso"
    );

    // E o palco volta ao caminho rápido depois de levantar: o custo é do estado apagado,
    // não uma penalização permanente por a funcionalidade existir.
    let depois = medir(|| {
        for _ in 0..FRAMES {
            om.send(&frame).unwrap();
        }
    });
    assert_eq!(
        depois, 0,
        "levantado o blackout, o caminho rápido volta a ser livre de alocação — {depois}"
    );
}
