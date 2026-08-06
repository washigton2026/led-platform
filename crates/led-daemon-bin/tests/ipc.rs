//! Testes de integração do IPC (GS3) — os seis cenários do enunciado.
//!
//! Sobem um `ControlPlane` + `Server` reais em socket temporário e falam o protocolo por
//! bytes. Não há mock do transporte: se o enquadramento por linha estiver errado, falha aqui.

#![cfg(unix)]

use led_daemon_bin::json::parse;
use led_daemon_bin::server::{ControlPlane, Server};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Fixture {
    path: std::path::PathBuf,
    cp: Arc<ControlPlane>,
    flag: Arc<AtomicBool>,
}

fn subir(nome: &str) -> Fixture {
    let path = std::env::temp_dir().join(format!("lumyx-gs3-{nome}-{}.sock", std::process::id()));
    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));
    let srv = Server::bind(&path).expect("bind");
    srv.spawn(Arc::clone(&cp));
    Fixture { path, cp, flag }
}

struct Cliente {
    s: UnixStream,
    r: BufReader<UnixStream>,
}

impl Cliente {
    fn ligar(f: &Fixture) -> std::io::Result<Self> {
        let s = UnixStream::connect(&f.path)?;
        let r = BufReader::new(s.try_clone()?);
        Ok(Self { s, r })
    }
    fn envia(&mut self, l: &str) -> String {
        writeln!(self.s, "{l}").unwrap();
        self.s.flush().unwrap();
        self.le()
    }
    fn le(&mut self) -> String {
        let mut l = String::new();
        self.r.read_line(&mut l).unwrap();
        l.trim().to_string()
    }
    fn hello(&mut self) -> String {
        self.envia(r#"{"v":1,"id":1,"cmd":"hello","client":"teste"}"#)
    }
}

/// Extrai o token do detalhe `repita com "confirm":"cfm-…"`.
///
/// Escrito como função e não inline porque a 1.ª versão usava `split('"').nth(1)`, que
/// devolve `confirm` — o rótulo, não o valor. Um teste de duas fases que usa o token errado
/// testa a rejeição, não a aceitação.
fn token_de(resposta: &str) -> String {
    let detalhe = parse(resposta)
        .unwrap()
        .get("error")
        .and_then(|e| e.get("detail").cloned())
        .and_then(|d| d.as_str().map(String::from))
        .expect("detalhe com o token");
    let marca = "\"confirm\":\"";
    let i = detalhe.find(marca).expect("marca do token") + marca.len();
    detalhe[i..].split('"').next().expect("token").to_string()
}

fn campo(l: &str, k: &str) -> String {
    parse(l).unwrap().get(k).map(|v| v.to_string()).unwrap_or_default()
}
fn erro_code(l: &str) -> String {
    parse(l)
        .unwrap()
        .get("error")
        .and_then(|e| e.get("code").cloned())
        .and_then(|c| c.as_str().map(String::from))
        .unwrap_or_default()
}

// ── 1. Conexão ────────────────────────────────────────────────────────────────
#[test]
fn conexao_e_handshake() {
    let f = subir("conn");
    let mut c = Cliente::ligar(&f).unwrap();
    let r = c.hello();
    assert_eq!(campo(&r, "ok"), "true", "{r}");
    assert_eq!(campo(&r, "id"), "1", "a resposta correlaciona pelo id");
    assert_eq!(campo(&r, "v"), "1");
    let p = c.envia(r#"{"v":1,"id":2,"cmd":"ping"}"#);
    assert_eq!(campo(&p, "pong"), "true", "{p}");
}

/// O socket é **owner-only**. Isto é `/security`, não estética.
#[test]
fn socket_e_owner_only_0600() {
    use std::os::unix::fs::PermissionsExt;
    let f = subir("perm");
    let m = std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777;
    assert_eq!(m, 0o600, "socket tem de ser 0600, veio {m:o}");
}

/// Nada é aceite antes do `hello`.
#[test]
fn comando_antes_do_hello_e_recusado() {
    let f = subir("auth");
    let mut c = Cliente::ligar(&f).unwrap();
    let r = c.envia(r#"{"v":1,"id":9,"cmd":"status"}"#);
    assert_eq!(erro_code(&r), "unauthenticated", "{r}");
    assert_eq!(campo(&r, "id"), "9", "mesmo a recusar, o id volta");
}

// ── 2. Reconexão ──────────────────────────────────────────────────────────────
#[test]
fn reconexao_apos_desligar() {
    let f = subir("recon");
    {
        let mut c = Cliente::ligar(&f).unwrap();
        assert_eq!(campo(&c.hello(), "ok"), "true");
    } // cai fora de escopo: socket fechado
    let mut c2 = Cliente::ligar(&f).expect("o servidor tem de continuar a aceitar");
    assert_eq!(campo(&c2.hello(), "ok"), "true", "reconexão tem de funcionar");
}

// ── 3. Múltiplos clientes ─────────────────────────────────────────────────────
#[test]
fn multiplos_clientes_em_simultaneo() {
    let f = subir("multi");
    let mut a = Cliente::ligar(&f).unwrap();
    let mut b = Cliente::ligar(&f).unwrap();
    let mut c = Cliente::ligar(&f).unwrap();
    assert_eq!(campo(&a.hello(), "ok"), "true");
    assert_eq!(campo(&b.hello(), "ok"), "true");
    assert_eq!(campo(&c.hello(), "ok"), "true");
    // Cada ligação tem o SEU handshake: o `hello` de um não autentica o outro.
    assert_eq!(campo(&a.envia(r#"{"v":1,"id":5,"cmd":"ping"}"#), "pong"), "true");
    assert_eq!(campo(&b.envia(r#"{"v":1,"id":6,"cmd":"version"}"#), "protocol"), "1");
    assert_eq!(campo(&c.envia(r#"{"v":1,"id":7,"cmd":"status"}"#), "ok"), "true");
}

#[test]
fn subscribe_recebe_eventos_e_so_quem_pediu() {
    let f = subir("sub");
    let mut a = Cliente::ligar(&f).unwrap();
    a.hello();
    assert_eq!(campo(&a.envia(r#"{"v":1,"id":2,"cmd":"subscribe"}"#), "subscribed"), "true");
    assert_eq!(f.cp.subscribers(), 1, "um subscritor registado");

    f.cp.broadcast(r#"{"event":"teste"}"#);
    let ev = a.le();
    assert_eq!(campo(&ev, "async"), "true", "evento assíncrono: {ev}");
    assert!(ev.contains(r#""event":"teste""#), "{ev}");
    assert!(parse(&ev).unwrap().get("id").is_none(), "evento NÃO tem id — não responde a nada");
}

/// Um subscritor que desliga é **podado** — sem isto o `Sender` acumula para sempre.
#[test]
fn subscritor_morto_e_podado() {
    let f = subir("prune");
    {
        let mut a = Cliente::ligar(&f).unwrap();
        a.hello();
        a.envia(r#"{"v":1,"id":2,"cmd":"subscribe"}"#);
        assert_eq!(f.cp.subscribers(), 1);
    }
    // Duas emissões: a 1ª descobre o canal morto, a 2ª confirma a poda.
    for _ in 0..2 {
        f.cp.broadcast(r#"{"event":"x"}"#);
        std::thread::yield_now();
    }
    assert_eq!(f.cp.subscribers(), 0, "ligação morta tem de ser removida");
}

// ── 4. Socket inexistente ─────────────────────────────────────────────────────
#[test]
fn socket_inexistente_falha_sem_panico() {
    let e = UnixStream::connect("/tmp/lumyx-nao-existe-de-todo.sock");
    assert!(e.is_err(), "ligar a um socket que não existe tem de dar erro");
}

/// Socket órfão de uma execução anterior **não** impede o arranque.
#[test]
fn socket_orfao_e_substituido() {
    let path = std::env::temp_dir().join(format!("lumyx-gs3-orfao-{}.sock", std::process::id()));
    std::fs::write(&path, b"lixo de uma execucao anterior").unwrap();
    let srv = Server::bind(&path).expect("socket órfão não pode bloquear o arranque");
    assert_eq!(srv.path(), path);
}

// ── 5. Comando inválido ───────────────────────────────────────────────────────
#[test]
fn comandos_invalidos_tem_codigo_proprio_e_preservam_o_id() {
    let f = subir("inval");
    let mut c = Cliente::ligar(&f).unwrap();
    c.hello();
    for (pedido, esperado) in [
        (r#"{"v":1,"id":10,"cmd":"autodestruir"}"#, "unknown_command"),
        (r#"{"v":99,"id":11,"cmd":"ping"}"#, "unsupported_version"),
        (r#"{"v":1,"id":12,"cmd":"seek"}"#, "invalid_args"),
        (r#"nao e json"#, "bad_request"),
    ] {
        let r = c.envia(pedido);
        assert_eq!(erro_code(&r), esperado, "{pedido} → {r}");
    }
    // A ligação SOBREVIVE a quatro pedidos inválidos seguidos.
    assert_eq!(campo(&c.envia(r#"{"v":1,"id":13,"cmd":"ping"}"#), "pong"), "true");
}

/// Um `load` de ficheiro inexistente é recusado com código próprio — e o daemon continua.
#[test]
fn load_de_ficheiro_inexistente_nao_derruba_o_daemon() {
    let f = subir("loaderr");
    let mut c = Cliente::ligar(&f).unwrap();
    c.hello();
    // Sem laço a drenar a fila, o pedido expira em `engine_busy` — que também é um código
    // enumerado e também prova que a ligação não morre.
    let r = c.envia(r#"{"v":1,"id":3,"cmd":"load","args":{"path":"/nao/existe.lumyx"}}"#);
    let code = erro_code(&r);
    assert!(
        code == "load_failed" || code == "engine_busy",
        "esperava um código enumerado, veio {r}"
    );
    assert_eq!(campo(&c.envia(r#"{"v":1,"id":4,"cmd":"ping"}"#), "pong"), "true");
}

// ── 6. Shutdown remoto ────────────────────────────────────────────────────────

/// **Duas fases.** Sem `confirm` nada acontece — só um token.
#[test]
fn shutdown_exige_confirmacao_em_duas_fases() {
    let f = subir("shut");
    let mut c = Cliente::ligar(&f).unwrap();
    c.hello();

    let r1 = c.envia(r#"{"v":1,"id":2,"cmd":"shutdown"}"#);
    assert_eq!(erro_code(&r1), "confirmation_required", "{r1}");
    assert!(!f.flag.load(Ordering::Relaxed), "a 1.ª fase NAO pode encerrar nada");

    let token = token_de(&r1);

    let r2 = c.envia(&format!(r#"{{"v":1,"id":3,"cmd":"shutdown","confirm":"{token}"}}"#));
    assert_eq!(campo(&r2, "shutting_down"), "true", "{r2}");
    assert!(f.flag.load(Ordering::Relaxed), "a 2.ª fase encerra");
}

#[test]
fn token_de_confirmacao_e_de_uso_unico() {
    let f = subir("tok");
    let mut c = Cliente::ligar(&f).unwrap();
    c.hello();
    let r1 = c.envia(r#"{"v":1,"id":2,"cmd":"shutdown"}"#);
    let token = token_de(&r1);

    assert_eq!(campo(&c.envia(&format!(r#"{{"v":1,"id":3,"cmd":"shutdown","confirm":"{token}"}}"#)), "shutting_down"), "true");
    let r3 = c.envia(&format!(r#"{{"v":1,"id":4,"cmd":"shutdown","confirm":"{token}"}}"#));
    assert_eq!(erro_code(&r3), "confirmation_required", "token reutilizado tem de falhar: {r3}");
}

#[test]
fn token_errado_e_recusado() {
    let f = subir("badtok");
    let mut c = Cliente::ligar(&f).unwrap();
    c.hello();
    c.envia(r#"{"v":1,"id":2,"cmd":"shutdown"}"#);
    let r = c.envia(r#"{"v":1,"id":3,"cmd":"shutdown","confirm":"cfm-inventado"}"#);
    assert_eq!(erro_code(&r), "confirmation_required", "{r}");
    assert!(!f.flag.load(Ordering::Relaxed), "token errado não encerra");
}
