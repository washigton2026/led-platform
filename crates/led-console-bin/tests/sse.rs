//! **F5 — SSE: uma subscrição no daemon, N browsers.**
//!
//! Sem mocks. O daemon é o real, e os eventos entram no fluxo pelo `ControlPlane::broadcast`
//! — a **mesma** função que o laço de produção chama (`run.rs`). O evento atravessa
//! daemon → UDS → subscrição do console → `Fanout` → SSE → browser, e é isso que se mede.

#![cfg(unix)]

use led_console_bin::http::{serve, Config, ConsoleServer};
use led_daemon_bin::server::{ControlPlane, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Rig {
    _path: std::path::PathBuf,
    cp: Arc<ControlPlane>,
    shutdown: Arc<AtomicBool>,
}

/// Só o servidor UDS + o `ControlPlane`. O laço aplicador não é preciso aqui: o que se mede
/// é a **difusão de eventos**, e quem difunde é o `ControlPlane`.
fn subir(nome: &str) -> Rig {
    let path = std::env::temp_dir().join(format!("lumyx-f5-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let shutdown = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&shutdown));
    Server::bind(&path).expect("bind").spawn(Arc::clone(&cp));
    Rig { _path: path, cp, shutdown }
}

impl Rig {
    fn socket(&self) -> String {
        self._path.to_str().unwrap().to_string()
    }
    fn console(&self) -> ConsoleServer {
        let c = serve(
            "127.0.0.1:0".parse().unwrap(),
            Config { socket_daemon: self.socket(), exporter: None },
        )
        .expect("console");
        // Barreira causal: esperar que a **única** subscrição se estabeleça, em vez de
        // assumir um atraso (TD-003).
        esperar(|| c.fanout.subscricoes_ipc() == 1, "a subscricao upstream nao se estabeleceu");
        c
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self._path);
    }
}

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

/// Um browser SSE: abre a ligação e lê linhas à medida que chegam.
struct Browser {
    leitor: BufReader<TcpStream>,
}

impl Browser {
    fn abre(c: &ConsoleServer) -> Self {
        let mut s = TcpStream::connect(c.addr).expect("ligar");
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(s, "GET /api/events HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        s.flush().unwrap();
        let mut leitor = BufReader::new(s);
        // Cabeçalhos até à linha em branco.
        let mut cab = String::new();
        loop {
            let mut l = String::new();
            if leitor.read_line(&mut l).unwrap_or(0) == 0 {
                panic!("o servidor fechou antes dos cabecalhos");
            }
            cab.push_str(&l);
            if l == "\r\n" {
                break;
            }
        }
        assert!(
            cab.contains("text/event-stream"),
            "SSE tem de se anunciar como text/event-stream:\n{cab}"
        );
        Self { leitor }
    }

    /// Lê o próximo `data:` — ignora comentários (`:`) e linhas em branco.
    fn proximo_dado(&mut self) -> Option<String> {
        loop {
            let mut l = String::new();
            match self.leitor.read_line(&mut l) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {
                    let t = l.trim_end();
                    if let Some(d) = t.strip_prefix("data: ") {
                        return Some(d.to_string());
                    }
                }
            }
        }
    }
}

// ── 1–2. O fluxo abre e o evento real chega ──────────────────────────────────

/// **A ligação fica aberta, e um evento real do daemon chega ao browser.**
#[test]
fn evento_real_do_daemon_chega_ao_browser() {
    let r = subir("evento");
    let c = r.console();
    let mut b = Browser::abre(&c);

    // Barreira: o browser tem de estar registado antes de difundirmos.
    esperar(|| c.fanout.ligados() == 1, "o browser nao se registou");
    r.cp.broadcast(r#"{"t_ms":1,"event":"position_changed"}"#);

    let d = b.proximo_dado().expect("o evento tem de chegar ao browser");
    assert!(d.contains("position_changed"), "o evento chegou alterado: {d}");
    assert!(d.contains("\"async\":true"), "o evento do daemon viaja verbatim: {d}");
    c.stop();
}

// ── 3–4. UMA subscrição upstream para N browsers ─────────────────────────────

/// **Dois browsers NÃO criam duas subscrições no daemon.**
///
/// É a decisão do ADR-0026 §4: sem ela, a carga no daemon passaria a depender de quantos
/// separadores o operador tem abertos.
#[test]
fn n_browsers_uma_so_subscricao_upstream() {
    let r = subir("uma");
    let c = r.console();
    assert_eq!(c.fanout.subscricoes_ipc(), 1, "arranque: exatamente uma");

    let mut bs: Vec<Browser> = (0..4).map(|_| Browser::abre(&c)).collect();
    esperar(|| c.fanout.ligados() == 4, "quatro browsers");

    assert_eq!(
        c.fanout.subscricoes_ipc(),
        1,
        "4 browsers criaram {} subscricoes no daemon — tem de ser UMA",
        c.fanout.subscricoes_ipc()
    );

    // E o mesmo evento chega aos quatro.
    r.cp.broadcast(r#"{"t_ms":2,"event":"transitioned"}"#);
    for (i, b) in bs.iter_mut().enumerate() {
        let d = b.proximo_dado().unwrap_or_else(|| panic!("browser {i} nao recebeu"));
        assert!(d.contains("transitioned"), "browser {i}: {d}");
    }
    c.stop();
}

/// **Reconnect do browser não cria subscrição upstream adicional.**
#[test]
fn reconnect_do_browser_nao_duplica_subscricao_upstream() {
    let r = subir("reconnect");
    let c = r.console();

    for volta in 0..5 {
        let b = Browser::abre(&c);
        esperar(|| c.fanout.ligados() >= 1, "browser ligado");
        drop(b); // o browser desliga-se
        esperar(|| c.fanout.ligados() == 0, "o subscritor tem de ser podado ao desligar");
        assert_eq!(
            c.fanout.subscricoes_ipc(),
            1,
            "volta {volta}: reconectar o browser criou subscricao nova a montante"
        );
    }
    c.stop();
}

// ── 5. Isolamento entre browsers ─────────────────────────────────────────────

/// **Um browser que se desliga não derruba os outros.**
#[test]
fn browser_desligado_nao_derruba_os_outros() {
    let r = subir("isola");
    let c = r.console();
    let a = Browser::abre(&c);
    let mut b = Browser::abre(&c);
    esperar(|| c.fanout.ligados() == 2, "dois browsers");

    drop(a); // A desaparece
    esperar(|| c.fanout.ligados() == 1, "A tem de ser podado");

    r.cp.broadcast(r#"{"t_ms":3,"event":"sobrevivi"}"#);
    let d = b.proximo_dado().expect("B continua a receber depois de A cair");
    assert!(d.contains("sobrevivi"), "{d}");
    c.stop();
}

// ── 6. Backpressure não sobe ─────────────────────────────────────────────────

/// **Um browser lento não bloqueia o daemon, e a sua fila é limitada.**
///
/// O browser abre e **não lê**. A difusão continua a devolver imediatamente, a fila dele
/// satura no teto, e os eventos perdidos são **contados** — não escondidos.
#[test]
fn browser_lento_nao_bloqueia_e_a_fila_e_limitada() {
    let r = subir("lento");
    let c = r.console();
    let _preguicoso = Browser::abre(&c); // abre e nunca lê
    esperar(|| c.fanout.ligados() == 1, "browser ligado");

    // Grande o suficiente para **encher o buffer do socket** e a thread de SSE ficar
    // genuinamente presa a escrever. Com poucos eventos o SO absorve tudo, a fila esvazia-se
    // sozinha e o "browser lento" não estaria lento — o teste passaria sem exercitar nada.
    let n = led_console_bin::fanout::FILA_POR_BROWSER * 100;

    // A difusão corre **noutra thread** de propósito: se ela bloquear, este teste tem de
    // **falhar com uma mensagem**, não ficar pendurado. Um gate que trava em vez de reprovar
    // perde o diagnóstico — e foi o que aconteceu na 1.ª versão, ao medir o tempo só *depois*
    // de um laço que nunca terminava.
    let cp = Arc::clone(&r.cp);
    let acabou = Arc::new(AtomicBool::new(false));
    let sinal = Arc::clone(&acabou);
    std::thread::spawn(move || {
        for i in 0..n {
            cp.broadcast(&format!(r#"{{"t_ms":{i},"event":"enchente"}}"#));
        }
        sinal.store(true, Ordering::Relaxed);
    });

    let limite = Instant::now() + Duration::from_secs(5);
    while !acabou.load(Ordering::Relaxed) && Instant::now() < limite {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        acabou.load(Ordering::Relaxed),
        "difundir {n} eventos com um browser parado nao terminou em 5 s — o browser lento \
         esta a aplicar BACKPRESSURE a montante. A entrega tem de descartar, nunca esperar."
    );
    esperar(
        || c.fanout.descartados_totais() > 0,
        "a fila devia ter saturado e contado o que perdeu",
    );
    c.stop();
}

// ── 8–9. Sem duplicação, e por ordem ─────────────────────────────────────────

/// **Nenhum evento é duplicado, e a ordem é preservada.**
#[test]
fn eventos_nao_duplicam_e_mantem_a_ordem() {
    let r = subir("ordem");
    let c = r.console();
    let mut b = Browser::abre(&c);
    esperar(|| c.fanout.ligados() == 1, "browser ligado");

    const N: usize = 50;
    for i in 0..N {
        r.cp.broadcast(&format!(r#"{{"t_ms":{i},"event":"seq"}}"#));
    }

    let mut vistos = Vec::new();
    for _ in 0..N {
        match b.proximo_dado() {
            Some(d) => vistos.push(d),
            None => break,
        }
    }
    assert_eq!(vistos.len(), N, "esperava {N} eventos, vieram {}", vistos.len());

    // Ordem: os `t_ms` saem estritamente crescentes, na ordem em que entraram.
    let ts: Vec<usize> = vistos
        .iter()
        .map(|d| {
            let i = d.find("\"t_ms\":").expect("t_ms") + 7;
            d[i..].split(|ch: char| !ch.is_ascii_digit()).next().unwrap().parse().unwrap()
        })
        .collect();
    let esperado: Vec<usize> = (0..N).collect();
    assert_eq!(ts, esperado, "a ordem dos eventos nao foi preservada");

    // Duplicação: nenhum `t_ms` aparece duas vezes.
    let unicos: std::collections::BTreeSet<_> = ts.iter().collect();
    assert_eq!(unicos.len(), N, "houve eventos DUPLICADOS no fanout");
    c.stop();
}

// ── 10–11. Nada é reinterpretado ─────────────────────────────────────────────

/// **O console não reinterpreta o conteúdo do evento.**
///
/// Um evento que o daemon marque como recusa continua a ser uma recusa no browser; e não
/// existe caminho que o transforme em `PASS` ou em "healthy". O console difunde bytes.
#[test]
fn o_console_nao_reinterpreta_o_evento() {
    let r = subir("verbatim");
    let c = r.console();
    let mut b = Browser::abre(&c);
    esperar(|| c.fanout.ligados() == 1, "browser ligado");

    r.cp.broadcast(r#"{"t_ms":9,"event":"fault","code":"preflight_failed"}"#);
    let d = b.proximo_dado().expect("evento");

    assert!(d.contains("preflight_failed"), "o codigo do daemon tem de sobreviver: {d}");
    for inventado in ["PASS", "healthy", "hardware_ok", "\"ok\":true"] {
        assert!(
            !d.contains(inventado),
            "o console injetou `{inventado}` num evento de falha: {d}"
        );
    }
    c.stop();
}

/// **Daemon ausente: sem subscrição, e sem eventos inventados.**
///
/// `subscricoes_ipc() == 0` é a verdade — fingir 1 seria dizer que há um fluxo que não há.
#[test]
fn daemon_ausente_nao_finge_subscricao_nem_inventa_eventos() {
    let ausente = std::env::temp_dir()
        .join(format!("lumyx-f5-ausente-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&ausente);

    let c = serve(
        "127.0.0.1:0".parse().unwrap(),
        Config { socket_daemon: ausente.to_str().unwrap().to_string(), exporter: None },
    )
    .expect("o console sobe mesmo sem daemon");

    let b = Browser::abre(&c); // o fluxo abre — OFFLINE e estado, nao erro de transporte
    assert_eq!(c.fanout.subscricoes_ipc(), 0, "nao ha daemon: nao pode haver subscricao");

    // E nada aparece do nada: sem daemon, sem eventos.
    let mut s = b.leitor.get_ref().try_clone().unwrap();
    s.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let mut buf = [0u8; 256];
    let n = s.read(&mut buf).unwrap_or(0);
    let visto = String::from_utf8_lossy(&buf[..n]);
    assert!(
        !visto.contains("data: "),
        "o console FABRICOU um evento sem daemon nenhum: {visto:?}"
    );
    // O que se vê é o **comentário** de vida (`:`), nunca um `data:` — a ligação está aberta
    // e honesta: sem daemon, sem eventos.
    drop(b);
    c.stop();
}

// ── 12–13. O SSE não abre caminhos novos ─────────────────────────────────────

/// **O SSE não expõe UDS nem o `ShowRuntime`.**
///
/// Gate estrutural: o browser fala HTTP e mais nada. Se algum dia o handler de eventos
/// aceitasse um caminho de socket ou tocasse no runtime, é aqui que fica vermelho.
#[test]
fn o_sse_nao_expoe_uds_nem_o_runtime() {
    const FONTE: &str = include_str!("../src/http.rs");
    let codigo: Vec<&str> = FONTE
        .lines()
        .map(|l| l.trim())
        .filter(|t| !t.starts_with("//") && !t.starts_with('*'))
        .collect();

    for proibido in ["ShowRuntime", "OutputManager", "UnixListener"] {
        assert!(
            !codigo.iter().any(|l| l.contains(proibido)),
            "`{proibido}` no servidor HTTP — o browser passaria a ter um caminho que nao e HTTP"
        );
    }
    // O socket do daemon existe como **dado de configuração**, nunca como algo que o
    // browser possa escolher: nenhuma rota o aceita como parâmetro.
    for r in led_console_bin::surface::ROTAS {
        assert!(!r.caminho.contains("socket"), "{}: o browser nao escolhe socket", r.caminho);
    }
}
