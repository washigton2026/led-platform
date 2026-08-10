//! **Nenhum snapshot observável pode ter um estado inválido.**
//!
//! O `status` do IPC v1 é a única leitura de estado que o control-plane oferece, e o seu
//! campo `state` é consumido como um dos **oito** valores do ADR-0023 — é isso que o
//! contrato TypeScript gerado (ADR-0027) declara ao browser como `DaemonState`.
//!
//! Havia uma janela em que isso não era verdade: `Snapshot::default()` tinha
//! `state: String::default()` — a **string vazia** — e o laço só publica o primeiro
//! instantâneo no fim do primeiro tick (`run.rs`). Entre o `ControlPlane::new` e essa
//! publicação, um `status` devolvia `"state":""`, que **não é** nenhum dos oito.
//!
//! O teste é **determinístico**: não corre o laço, não dorme e não depende de ganhar uma
//! corrida. Olha para o instantâneo que o control-plane publica **antes** de qualquer tick —
//! que é exatamente o que a janela expunha.

#![cfg(unix)]

use led_daemon::State;
use led_daemon_bin::server::ControlPlane;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Os oito estados do ADR-0023, como aparecem no fio.
fn estados_validos() -> Vec<&'static str> {
    State::ALL.iter().map(|s| s.as_str()).collect()
}

/// **O instantâneo inicial já é um estado válido.**
///
/// Não é sobre estética: o consumidor lê este campo como uma união fechada de oito valores.
/// Um nono valor — e a string vazia é um nono valor — cai fora do contrato, e o browser
/// recebe algo que o seu tipo diz ser impossível.
#[test]
fn o_instantaneo_inicial_nunca_tem_estado_invalido() {
    let cp = ControlPlane::new(Arc::new(AtomicBool::new(false)));
    let s = cp.snapshot.lock().expect("snapshot").clone();
    let no_fio = s.state.as_str();

    assert!(
        !no_fio.is_empty(),
        "o instantaneo publicado antes do primeiro tick tem `state` VAZIO — e a string \
         vazia nao e nenhum dos oito estados do ADR-0023. Um consumidor que trate `state` \
         como uniao fechada recebe um valor que o seu tipo diz ser impossivel."
    );
    assert!(
        estados_validos().contains(&no_fio),
        "`state` = {no_fio:?} nao esta entre os oito estados validos: {:?}",
        estados_validos()
    );
}

/// **E é `idle`** — o estado em que `ShowRuntime::new()` realmente começa.
///
/// Isto não é fabricar uma medição: é o estado inicial **contratual** da máquina
/// (ADR-0023). O que se corrigiu foi o tipo poder representar um valor que a máquina nunca
/// tem; não se passou a afirmar nada que o runtime não afirme.
#[test]
fn o_estado_inicial_e_o_mesmo_com_que_o_runtime_comeca() {
    let cp = ControlPlane::new(Arc::new(AtomicBool::new(false)));
    let s = cp.snapshot.lock().expect("snapshot").clone();
    assert_eq!(
        s.state.as_str(),
        State::Idle.as_str(),
        "o instantaneo inicial tem de coincidir com o estado inicial do ShowRuntime"
    );
    // E o resto do instantâneo não inventa progresso nenhum.
    assert_eq!(s.position_ms, 0);
    assert_eq!(s.ticks, 0);
    assert_eq!(s.show_id, None, "sem show carregado, `show_id` e null — nunca 0");
}

/// **Todo estado do runtime tem representação no fio, e nenhuma é vazia.**
///
/// Percorre os oito: se algum dia um deles serializar para `""`, é aqui que fica vermelho.
#[test]
fn os_oito_estados_tem_representacao_nao_vazia() {
    for e in State::ALL {
        assert!(!e.as_str().is_empty(), "{e:?} serializa para string vazia");
    }
    let unicos: std::collections::BTreeSet<_> = State::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(unicos.len(), State::ALL.len(), "dois estados partilham a mesma string");
}
