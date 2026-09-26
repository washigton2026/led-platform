//! `led-console` — o **processo** do console (ADR-0028 D6).
//!
//! # Porque este binário existe
//!
//! O ADR-0013 exige que o output em tempo real **não partilhe processo de falha** com a UI, e
//! o ADR-0026 §2 concretiza-o: *"um pânico no parser HTTP mata o console, **não** o show"*.
//! Essa garantia exige que o console **seja** um processo — e até aqui não era: o crate era só
//! uma biblioteca, e apenas os testes chamavam [`serve`].
//!
//! # O que este ficheiro **não** faz
//!
//! Não abre sockets, não encaminha rotas, não fala IPC e não decide nada sobre segurança.
//! Tudo isso já existe em [`http::serve`], provado por testes de integração contra o daemon
//! real. Este ficheiro é **só o invólucro**: lê argumentos, constrói a [`Config`] e espera.
//!
//! Em particular, a política de bind **não é repetida aqui**. `serve()` chama
//! `limits::bind_permitido` **antes** de abrir o socket; um endereço que não devia existir
//! nunca chega a existir, e um `main` que voltasse a verificar criaria uma segunda regra que
//! podia divergir da primeira.
//!
//! # Encerramento
//!
//! Linha `shutdown` no stdin, como o `led-daemon` já faz. **Não há tratamento de
//! `SIGINT`/`SIGTERM`** — exigiria uma dependência de sinais, que o daemon também recusou
//! pela mesma razão. Ctrl-C termina o processo, mas abruptamente.

#[cfg(unix)]
use led_console_bin::http::{serve, Config};
#[cfg(unix)]
use std::net::SocketAddr;

const AJUDA: &str = "\
led-console — a fronteira HTTP/SSE entre o browser e o daemon (ADR-0026)

USO:
    led-console --bind ADDR --socket CAMINHO [--exporter ADDR]

    --bind ADDR           Endereco HTTP. OBRIGATORIO: o console nao escolhe a porta.
                          E a convencao do projeto — serve_metrics e serve_readmodel
                          recebem o endereco de quem chama, e o led-player exige-o.
                          LOOPBACK-ONLY: qualquer outro endereco e recusado enquanto
                          o ADR-0014 nao der auth.  Ex.: --bind 127.0.0.1:7878
    --socket CAMINHO      Socket UDS do daemon (o mesmo que o `led-daemon --socket`
                          abriu). OBRIGATORIO: o console nao descobre o daemon.
    --exporter ADDR       Exporter Prometheus a repassar em /api/metrics. Sem ele, a
                          rota diz que nao ha exporter em vez de inventar zeros.
    -h, --help            Esta ajuda.

ENCERRAR:
    Escreva `shutdown` no stdin.
    NAO ha tratamento de SIGINT/SIGTERM — a mesma limitacao (e a mesma razao) do
    led-daemon: exigiria uma dependencia de sinais.

O daemon NAO precisa de estar de pe para o console arrancar: OFFLINE e um estado
(ADR-0026 §7), reportado como 503, nunca um 200 com dados inventados.

CODIGOS DE SAIDA:
    0  encerrou por pedido
    1  nao foi possivel abrir o socket HTTP (inclui bind recusado)
    2  erro de uso
";

#[cfg(unix)]
#[derive(Debug, PartialEq)]
struct Args {
    bind: SocketAddr,
    socket: String,
    exporter: Option<SocketAddr>,
}

/// Analisa a linha de comando. **À mão**, como todos os binários deste workspace — não há
/// `clap` nem equivalente em lado nenhum, e um binário com três flags não justifica a
/// primeira dependência de CLI do projeto.
#[cfg(unix)]
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut bind: Option<SocketAddr> = None;
    let mut socket: Option<String> = None;
    let mut exporter: Option<SocketAddr> = None;

    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        let mut valor = |flag: &str| -> Result<String, String> {
            i += 1;
            argv.get(i).cloned().ok_or_else(|| format!("{flag} exige um valor"))
        };
        match a.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--bind" => {
                let v = valor("--bind")?;
                bind = Some(v.parse().map_err(|_| format!("--bind invalido: {v}"))?);
            }
            "--socket" => socket = Some(valor("--socket")?),
            "--exporter" => {
                let v = valor("--exporter")?;
                exporter = Some(v.parse().map_err(|_| format!("--exporter invalido: {v}"))?);
            }
            outro => return Err(format!("argumento desconhecido: {outro}")),
        }
        i += 1;
    }

    // `--socket` não tem omissão **de propósito**: o `led-daemon` também não a tem, e o
    // console não descobre o daemon — o endereço é dado injetado (ADR-0026).
    let socket = socket.ok_or("falta --socket: o console nao descobre o daemon")?;

    // `--bind` também não tem omissão, e a razão é a convenção do workspace: **o endereço é
    // sempre dado pelo chamador**. `serve_metrics` e `serve_readmodel` recebem `SocketAddr`,
    // `led-player --metrics` exige valor, `led-daemon --socket` exige caminho. Nenhum
    // servidor deste projeto escolhe sozinho onde escuta.
    //
    // A primeira versão deste binário caía em `127.0.0.1:0` — não inventava um número, mas
    // inventava a **omissão**, e com porta 0 o endereço muda a cada arranque. Um binário que
    // escolhe a porta em silêncio é um binário a que o operador não sabe voltar.
    let bind = bind.ok_or("falta --bind: o console nao escolhe a porta (ex.: --bind 127.0.0.1:7878)")?;

    Ok(Args { bind, socket, exporter })
}

/// Lê o stdin numa thread própria e sinaliza o encerramento na linha `shutdown`.
///
/// EOF **não** encerra: um processo supervisionado corre com o stdin fechado, e fazer o EOF
/// encerrar mataria o console à nascença quando o stdin fosse `/dev/null`. É a mesma decisão
/// que o `led-daemon` tomou, pela mesma razão.
#[cfg(unix)]
fn spawn_stdin_shutdown(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for linha in stdin.lock().lines() {
            match linha {
                Ok(l) if l.trim().eq_ignore_ascii_case("shutdown") => {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
}

#[cfg(unix)]
fn main() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) if e.is_empty() => {
            print!("{AJUDA}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("erro: {e}\n");
            print!("{AJUDA}");
            std::process::exit(2);
        }
    };

    // A `Config` é **dado injetado**: nada é descoberto aqui.
    let cfg = Config { socket_daemon: args.socket.clone(), exporter: args.exporter };

    // `serve` impõe a fronteira loopback ANTES do bind. Um erro aqui inclui o endereço
    // recusado, e é por isso que a mensagem é repassada tal como vem.
    let servidor = match serve(args.bind, cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erro: nao foi possivel abrir o console em {}: {e}", args.bind);
            std::process::exit(1);
        }
    };

    // O endereço REAL, e não o pedido. Hoje coincidem, porque `--bind` é obrigatório; ler
    // `servidor.addr` mantém a impressão verdadeira mesmo que alguma vez volte a haver um
    // caminho em que o SO escolha a porta.
    println!("console em http://{}", servidor.addr);
    println!("daemon  : {}", args.socket);
    match args.exporter {
        Some(a) => println!("exporter: {a}"),
        None => println!("exporter: (nenhum) — /api/metrics dira que nao ha, sem inventar zeros"),
    }
    println!("escreva `shutdown` no stdin para encerrar");

    let parar = Arc::new(AtomicBool::new(false));
    spawn_stdin_shutdown(Arc::clone(&parar));

    // O laço de accept vive na thread do servidor; aqui só se espera pelo pedido de paragem.
    while !parar.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    servidor.stop();
    println!("console encerrado");
}

/// O console é Unix-only porque o IPC v1 é UDS (ADR-0014, socket owner-only `0600`), e o
/// módulo `http` está sob `#![cfg(unix)]`. Windows é suportado mas **não conduz o desenho**
/// (ADR-0013), e por isso o binário existe e recusa-se em vez de não compilar.
#[cfg(not(unix))]
fn main() {
    eprintln!(
        "led-console: o console e Unix-only — o IPC v1 usa Unix Domain Sockets (ADR-0014)."
    );
    std::process::exit(2);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Args, String> {
        parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    /// **`--socket` é obrigatório**, e a mensagem diz porquê.
    ///
    /// Não tem omissão de propósito: o `led-daemon` também não a tem, e inventar um caminho
    /// por omissão faria o console procurar um daemon onde ninguém prometeu que ele estava.
    #[test]
    fn sem_socket_e_erro_de_uso() {
        let e = args(&[]).expect_err("sem --socket tem de falhar");
        assert!(e.contains("--socket"), "a mensagem tem de nomear a flag em falta: {e}");
    }

    /// **`--bind` é obrigatório: o console não escolhe a porta.**
    ///
    /// É a convenção do workspace — `serve_metrics` e `serve_readmodel` recebem o
    /// `SocketAddr` de quem chama, e `led-player --metrics` exige o valor. Nenhum servidor
    /// deste projeto decide sozinho onde escuta.
    ///
    /// A primeira versão caía em `127.0.0.1:0`. Não inventava um número, mas inventava a
    /// **omissão** — e com porta 0 o endereço muda a cada arranque, o que torna o console
    /// inalcançável para um browser que queira lá voltar.
    #[test]
    fn sem_bind_e_erro_de_uso() {
        let e = args(&["--socket", "/tmp/x.sock"]).expect_err("sem --bind tem de falhar");
        assert!(e.contains("--bind"), "a mensagem tem de nomear a flag em falta: {e}");
    }

    /// **Nenhuma porta por omissão existe no código.**
    ///
    /// Controlo negativo do teste acima: se alguém reintroduzir um `unwrap_or` com um
    /// endereço, é aqui que fica vermelho — mesmo que o parser continue a aceitar `--bind`.
    ///
    /// **Só varre o código de produção.** A primeira versão varria o ficheiro inteiro e
    /// reprovava por causa da **sua própria linha de asserção**, que contém as duas palavras
    /// que procura. É a mesma armadilha que a lista de palavras proibidas do ADR-0017 já
    /// tinha apanhado — um gate não pode ser o sítio onde o proibido é escrito.
    #[test]
    fn o_codigo_de_producao_nao_contem_nenhuma_porta_por_omissao() {
        const FONTE: &str = include_str!("main.rs");
        let producao = FONTE.split("mod tests").next().expect("ha codigo antes dos testes");
        assert!(producao.contains("fn parse_args"), "sanidade: varri a parte errada");

        for linha in producao.lines().map(str::trim) {
            if linha.starts_with("//") {
                continue;
            }
            assert!(
                !(linha.contains("unwrap_or") && linha.contains("bind")),
                "voltou a haver uma porta por omissao: {linha}"
            );
        }
    }

    /// Sem `--exporter` não há exporter — e isso é dito, não preenchido com zeros.
    #[test]
    fn sem_exporter_nao_ha_exporter() {
        let a = args(&["--bind", "127.0.0.1:7878", "--socket", "/tmp/x.sock"]).expect("valida");
        assert_eq!(a.exporter, None, "sem --exporter nao ha exporter — e nao zeros inventados");
    }

    /// As três flags são analisadas, e os tipos vêm da `Config` real.
    #[test]
    fn as_tres_flags_sao_analisadas() {
        let a = args(&[
            "--bind",
            "127.0.0.1:9999",
            "--socket",
            "/tmp/lumyx.sock",
            "--exporter",
            "127.0.0.1:9100",
        ])
        .expect("linha valida");
        assert_eq!(a.bind, "127.0.0.1:9999".parse().unwrap());
        assert_eq!(a.socket, "/tmp/lumyx.sock");
        assert_eq!(a.exporter, Some("127.0.0.1:9100".parse().unwrap()));
    }

    /// Um endereço ilegível é **erro de uso**, não um bind silencioso noutro sítio.
    #[test]
    fn endereco_invalido_e_recusado_em_vez_de_adivinhado() {
        for (flag, valor) in [("--bind", "nao-e-um-endereco"), ("--exporter", "1.2.3")] {
            let e = args(&["--socket", "/tmp/x.sock", flag, valor])
                .expect_err("{flag} invalido tem de falhar");
            assert!(e.contains(flag), "a mensagem tem de nomear a flag: {e}");
        }
    }

    /// Uma flag desconhecida não é ignorada em silêncio.
    #[test]
    fn flag_desconhecida_nao_passa_despercebida() {
        let e = args(&["--socket", "/tmp/x.sock", "--inventada"]).expect_err("tem de falhar");
        assert!(e.contains("--inventada"), "{e}");
    }

    /// **A ajuda não promete o que o console não faz.**
    ///
    /// Nomeia as três flags reais, diz que é loopback-only, e declara a ausência de
    /// tratamento de sinais em vez de a deixar por descobrir.
    #[test]
    fn a_ajuda_descreve_as_flags_reais_e_os_limites_reais() {
        for esperado in ["--socket", "--bind", "--exporter", "LOOPBACK-ONLY", "SIGINT"] {
            assert!(AJUDA.contains(esperado), "a ajuda nao menciona `{esperado}`");
        }
        // E não anuncia superfície que o ADR-0026 §14 proíbe.
        for proibido in ["blackout", "--auth", "--cors", "0.0.0.0"] {
            assert!(!AJUDA.contains(proibido), "a ajuda anuncia `{proibido}`");
        }
    }
}
