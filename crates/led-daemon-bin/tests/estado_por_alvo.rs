//! ADR-0029 §8 — **o estado por alvo chega ao operador pelo `status`.**
//!
//! `por_alvo()` existia e não tinha consumidor: a contabilidade de cada nó morria dentro do
//! processo, e o operador via um agregado que **não distingue** cinco nós a funcionar de
//! quatro a funcionar e um morto. Este ficheiro prova o elo que faltava, no fio.
//!
//! **Três alvos, de propósito.** Com **um** alvo, "por nó" e "agregado" são indistinguíveis —
//! foi exactamente assim que duas mutações do passo 1 do ADR-0029 (`all`→`any` e "sondar só o
//! primeiro") não apanharam nada. Um teste que não distingue os dois mundos não prova nenhum
//! (KB-012). Aqui o nó do meio **falha**, e os outros dois não: um relatório que repetisse o
//! agregado daria `frames=2, errors=1` nos três e reprova.

#![cfg(unix)]

use led_daemon_bin::json::{parse, Json};
use led_daemon_bin::output::Alvo;
use led_daemon_bin::server::{ControlPlane, Server, Snapshot};
use led_daemon_bin::{profile_by_name, OutputConfig, OutputManager};
use led_core::{LogicalFrame, PixelColor};
use std::io::{BufRead, BufReader, Write};
use std::net::UdpSocket;
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Cliente mínimo do IPC v1.
///
/// É a mesma forma que o `ipc.rs` usa, e **não** foi extraída para um módulo comum de
/// propósito: fazê-lo obrigaria a editar o `ipc.rs`, que guarda o GS3, numa fatia que é sobre
/// o §8. Vinte linhas duplicadas num teste custam menos que tocar num ficheiro fora do âmbito.
fn status_no_fio(cp: &Arc<ControlPlane>, nome: &str) -> String {
    let path = std::env::temp_dir().join(format!("lumyx-{nome}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let srv = Server::bind(&path).expect("bind");
    srv.spawn(Arc::clone(cp));

    let s = UnixStream::connect(&path).expect("ligar");
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut w = s;
    let mut linha = String::new();

    writeln!(w, r#"{{"v":1,"id":1,"cmd":"hello","client":"teste-por-alvo"}}"#).unwrap();
    w.flush().unwrap();
    r.read_line(&mut linha).unwrap();

    linha.clear();
    writeln!(w, r#"{{"v":1,"id":2,"cmd":"status"}}"#).unwrap();
    w.flush().unwrap();
    r.read_line(&mut linha).unwrap();
    linha.trim().to_string()
}

/// **O nó morto portátil** — ver a doc de `um_no_em_falha_…` em `output.rs`.
///
/// Duplicado aqui, e não extraído: este é um teste de **integração**, logo não alcança os
/// ajudantes de `#[cfg(test)]` do `output.rs`. Extraí-lo obrigaria a pôr o laço na superfície
/// pública do crate — infraestrutura de teste a vazar para produção, que é pior troca.
const ALVO_MORTO: &str = "127.0.0.1:1";

/// Envia até o alvo `indice` acusar erro; devolve quantos envios foram precisos.
///
/// Espera **causal**, nunca `sleep` (TD-003): o erro chega com o ICMP, e o número de envios
/// até lá não é fixo — medido, 2 em Ubuntu e até 9 no runner macOS. Entra em pânico se a
/// premissa não se estabelecer, porque um nó que não morre não pode virar um verde calado.
fn enviar_ate_o_no_morrer(mgr: &OutputManager, pixels: &[PixelColor], indice: usize) -> u32 {
    let prazo = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut enviados = 0u32;
    while std::time::Instant::now() < prazo {
        enviados += 1;
        let _ = mgr.send(&LogicalFrame::new(pixels.to_vec(), 0));
        if mgr.por_alvo()[indice].2 > 0 {
            return enviados;
        }
        std::thread::yield_now();
    }
    panic!(
        "a condição de nó morto NÃO foi estabelecida em {enviados} envios: {ALVO_MORTO} nunca \
         acusou erro. Ou há um ouvinte nessa porta, ou o stack deixou de devolver o erro — nos \
         dois casos este teste deixou de exercitar o ADR-0029 §5.\n  estado: {:?}",
        mgr.por_alvo()
    )
}

/// **A perda é atribuída ao nó certo, e sobrevive até ao fio.**
#[test]
fn o_status_distingue_o_no_morto_dos_que_acenderam() {
    let vivo1 = UdpSocket::bind("127.0.0.1:0").unwrap();
    let vivo2 = UdpSocket::bind("127.0.0.1:0").unwrap();

    let perfil = profile_by_name("esp32-poe-wled-ddp").expect("preset");
    let mut cfg = OutputConfig::resolve(&perfil, "127.0.0.1", 24).expect("resolver");
    cfg.alvos = vec![
        Alvo {
            addr: vivo1.local_addr().unwrap(),
            first_universe: 1,
            pixel_offset: 0,
            pixel_count: 8,
        },
        // O nó morto: porta local sem ouvinte. Ver [`ALVO_MORTO`].
        Alvo {
            addr: ALVO_MORTO.parse().unwrap(),
            first_universe: 1,
            pixel_offset: 8,
            pixel_count: 8,
        },
        Alvo {
            addr: vivo2.local_addr().unwrap(),
            first_universe: 1,
            pixel_offset: 16,
            pixel_count: 8,
        },
    ];

    let mgr = OutputManager::open(cfg)
        .expect("o `connect` a 127.0.0.1:1 passa em Ubuntu e macOS (medido, C0.3); se a \
                 abertura falhou, este teste deixou de exercitar o que afirma");

    let pixels = vec![PixelColor { r: 9, g: 0, b: 0 }; 24];
    let enviados = enviar_ate_o_no_morrer(&mgr, &pixels, 1);

    let por_alvo = mgr.por_alvo();
    assert_eq!(por_alvo.len(), 3, "tres alvos, tres contabilidades: {por_alvo:?}");

    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));
    *cp.snapshot.lock().unwrap() = Snapshot { outputs: por_alvo.clone(), ..Default::default() };

    let linha = status_no_fio(&cp, "por-alvo");
    let j = parse(&linha).unwrap_or_else(|e| panic!("o status tem de ser JSON valido: {e}\n{linha}"));

    let Some(Json::Arr(saidas)) = j.get("outputs") else {
        panic!("o `status` nao traz `outputs` como array — o §8 nao chegou ao fio.\n{linha}");
    };
    assert_eq!(saidas.len(), 3, "tres nos, tres entradas: {linha}");

    // Cada entrada nomeia o SEU endereço — sem nome, cinco robôs mandam procurar em cinco
    // sítios, que é o que o §8 existe para impedir.
    for i in 0..3 {
        let addr = saidas[i].get("addr").and_then(Json::as_str).unwrap_or("<sem addr>");
        assert_eq!(addr, por_alvo[i].0.to_string(), "a entrada {i} nomeia o no errado\n{linha}");
    }

    let campo = |i: usize, k: &str| saidas[i].get(k).and_then(Json::as_u64);

    // **A asserção que mata o agregado repetido.** Se o relatório somasse, os três nós
    // diriam o mesmo par — e o nó morto tornar-se-ia indistinguível dos vivos.
    for i in [0usize, 2] {
        assert_eq!(
            (campo(i, "frames"), campo(i, "errors")),
            (Some(u64::from(enviados)), Some(0)),
            "o no vivo {i} tinha de contar os {enviados} envios sem um unico erro.\
             \n  Se os tres nos disserem o mesmo par, o relatorio esta a repetir o AGREGADO \
             — exactamente o que o §8 proibe.\n  no fio: {linha}"
        );
    }
    let (frames_morto, erros_morto) = (campo(1, "frames"), campo(1, "errors"));
    assert!(
        erros_morto.is_some_and(|e| e > 0)
            && frames_morto.is_some_and(|f| f < u64::from(enviados)),
        "o no morto tinha de acusar erro E perder envios no fio; veio \
         (frames={frames_morto:?}, errors={erros_morto:?}) em {enviados} envios.\n  {linha}"
    );
}

/// **Sem saída, a lista é vazia — nunca zeros fabricados.**
///
/// A alternativa D que o ADR-0029 §8 rejeita é reportar um total somado; a que este teste
/// fecha é a irmã dela — inventar uma entrada com `frames: 0` para um nó que não existe.
/// Ausência de saída e uma saída parada são factos diferentes.
#[test]
fn sem_saida_o_status_nao_inventa_nos() {
    let flag = Arc::new(AtomicBool::new(false));
    let cp = ControlPlane::new(Arc::clone(&flag));

    let linha = status_no_fio(&cp, "sem-saida");
    let j = parse(&linha).expect("JSON valido");

    let Some(Json::Arr(saidas)) = j.get("outputs") else {
        panic!("`outputs` tem de existir sempre, mesmo vazio — um campo ausente e um campo \
                vazio dizem coisas diferentes ao frontend.\n{linha}");
    };
    assert!(saidas.is_empty(), "sem palco aberto nao ha nos: {linha}");
}
