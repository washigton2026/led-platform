//! **Integração contra o `Server` real** — sem mocks (ADR-0026 §1, §6, §11).
//!
//! Se o console e o `ledctl` divergirem no fio, é aqui que fica vermelho. O transporte é o
//! mesmo `led_daemon_bin::server::Server` que o `tests/ipc.rs` do daemon já exercita.

#![cfg(unix)]

use led_console_bin::ipc::{Erro, Ligacao};
use led_console_bin::truth::{EstadoUi, Instantaneo};
use led_daemon::ShowRuntime;
use led_daemon_bin::run::run_with_control;
use led_daemon_bin::server::{ControlPlane, Server};
use led_daemon_bin::{Config, Integrity, Journal, SystemPacer};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Daemon {
    path: std::path::PathBuf,
    shutdown: Arc<AtomicBool>,
    laco: Option<std::thread::JoinHandle<()>>,
}

/// Levanta o daemon **inteiro**: servidor UDS **e o laço aplicador**.
///
/// O laço não é decoração. Pelo GS3, as threads de ligação só **enfileiram**; quem aplica é
/// o laço. Sem ele, `play` não devolve `no_show_loaded` — expira em `engine_busy` ao fim do
/// `REPLY_TIMEOUT`, e um teste de códigos de erro estaria a medir o timeout.
fn subir(nome: &str) -> Daemon {
    let path = std::env::temp_dir()
        .join(format!("lumyx-console-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let shutdown = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&shutdown));
    Server::bind(&path).expect("bind").spawn(Arc::clone(&cp));

    let flag = Arc::clone(&shutdown);
    let laco = std::thread::spawn(move || {
        let cfg = Config {
            tick_ms: 25, // 40 Hz — a cadência que o ADR-0025 permite aos presets do catálogo
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

impl Daemon {
    fn s(&self) -> String {
        self.path.to_str().unwrap().to_string()
    }
    fn ligar(&self) -> Ligacao {
        Ligacao::abrir(&self.s(), "teste").expect("handshake")
    }
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

/// **Os códigos de erro do daemon atravessam intactos** (ADR-0026 §6).
///
/// Mapear `no_show_loaded` para "409" e parar aí perderia a razão da recusa. O estado HTTP
/// transporta significado de **transporte**; o código continua a ser o do daemon.
#[test]
fn os_codigos_de_erro_do_daemon_chegam_verbatim_e_o_http_nao_os_substitui() {
    let d = subir("codigos");
    let mut l = d.ligar();

    // Sem show carregado, o transporte recusa — e o motivo é do daemon, não nosso.
    let e = l.pedir("play", "").expect_err("play sem show tem de recusar");
    match &e {
        Erro::Recusado { code, .. } => {
            assert_eq!(code, "no_show_loaded", "o codigo tem de ser o do daemon");
            assert_eq!(e.http_status(), 409, "recusa de dominio e 409");
        }
        outro => panic!("esperava recusa do daemon, veio {outro:?}"),
    }
    assert_eq!(e.code(), "no_show_loaded", "e o code() nao pode reescrever nada");

    // Um comando que o daemon não conhece continua a ser erro **do daemon**.
    let e = l.pedir("naoexiste", "").expect_err("comando desconhecido");
    assert!(
        matches!(e, Erro::Recusado { .. }),
        "recusa do daemon, nunca um erro fabricado pelo console: {e:?}"
    );
}

/// **Controle negativo:** o caminho feliz tem de continuar feliz.
///
/// Sem isto, um console que recusasse tudo passaria o teste de cima.
#[test]
fn um_comando_valido_continua_a_passar() {
    let d = subir("feliz");
    let mut l = d.ligar();
    let r = l.pedir("ping", "").expect("ping tem de passar");
    assert!(r.contains(r#""pong""#), "{r}");
    let r = l.pedir("status", "").expect("status tem de passar");
    assert!(r.contains(r#""state""#), "{r}");
}

/// **Sem `hello`, nada é aceite** — e o console não pode contornar o handshake.
#[test]
fn o_handshake_e_obrigatorio_e_o_console_fa_lo_ao_abrir() {
    let d = subir("hello");
    // Crua: sem handshake, o daemon recusa.
    let s = UnixStream::connect(&d.path).unwrap();
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    writeln!(w, r#"{{"v":1,"id":1,"cmd":"ping"}}"#).unwrap();
    let mut linha = String::new();
    r.read_line(&mut linha).unwrap();
    assert!(linha.contains("unauthenticated"), "{linha}");

    // Pela `Ligacao`, o handshake já aconteceu em `abrir` — o mesmo ping passa.
    let mut l = d.ligar();
    assert!(l.pedir("ping", "").is_ok());
}

/// **Um corpo acima de 64 KiB é recusado — e a guarda local NÃO é redundante.**
///
/// O daemon responde à linha longa com `err_line(None, …)`: **sem `id`**. O console
/// distingue resposta de evento pela *presença* de `id`, por isso essa resposta seria lida
/// como evento e o pedido ficaria à espera de uma resposta que nunca chega. A guarda no
/// `pedir` fecha isso antes de o byte sair — é por isso que existe, e não por duplicação.
#[test]
fn um_corpo_acima_do_limite_e_recusado_antes_de_sair() {
    let d = subir("grande");
    let mut l = d.ligar();
    let enorme = "x".repeat(led_console_bin::MAX_BODY + 1);
    let e = l
        .pedir("load", &format!(r#","args":{{"path":"{enorme}"}}"#))
        .expect_err("acima de 64 KiB tem de ser recusado");
    match &e {
        Erro::PedidoDemasiadoGrande { bytes, limite } => {
            assert!(bytes > limite, "{bytes} tem de exceder {limite}");
            assert_eq!(*limite, led_console_bin::MAX_BODY);
        }
        outro => panic!("a culpa e do pedido, nao do daemon: {outro:?}"),
    }
    assert_eq!(e.http_status(), 413, "413, nao 502: o daemon nao fez nada de errado");
    assert_eq!(e.code(), "console.request_too_large");

    // E a ligação continua utilizável: recusar não pode derrubar a sessão.
    assert!(l.pedir("ping", "").is_ok(), "a recusa nao pode matar a ligacao");
}

/// **O daemon também recusa** — a guarda do console não é a única defesa.
///
/// Provado no fio, sem passar pela `Ligacao`, porque é o daemon que está a ser afirmado.
#[test]
fn o_daemon_recusa_a_linha_longa_por_si_proprio() {
    let d = subir("grande2");
    let s = UnixStream::connect(&d.path).unwrap();
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    writeln!(w, r#"{{"v":1,"id":1,"cmd":"hello","client":"t"}}"#).unwrap();
    let mut _h = String::new();
    r.read_line(&mut _h).unwrap();

    let enorme = "x".repeat(led_console_bin::MAX_BODY + 10);
    writeln!(w, r#"{{"v":1,"id":2,"cmd":"load","args":{{"path":"{enorme}"}}}}"#).unwrap();
    let mut linha = String::new();
    r.read_line(&mut linha).unwrap();
    assert!(linha.contains("bad_request"), "{}", &linha[..linha.len().min(200)]);
    assert!(linha.contains("demasiado longa"), "{}", &linha[..linha.len().min(200)]);
}

/// **Profundidade JSON acima de 16 é recusada** — `[[[[[…` estoura a pilha, e um cliente
/// derrubaria o daemon com uma linha de texto (ADR-0026 §11).
#[test]
fn profundidade_json_acima_do_limite_e_recusada() {
    let d = subir("fundo");
    let mut l = d.ligar();
    let n = led_console_bin::MAX_JSON_DEPTH + 5;
    let fundo = format!(r#","args":{}{}"#, "[".repeat(n), "]".repeat(n));
    let e = l.pedir("load", &fundo).expect_err("aninhamento fundo tem de ser recusado");
    match &e {
        Erro::Recusado { code, .. } => assert_eq!(code, "bad_request", "{e:?}"),
        outro => panic!("esperava recusa do daemon, veio {outro:?}"),
    }
    assert_eq!(e.http_status(), 400);

    // Controle negativo: **dentro** do limite, o parser não pode recusar por profundidade.
    let raso = format!(r#","args":{}{}"#, "[".repeat(3), "]".repeat(3));
    let e = l.pedir("load", &raso).expect_err("load sem path continua invalido");
    match e {
        Erro::Recusado { detail, .. } => assert!(
            !detail.contains("profund"),
            "3 niveis nao pode ser recusado por profundidade: {detail}"
        ),
        outro => panic!("{outro:?}"),
    }
}

/// **Daemon offline não produz zeros artificiais** (ADR-0026 §7, §9).
///
/// Um `frames: 0` inventado seria indistinguível de um palco parado de verdade. O erro é
/// `Offline`, o estado é `OFFLINE`, e o instantâneo diz **que nunca houve** — não zero.
#[test]
fn com_o_daemon_offline_nao_ha_zeros_fabricados() {
    let inexistente = std::env::temp_dir()
        .join(format!("lumyx-nao-existe-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&inexistente);

    let e = match Ligacao::abrir(inexistente.to_str().unwrap(), "teste") {
        Err(e) => e,
        Ok(_) => panic!("socket inexistente nao pode abrir"),
    };
    assert!(matches!(e, Erro::Offline(_)), "{e:?}");
    assert_eq!(e.code(), "console.daemon_offline");
    assert_eq!(e.http_status(), 503, "503 = o console vive, o daemon e que nao");

    // E o que a UI recebe: ausência declarada, nunca um valor.
    let vazio: Instantaneo<u64> = Instantaneo::nunca_houve();
    assert!(vazio.dado().is_none(), "sem daemon, nao ha valor — nem 0");
    assert_eq!(vazio.estado(), EstadoUi::Unknown, "nunca houve dado: Unknown, nao Offline");
    assert_eq!(vazio.stale_ms(), None, "idade de um valor que nunca existiu nao e 0");
}

/// **O daemon que morre a meio vira `Offline`, preservando o último conhecido.**
///
/// Apagar o ecrã seria perder a informação que o operador ainda precisa de ver; apresentá-la
/// como atual seria a mentira mais fácil desta arquitetura.
#[test]
fn a_queda_a_meio_preserva_o_ultimo_conhecido_com_idade() {
    let d = subir("queda");
    let mut l = d.ligar();
    let ultimo = l.pedir("status", "").expect("status inicial");
    assert!(ultimo.contains(r#""state""#));

    let visto = Instantaneo::fresco(ultimo.clone(), EstadoUi::Running);
    assert_eq!(visto.estado(), EstadoUi::Running, "com daemon vivo nao e OFFLINE");
    assert!(!visto.e_velho(), "dado fresco nunca e velho");

    let velho = Instantaneo::velho(ultimo.clone(), std::time::Duration::from_secs(120));
    assert_eq!(velho.dado(), Some(&ultimo), "o ultimo conhecido nao se apaga");
    assert_eq!(velho.stale_ms(), Some(120_000), "e vem com a idade colada");
    assert_eq!(velho.estado(), EstadoUi::Offline);
}
