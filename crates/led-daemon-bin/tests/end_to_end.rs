//! Percurso completo do processo, **sem processo**: `.lumyx` em disco → descritor → laço.
//!
//! Os testes de unidade cobrem o laço (com pacer virtual) e o loader (sobre um buffer). Falta
//! o que o `main` realmente faz: ler de um **caminho de ficheiro** e alimentar o laço com o
//! resultado. É aqui que um erro de cablagem entre os dois apareceria.

use led_core::PixelColor;
use led_daemon::{ShowId, ShowRuntime, State};
use led_daemon_bin::{descriptor_from_path, run, Config, ExitReason, Integrity, Journal, Pacer};
use led_show_recorder::{ShowRecord, ShowWriter};
use std::sync::atomic::AtomicBool;

/// Pacer virtual — repetido aqui porque o de `src` é `#[cfg(test)]` e não atravessa a
/// fronteira do crate. Manter os testes de integração livres de relógio de parede vale a
/// duplicação de dez linhas.
struct VPacer {
    now: u64,
    sleeps: usize,
}
impl Pacer for VPacer {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn sleep_until(&mut self, deadline_ms: u64) {
        self.sleeps += 1;
        self.now = self.now.max(deadline_ms);
    }
}

/// Escreve um `.lumyx` real num ficheiro temporário e devolve o caminho.
fn escrever_show(nome: &str, frames: u32, passo_ms: u64, pixels: u32) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(nome);
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ShowWriter::new(f, pixels).unwrap();
    for i in 0..frames {
        w.write_frame(&ShowRecord {
            timestamp_ms: i as u64 * passo_ms,
            pixels: vec![PixelColor { r: i as u8, g: 0, b: 0 }; pixels as usize],
            audio: None,
        })
        .unwrap();
    }
    w.flush().unwrap();
    path
}

#[test]
fn do_ficheiro_ao_fim_do_show() {
    let path = escrever_show("lumyx_gs2_e2e.lumyx", 10, 25, 8);
    let desc = descriptor_from_path(path.to_str().unwrap(), ShowId(7)).expect("carregar");

    assert_eq!(desc.id, ShowId(7));
    assert_eq!(desc.frame_count, 10);
    assert_eq!(desc.pixel_count, 8);
    assert_eq!(desc.duration_ms, 225, "9 intervalos de 25 ms");

    let cfg = Config {
        tick_ms: 50,
        max_ticks: None,
        autoplay: true,
        exit_on_finish: true,
        integrity: Integrity::AssumedByOperator,
        output: Vec::new(),
        profile: None,
    };
    let mut rt = ShowRuntime::new();
    let mut p = VPacer { now: 0, sleeps: 0 };
    let mut buf = Vec::new();
    let flag = AtomicBool::new(false);
    let out = {
        let mut j = Journal::new(&mut buf);
        run(&mut rt, path.to_str().unwrap(), desc, &cfg, &mut p, &mut j, &flag)
    };

    assert_eq!(out.reason, ExitReason::ReachedEnd);
    assert_eq!(out.final_state, State::Finished);
    assert_eq!(out.final_position_ms, 225, "pára exatamente na duração, nunca a excede");
    assert_eq!(out.ticks, 5, "225 ms / 50 ms por tick, arredondando para cima");
    assert_eq!(p.sleeps, out.ticks as usize, "uma espera por tick — sem busy-loop");

    let log = String::from_utf8(buf).unwrap();
    assert!(log.contains(r#""event":"show_loaded","show_id":7"#), "{log}");
    assert!(log.contains(r#""event":"reached_end""#), "{log}");
    // Toda linha do journal é um objeto JSON completo, uma por linha.
    for l in log.lines() {
        assert!(l.starts_with('{') && l.ends_with('}'), "linha malformada: {l}");
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn ficheiro_que_nao_e_lumyx_e_recusado_com_erro_legivel() {
    let path = std::env::temp_dir().join("lumyx_gs2_lixo.bin");
    std::fs::write(&path, b"isto nao e um show").unwrap();
    let e = descriptor_from_path(path.to_str().unwrap(), ShowId(1)).unwrap_err();
    let msg = format!("{e}");
    assert!(msg.contains("magic"), "o erro tem de dizer o que está errado: {msg}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn caminho_inexistente_falha_sem_panico() {
    assert!(descriptor_from_path("/nao/existe/de/todo.lumyx", ShowId(1)).is_err());
}
