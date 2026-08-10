//! ADR-0026 — o **servidor HTTP do console**.
//!
//! # O único caminho do browser
//!
//! ```text
//! Browser ──HTTP/SSE──> led-console-bin ──IPC v1 (UDS 0600)──> led-daemon
//!                              └────────HTTP────> led_hal::serve_metrics
//! ```
//!
//! O browser **nunca** fala UDS, **nunca** toca no `ShowRuntime` e **nunca** alcança o
//! exporter diretamente. Não há segunda origem.
//!
//! # Porque não há framework
//!
//! O workspace não tem nenhum (verificado: zero `axum`/`hyper`/`actix`/`warp`), e já tem
//! **dois** servidores HTTP escritos à mão pela mesma razão — `led_hal::serve_metrics` e
//! `led_readmodel::serve_readmodel`. A superfície aqui são 11 rotas sem query-strings, sem
//! upload e sem cookies. Um framework traria uma árvore de dependências para resolver um
//! problema que o `std` resolve em cem linhas, e violaria a convenção *"std-only where
//! possible; add a dependency only with a reason"*.
//!
//! # O router não é uma segunda fonte de verdade
//!
//! O encaminhamento percorre a [`ROTAS`](crate::surface::ROTAS) — a **mesma tabela** que os
//! gates estruturais auditam. Uma rota que não esteja lá não existe aqui, por construção:
//! não há um segundo sítio onde acrescentar caminhos.
//!
//! # O que este módulo nunca faz
//!
//! Não recalcula estado, não fabrica dados e não reinterpreta códigos de erro. Um daemon em
//! baixo é `OFFLINE` **503**, nunca um `200` com um instantâneo inventado — que é o modo de
//! falha mais caro desta camada: um ecrã verde sobre um palco morto.

#![cfg(unix)]

use crate::ipc::{Erro, Ligacao};
use crate::surface::{Verbo, ROTAS};
use led_daemon_bin::json::escape;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// De onde o console fala com o resto. **Dado injetado** — nada é descoberto aqui.
#[derive(Clone, Debug)]
pub struct Config {
    /// O socket UDS do daemon.
    pub socket_daemon: String,
    /// O exporter Prometheus a repassar em `/api/metrics` (ADR-0026 §9-bis).
    ///
    /// `None` = não há exporter configurado, e `/api/metrics` di-lo em vez de inventar zeros.
    pub exporter: Option<SocketAddr>,
}

pub struct ConsoleServer {
    pub addr: SocketAddr,
    /// O difusor. **Uma** subscrição no daemon, N browsers (ADR-0026 §4).
    pub fanout: Arc<crate::fanout::Fanout>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ConsoleServer {
    /// Pede ao laço de accept que termine e junta a thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr); // desbloqueia o accept()
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Sobe o console em `addr`. **Recusa bind não-loopback** enquanto não houver auth.
pub fn serve(addr: SocketAddr, cfg: Config) -> std::io::Result<ConsoleServer> {
    // A guarda vem PRIMEIRO: nunca se chega a abrir um socket que não devia existir.
    crate::limits::bind_permitido(addr)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;

    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);

    let fanout = Arc::new(crate::fanout::Fanout::novo());

    // **UMA** subscrição no daemon, para todo o console. Não é por browser, e não é por
    // pedido: é uma, aqui, no arranque. Se fosse por browser, a carga no daemon passaria a
    // depender de quantos separadores o operador tem abertos (ADR-0026 §4).
    subir_subscricao(&cfg, Arc::clone(&fanout), Arc::clone(&stop));

    let f = Arc::clone(&fanout);
    let handle = std::thread::Builder::new().name("lumyx-console-http".into()).spawn(
        move || {
            for conn in listener.incoming() {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                let cfg = cfg.clone();
                let f = Arc::clone(&f);
                let parar = Arc::clone(&flag);
                // Uma ligação problemática não pode derrubar o console — a mesma
                // disciplina do `Server::spawn` do GS3. E um browser lento fica preso na
                // **sua** thread, nunca na do accept nem na da subscrição.
                std::thread::spawn(move || {
                    let _ = atender(stream, &cfg, &f, &parar);
                });
            }
        },
    )?;

    Ok(ConsoleServer { addr: bound, fanout, stop, handle: Some(handle) })
}

/// **O supervisor da subscrição upstream.** Uma thread, para todo o console.
///
/// # O único autorizado a subscrever
///
/// Browsers não subscrevem. O `Fanout` não subscreve. Os handlers HTTP não subscrevem. Só
/// esta função — e ela só consegue abrir uma de cada vez, porque
/// [`Fanout::reivindicar_subscricao`] devolve `None` quando já existe uma. **Duas ao mesmo
/// tempo não são representáveis**, e não apenas proibidas.
///
/// # O ciclo
///
/// ```text
/// reivindica ──> liga ──> subscribe ──> lê e difunde
///      ^                                    │ (morreu)
///      └──────── backoff <── liberta ───────┘
/// ```
///
/// A guarda é **libertada antes de esperar**: durante a espera `subscricao_viva()` é `false`,
/// que é a verdade. Fingir que a subscrição continua viva enquanto se espera seria o mesmo
/// tipo de mentira que um `200` sobre um daemon morto.
///
/// # O que não faz
///
/// Não fabrica eventos, não injeta estados e não inventa um `healthy` para tapar a ausência.
/// Sem daemon há **silêncio** — e o silêncio é observável em `subscricao_viva()`.
fn subir_subscricao(cfg: &Config, fanout: Arc<crate::fanout::Fanout>, stop: Arc<AtomicBool>) {
    let socket = cfg.socket_daemon.clone();
    std::thread::Builder::new()
        .name("lumyx-console-eventos".into())
        .spawn(move || {
            let mut espera = crate::limits::BACKOFF_INICIAL;
            while !stop.load(Ordering::Relaxed) {
                // Se já houver uma subscrição, este supervisor não tem nada a fazer. Não é
                // esperado (há um só), e é por isso que sai em vez de insistir.
                let Some(guarda) = fanout.reivindicar_subscricao() else { return };

                fanout.marcar_tentativa();
                let teve_fluxo = fluxo(&socket, &fanout, &stop, &guarda);

                // Libertar **antes** do backoff: em baixo, o medidor tem de dizer 0.
                drop(guarda);

                if teve_fluxo {
                    // Houve ligação: a próxima falha recomeça do início, senão um daemon que
                    // reinicia de vez em quando acabaria com esperas de segundos sem razão.
                    espera = crate::limits::BACKOFF_INICIAL;
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                dormir(espera, &stop);
                espera = (espera * 2).min(crate::limits::BACKOFF_MAX);
            }
        })
        .ok();
}

/// Abre a ligação, subscreve e difunde até morrer. `true` se chegou a haver fluxo.
fn fluxo(
    socket: &str,
    fanout: &crate::fanout::Fanout,
    stop: &AtomicBool,
    guarda: &crate::fanout::GuardaSubscricao<'_>,
) -> bool {
    let Ok(mut l) = Ligacao::abrir(socket, "lumyx-console-eventos") else {
        return false; // daemon ausente: sem subscrição, e sem mentira.
    };
    if l.subscrever().is_err() {
        return false;
    }
    // Só **agora** existe fluxo: o `subscribe` foi aceite pelo daemon. Antes deste ponto,
    // `subscricao_viva()` diz `false` — e é isso que impede alguém de difundir para uma
    // subscrição que ainda não está de pé.
    guarda.estabelecida();
    while !stop.load(Ordering::Relaxed) {
        match l.ler_linha() {
            // A linha do daemon é difundida **tal como veio**. O console não a
            // reinterpreta, não a filtra e não lhe acrescenta nada.
            Ok(linha) if !linha.is_empty() => fanout.difundir(&linha),
            Ok(_) => {}
            Err(_) => break, // o daemon fechou; não se inventa um fluxo.
        }
    }
    true
}

/// Espera `total`, mas **acorda** se o console estiver a encerrar.
///
/// Sem isto, parar o console podia demorar o teto do backoff — e um `stop()` que demora
/// segundos acaba por ser um `stop()` que alguém contorna.
fn dormir(total: std::time::Duration, stop: &AtomicBool) {
    const FATIA: std::time::Duration = std::time::Duration::from_millis(10);
    let fim = std::time::Instant::now() + total;
    while std::time::Instant::now() < fim {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(FATIA);
    }
}

/// Uma resposta pronta a escrever.
struct Saida {
    status: u16,
    content_type: &'static str,
    corpo: String,
    /// `Allow:` — obrigatório num 405 para o cliente saber o que fazer.
    allow: Option<&'static str>,
}

impl Saida {
    fn json(status: u16, corpo: String) -> Self {
        Self { status, content_type: "application/json", corpo, allow: None }
    }
    fn erro(status: u16, code: &str, detail: &str) -> Self {
        Self::json(
            status,
            format!(
                r#"{{"ok":false,"error":{{"code":"{}","detail":"{}"}}}}"#,
                escape(code),
                escape(detail)
            ),
        )
    }
}

fn frase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

/// **O fluxo SSE.** Segura a ligação aberta e entrega o que o `Fanout` tiver para este
/// browser — e só para este.
///
/// Ligar-se aqui **não** abre nada no daemon: o `Fanout::ligar` só cria uma fila local. É por
/// isso que N browsers, e as suas reconexões, não multiplicam subscrições a montante.
fn sse(
    mut stream: TcpStream,
    fanout: &Arc<crate::fanout::Fanout>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
         Connection: keep-alive\r\n\r\n"
    )?;
    stream.flush()?;

    let sub = fanout.ligar();
    // Um browser preso não pode segurar a thread para sempre; e como a thread é só dele,
    // ficar preso aqui não atrasa mais ninguém.
    let _ = stream.set_write_timeout(Some(crate::limits::http_timeout()));

    let r = (|| -> std::io::Result<()> {
        // Comentário inicial: confirma ao cliente que o fluxo abriu, sem ser um evento.
        // Um comentário SSE (`:`) não é entregue como dado — não inventa nada.
        write!(stream, ": lumyx\n\n")?;
        stream.flush()?;

        // Quanto tempo sem eventos antes de mandar um sinal de vida.
        //
        // **Não é decoração.** Sem escrever nada, um browser que desaparece não é detetado:
        // a thread ficaria a dormir para sempre e o `Subscriber` nunca seria podado — a
        // lista cresceria sem limite, que é exatamente o defeito que o servidor IPC do GS3
        // poda do seu lado. Foi um teste desta fatia que o apanhou.
        //
        // Um **comentário** SSE (`:`) não é entregue como dado ao cliente: mantém a ligação
        // viva e falha a escrita quando ela morre, sem inventar um evento.
        const SINAL_DE_VIDA: std::time::Duration = std::time::Duration::from_millis(200);
        let mut parado_desde = std::time::Instant::now();

        while !stop.load(Ordering::Relaxed) {
            match sub.proximo() {
                Some(linha) => {
                    // A linha do daemon vai no `data:` **inalterada**.
                    write!(stream, "data: {linha}\n\n")?;
                    stream.flush()?;
                    parado_desde = std::time::Instant::now();
                }
                None => {
                    if parado_desde.elapsed() >= SINAL_DE_VIDA {
                        write!(stream, ":\n\n")?; // comentário: mantém vivo e deteta a queda
                        stream.flush()?;
                        parado_desde = std::time::Instant::now();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        Ok(())
    })();

    // **Sempre** — saia como sair. Sem isto, a lista de subscritores cresceria para sempre,
    // que é o mesmo defeito que o servidor IPC do GS3 poda do seu lado.
    fanout.desligar(sub.id());
    r
}

fn atender(
    mut stream: TcpStream,
    cfg: &Config,
    fanout: &Arc<crate::fanout::Fanout>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(crate::limits::http_timeout()))?;

    // Cabeçalhos podem vir em vários segmentos TCP. Teto explícito: o mesmo `MAX_BODY` do
    // GS3, pela mesma razão — um cliente que nunca feche não pode fazer o processo crescer.
    let mut bruto = Vec::new();
    let mut janela = [0u8; 1024];
    while !bruto.windows(4).any(|w| w == b"\r\n\r\n") && bruto.len() < crate::limits::MAX_BODY {
        match stream.read(&mut janela) {
            Ok(0) => break,
            Ok(n) => bruto.extend_from_slice(&janela[..n]),
            Err(_) => break,
        }
    }
    let texto = String::from_utf8_lossy(&bruto).to_string();
    let Some(corte) = texto.find("\r\n\r\n") else {
        return escrever(&mut stream, Saida::erro(400, "console.bad_request", "pedido incompleto"));
    };
    let (cabecalhos, resto) = texto.split_at(corte);
    let mut corpo = resto[4..].to_string();

    let Some(linha) = cabecalhos.lines().next() else {
        return escrever(&mut stream, Saida::erro(400, "console.bad_request", "sem linha de pedido"));
    };
    let mut partes = linha.split_whitespace();
    let (Some(metodo), Some(caminho)) = (partes.next(), partes.next()) else {
        return escrever(&mut stream, Saida::erro(400, "console.bad_request", "linha ilegivel"));
    };

    // Ler o resto do corpo, se o `Content-Length` anunciar mais do que já chegou.
    if let Some(n) = content_length(cabecalhos) {
        while corpo.len() < n && corpo.len() < crate::limits::MAX_BODY {
            match stream.read(&mut janela) {
                Ok(0) => break,
                Ok(k) => corpo.push_str(&String::from_utf8_lossy(&janela[..k])),
                Err(_) => break,
            }
        }
    }

    // O SSE **toma conta do stream**: não escreve uma `Saida` e sai, fica a servir. Só
    // depois de a rota e o método terem sido validados como qualquer outra.
    let limpo = caminho.split('?').next().unwrap_or(caminho);
    if limpo == "/api/events" && metodo == "GET" {
        return sse(stream, fanout, stop);
    }

    escrever(&mut stream, encaminhar(metodo, caminho, &corpo, cfg))
}

fn content_length(cabecalhos: &str) -> Option<usize> {
    cabecalhos
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l["content-length:".len()..].trim().parse().ok())
}

/// **O encaminhamento percorre a `ROTAS`.** Não há segunda tabela de caminhos.
fn encaminhar(metodo: &str, caminho: &str, corpo: &str, cfg: &Config) -> Saida {
    // Sem query-string: a superfície não tem nenhuma, e aceitá-la em silêncio abriria uma
    // forma de o mesmo recurso ter dois nomes.
    let caminho = caminho.split('?').next().unwrap_or(caminho);

    let Some(rota) = ROTAS.iter().find(|r| r.caminho == caminho) else {
        // 404 ANTES de olhar para o método: um caminho que não existe não tem métodos.
        return Saida::erro(404, "console.not_found", &format!("{caminho} nao existe"));
    };

    let esperado = match rota.verbo {
        Verbo::Get => "GET",
        Verbo::Post => "POST",
    };
    if metodo != esperado {
        let mut s = Saida::erro(
            405,
            "console.method_not_allowed",
            &format!("{caminho} aceita {esperado}, nao {metodo}"),
        );
        s.allow = Some(esperado);
        return s;
    }

    match caminho {
        "/api/state" => daemon(cfg, "status", ""),
        "/api/version" => daemon(cfg, "version", ""),
        "/api/events" => eventos(),
        "/api/metrics" => metricas(cfg),
        "/api/profiles" => perfis(),
        // Transporte: o comando é o segmento final, e vem da própria `ROTAS`.
        _ => match rota.cmd_ipc {
            Some(cmd) => {
                // O corpo do browser viaja como `args` **sem ser reinterpretado**. O console
                // não constrói tipos de comando próprios; quem valida é o daemon.
                let t = corpo.trim();
                let args = if t.is_empty() { String::new() } else { format!(r#","args":{t}"#) };
                daemon(cfg, cmd, &args)
            }
            None => Saida::erro(501, "console.not_implemented", &format!("{caminho} sem handler")),
        },
    }
}

/// Fala com o daemon e **repassa a resposta dele**.
///
/// Uma ligação por pedido: simples e correta. O daemon aceita várias, e o handshake é uma
/// linha. Não há aqui pool nenhum porque ainda não há medição que o justifique — e uma
/// abstração prematura seria mais caro que o `connect`.
fn daemon(cfg: &Config, cmd: &str, args: &str) -> Saida {
    let mut l = match Ligacao::abrir(&cfg.socket_daemon, "lumyx-console") {
        Ok(l) => l,
        Err(e) => return de_erro(e),
    };
    match l.pedir(cmd, args) {
        // A linha do daemon **é** a resposta. O console não a reescreve.
        Ok(linha) => Saida::json(200, linha),
        Err(e) => de_erro(e),
    }
}

/// Traduz o erro do IPC para HTTP **sem lhe tocar na semântica**.
///
/// O `code` é o do daemon, verbatim (ADR-0026 §6); o estado HTTP é **transporte**, e o
/// significado continua no `code`. Mapear `no_show_loaded` para "409" e parar aí perderia a
/// razão da recusa.
fn de_erro(e: Erro) -> Saida {
    let detail = match &e {
        Erro::Offline(d) | Erro::Protocolo(d) => d.clone(),
        Erro::Recusado { detail, .. } => detail.clone(),
        Erro::PedidoDemasiadoGrande { bytes, limite } => {
            format!("{bytes} bytes excede o limite de {limite}")
        }
    };
    Saida::erro(e.http_status(), e.code(), &detail)
}

/// `/api/metrics` — **proxy read-only** do exporter (ADR-0026 §9-bis).
fn metricas(cfg: &Config) -> Saida {
    let Some(addr) = cfg.exporter else {
        return Saida::erro(
            503,
            "console.exporter_nao_configurado",
            "nao ha exporter configurado; nao ha metricas para repassar",
        );
    };
    match crate::metrics::buscar(addr) {
        Ok(m) => Saida {
            status: 200,
            // O `Content-Type` do exporter atravessa; o formato de exposição é dele.
            content_type: "text/plain; version=0.0.4",
            corpo: m.corpo,
            allow: None,
        },
        Err(e) => Saida::erro(e.http_status(), "console.exporter_indisponivel", &format!("{e:?}")),
    }
}

/// `/api/events` — a superfície SSE.
///
/// **Mínima de propósito nesta fatia:** anuncia-se como `text/event-stream` e fecha. O
/// fan-out para N browsers já existe (`Fanout`), mas ligá-lo exige uma subscrição viva ao
/// daemon e uma política de reconexão — trabalho da fatia seguinte. Anunciar o
/// `Content-Type` errado agora seria fixar um contrato que depois teria de mudar.
fn eventos() -> Saida {
    Saida {
        status: 200,
        content_type: "text/event-stream",
        corpo: ": lumyx console — fluxo de eventos (fan-out por ligar)\n\n".to_string(),
        allow: None,
    }
}

/// `/api/profiles` — **declarado e ainda não servido**, com a razão à vista.
///
/// O catálogo vive no `led-hardware-profile`, e o console **não depende dele**. Servi-lo
/// exigiria uma de duas coisas, ambas proibidas nesta fatia: acrescentar um crate de domínio
/// ao tradutor — que o gate `nenhuma_segunda_fonte_de_verdade_no_console` recusa — ou
/// acrescentar um comando ao IPC v1, que está fechado.
///
/// **501 e não 404**: a rota existe e está no contrato; o que falta é a decisão de por onde
/// o catálogo chega. Um 404 diria que a rota não existe, e seria mentira.
fn perfis() -> Saida {
    Saida::erro(
        501,
        "console.not_implemented",
        "o catalogo de presets vive a montante e ainda nao tem caminho ate ao console; \
         ver ADR-0026 e a decisao pendente na F4",
    )
}

fn escrever(stream: &mut TcpStream, s: Saida) -> std::io::Result<()> {
    let allow = s.allow.map(|a| format!("Allow: {a}\r\n")).unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        s.status,
        frase(s.status),
        s.content_type,
        s.corpo.len(),
        allow,
        s.corpo
    )?;
    stream.flush()
}
