//! `ledctl` — cliente de linha de comando do daemon (GS3).
//!
//! Fala o protocolo v1 sobre UDS. Faz o `hello` sozinho, envia um comando, imprime a
//! resposta crua e sai com um código que um script consegue usar.

#![cfg(unix)]

use led_daemon_bin::json::parse;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

const USAGE: &str = "\
ledctl — cliente de controlo do led-daemon (protocolo v1, UDS)

USO:
    ledctl --socket <CAMINHO> <COMANDO> [ARGS]

COMANDOS:
    ping                       Vivo?
    version                    Versão do protocolo e do motor
    status                     Estado, posição, duração, ticks e a contabilidade
                               de CADA nó da saída (`outputs`: addr/frames/errors)
    load <FICHEIRO> [--assume-integrity]
                               Carrega um .lumyx. Com --assume-integrity ARMA
                               também; sem ela fica em `loaded` e o `play` recusa
                               com `not_armed` — o gate do pré-voo fica visível.
    unload | play | pause | stop
    seek <MS>
    subscribe                  Fica a imprimir eventos até Ctrl-C
    shutdown [--yes]           DUAS FASES: sem --yes o daemon devolve um token e
                               nada acontece; com --yes o ledctl pede o token e
                               repete o comando.

SAÍDA: 0 ok · 1 o daemon recusou · 2 erro de uso · 3 não consigo falar com o socket
";

fn send(stream: &mut UnixStream, linha: &str) -> std::io::Result<()> {
    stream.write_all(linha.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// Lê UMA resposta (ignora eventos assíncronos, que não têm `id`).
fn read_reply(r: &mut impl BufRead) -> std::io::Result<String> {
    loop {
        let mut l = String::new();
        if r.read_line(&mut l)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "socket fechou"));
        }
        let l = l.trim().to_string();
        if l.is_empty() {
            continue;
        }
        // Um evento pode chegar entre o pedido e a resposta; só a resposta tem `id`.
        if parse(&l).ok().and_then(|j| j.get("id").cloned()).is_some() {
            return Ok(l);
        }
        println!("{l}"); // evento: mostra e continua à espera
    }
}

fn ok(linha: &str) -> bool {
    parse(linha)
        .ok()
        .and_then(|j| j.get("ok").cloned())
        .map(|v| v == led_daemon_bin::json::Json::Bool(true))
        .unwrap_or(false)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut socket = None;
    let mut resto: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--socket" => {
                i += 1;
                match argv.get(i) {
                    Some(s) => socket = Some(s.clone()),
                    None => {
                        eprintln!("erro: --socket exige um caminho\n\n{USAGE}");
                        std::process::exit(2);
                    }
                }
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            outro => resto.push(outro.to_string()),
        }
        i += 1;
    }

    let (Some(socket), Some(cmd)) = (socket, resto.first().cloned()) else {
        eprintln!("erro: faltam --socket e/ou o comando\n\n{USAGE}");
        std::process::exit(2);
    };

    let pedido = match cmd.as_str() {
        "ping" | "version" | "status" | "unload" | "play" | "pause" | "stop" | "subscribe" => {
            format!(r#"{{"v":1,"id":2,"cmd":"{cmd}"}}"#)
        }
        "load" => {
            let Some(path) = resto.get(1) else {
                eprintln!("erro: `load` exige um ficheiro\n\n{USAGE}");
                std::process::exit(2);
            };
            let ai = resto.iter().any(|a| a == "--assume-integrity");
            format!(
                r#"{{"v":1,"id":2,"cmd":"load","args":{{"path":"{}","assume_integrity":{ai}}}}}"#,
                led_daemon_bin::json::escape(path)
            )
        }
        "seek" => {
            let Some(ms) = resto.get(1).and_then(|s| s.parse::<u64>().ok()) else {
                eprintln!("erro: `seek` exige milissegundos (inteiro >= 0)\n\n{USAGE}");
                std::process::exit(2);
            };
            format!(r#"{{"v":1,"id":2,"cmd":"seek","args":{{"to_ms":{ms}}}}}"#)
        }
        "shutdown" => r#"{"v":1,"id":2,"cmd":"shutdown"}"#.to_string(),
        outro => {
            eprintln!("erro: comando desconhecido `{outro}`\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erro: não consigo falar com {socket}: {e}");
            std::process::exit(3);
        }
    };
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(3);
        }
    });

    // Handshake obrigatório.
    let hello = r#"{"v":1,"id":1,"cmd":"hello","client":"ledctl/0.1"}"#;
    if send(&mut stream, hello).is_err() {
        eprintln!("erro: falha ao enviar o handshake");
        std::process::exit(3);
    }
    match read_reply(&mut reader) {
        Ok(l) if ok(&l) => {}
        Ok(l) => {
            println!("{l}");
            eprintln!("erro: handshake recusado");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(3);
        }
    }

    let _ = send(&mut stream, &pedido);
    let resposta = match read_reply(&mut reader) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(3);
        }
    };
    println!("{resposta}");

    // `shutdown --yes`: a 1.ª fase devolve o token; a 2.ª usa-o.
    if cmd == "shutdown" && resto.iter().any(|a| a == "--yes") && !ok(&resposta) {
        let detail = parse(&resposta)
            .ok()
            .and_then(|j| j.get("error")?.get("detail")?.as_str().map(String::from))
            .unwrap_or_default();
        // O token vem no detalhe, entre aspas: `repita com "confirm":"cfm-…"`.
        if let Some(tok) = detail.split('"').nth(3) {
            let segunda = format!(r#"{{"v":1,"id":3,"cmd":"shutdown","confirm":"{tok}"}}"#);
            let _ = send(&mut stream, &segunda);
            match read_reply(&mut reader) {
                Ok(l) => {
                    println!("{l}");
                    std::process::exit(if ok(&l) { 0 } else { 1 });
                }
                Err(_) => std::process::exit(0), // o daemon fechou: encerrou mesmo
            }
        }
    }

    std::process::exit(if ok(&resposta) { 0 } else { 1 });
}
