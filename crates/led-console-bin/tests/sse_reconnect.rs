//! **F6 — reconexão upstream: N browsers, 0 ou 1 subscrição, nunca 2.**
//!
//! # Porque existe um proxy UDS aqui
//!
//! Para provar reconexão é preciso **matar** uma subscrição viva, e o
//! `led_daemon_bin::server::Server` não tem `stop()` — o laço de `accept` vive na thread e o
//! crate é para ficar como está. O proxy é um cano de bytes que o teste controla: o **daemon
//! real** continua no circuito, e o que se liga e desliga é o caminho até ele.
//!
//! Não é um mock do daemon. É um cabo que se pode desligar — o equivalente, em UDS, ao
//! `UdpChaosProxy` que o repo já usa para "puxar o cabo" em UDP.

#![cfg(unix)]

use led_console_bin::http::{serve, Config, ConsoleServer};
use led_daemon_bin::server::{ControlPlane, Server};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Proxy UDS que o teste liga e desliga ─────────────────────────────────────

struct Proxy {
    caminho: std::path::PathBuf,
    parar: Arc<AtomicBool>,
    vivas: Arc<Mutex<Vec<UnixStream>>>,
}

impl Proxy {
    /// Levanta o cano em `caminho`, a encaminhar para `alvo` (o daemon real).
    fn ligar(caminho: &std::path::Path, alvo: &std::path::Path) -> Self {
        let _ = std::fs::remove_file(caminho);
        let listener = UnixListener::bind(caminho).expect("bind do proxy");
        let parar = Arc::new(AtomicBool::new(false));
        let vivas: Arc<Mutex<Vec<UnixStream>>> = Arc::new(Mutex::new(Vec::new()));

        let (p, v, alvo) = (Arc::clone(&parar), Arc::clone(&vivas), alvo.to_path_buf());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if p.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(cliente) = conn else { continue };
                let Ok(servidor) = UnixStream::connect(&alvo) else { continue };
                // Guardamos as duas pontas para as poder **fechar** ao desligar o cano.
                for s in [&cliente, &servidor] {
                    if let Ok(c) = s.try_clone() {
                        v.lock().expect("vivas").push(c);
                    }
                }
                bombear(cliente, servidor);
            }
        });
        Proxy { caminho: caminho.to_path_buf(), parar, vivas }
    }

    /// Desliga o cano: fecha o listener e **derruba as ligações vivas**.
    ///
    /// É isto que faz a subscrição do console morrer — sem isto, remover o ficheiro do socket
    /// só impediria ligações **novas**, e a que existe continuaria a ler para sempre.
    fn desligar(&self) {
        self.parar.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.caminho);
        let _ = UnixStream::connect(&self.caminho); // desbloqueia o accept()
        for s in self.vivas.lock().expect("vivas").drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Copia bytes nos dois sentidos, cada um na sua thread.
fn bombear(cliente: UnixStream, servidor: UnixStream) {
    for (mut de, mut para) in [
        (cliente.try_clone().unwrap(), servidor.try_clone().unwrap()),
        (servidor, cliente),
    ] {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match de.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if para.write_all(&buf[..n]).and_then(|()| para.flush()).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = para.shutdown(std::net::Shutdown::Both);
        });
    }
}

// ── Daemon real, por trás do proxy ───────────────────────────────────────────

struct Rig {
    daemon: std::path::PathBuf,
    proxy_path: std::path::PathBuf,
    cp: Arc<ControlPlane>,
    shutdown: Arc<AtomicBool>,
}

fn subir(nome: &str) -> Rig {
    let daemon =
        std::env::temp_dir().join(format!("lumyx-f6-d-{nome}-{}.sock", std::process::id()));
    let proxy_path =
        std::env::temp_dir().join(format!("lumyx-f6-p-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&daemon);
    let _ = std::fs::remove_file(&proxy_path);
    let shutdown = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&shutdown));
    Server::bind(&daemon).expect("bind do daemon").spawn(Arc::clone(&cp));
    Rig { daemon, proxy_path, cp, shutdown }
}

impl Rig {
    fn proxy(&self) -> Proxy {
        Proxy::ligar(&self.proxy_path, &self.daemon)
    }
    fn console(&self) -> ConsoleServer {
        serve(
            "127.0.0.1:0".parse().unwrap(),
            Config {
                socket_daemon: self.proxy_path.to_str().unwrap().to_string(),
                exporter: None,
            },
        )
        .expect("console")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.daemon);
        let _ = std::fs::remove_file(&self.proxy_path);
    }
}

fn esperar(cond: impl Fn() -> bool, porque: &str) {
    let limite = Instant::now() + Duration::from_secs(20);
    while Instant::now() < limite {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timeout: {porque}");
}

struct Browser {
    leitor: BufReader<TcpStream>,
}

impl Browser {
    fn abre(c: &ConsoleServer) -> Self {
        let mut s = TcpStream::connect(c.addr).expect("ligar");
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        write!(s, "GET /api/events HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        s.flush().unwrap();
        let mut leitor = BufReader::new(s);
        loop {
            let mut l = String::new();
            if leitor.read_line(&mut l).unwrap_or(0) == 0 {
                panic!("fechou antes dos cabecalhos");
            }
            if l == "\r\n" {
                break;
            }
        }
        Self { leitor }
    }
    /// Lê o próximo `data:`, **com prazo**.
    ///
    /// O prazo não é zelo: o fluxo manda um comentário de vida a cada 200 ms, portanto
    /// `read_line` **nunca** expira — havendo ou não eventos. Sem prazo, um evento que não
    /// chega faz este ajudante girar para sempre a consumir comentários, e o teste **pendura**
    /// em vez de reprovar. Um gate que trava perde o diagnóstico.
    fn proximo_dado(&mut self) -> Option<String> {
        let prazo = Instant::now() + Duration::from_secs(5);
        while Instant::now() < prazo {
            let mut l = String::new();
            match self.leitor.read_line(&mut l) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {
                    if let Some(d) = l.trim_end().strip_prefix("data: ") {
                        return Some(d.to_string());
                    }
                }
            }
        }
        None // prazo esgotado: nenhum evento chegou
    }
}

// ── 1–2. Cai e volta ─────────────────────────────────────────────────────────

/// **O daemon cai, a subscrição termina; o daemon volta, a subscrição regressa.**
///
/// E o console **não é reiniciado** entre as duas coisas.
#[test]
fn a_subscricao_morre_com_o_daemon_e_volta_sozinha() {
    let r = subir("ciclo");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "a subscricao inicial nao subiu");

    // DOWN
    p.desligar();
    esperar(|| !c.fanout.subscricao_viva(), "a subscricao devia ter MORRIDO com o daemon");

    // UP — o mesmo console, sem reinicio.
    let _p2 = r.proxy();
    esperar(
        || c.fanout.subscricao_viva(),
        "a subscricao NAO regressou sozinha: o console fica mudo ate ser reiniciado",
    );
    c.stop();
}

// ── 3–6. Nunca duas, aconteça o que acontecer ────────────────────────────────

/// **Duas subscrições ao mesmo tempo não são representáveis.**
///
/// Prova estrutural: com uma viva, reivindicar outra devolve `None`. Não é uma contagem que
/// se observa a passar — é a impossibilidade de a abrir.
#[test]
fn uma_segunda_subscricao_nao_e_reivindicavel() {
    let f = led_console_bin::fanout::Fanout::novo();
    let g = f.reivindicar_subscricao().expect("a primeira tem de ser possivel");

    // **Reivindicar não é estar viva.** Entre uma coisa e a outra há a ligação e o
    // `subscribe`, e durante essa janela não existe fluxo. Dizer o contrário fez um teste
    // ficar intermitente: ele difundia um evento na janela e o evento não chegava a ninguem.
    assert!(
        !f.subscricao_viva(),
        "reivindicada mas ainda nao estabelecida: `subscricao_viva()` tem de ser false"
    );
    assert_eq!(f.subscricoes_ipc(), 0, "uma reivindicacao nao conta como subscricao");

    assert!(
        f.reivindicar_subscricao().is_none(),
        "uma SEGUNDA subscricao upstream foi reivindicada com uma ja em curso"
    );

    g.estabelecida();
    assert!(f.subscricao_viva(), "depois de o daemon aceitar, ha fluxo");
    assert_eq!(f.subscricoes_ipc(), 1, "e AGORA conta como subscricao");

    drop(g);
    assert!(!f.subscricao_viva(), "o Drop da guarda tem de encerrar o fluxo");
    assert!(f.reivindicar_subscricao().is_some(), "depois de libertar, volta a ser possivel");
}

/// **4 browsers durante uma queda não produzem 4 subscrições.**
#[test]
fn quatro_browsers_durante_a_queda_nao_multiplicam_a_subscricao() {
    let r = subir("quatro");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "subscricao inicial");

    // **Medir ANTES de abrir os browsers.** A 1.ª versao media depois, e por isso as
    // subscricoes que os browsers criassem ficavam *dentro* da linha de base — o teste
    // passava com o defeito plantado. Falso-verde apanhado a falsificar (KB-012).
    let antes = c.fanout.subscricoes_ipc();

    let _bs: Vec<Browser> = (0..4).map(|_| Browser::abre(&c)).collect();
    esperar(|| c.fanout.ligados() == 4, "quatro browsers");
    assert_eq!(
        c.fanout.subscricoes_ipc(),
        antes,
        "abrir 4 browsers criou subscricoes upstream: {} -> {}",
        antes,
        c.fanout.subscricoes_ipc()
    );

    p.desligar();
    esperar(|| !c.fanout.subscricao_viva(), "a subscricao cai");

    // Com o daemon em baixo e 4 browsers ligados, nenhum deles abre nada a montante.
    assert_eq!(
        c.fanout.subscricoes_ipc(),
        antes,
        "os browsers criaram subscricoes upstream durante a queda"
    );
    assert!(!c.fanout.subscricao_viva(), "sem daemon nao pode haver subscricao viva");
    c.stop();
}

/// **Flapping UP/DOWN/UP/DOWN/UP: nunca há mais de uma ao mesmo tempo.**
///
/// O acumulador cresce — cada regresso é uma subscrição nova de facto — mas o **medidor**
/// nunca passa de um, e uma amostragem contínua confirma-o durante todo o ciclo.
#[test]
fn flapping_nunca_passa_de_uma_subscricao_simultanea() {
    let r = subir("flap");
    let c = r.console();

    // Um vigia a amostrar o medidor durante todo o teste.
    let viu_duas = Arc::new(AtomicBool::new(false));
    let parar_vigia = Arc::new(AtomicBool::new(false));
    {
        let f = Arc::clone(&c.fanout);
        let vd = Arc::clone(&viu_duas);
        let pv = Arc::clone(&parar_vigia);
        std::thread::spawn(move || {
            while !pv.load(Ordering::Relaxed) {
                if f.subscricoes_simultaneas() > 1 {
                    vd.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
    }

    for volta in 0..3 {
        let p = r.proxy();
        esperar(|| c.fanout.subscricao_viva(), &format!("volta {volta}: subir"));
        p.desligar();
        esperar(|| !c.fanout.subscricao_viva(), &format!("volta {volta}: cair"));
    }
    let _final = r.proxy();
    esperar(|| c.fanout.subscricao_viva(), "regresso final");

    parar_vigia.store(true, Ordering::Relaxed);
    assert!(!viu_duas.load(Ordering::Relaxed), "houve DUAS subscricoes upstream simultaneas");
    c.stop();
}

/// **Browser a reconectar com o daemon em baixo não cria subscrição.**
#[test]
fn browser_reconecta_com_daemon_em_baixo_e_nada_sobe() {
    let r = subir("brdown");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "subscricao inicial");
    p.desligar();
    esperar(|| !c.fanout.subscricao_viva(), "a subscricao cai");

    let antes = c.fanout.subscricoes_ipc();
    for _ in 0..5 {
        let b = Browser::abre(&c);
        drop(b);
    }
    assert_eq!(
        c.fanout.subscricoes_ipc(),
        antes,
        "reconectar browsers com o daemon em baixo criou subscricoes upstream"
    );
    c.stop();
}

// ── 7. O evento real chega depois da reconexão ───────────────────────────────

/// **Depois de cair e voltar, um evento real do daemon chega ao browser.**
///
/// É o teste que fecha a fatia: sem ele, "a subscrição voltou" seria um contador a mexer sem
/// consequência observável.
#[test]
fn depois_de_reconectar_o_evento_real_chega_ao_browser() {
    let r = subir("evento");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "subscricao inicial");

    p.desligar();
    esperar(|| !c.fanout.subscricao_viva(), "cai");
    let _p2 = r.proxy();
    esperar(|| c.fanout.subscricao_viva(), "volta");

    // O browser liga-se **depois** da reconexão e tem de receber o que o daemon difundir.
    let mut b = Browser::abre(&c);
    esperar(|| c.fanout.ligados() == 1, "browser ligado");

    r.cp.broadcast(r#"{"t_ms":77,"event":"depois_do_reconnect"}"#);
    let d = b.proximo_dado().expect("o evento real tem de chegar apos a reconexao");
    assert!(d.contains("depois_do_reconnect"), "chegou alterado: {d}");
    c.stop();
}

// ── 8–9. Backoff: nem busy-loop, nem threads a crescer ───────────────────────

/// **Com o daemon offline por muito tempo, as tentativas são limitadas pelo backoff.**
///
/// Um busy-loop faria milhares de tentativas por segundo. O backoff faz poucas dezenas — e a
/// diferença é de ordens de grandeza, que é o que torna este teste discriminante em vez de
/// sensível ao acaso.
#[test]
fn daemon_offline_prolongado_nao_vira_busy_loop() {
    let r = subir("backoff");
    let c = r.console(); // nunca há proxy: o daemon está inalcançável desde o início

    std::thread::sleep(Duration::from_millis(1500));
    let t = c.fanout.tentativas_de_ligacao();

    assert!(t >= 2, "esperava o supervisor a tentar mais que uma vez; fez {t}");
    assert!(
        t < 200,
        "{t} tentativas em 1,5 s e busy-loop — o backoff nao esta a limitar nada"
    );
    assert!(!c.fanout.subscricao_viva(), "sem daemon nao pode haver subscricao viva");
    c.stop();
}

/// **Uma thread de supervisão, não uma por browser nem uma por tentativa.**
#[test]
fn o_supervisor_e_uma_thread_e_nao_uma_por_browser() {
    let r = subir("threads");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "subscricao");

    // A linha de base vem **antes** dos browsers: o medidor sozinho nao apanha um browser
    // que subscreva por si (ele nao reivindica, so subscreve), mas o acumulador apanha.
    let antes = c.fanout.subscricoes_ipc();
    let _bs: Vec<Browser> = (0..6).map(|_| Browser::abre(&c)).collect();
    esperar(|| c.fanout.ligados() == 6, "seis browsers");
    assert_eq!(
        c.fanout.subscricoes_ipc(),
        antes,
        "6 browsers criaram {} subscricoes upstream — nenhum browser pode subscrever",
        c.fanout.subscricoes_ipc() - antes
    );

    // Seis browsers, e continua a haver exatamente **uma** subscrição.
    assert!(c.fanout.subscricao_viva());
    assert_eq!(
        c.fanout.subscricoes_simultaneas(),
        1,
        "6 browsers e {} subscricoes simultaneas",
        c.fanout.subscricoes_simultaneas()
    );
    drop(p);
    c.stop();
}

/// **Depois de várias falhas, o regresso do daemon dá exatamente uma subscrição saudável.**
#[test]
fn depois_de_varias_falhas_fica_exatamente_uma() {
    let r = subir("apos");
    let c = r.console(); // sem proxy: falha repetidamente
    esperar(|| c.fanout.tentativas_de_ligacao() >= 3, "o supervisor tem de insistir");
    assert!(!c.fanout.subscricao_viva());

    let _p = r.proxy();
    esperar(|| c.fanout.subscricao_viva(), "o daemon voltou e a subscricao tem de subir");
    assert_eq!(c.fanout.subscricoes_simultaneas(), 1, "exatamente uma, saudavel");
    c.stop();
}

// ── F-01: o estado da subscrição passa a ser OBSERVÁVEL ──────────────────────
//
// ADR-0026 §9-quinquies. O medidor `subscricao_viva()` existia, estava testado, e
// **nenhuma rota o expunha** — o frontend só tinha o `onopen` do `EventSource`, que mede
// *browser→console*. Esta secção prova que `/api/upstream` mede o elo certo, e que ele
// diverge do SSE do browser exactamente quando tem de divergir.

/// `GET <caminho>` no console. Devolve `(status, corpo)`.
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

/// **CASO A — daemon vivo e subscrição viva: a rota diz `true`.**
#[test]
fn upstream_diz_true_quando_a_subscricao_esta_de_pe() {
    let r = subir("up-a");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "a subscricao tem de subir");

    let (status, corpo) = get(&c, "/api/upstream");
    assert_eq!(status, 200);
    assert_eq!(corpo, r#"{"upstream":true}"#, "corpo minimo, sem envelope do IPC");

    // O corpo NAO leva `v`/`ok`/`id`: nao atravessa o IPC v1, e afirmar essa versao
    // seria declarar uma proveniencia que este corpo nao tem (ADR-0026 §9-quinquies).
    for proibido in ["\"v\"", "\"ok\"", "\"id\""] {
        assert!(!corpo.contains(proibido), "o corpo do console nao pode falar {proibido}: {corpo}");
    }
    drop(p);
    c.stop();
}

/// **CASO B — O DISCRIMINANTE. Browser ligado, daemon morto.**
///
/// É a única combinação em que a fonte errada parece saudável: a ligação SSE do browser
/// continua **genuinamente aberta** — o console mantém-na viva com comentários de
/// keep-alive — enquanto a subscrição a montante já morreu. Foi exactamente isto que
/// apareceu no ecrã a dizer `● Streaming` sobre silêncio.
///
/// Um teste que matasse o *browser* passaria com o defeito presente. Tem de ser o daemon
/// a cair **com o browser vivo**.
#[test]
fn com_o_daemon_morto_o_sse_fica_aberto_e_o_upstream_diz_false() {
    let r = subir("up-b");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "subscricao inicial");

    let mut b = Browser::abre(&c);
    esperar(|| c.fanout.ligados() == 1, "browser ligado");
    assert_eq!(get(&c, "/api/upstream").1, r#"{"upstream":true}"#);

    // Puxa o cabo até ao daemon. O browser NÃO é tocado.
    p.desligar();
    esperar(|| !c.fanout.subscricao_viva(), "a subscricao tem de morrer com o daemon");

    let (status, corpo) = get(&c, "/api/upstream");
    assert_eq!(status, 200, "a rota responde: a medicao e local, nao depende do daemon");
    assert_eq!(corpo, r#"{"upstream":false}"#, "sem daemon nao ha subscricao");

    // **E a outra metade do achado**: a ligação do browser continua viva. É esta
    // asserção que torna o caso discriminante — sem ela, o teste não distinguiria
    // "o upstream caiu" de "caiu tudo", e o defeito original não seria reproduzido.
    assert!(
        b.proximo_dado().is_none(),
        "nenhum evento pode chegar com o daemon morto — mas a ligacao NAO fechou"
    );
    assert_eq!(c.fanout.ligados(), 1, "o browser continua ligado ao console");

    c.stop();
}

/// **CASO D — o daemon reinicia: `true → false → true`, sem o browser recarregar.**
#[test]
fn upstream_volta_a_true_quando_o_daemon_regressa() {
    let r = subir("up-d");
    let p = r.proxy();
    let c = r.console();
    esperar(|| c.fanout.subscricao_viva(), "subscricao inicial");
    let _b = Browser::abre(&c);
    esperar(|| c.fanout.ligados() == 1, "browser ligado");
    assert_eq!(get(&c, "/api/upstream").1, r#"{"upstream":true}"#);

    p.desligar();
    esperar(|| !c.fanout.subscricao_viva(), "cai");
    assert_eq!(get(&c, "/api/upstream").1, r#"{"upstream":false}"#);

    let _p2 = r.proxy();
    esperar(|| c.fanout.subscricao_viva(), "volta");
    assert_eq!(
        get(&c, "/api/upstream").1,
        r#"{"upstream":true}"#,
        "o mesmo browser ve a recuperacao sem recarregar"
    );
    c.stop();
}

/// **A rota reporta o AGORA, nunca o acumulado.**
///
/// `subscricoes_ipc()` é cumulativo — depois de uma reconexão vale 2 e **nunca desce**. Se
/// a rota o devolvesse, responderia *"já houve"* a uma pergunta que é *"há agora"*, e um
/// daemon morto **depois** de um ciclo pareceria vivo. É o mesmo erro que o `stale_ms()` a
/// devolver `0` cometia, e a razão de o `fanout.rs` avisar que confundir os dois é *"fácil
/// e caro"*.
///
/// O proxy nasce e morre **dentro** da volta, como no teste dos três ciclos. Guardá-lo
/// numa variável e chamar `drop` não o desliga: o `Proxy` não tem `Drop` — quem corta o
/// cano é o `desligar()`, e a primeira versão deste teste esperou 20 s por uma queda que
/// nunca ia acontecer porque o proxy continuava a servir.
#[test]
fn upstream_reporta_o_agora_e_nao_o_acumulado() {
    let r = subir("up-acum");
    let c = r.console();

    for volta in 0..2 {
        let p = r.proxy();
        esperar(|| c.fanout.subscricao_viva(), &format!("volta {volta}: subir"));
        assert_eq!(get(&c, "/api/upstream").1, r#"{"upstream":true}"#);
        p.desligar();
        esperar(|| !c.fanout.subscricao_viva(), &format!("volta {volta}: cair"));
    }

    assert!(c.fanout.subscricoes_ipc() >= 2, "houve duas subscricoes ao longo do tempo");
    assert_eq!(
        get(&c, "/api/upstream").1,
        r#"{"upstream":false}"#,
        "duas subscricoes NO PASSADO nao sao uma subscricao AGORA"
    );
    c.stop();
}
