//! ADR-0029 §9 — **o custo do fan-out é medido antes de ser optimizado.**
//!
//! `OutputManager::send` faz `buf.clone()` e toma um `Mutex` **por alvo, por frame**. Nenhum
//! gate de `no_alloc` cobre o caminho de saída do daemon — verificado, não presumido: os
//! cinco que existem cobrem `led-hal`, `led-protocols`, `led-sequencer`, `led-pixel-engine` e
//! `audio-core`, e o do `led-hal` não vale aqui porque o caminho DDP **contorna o `Hal`** por
//! decisão de 2026-07-09d.
//!
//! **Este ficheiro não promete `no_alloc` e não optimiza nada.** Mede, regista, e guarda
//! contra desvio **superlinear** — a disciplina do TD-011 e do TD-012, que mediram e
//! mandaram *não* optimizar.
//!
//! ## Porque a carga por alvo é idêntica nas duas configurações
//!
//! A repartição é derivada: fatias de `max_pixels` com o resto no último nó. Com 6200 px
//! (o rig real) os quatro primeiros levariam 1500 e o quinto 200 — e aí a comparação mediria
//! *fatias diferentes*, não *número de alvos*. Com **7500 px** os cinco levam exactamente
//! 1500, que é o mesmo que o alvo único leva. A única variável passa a ser quantos são.
//!
//! ## Um só `#[test]`, de propósito
//!
//! O alocador é **global ao processo** e cada ficheiro de teste é um binário próprio. Um
//! `#[test]` vizinho a alocar em paralelo tornaria a medição ruído — é a mesma razão que o
//! `led-protocols/tests/no_alloc.rs` já regista.

use led_core::{LogicalFrame, PixelColor};
use led_daemon_bin::output::Alvo;
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use led_hardware_profile::{Calibration as ProfileCalibration, HardwareProfile};
use std::alloc::{GlobalAlloc, Layout, System};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

// ── O contador ───────────────────────────────────────────────────────────────
//
// Fechado atrás de `MEDINDO` para que a **fase de tempo** não pague o `fetch_add`: contar
// alocações penalizaria a configuração de 5 alvos mais que a de 1 (tem mais alocações), e
// isso inflacionaria exactamente o rácio que este ficheiro existe para medir.

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

/// Píxeis por nó — o `max_pixels` do preset, para que as fatias sejam todas iguais.
const POR_NO: usize = 1500;
const FRAMES: usize = 200;
const RONDAS: usize = 5;

fn perfil(gamma: f32) -> HardwareProfile {
    let mut p = profile_by_name("esp32-poe-wled-ddp").expect("preset do catálogo");
    // γ=1.0 e brilho=1.0 é a **identidade**, e nesse caso nenhuma LUT é construída — é o
    // "sem calibração" real, não uma aproximação.
    p.calibration = ProfileCalibration { gamma, brightness: 1.0 };
    p
}

/// Abre uma saída com `n` alvos de loopback, cada um com [`POR_NO`] píxeis.
fn abrir(n: usize, gamma: f32) -> (OutputManager, Vec<UdpSocket>, usize) {
    let p = perfil(gamma);
    let px = POR_NO * n;
    let socks: Vec<UdpSocket> = (0..n)
        .map(|_| {
            let s = UdpSocket::bind("127.0.0.1:0").unwrap();
            s.set_nonblocking(true).unwrap();
            s
        })
        .collect();

    // `resolve_muitos` e não `alvos` forjados à mão: é a repartição **derivada** que corre em
    // produção, e um benchmark que montasse os alvos por outro caminho estaria a medir um
    // caminho que ninguém usa.
    let specs: Vec<String> = socks.iter().map(|s| s.local_addr().unwrap().to_string()).collect();
    let cfg = OutputConfig::resolve_muitos(&p, &specs, px).expect("resolver");
    assert!(
        cfg.alvos.iter().all(|a: &Alvo| a.pixel_count == POR_NO),
        "as fatias têm de ser TODAS iguais, senão isto mede fatias diferentes em vez de \
         número de alvos: {:?}",
        cfg.alvos.iter().map(|a| a.pixel_count).collect::<Vec<_>>()
    );
    let mgr = OutputManager::open(cfg).expect("abrir saída");
    (mgr, socks, px)
}

/// Esvazia os receptores **fora** da janela de medição: um buffer cheio no núcleo mudaria o
/// custo do `sendto` a meio da medição, e o que se está a medir é o remetente.
fn drenar(socks: &[UdpSocket]) {
    let mut b = [0u8; 2048];
    for s in socks {
        while s.recv(&mut b).is_ok() {}
    }
}

struct Medida {
    alocacoes_por_frame: f64,
    ns_por_frame: u128,
}

fn medir(n: usize, gamma: f32) -> Medida {
    let (mgr, socks, px) = abrir(n, gamma);
    let frame = LogicalFrame::new(vec![PixelColor { r: 128, g: 64, b: 32 }; px], 0);

    // Aquecimento: o primeiro envio pode alocar buffers que depois são reusados, e contá-lo
    // mediria o arranque em vez do regime.
    for _ in 0..20 {
        let _ = mgr.send(&frame);
    }
    drenar(&socks);

    // ── Fase 1: alocações ────────────────────────────────────────────────────
    ALOCACOES.store(0, Ordering::SeqCst);
    MEDINDO.store(true, Ordering::SeqCst);
    for _ in 0..FRAMES {
        let _ = mgr.send(&frame);
    }
    MEDINDO.store(false, Ordering::SeqCst);
    let alocacoes = ALOCACOES.load(Ordering::SeqCst);
    drenar(&socks);

    // ── Fase 2: tempo ────────────────────────────────────────────────────────
    //
    // **Mínimo de várias rondas, não média.** A média mede também o escalonador da máquina;
    // o mínimo é o estimador menos ruidoso para "quanto custa quando nada interfere", e é a
    // pergunta certa quando o que se procura é desvio superlinear.
    let mut melhor = u128::MAX;
    for _ in 0..RONDAS {
        let t = Instant::now();
        for _ in 0..FRAMES {
            let _ = mgr.send(&frame);
        }
        melhor = melhor.min(t.elapsed().as_nanos());
        drenar(&socks);
    }

    Medida {
        alocacoes_por_frame: alocacoes as f64 / FRAMES as f64,
        ns_por_frame: melhor / FRAMES as u128,
    }
}

#[test]
fn o_custo_do_fanout_e_medido_e_nao_cresce_superlinearmente() {
    let um_sem = medir(1, 1.0);
    let um_com = medir(1, 2.2);
    let cinco_sem = medir(5, 1.0);
    let cinco_com = medir(5, 2.2);

    let perfil_build = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!(
        "\n── ADR-0029 §9 · custo do fan-out ({perfil_build}, {POR_NO} px por alvo, \
         {FRAMES} frames × {RONDAS} rondas, mínimo)\n\
         {:<28} {:>14} {:>16}\n\
         {:<28} {:>14.2} {:>16}\n\
         {:<28} {:>14.2} {:>16}\n\
         {:<28} {:>14.2} {:>16}\n\
         {:<28} {:>14.2} {:>16}\n",
        "configuração",
        "aloc/frame",
        "ns/frame",
        "1 alvo, sem calibração",
        um_sem.alocacoes_por_frame,
        um_sem.ns_por_frame,
        "1 alvo, com γ 2.2",
        um_com.alocacoes_por_frame,
        um_com.ns_por_frame,
        "5 alvos, sem calibração",
        cinco_sem.alocacoes_por_frame,
        cinco_sem.ns_por_frame,
        "5 alvos, com γ 2.2",
        cinco_com.alocacoes_por_frame,
        cinco_com.ns_por_frame,
    );

    let razao_sem = cinco_sem.ns_por_frame as f64 / um_sem.ns_por_frame.max(1) as f64;
    let razao_com = cinco_com.ns_por_frame as f64 / um_com.ns_por_frame.max(1) as f64;
    println!("razão 5:1 — sem calibração {razao_sem:.2}× · com γ 2.2 {razao_com:.2}×\n");

    // ── As alocações: gate EXACTO, porque a propriedade é exacta ─────────────
    //
    // Contagem de alocações não tem ruído — ao contrário do tempo. Uma propriedade exacta
    // merece uma asserção exacta; arredondá-la para "menos que X" desperdiçaria o único
    // sinal deste ficheiro que não depende da máquina.

    // **O caminho rápido é livre de alocação, e isto fixa-o.** Com um alvo e offset 0, o
    // `send` entrega o frame do chamador ao driver sem fatiar nem clonar. É a única
    // configuração do daemon que hoje cumpre a regra do hot-path do `CLAUDE.md`.
    assert_eq!(
        um_sem.alocacoes_por_frame, 0.0,
        "o caminho rápido (1 alvo, offset 0) deixou de ser livre de alocação"
    );

    // Uma clonagem por alvo, nem mais. `n` alvos ⇒ `n` alocações: se alguém clonar o frame
    // INTEIRO por alvo em vez da fatia, o número não muda mas o custo sim — é o gate de
    // tempo que apanha esse; este apanha uma alocação a mais por nó.
    assert_eq!(
        cinco_sem.alocacoes_por_frame, 5.0,
        "5 alvos têm de custar exactamente 5 alocações por frame (uma clonagem por nó)"
    );

    // **A calibração é aplicada UMA vez, antes do fan-out** — e este é o gate que o prova.
    // O ADR-0019 (Emenda 1) põe-na na fronteira lógica de saída, partilhada pelos três
    // protocolos. Se alguém a mover para dentro do laço por alvo, este delta passa de 1
    // para 5 e o teste reprova. Nenhum outro teste do repositório afirma isto.
    assert_eq!(
        um_com.alocacoes_por_frame - um_sem.alocacoes_por_frame,
        1.0,
        "a calibração tem de custar exactamente 1 alocação por frame, com 1 alvo"
    );
    assert_eq!(
        cinco_com.alocacoes_por_frame - cinco_sem.alocacoes_por_frame,
        1.0,
        "e a MESMA 1 com 5 alvos — se for 5, a calibração passou a correr por nó e o \
         ADR-0019 Emenda 1 foi violado: ela é aplicada uma vez, ANTES do fan-out"
    );

    // ── O tempo: gate GENEROSO, e a razão está escrita ───────────────────────
    //
    // Medido nesta máquina (debug, mínimo de 5 rondas): **5.20× sem calibração** e **4.40×
    // com γ 2.2**. Ou seja, o "~5×" do ADR **é ultrapassado** no caso sem calibração — e
    // não é desvio superlinear, é estrutural: o alvo único usa o caminho rápido, que não
    // fatia nem clona, portanto o denominador é mais barato que um quinto do numerador. Com
    // calibração a razão **desce**, porque a calibração é custo fixo (aplicada uma vez) e
    // portanto engorda o denominador.
    //
    // O limite fica em 8×: acima do estrutural com folga para o ruído de um runner
    // partilhado, e muito abaixo do que uma regressão quadrática produziria (5 alvos em
    // O(n²) dariam ~25×). Um limite colado aos 5.20 medidos seria um gate que reprova por
    // carga da máquina em vez de por defeito — o erro que o TD-006 já nomeou.
    const LIMITE: f64 = 8.0;
    assert!(
        razao_sem <= LIMITE,
        "5 alvos custaram {razao_sem:.2}× um alvo (sem calibração), acima do limite de \
         {LIMITE}×. Linear seria ~5×; isto é desvio SUPERLINEAR e o ADR-0012 volta à mesa \
         — com o número na mão, como o critério de reversão exige."
    );
    assert!(
        razao_com <= LIMITE,
        "5 alvos custaram {razao_com:.2}× um alvo (com γ 2.2), acima do limite de {LIMITE}×."
    );
}
