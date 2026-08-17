//! C0.3b — validar a premissa da técnica escolhida: `127.0.0.1:1` como nó morto.
//!
//! **Branch descartável. Não faz parte do produto.**
//!
//! O C0.3 encontrou o mecanismo portátil: porta local fechada → ICMP port-unreachable → o
//! `send` seguinte falha. Ubuntu e macOS, sem privilégios, sem rota, sem multicast.
//!
//! Esta versão responde às perguntas que faltam para escrever o teste **correctamente**:
//!
//!   1. A porta 1 está mesmo sem ouvinte nos runners? Se algum dia tiver um serviço, a
//!      premissa cai — e o teste tem de reprovar a dizer isso, nunca passar em silêncio.
//!   2. **Quantos envios** são precisos até o erro aparecer? Se for sempre 2, o laço é
//!      trivial; se variar, o prazo tem de o cobrir.
//!   3. **Quanto tempo** demora? É esse número que dimensiona o prazo — e escolhê-lo por
//!      medição é o ponto todo, porque um prazo inventado é um `sleep` disfarçado.
//!
//! Nada aqui afirma um errno. O contrato é "o transporte falhou", e a própria sonda já
//! mostrou o mesmo alvo a dar `PermissionDenied` numa máquina e `BrokenPipe` noutra.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

const ALVO: &str = "127.0.0.1:1";
const PRAZO: Duration = Duration::from_secs(2);

/// Envia em laço até o transporte acusar erro. Devolve `(envios, tempo)` ou `None` se o
/// prazo esgotar — que é o caso em que a **premissa não se estabeleceu**.
fn ate_falhar(alvo: &str) -> Option<(u32, Duration)> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(alvo.parse::<std::net::SocketAddr>().ok()?).ok()?;
    let payload = vec![7u8; 910];
    let t0 = Instant::now();
    let mut envios = 0u32;
    while t0.elapsed() < PRAZO {
        envios += 1;
        if sock.send(&payload).is_err() {
            return Some((envios, t0.elapsed()));
        }
        // Sem `sleep`: cede a vez ao escalonador para o ICMP poder ser processado, e
        // continua a perguntar. É espera CAUSAL — a condição é observada, não cronometrada.
        std::thread::yield_now();
    }
    None
}

fn main() {
    println!("### PLATAFORMA: {}", std::env::consts::OS);

    // ── 1 · A premissa: a porta está livre? ──────────────────────────────────
    //
    // Não se testa com `bind`: portas < 1024 exigem privilégios, portanto um `bind` falhado
    // não distingue "ocupada" de "não autorizada". O que se mede é o COMPORTAMENTO de que a
    // técnica depende — que é a única coisa que importa.
    println!("\n### 1 · a premissa (porta 1 sem ouvinte) e o custo do laço");

    const TENTATIVAS: u32 = 30;
    let mut envios_min = u32::MAX;
    let mut envios_max = 0u32;
    let mut tempo_max = Duration::ZERO;
    let mut falhas_de_premissa = 0u32;

    for _ in 0..TENTATIVAS {
        match ate_falhar(ALVO) {
            Some((n, t)) => {
                envios_min = envios_min.min(n);
                envios_max = envios_max.max(n);
                tempo_max = tempo_max.max(t);
            }
            None => falhas_de_premissa += 1,
        }
    }

    if falhas_de_premissa > 0 {
        println!("###   PREMISSA FALHOU em {falhas_de_premissa}/{TENTATIVAS} — ha ouvinte em {ALVO}");
        println!("###   ou o stack nao devolve o erro. A tecnica NAO e utilizavel aqui.");
    } else {
        println!("###   premissa OK em {TENTATIVAS}/{TENTATIVAS} tentativas");
        println!("###   envios ate falhar: min={envios_min} max={envios_max}");
        println!("###   tempo maximo ate falhar: {:?}", tempo_max);
    }

    // ── 2 · Controlo negativo: um alvo VIVO nunca pode falhar ────────────────
    //
    // Sem isto, um laço que falhasse por qualquer razão (socket partido, prazo curto)
    // seria indistinguivel de "o no morreu". O controlo prova que o laço distingue os dois.
    println!("\n### 2 · controlo negativo (alvo vivo NAO pode falhar)");
    let rx = UdpSocket::bind("127.0.0.1:0").expect("rx");
    let vivo = rx.local_addr().unwrap().to_string();
    match ate_falhar(&vivo) {
        Some((n, t)) => println!("###   ERRO GRAVE: um alvo vivo falhou ao {n}o envio ({t:?})"),
        None => println!("###   correcto: alvo vivo nunca falhou dentro do prazo de {PRAZO:?}"),
    }

    // ── 3 · O erro concreto, so para o REGISTO — nunca para asserção ─────────
    println!("\n### 3 · que erro o SO devolve (REGISTO, nao contrato)");
    let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
    sock.connect(ALVO.parse::<std::net::SocketAddr>().unwrap()).unwrap();
    let p = vec![7u8; 910];
    for i in 1..=3 {
        match sock.send(&p) {
            Ok(_) => println!("###   envio {i}: ok"),
            Err(e) => println!("###   envio {i}: {:?} / {}", e.kind(), e),
        }
        std::thread::yield_now();
    }
}
