//! **F4 — o servidor HTTP real, contra o daemon real e o exporter real.**
//!
//! Sem mocks, em nenhuma camada. O daemon é o `led_daemon_bin::server::Server` com o
//! **laço aplicador** a correr (sem ele, `play` expira em `engine_busy` e um teste de
//! códigos estaria a medir o timeout). O exporter é o `led_hal::serve_metrics`. O cliente
//! HTTP é um `TcpStream` cru — se o enquadramento HTTP estiver errado, falha aqui.

#![cfg(unix)]

use led_console_bin::http::{serve, Config};
use led_daemon::ShowRuntime;
use led_daemon_bin::run::run_with_control;
use led_daemon_bin::server::{ControlPlane, Server};
use led_daemon_bin::{Config as DaemonConfig, Integrity, Journal, SystemPacer};
use led_hal::{prometheus_text, serve_metrics, MetricsEmitter};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── Fixture: daemon inteiro (servidor UDS + laço) ────────────────────────────

struct Daemon {
    path: std::path::PathBuf,
    shutdown: Arc<AtomicBool>,
    laco: Option<std::thread::JoinHandle<()>>,
}

fn subir_daemon(nome: &str) -> Daemon {
    let path =
        std::env::temp_dir().join(format!("lumyx-f4-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let shutdown = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&shutdown));
    Server::bind(&path).expect("bind").spawn(Arc::clone(&cp));

    let flag = Arc::clone(&shutdown);
    let laco = std::thread::spawn(move || {
        let cfg = DaemonConfig {
            tick_ms: 25,
            max_ticks: None,
            autoplay: false,
            exit_on_finish: false,
            integrity: Integrity::AssumedByOperator,
            output: None,
            profile: None,
        };
        let mut rt = ShowRuntime::new();
        let mut pacer = SystemPacer::default();
        let mut journal = Journal::new(std::io::sink());
        run_with_control(&mut rt, None, &cfg, &mut pacer, &mut journal, &flag, &cp);
    });
    Daemon { path, shutdown, laco: Some(laco) }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.laco.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

// ── Cliente HTTP cru ─────────────────────────────────────────────────────────

struct Resposta {
    status: u16,
    cabecalhos: String,
    corpo: String,
}

impl Resposta {
    fn content_type(&self) -> String {
        self.cabecalhos
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
            .map(|l| l["content-type:".len()..].trim().to_string())
            .unwrap_or_default()
    }
}

fn pedir(addr: SocketAddr, metodo: &str, caminho: &str, corpo: &str) -> Resposta {
    let mut s = TcpStream::connect(addr).expect("ligar ao console");
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    let req = format!(
        "{metodo} {caminho} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{corpo}",
        corpo.len()
    );
    s.write_all(req.as_bytes()).unwrap();
    s.flush().unwrap();

    let mut bruto = Vec::new();
    let _ = s.read_to_end(&mut bruto);
    let texto = String::from_utf8_lossy(&bruto).to_string();
    let corte = texto.find("\r\n\r\n").unwrap_or_else(|| {
        panic!("resposta sem fim de cabecalhos (o servidor respondeu?): {texto:?}")
    });
    let (cabecalhos, resto) = texto.split_at(corte);
    let status: u16 = cabecalhos
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("estado ilegivel: {cabecalhos:?}"));
    Resposta { status, cabecalhos: cabecalhos.to_string(), corpo: resto[4..].to_string() }
}

fn subir_console(d: &Daemon, exporter: Option<SocketAddr>) -> led_console_bin::http::ConsoleServer {
    serve(
        "127.0.0.1:0".parse().unwrap(),
        Config { socket_daemon: d.path.to_str().unwrap().to_string(), exporter },
    )
    .expect("o console tem de subir em loopback")
}

// ── 1–3. Leituras chegam ao backend real ─────────────────────────────────────

/// **`GET /api/state` fala com o daemon a sério.**
#[test]
fn get_api_state_chega_ao_daemon_real() {
    let d = subir_daemon("state");
    let c = subir_console(&d, None);

    // **Sem barreira, de propósito.** A F4 precisou de esperar aqui pela primeira publicação
    // do laço, porque `Snapshot::default()` tinha `state: ""` e havia uma janela em que o
    // `status` respondia com um valor fora dos oito do ADR-0023. A F5 corrigiu-o na origem —
    // `Snapshot.state` passou a ser `State`, e a string vazia **não é representável**.
    //
    // Perguntar de imediato é agora a asserção mais forte disponível: se a janela voltasse,
    // este pedido apanhá-la-ia.
    let r = pedir(c.addr, "GET", "/api/state", "");
    assert!(
        !r.corpo.contains(r#""state":"""#),
        "voltou a haver uma janela com `state` vazio: {}",
        r.corpo
    );
    assert_eq!(r.status, 200, "corpo: {}", r.corpo);
    assert!(r.corpo.contains("\"state\""), "sem campo `state`: {}", r.corpo);
    // `idle` e o estado real de um daemon sem show — nao um valor inventado pelo console.
    assert!(r.corpo.contains("idle"), "o estado real do daemon tem de atravessar: {}", r.corpo);
    assert!(r.corpo.contains("\"ticks\""), "o snapshot real traz `ticks`: {}", r.corpo);
    c.stop();
}

/// **`GET /api/version` chega ao daemon.**
#[test]
fn get_api_version_chega_ao_daemon_real() {
    let d = subir_daemon("version");
    let c = subir_console(&d, None);
    let r = pedir(c.addr, "GET", "/api/version", "");
    assert_eq!(r.status, 200, "{}", r.corpo);
    assert!(r.corpo.contains("\"protocol\""), "{}", r.corpo);
    c.stop();
}

/// **`GET /api/events` é a superfície SSE, não uma resposta JSON comum.**
///
/// Lê **só os cabeçalhos**, e isso é o ponto: desde a F5 o fluxo fica **aberto**, por isso um
/// `read_to_end` aqui nunca voltaria. A primeira versão deste teste usava o cliente comum e
/// passou a pendurar-se assim que o SSE começou a funcionar — o teste estava a assumir que a
/// resposta terminava.
#[test]
fn get_api_events_e_sse_e_nao_json() {
    let d = subir_daemon("events");
    let c = subir_console(&d, None);

    let mut s = TcpStream::connect(c.addr).expect("ligar");
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    write!(s, "GET /api/events HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    s.flush().unwrap();

    let mut cab = Vec::new();
    let mut b = [0u8; 1];
    while !cab.windows(4).any(|w| w == b"\r\n\r\n") && cab.len() < 2048 {
        match s.read(&mut b) {
            Ok(0) | Err(_) => break,
            Ok(_) => cab.push(b[0]),
        }
    }
    let cab = String::from_utf8_lossy(&cab).to_string();

    assert!(cab.starts_with("HTTP/1.1 200"), "{cab}");
    assert!(
        cab.to_ascii_lowercase().contains("content-type: text/event-stream"),
        "SSE tem de se anunciar como text/event-stream:\n{cab}"
    );
    assert!(!cab.contains("application/json"), "eventos nao sao uma resposta JSON comum");
    drop(s);
    c.stop();
}

// ── 5–10. Comandos produzem o comando IPC correspondente ─────────────────────

/// **Cada POST de transporte produz o comando IPC correspondente — e o daemon responde.**
///
/// Sem show carregado, o daemon recusa com o **seu** código. É isso que prova que o comando
/// chegou lá: um console que fabricasse a resposta não saberia dizer `no_show_loaded`.
#[test]
fn cada_post_de_transporte_produz_o_comando_ipc_correspondente() {
    let d = subir_daemon("cmds");
    let c = subir_console(&d, None);

    for (caminho, corpo) in [
        ("/api/transport/unload", ""),
        ("/api/transport/play", ""),
        ("/api/transport/pause", ""),
        ("/api/transport/stop", ""),
        ("/api/transport/seek", r#"{"to_ms":1000}"#),
    ] {
        let r = pedir(c.addr, "POST", caminho, corpo);
        assert!(
            r.corpo.contains("no_show_loaded"),
            "{caminho}: sem show, o daemon recusa com `no_show_loaded`. \
             Veio {} / {}. Se o comando nao chegou ao daemon, isto nao aparece.",
            r.status,
            r.corpo
        );
        assert_eq!(r.status, 409, "{caminho}: recusa de dominio e 409");
    }

    // `load` de um ficheiro inexistente: o daemon responde com o SEU erro de carregamento.
    let r = pedir(c.addr, "POST", "/api/transport/load", r#"{"path":"/nao/existe.lumyx"}"#);
    assert!(
        r.corpo.contains("load_failed") || r.corpo.contains("invalid_args"),
        "load tem de chegar ao daemon e devolver o erro DELE: {} / {}",
        r.status,
        r.corpo
    );
    c.stop();
}

/// **`GET /api/profiles` responde 501 sobre HTTP real** — nunca `200 []`, nunca 404.
///
/// O gate textual de `profiles_501.rs` guarda o código; este guarda o que sai **no fio**. Sem
/// ele, plantar um `200 []` passava por aqui sem ninguém reparar — foi exatamente o que
/// aconteceu ao falsificar, e é a razão deste teste existir.
#[test]
fn get_api_profiles_e_501_no_fio() {
    let d = subir_daemon("profiles");
    let c = subir_console(&d, None);

    let r = pedir(c.addr, "GET", "/api/profiles", "");
    assert_eq!(
        r.status, 501,
        "a rota EXISTE (404 seria mentira) e o catalogo NAO esta vazio (200 [] seria pior): \
         veio {} com {}",
        r.status, r.corpo
    );
    assert!(
        r.corpo.contains("console.not_implemented"),
        "a recusa tem de se nomear: {}",
        r.corpo
    );
    // E nada que pareça um catálogo.
    for parece_catalogo in ["[]", "\"profiles\"", "esp32"] {
        assert!(
            !r.corpo.contains(parece_catalogo),
            "o corpo contem `{parece_catalogo}` — nao pode parecer um catalogo: {}",
            r.corpo
        );
    }
    c.stop();
}

// ── 11–13. Métodos e rotas ───────────────────────────────────────────────────

/// **GET num endpoint de comando → 405.** Um comando não se executa por leitura.
#[test]
fn get_num_endpoint_de_comando_e_405() {
    let d = subir_daemon("m405a");
    let c = subir_console(&d, None);
    for caminho in ["/api/transport/play", "/api/transport/stop", "/api/transport/load"] {
        let r = pedir(c.addr, "GET", caminho, "");
        assert_eq!(
            r.status, 405,
            "{caminho}: GET num comando tem de ser 405 — senao um <img src> dispara o show"
        );
    }
    c.stop();
}

/// **POST num endpoint só de leitura → 405.**
#[test]
fn post_num_endpoint_de_leitura_e_405() {
    let d = subir_daemon("m405b");
    let c = subir_console(&d, None);
    for caminho in ["/api/state", "/api/version", "/api/metrics", "/api/events"] {
        let r = pedir(c.addr, "POST", caminho, "");
        assert_eq!(r.status, 405, "{caminho}: POST numa leitura tem de ser 405");
    }
    c.stop();
}

/// **Rota inexistente → 404.**
#[test]
fn rota_inexistente_e_404() {
    let d = subir_daemon("m404");
    let c = subir_console(&d, None);
    for caminho in ["/", "/api", "/api/inventado", "/admin", "/api/transport"] {
        let r = pedir(c.addr, "GET", caminho, "");
        assert_eq!(r.status, 404, "{caminho} nao existe e tem de ser 404");
    }
    c.stop();
}

// ── 17–18. O exporter não é alcançável pelo browser ──────────────────────────

/// **`/metrics` NÃO existe como rota pública do console.**
///
/// O caminho do exporter no console seria uma **segunda origem** (ADR-0026 §9-bis).
#[test]
fn metrics_direto_nao_existe_no_console() {
    let d = subir_daemon("nometrics");
    let e = Arc::new(MetricsEmitter::new("f4"));
    e.record_frame(1000);
    let exp = serve_metrics(vec![Arc::clone(&e)], "127.0.0.1:0".parse().unwrap()).expect("exp");
    let c = subir_console(&d, Some(exp.addr));

    let r = pedir(c.addr, "GET", "/metrics", "");
    assert_eq!(
        r.status, 404,
        "`/metrics` no console seria a 2.a origem que o ADR-0026 §9-bis proibe. \
         Veio {} com corpo {:?}",
        r.status, r.corpo
    );
    assert!(
        !r.corpo.contains("lumyx_frames_total"),
        "o console NAO pode servir o exporter neste caminho: {}",
        r.corpo
    );
    c.stop();
    exp.stop();
}

// ── 19. O proxy preserva corpo, Content-Type e o erro de exporter offline ────

/// **`/api/metrics` atravessa o console e chega ao exporter — byte a byte.**
#[test]
fn api_metrics_preserva_corpo_e_content_type() {
    let d = subir_daemon("metrics");
    let e = Arc::new(MetricsEmitter::new("f4"));
    e.record_frame(1_000);
    e.record_frame(2_000);
    e.record_drop();
    let exp = serve_metrics(vec![Arc::clone(&e)], "127.0.0.1:0".parse().unwrap()).expect("exp");
    let c = subir_console(&d, Some(exp.addr));

    let esperado = prometheus_text(std::slice::from_ref(&e));
    let r = pedir(c.addr, "GET", "/api/metrics", "");

    assert_eq!(r.status, 200, "{}", r.corpo);
    assert_eq!(
        r.corpo, esperado,
        "o corpo foi alterado entre o exporter e o browser — o proxy repassa verbatim"
    );
    assert!(
        r.content_type().contains("version=0.0.4"),
        "o formato de exposicao do Prometheus tem de sobreviver: {:?}",
        r.content_type()
    );
    assert!(r.corpo.contains("lumyx_frames_total"), "sanidade: corpo vazio passaria sem provar");
    c.stop();
    exp.stop();
}

/// **Exporter em baixo não vira 200 com métricas vazias.**
#[test]
fn exporter_em_baixo_nao_vira_200_vazio() {
    let d = subir_daemon("expoff");
    let morto: SocketAddr = {
        let s = serve_metrics(vec![], "127.0.0.1:0".parse().unwrap()).expect("exp");
        let a = s.addr;
        s.stop();
        a
    };
    let c = subir_console(&d, Some(morto));

    let r = pedir(c.addr, "GET", "/api/metrics", "");
    assert_ne!(r.status, 200, "exporter em baixo NAO pode ser 200: corpo {:?}", r.corpo);
    assert!(
        r.status == 503 || r.status == 502,
        "a falha e do componente a montante (503/502), nunca 500: veio {}",
        r.status
    );
    c.stop();
}

// ── 14–15. Erros e OFFLINE atravessam sem serem mascarados ───────────────────

/// **O código semântico do daemon atravessa o HTTP intacto.**
#[test]
fn o_codigo_de_erro_do_daemon_atravessa_com_a_semantica_original() {
    let d = subir_daemon("erros");
    let c = subir_console(&d, None);

    let r = pedir(c.addr, "POST", "/api/transport/play", "");
    assert_eq!(r.status, 409, "{}", r.corpo);
    assert!(
        r.corpo.contains("no_show_loaded"),
        "o codigo do daemon tem de chegar VERBATIM ao browser (ADR-0026 §6): {}",
        r.corpo
    );
    assert!(
        !r.corpo.contains("console."),
        "o console nao pode reescrever um erro do daemon com um codigo seu: {}",
        r.corpo
    );
    c.stop();
}

/// **Daemon OFFLINE não vira 200, nem zero, nem snapshot inventado.**
///
/// É o modo de falha mais caro desta camada: um ecrã verde sobre um daemon que morreu.
#[test]
fn daemon_offline_nao_vira_200_nem_estado_fabricado() {
    // Um socket que não existe: o daemon nunca esteve lá.
    let inexistente = std::env::temp_dir()
        .join(format!("lumyx-f4-ausente-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&inexistente);

    let c = serve(
        "127.0.0.1:0".parse().unwrap(),
        Config {
            socket_daemon: inexistente.to_str().unwrap().to_string(),
            exporter: None,
        },
    )
    .expect("o console sobe mesmo sem daemon — OFFLINE e estado, nao erro de arranque");

    let r = pedir(c.addr, "GET", "/api/state", "");
    assert_ne!(r.status, 200, "daemon ausente NAO pode ser 200: {}", r.corpo);
    assert_eq!(r.status, 503, "OFFLINE e 503 (ADR-0026 §7): veio {}", r.status);
    assert!(
        r.corpo.contains("console.daemon_offline"),
        "o estado OFFLINE tem de ser nomeado: {}",
        r.corpo
    );
    // E nada de estado fabricado.
    for inventado in ["\"state\":\"idle\"", "\"position_ms\":0", "\"ok\":true"] {
        assert!(
            !r.corpo.contains(inventado),
            "o console FABRICOU `{inventado}` para um daemon que nao respondeu: {}",
            r.corpo
        );
    }
    c.stop();
}

// ── 20. Loopback-only ────────────────────────────────────────────────────────

/// **O servidor recusa bind não-loopback** enquanto o `ClientRegistry` estiver vazio.
#[test]
fn o_servidor_e_loopback_only() {
    let d = subir_daemon("bind");
    for addr in ["0.0.0.0:0", "0.0.0.0:8080"] {
        let r = serve(
            addr.parse().unwrap(),
            Config { socket_daemon: d.path.to_str().unwrap().to_string(), exporter: None },
        );
        assert!(
            r.is_err(),
            "{addr} tem de ser RECUSADO: sem auth (ADR-0014) nao existe console em LAN"
        );
    }
    // E o loopback é aceite.
    let ok = subir_console(&d, None);
    assert!(ok.addr.ip().is_loopback());
    ok.stop();
}
