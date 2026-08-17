//! C0.3 — sonda de portabilidade da indução de "nó morto".
//!
//! **Branch descartável. Não faz parte do produto.**
//!
//! A pergunta, exactamente: existe um alvo UDP para o qual
//!
//!     bind("0.0.0.0:0")  →  connect(alvo)   PASSA
//!                        →  send(payload)   FALHA
//!
//! de forma determinística, sem privilégios e sem depender de rota multicast, **nos dois
//! sistemas**? É esse par que os testes do ADR-0029 §5 precisam para matar um nó e deixar
//! os outros vivos.
//!
//! No macOS (medido localmente) existe exactamente UM: `255.255.255.255:1`, que falha no
//! `send` com EACCES por o `SO_BROADCAST` não estar posto. Em Ubuntu esse mesmo alvo falha
//! no **connect**, o que rebenta o `open()` e é a causa das 2 falhas da CI.
//!
//! O segundo `send` é feito **depois de uma pausa**: no Linux um socket UDP ligado devolve
//! ECONNREFUSED no envio seguinte à chegada do ICMP port-unreachable. Sem a pausa esse
//! mecanismo não teria tempo de se manifestar e seria registado como "não falha" — um
//! falso negativo dentro da própria sonda.

use std::net::UdpSocket;
use std::time::Duration;

struct Resultado {
    alvo: &'static str,
    connect: String,
    send1: String,
    send2: String,
    serve: bool,
}

fn sondar(alvo: &'static str, bytes: usize) -> Resultado {
    let mut r = Resultado {
        alvo,
        connect: "—".into(),
        send1: "—".into(),
        send2: "—".into(),
        serve: false,
    };
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            r.connect = format!("bind falhou: {:?}", e.kind());
            return r;
        }
    };
    let addr: std::net::SocketAddr = match alvo.parse() {
        Ok(a) => a,
        Err(_) => {
            r.connect = "parse falhou".into();
            return r;
        }
    };
    match sock.connect(addr) {
        Ok(()) => r.connect = "ok".into(),
        Err(e) => {
            r.connect = format!("FALHA {:?}", e.kind());
            return r; // rebenta o open(): inútil para o nosso fim
        }
    }
    let p = vec![7u8; bytes];
    r.send1 = match sock.send(&p) {
        Ok(_) => "ok".into(),
        Err(e) => format!("FALHA {:?}", e.kind()),
    };
    // A pausa é o que dá ao ICMP tempo de voltar. Ver doc do módulo.
    std::thread::sleep(Duration::from_millis(200));
    r.send2 = match sock.send(&p) {
        Ok(_) => "ok".into(),
        Err(e) => format!("FALHA {:?}", e.kind()),
    };
    // Serve se o connect passou E algum send falhou.
    r.serve = r.connect == "ok" && (r.send1.starts_with("FALHA") || r.send2.starts_with("FALHA"));
    r
}

fn main() {
    println!("### PLATAFORMA: {}", std::env::consts::OS);
    println!("### payload 910 B (o do rig real, ADR-0029)\n");

    let candidatos: &[&'static str] = &[
        "255.255.255.255:1",   // broadcast limitado — o que os testes usam hoje
        "255.255.255.255:0",   // idem, porta 0
        "240.0.0.1:1",         // classe E reservada
        "192.0.2.1:1",         // TEST-NET-1
        "198.51.100.1:1",      // TEST-NET-2
        "203.0.113.1:1",       // TEST-NET-3
        "127.0.0.1:1",         // porta local fechada → ICMP
        "127.0.0.2:1",         // outra loopback, porta fechada
        "127.255.255.255:1",   // broadcast dirigido da loopback
        "10.255.255.255:1",    // broadcast dirigido privado
        "192.168.255.255:1",   // idem
        "169.254.1.1:1",       // link-local
        "224.0.0.1:1",         // multicast all-hosts
        "239.255.255.250:1",   // multicast administrativo
        "0.0.0.0:1",           // `any` como destino
        "1.2.3.4:1",           // encaminhável mas inalcançável
        "127.0.0.1:0",         // porta 0
        "[::1]:1",             // IPv6 a partir de socket AF_INET
    ];

    println!("{:<22} {:<16} {:<18} {:<18} {}", "ALVO", "CONNECT", "SEND#1", "SEND#2 (+200ms)", "SERVE?");
    println!("{}", "-".repeat(96));
    let mut uteis = Vec::new();
    for a in candidatos {
        let r = sondar(a, 910);
        println!(
            "{:<22} {:<16} {:<18} {:<18} {}",
            r.alvo,
            r.connect,
            r.send1,
            r.send2,
            if r.serve { "*** SIM ***" } else { "nao" }
        );
        if r.serve {
            uteis.push(r.alvo);
        }
    }

    println!("\n### CANDIDATOS QUE SERVEM NESTA PLATAFORMA: {}", uteis.len());
    for u in &uteis {
        println!("###   {u}");
    }
    if uteis.is_empty() {
        println!("###   NENHUM — a inducao por socket nao e viavel aqui");
    }

    // Eixo do tamanho: portátil, mas NÃO selectivo por alvo (afecta todos os nós ao mesmo
    // tempo). Medido para o registo ficar completo, não como candidato.
    println!("\n### EIXO DO TAMANHO (nao selectivo — so para o registo)");
    let rx = UdpSocket::bind("127.0.0.1:0").expect("rx");
    let valido: &'static str = Box::leak(rx.local_addr().unwrap().to_string().into_boxed_str());
    let mut maior_ok = 0usize;
    for bytes in [910usize, 8_000, 9_216, 9_217, 16_000, 65_507, 65_508] {
        let r = sondar(valido, bytes);
        let estado = if r.send1 == "ok" { "passa" } else { &r.send1 };
        if r.send1 == "ok" {
            maior_ok = bytes;
        }
        println!("###   {bytes:>6} B -> {estado}");
    }
    println!("###   maior que passou: {maior_ok} B");
}
