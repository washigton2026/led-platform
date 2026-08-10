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

fn erro_detalhe(l: &str) -> String {
    parse(l)
        .unwrap()
        .get("error")
        .and_then(|e| e.get("detail").cloned())
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
    // **Barreira causal, não `yield_now()`.**
    //
    // A poda só acontece quando o `send` falha, e ele só falha depois de a thread escritora
    // do subscritor notar o socket fechado e sair (largando o `Receiver`). Isso é assíncrono:
    // `yield_now()` não é sincronização, é uma sugestão ao escalonador. Com ele, este teste
    // reprovava em cerca de 3 de 5 execuções — flake **anterior** a esta fatia, encontrado ao
    // correr a suíte repetidamente. É a mesma classe do TD-003, e a correção é a mesma:
    // esperar pela **condição observável**, com prazo.
    let limite = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while f.cp.subscribers() != 0 && std::time::Instant::now() < limite {
        f.cp.broadcast(r#"{"event":"x"}"#);
        std::thread::sleep(std::time::Duration::from_millis(2));
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

/// A fronteira dos 64 KiB é uma **linha**, não uma zona — e é o `+1` do `take(MAX_LINE + 1)`
/// que a mantém no sítio certo.
///
/// Um teto imposto durante a leitura tem uma forma fácil de errar: cortar um byte cedo e
/// recusar um pedido legítimo de exatamente 64 KiB. Este teste anda um passo para cada lado.
/// Sem ele, `take(MAX_LINE)` passaria em tudo o resto e partiria o caso da fronteira.
#[test]
fn um_passo_de_cada_lado_do_teto_muda_o_veredito() {
    use led_daemon_bin::server::MAX_LINE;

    // Um `hello` válido, enchido no campo `client` até dar exatamente MAX_LINE bytes.
    let molde = |enchimento: usize| {
        format!(r#"{{"v":1,"id":1,"cmd":"hello","client":"{}"}}"#, "A".repeat(enchimento))
    };
    let base = molde(0).len();
    let no_teto = molde(MAX_LINE - base);
    assert_eq!(no_teto.len(), MAX_LINE, "o molde tem de assentar no teto exato");

    let f = subir("teto");
    let mut c = Cliente::ligar(&f).unwrap();
    let r = c.envia(&no_teto);
    assert_eq!(
        campo(&r, "client").len(),
        MAX_LINE - base + 2, // +2 pelas aspas do JSON
        "uma linha de exatamente {MAX_LINE} bytes é legítima e tem de ser aceite: {}",
        &r[..r.len().min(120)]
    );

    // Um byte acima: recusado — e a ligação fecha, por isso a escrita pode partir a meio.
    // Um `writeln!().unwrap()` aqui rebentaria com `BrokenPipe` e o teste culparia o cliente
    // por aquilo que o daemon fez de propósito.
    let acima = molde(MAX_LINE - base + 1);
    assert_eq!(acima.len(), MAX_LINE + 1);
    let mut c2 = Cliente::ligar(&f).unwrap();
    let _ = c2.s.write_all(acima.as_bytes());
    let _ = c2.s.write_all(b"\n");
    let _ = c2.s.flush();
    let r2 = c2.le();
    assert_eq!(erro_code(&r2), "bad_request", "um byte acima do teto tem de ser recusado: {r2}");

    // O **código** não chega para distinguir: JSON malformado também é `bad_request`. Se o
    // teto fosse `take(MAX_LINE)` sem o `+1`, a linha seria **truncada** e o daemon
    // responderia `bad_request` por JSON inválido — mesma resposta, motivo errado, e o resto
    // da linha ficaria no socket a ser lido como um pedido novo. Estas duas asserções são o
    // que separa "recusei porque é longa" de "engasguei-me com um fragmento".
    assert!(
        erro_detalhe(&r2).contains("demasiado longa"),
        "a recusa tem de ser por comprimento, não por JSON truncado: {r2}"
    );
    let mut sobra = String::new();
    let n = c2.r.read_line(&mut sobra).unwrap_or(0);
    assert_eq!(n, 0, "a ligação devia ter fechado; o resto da linha voltou como pedido: {sobra}");
}
