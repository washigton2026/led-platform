//! O limite de 64 KiB é imposto **durante** a leitura — não depois dela.
//!
//! Este ficheiro tem um teste só, e está separado do `ipc.rs` de propósito: instala um
//! **alocador global contador** para medir o pico de memória viva do processo. Um `#[test]`
//! vizinho a alocar em paralelo tornaria a medição ruído, e cada ficheiro de teste é um
//! binário próprio — é o isolamento que a medição exige.
//!
//! ## O que distingue este teste
//!
//! O ataque não é "uma linha comprida". É uma linha que **nunca termina**: o cliente escreve
//! megabytes sem um único `\n`. Contra isso, verificar `linha.len() > MAX_LINE` *depois* de
//! `BufReader::lines()` ter devolvido a linha não protege nada — para a verificação correr, a
//! linha já teve de ser inteiramente materializada. O teste falha nos dois eixos que o
//! defeito toca: a memória cresce com o que o atacante escreve, e a recusa nunca chega.

#![cfg(unix)]

use led_daemon_bin::json::parse;
use led_daemon_bin::server::{ControlPlane, MAX_LINE, Server};
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ── Alocador contador ────────────────────────────────────────────────────────

static VIVO: AtomicUsize = AtomicUsize::new(0);
static PICO: AtomicUsize = AtomicUsize::new(0);

struct Contador;

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let v = VIVO.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PICO.fetch_max(v, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        VIVO.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, novo: usize) -> *mut u8 {
        let n = unsafe { System.realloc(p, l, novo) };
        if !n.is_null() {
            let v = VIVO.fetch_add(novo, Ordering::Relaxed) + novo;
            PICO.fetch_max(v, Ordering::Relaxed);
            VIVO.fetch_sub(l.size(), Ordering::Relaxed);
        }
        n
    }
}

#[global_allocator]
static ALOC: Contador = Contador;

// ── O teste ──────────────────────────────────────────────────────────────────

/// Quanto o atacante escreve sem nunca enviar `\n`.
const ATAQUE: usize = 8 * 1024 * 1024;
/// Folga permitida ao daemon acima do seu próprio limite de linha.
///
/// `read_until` cresce o `Vec` por duplicação, portanto um limite de 64 KiB pode chegar a
/// ~128 KiB de capacidade. 1 MiB dá folga de sobra para isso e para o ruído das threads, e
/// continua **oito vezes** abaixo do que o ataque escreve — a margem é o que torna o teste
/// discriminante em vez de sensível ao acaso.
const FOLGA: usize = 1024 * 1024;

#[test]
fn linha_sem_fim_nao_faz_o_daemon_crescer() {
    let path =
        std::env::temp_dir().join(format!("lumyx-linha-sem-fim-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));
    let srv = Server::bind(&path).expect("bind");
    srv.spawn(cp);

    let cliente = UnixStream::connect(&path).expect("connect");
    cliente.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    cliente.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut leitor = BufReader::new(cliente.try_clone().unwrap());
    let mut escritor = cliente;

    // Só a partir daqui é que a medição conta: o que interessa é o que **o ataque** provoca,
    // não o custo de arrancar o servidor.
    let base = VIVO.load(Ordering::Relaxed);
    PICO.store(base, Ordering::Relaxed);

    // Um bloco reutilizado: o cliente não pode ser ele próprio a fonte do crescimento.
    let bloco = vec![b'A'; 64 * 1024];
    let mut escrito = 0usize;
    while escrito < ATAQUE {
        match escritor.write_all(&bloco) {
            Ok(()) => escrito += bloco.len(),
            // O daemon fechou — é exatamente o desfecho correto, e não um erro do teste.
            Err(_) => break,
        }
    }
    let _ = escritor.flush();

    let pico = PICO.load(Ordering::Relaxed);
    let crescimento = pico.saturating_sub(base);

    // Eixo 1 — memória. O daemon não pode crescer com o que o atacante escreve.
    assert!(
        crescimento < MAX_LINE + FOLGA,
        "o daemon cresceu {crescimento} bytes ({} KiB) enquanto o cliente escrevia {escrito} \
         bytes sem `\\n`; o limite de linha é {MAX_LINE}. O limite está a ser verificado \
         depois da leitura, não durante.",
        crescimento / 1024,
    );

    // Eixo 2 — a recusa. Tem de chegar, ser uma linha só, e ser bem formada.
    let mut resposta = String::new();
    let n = leitor
        .read_line(&mut resposta)
        .expect("o daemon devia ter recusado; em vez disso não respondeu nada");
    assert!(n > 0, "o daemon fechou sem recusar");
    assert!(
        resposta.len() < 4096,
        "a recusa devia ser limitada, veio com {} bytes",
        resposta.len()
    );

    let j = parse(resposta.trim()).expect("a recusa tem de ser JSON bem formado");
    assert_eq!(
        j.get("ok").cloned(),
        Some(led_daemon_bin::json::Json::Bool(false)),
        "a recusa tem de ser um erro: {resposta}"
    );
    let code = j
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str().map(String::from))
        .expect("erro com código enumerado");
    assert_eq!(code, "bad_request", "código errado na recusa: {resposta}");

    // Eixo 3 — a ligação fecha. O resto da linha gigante **não** pode voltar como pedido.
    let mut sobra = String::new();
    let n = leitor.read_line(&mut sobra).unwrap_or(0);
    assert_eq!(n, 0, "a ligação devia ter fechado; veio mais: {sobra}");

    let _ = std::fs::remove_file(&path);
}
