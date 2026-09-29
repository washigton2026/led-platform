//! **TD-014 — `console.dropped` chega ao operador** (ADR-0026 §13-bis).
//!
//! Sem mocks: daemon real (servidor UDS + `ControlPlane`), console real, browser real por TCP.
//! A perda é **induzida** como em produção — um browser que abre o SSE e nunca lê, e uma
//! enchente de eventos do daemon — e o número é lido pela rota, não pelo `Fanout`.
//!
//! Três propriedades, três grupos:
//! 1. **Produtor → rota:** uma perda induzida faz `dropped` crescer, com o mesmo `since`.
//! 2. **Fronteira Rust → TS:** as chaves que a rota emite são exatamente as da interface
//!    `EstadoDescartes` do contrato gerado.
//! 3. **`/api/upstream` intocado:** o corpo e o tipo gerado são os de `57cf21d`.

#![cfg(unix)]

use led_console_bin::http::{serve, Config, ConsoleServer};
use led_daemon_bin::server::{ControlPlane, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TS_VERSIONADO: &str = include_str!("../contract/lumyx-contract.generated.ts");

struct Rig {
    path: std::path::PathBuf,
    cp: Arc<ControlPlane>,
    shutdown: Arc<AtomicBool>,
}

fn subir(nome: &str) -> Rig {
    let path =
        std::env::temp_dir().join(format!("lumyx-td014-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let shutdown = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&shutdown));
    Server::bind(&path).expect("bind").spawn(Arc::clone(&cp));
    Rig { path, cp, shutdown }
}

impl Rig {
    fn console(&self) -> ConsoleServer {
        let c = serve(
            "127.0.0.1:0".parse().unwrap(),
            Config { socket_daemon: self.path.to_str().unwrap().to_string(), exporter: None },
        )
        .expect("console");
        esperar(|| c.fanout.subscricoes_ipc() == 1, "a subscricao upstream nao se estabeleceu");
        c
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Barreira causal com prazo (TD-003): espera uma condição, nunca um atraso fixo.
fn esperar(cond: impl Fn() -> bool, porque: &str) {
    let limite = Instant::now() + Duration::from_secs(5);
    while Instant::now() < limite {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timeout: {porque}");
}

fn get(c: &ConsoleServer, caminho: &str) -> (u16, String) {
    let mut s = TcpStream::connect(c.addr).expect("ligar");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    write!(s, "GET {caminho} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    s.flush().unwrap();
    let mut bruto = String::new();
    let _ = s.read_to_string(&mut bruto);
    let status = bruto
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("resposta sem status: {bruto:?}"));
    let corpo = bruto.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, corpo)
}

/// Um browser que abre o SSE e **nunca lê** — é assim que a fila enche em produção.
fn browser_parado(c: &ConsoleServer) -> BufReader<TcpStream> {
    let mut s = TcpStream::connect(c.addr).expect("ligar");
    write!(s, "GET /api/events HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    s.flush().unwrap();
    let mut r = BufReader::new(s);
    let mut l = String::new();
    while r.read_line(&mut l).unwrap_or(0) > 0 && l != "\r\n" {
        l.clear();
    }
    r
}

/// Lê um inteiro sem sinal de um corpo JSON plano (`"chave":123`). Sem `serde` de propósito:
/// o corpo é de dois campos, e um parser à medida torna visível qualquer campo a mais.
fn campo_u64(corpo: &str, chave: &str) -> u64 {
    let marca = format!(r#""{chave}":"#);
    let i = corpo.find(&marca).unwrap_or_else(|| panic!("sem `{chave}` em {corpo}")) + marca.len();
    corpo[i..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("`{chave}` nao e u64 em {corpo}"))
}

/// As chaves de topo de um objeto JSON plano, por ordem.
fn chaves_do_json(corpo: &str) -> Vec<String> {
    corpo
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split(',')
        .map(|par| par.split(':').next().unwrap().trim().trim_matches('"').to_string())
        .collect()
}

/// Os campos `readonly x:` de uma interface do contrato TS gerado.
fn campos_da_interface(ts: &str, nome: &str) -> Vec<String> {
    let inicio = ts
        .find(&format!("export interface {nome} {{"))
        .unwrap_or_else(|| panic!("interface {nome} ausente do contrato gerado"));
    let corpo = &ts[inicio..];
    let fim = corpo.find("\n}").expect("interface sem fecho");
    corpo[..fim]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("readonly "))
        .map(|l| l.split(':').next().unwrap().trim().to_string())
        .collect()
}

// ── 1. Produtor → rota ───────────────────────────────────────────────────────

/// **Uma perda induzida faz `dropped` crescer na rota, com o mesmo `since`.**
#[test]
fn perda_induzida_faz_dropped_crescer_na_rota() {
    let r = subir("cresce");
    let c = r.console();

    let (status, antes) = get(&c, "/api/dropped");
    assert_eq!(status, 200, "{antes}");
    let d0 = campo_u64(&antes, "dropped");
    let s0 = campo_u64(&antes, "since");
    assert!(s0 > 0, "`since` tem de ser o arranque real do console, nao 0: {antes}");

    let _parado = browser_parado(&c);
    esperar(|| c.fanout.ligados() == 1, "browser ligado");

    // O mesmo volume do `browser_lento_…` do sse.rs: enche o buffer do socket, para a thread
    // de SSE ficar genuinamente presa e a fila do browser saturar.
    let n = led_console_bin::fanout::FILA_POR_BROWSER * 100;
    let cp = Arc::clone(&r.cp);
    std::thread::spawn(move || {
        for i in 0..n {
            cp.broadcast(&format!(r#"{{"t_ms":{i},"event":"enchente"}}"#));
        }
    });
    esperar(|| c.fanout.descartados_desde_arranque() > d0, "a fila devia ter saturado");

    let (status, depois) = get(&c, "/api/dropped");
    assert_eq!(status, 200, "{depois}");
    let d1 = campo_u64(&depois, "dropped");
    assert!(d1 > d0, "a perda induzida nao chegou a rota: {antes} -> {depois}");
    assert_eq!(campo_u64(&depois, "since"), s0, "o mesmo console, o mesmo `since`");
    c.stop();
}

/// **Um browser que lê tudo reporta zero** — o controlo negativo do `falsification_required`
/// do TD-014: sem ele, uma rota que devolvesse sempre uma constante passaria no teste acima.
#[test]
fn browser_que_le_tudo_reporta_zero() {
    let r = subir("zero");
    let c = r.console();
    let mut leitor = browser_parado(&c); // abre como o outro, mas este LÊ
    esperar(|| c.fanout.ligados() == 1, "browser ligado");

    let n = 50; // bem abaixo da capacidade da fila: um browser que lê nunca a enche
    for i in 0..n {
        r.cp.broadcast(&format!(r#"{{"t_ms":{i},"event":"leitura"}}"#));
    }
    let mut lidos = 0;
    while lidos < n {
        let mut l = String::new();
        if leitor.read_line(&mut l).unwrap_or(0) == 0 {
            panic!("o SSE fechou a meio: {lidos} de {n}");
        }
        if l.starts_with("data: ") {
            lidos += 1;
        }
    }
    let (_, corpo) = get(&c, "/api/dropped");
    assert_eq!(campo_u64(&corpo, "dropped"), 0, "quem le tudo nao perde: {corpo}");
    c.stop();
}

/// **O corpo é `{dropped, since}` e mais nada** — sem o envelope do IPC v1.
#[test]
fn o_corpo_de_dropped_e_dropped_e_since_e_mais_nada() {
    let r = subir("corpo");
    let c = r.console();
    let (status, corpo) = get(&c, "/api/dropped");
    assert_eq!(status, 200);
    assert_eq!(chaves_do_json(&corpo), ["dropped", "since"], "corpo: {corpo}");
    c.stop();
}

// ── 2. Fronteira Rust → TS ───────────────────────────────────────────────────

/// **As chaves que a rota emite são exatamente os campos de `EstadoDescartes`.** Um campo
/// renomeado num lado e não no outro reprova aqui, e não no browser.
#[test]
fn a_rota_e_o_contrato_ts_tem_os_mesmos_campos() {
    let r = subir("contrato");
    let c = r.console();
    let (_, corpo) = get(&c, "/api/dropped");
    assert_eq!(
        chaves_do_json(&corpo),
        campos_da_interface(TS_VERSIONADO, "EstadoDescartes"),
        "o produtor Rust e o consumidor TS divergem"
    );
    c.stop();
}

// ── 3. `/api/upstream` intocado (ADR-0026 §9-quinquies) ─────────────────────

/// **Snapshot de `57cf21d`:** o corpo continua `{"upstream": boolean}` e o tipo gerado
/// continua a ter um só campo. O TD-014 abriu uma rota nova; não alargou esta.
#[test]
fn upstream_continua_um_booleano_e_mais_nada() {
    let r = subir("upstream");
    let c = r.console();
    let (status, corpo) = get(&c, "/api/upstream");
    assert_eq!(status, 200);
    assert!(
        corpo == r#"{"upstream":true}"# || corpo == r#"{"upstream":false}"#,
        "o corpo de /api/upstream mudou: {corpo}"
    );
    // O tipo gerado, tal como em 57cf21d (linhas 231-233 do .ts desse commit).
    assert!(
        TS_VERSIONADO
            .contains("export interface EstadoUpstream {\n  readonly upstream: boolean;\n}\n"),
        "o tipo EstadoUpstream do contrato gerado mudou"
    );
    c.stop();
}
