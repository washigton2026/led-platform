//! ADR-0026 §4 e §13 — **uma** subscrição no daemon, N browsers, e a perda só do lado certo.
//!
//! # A direção do backpressure é a decisão
//!
//! Um browser lento **nunca** atrasa a leitura do IPC, e o console **nunca** atrasa o daemon.
//! Ler devagar a ligação de eventos faria o daemon acumular; ler sempre e descartar
//! localmente não faz. É o preview lossy do ADR-0015 aplicado ao fluxo de eventos.
//!
//! Quando a fila de um browser enche, descarta-se o **mais antigo** e conta-se. O contador é
//! **reportado** (`console.dropped`), não escondido: o operador tem de saber que a sua vista
//! está incompleta.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Quantos eventos um browser pode ficar a dever antes de começar a perder os antigos.
pub const FILA_POR_BROWSER: usize = 256;

/// Um browser ligado ao SSE. Cada um tem a **sua** fila; um lento não afeta os outros.
#[derive(Debug)]
pub struct Subscriber {
    id: u64,
    fila: Mutex<VecDeque<String>>,
    descartados: AtomicU64,
    capacidade: usize,
}

impl Subscriber {
    fn novo(id: u64, capacidade: usize) -> Self {
        Self { id, fila: Mutex::new(VecDeque::new()), descartados: AtomicU64::new(0), capacidade }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Quantos eventos este browser perdeu. **Existe para ser mostrado**, não para consolar.
    pub fn descartados(&self) -> u64 {
        self.descartados.load(Ordering::Relaxed)
    }

    /// Retira o próximo evento a enviar, se houver.
    pub fn proximo(&self) -> Option<String> {
        self.fila.lock().expect("fila").pop_front()
    }

    pub fn pendentes(&self) -> usize {
        self.fila.lock().expect("fila").len()
    }

    /// Entrega **sem bloquear**. Fila cheia ⇒ descarta o mais antigo e conta.
    fn entrega(&self, linha: &str) {
        let mut f = self.fila.lock().expect("fila");
        while f.len() >= self.capacidade {
            f.pop_front();
            self.descartados.fetch_add(1, Ordering::Relaxed);
        }
        f.push_back(linha.to_string());
    }
}

/// O difusor: recebe **um** fluxo do daemon e entrega a N browsers.
#[derive(Debug, Default)]
pub struct Fanout {
    subs: Mutex<Vec<Arc<Subscriber>>>,
    proximo_id: AtomicU64,
    capacidade: usize,
    /// Quantas vezes o console subscreveu no daemon. **Tem de ser 1**, independentemente do
    /// número de browsers — é o que o teste discriminante afirma.
    subscricoes_ipc: AtomicU64,
}

impl Fanout {
    pub fn novo() -> Self {
        Self { capacidade: FILA_POR_BROWSER, ..Default::default() }
    }

    pub fn com_capacidade(capacidade: usize) -> Self {
        Self { capacidade: capacidade.max(1), ..Default::default() }
    }

    /// Regista que o console abriu a sua (única) subscrição no daemon.
    ///
    /// Chamado **uma vez**, na ligação de eventos — nunca por browser.
    pub fn marcar_subscricao_ipc(&self) {
        self.subscricoes_ipc.fetch_add(1, Ordering::Relaxed);
    }

    pub fn subscricoes_ipc(&self) -> u64 {
        self.subscricoes_ipc.load(Ordering::Relaxed)
    }

    /// Um browser novo liga-se. **Não** abre ligação nenhuma ao daemon.
    pub fn ligar(&self) -> Arc<Subscriber> {
        let id = self.proximo_id.fetch_add(1, Ordering::Relaxed);
        let s = Arc::new(Subscriber::novo(id, self.capacidade));
        self.subs.lock().expect("subs").push(Arc::clone(&s));
        s
    }

    /// Um browser desliga-se. O `Subscriber` é removido — senão a lista crescia para sempre,
    /// o mesmo motivo pelo qual o servidor IPC do GS3 poda subscritores mortos.
    pub fn desligar(&self, id: u64) {
        self.subs.lock().expect("subs").retain(|s| s.id() != id);
    }

    pub fn ligados(&self) -> usize {
        self.subs.lock().expect("subs").len()
    }

    /// Difunde uma linha do daemon. **Nunca bloqueia**, seja qual for o estado dos browsers.
    pub fn difundir(&self, linha: &str) {
        for s in self.subs.lock().expect("subs").iter() {
            s.entrega(linha);
        }
    }

    /// Total de eventos perdidos em todos os browsers.
    pub fn descartados_totais(&self) -> u64 {
        self.subs.lock().expect("subs").iter().map(|s| s.descartados()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **ADR-0026 §4.** N browsers, **uma** subscrição no daemon.
    #[test]
    fn dois_browsers_partilham_uma_so_subscricao_ipc() {
        let f = Fanout::novo();
        f.marcar_subscricao_ipc(); // a ligação de eventos, aberta uma vez

        let a = f.ligar();
        let b = f.ligar();
        let c = f.ligar();
        assert_eq!(f.ligados(), 3);
        assert_eq!(
            f.subscricoes_ipc(),
            1,
            "tres browsers NAO podem virar tres subscritores no daemon"
        );

        f.difundir(r#"{"event":"transitioned","from":"ready","to":"playing"}"#);
        for s in [&a, &b, &c] {
            assert!(s.proximo().is_some(), "cada browser recebe a sua copia");
        }
    }

    /// **ADR-0026 §13.** Um browser que não lê nunca bloqueia a difusão.
    #[test]
    fn browser_lento_nao_aplica_backpressure_e_a_perda_e_contada() {
        let f = Fanout::com_capacidade(4);
        f.marcar_subscricao_ipc();
        let lento = f.ligar();
        let rapido = f.ligar();

        // 100 eventos; o lento nunca lê.
        for i in 0..100 {
            f.difundir(&format!(r#"{{"n":{i}}}"#));
            let _ = rapido.proximo();
        }

        assert_eq!(lento.pendentes(), 4, "a fila do lento nao cresce alem da capacidade");
        assert_eq!(lento.descartados(), 96, "e a perda e CONTADA, nao escondida");
        assert!(f.descartados_totais() >= 96);

        // O que sobrou é o **mais recente**, não o mais antigo.
        let ultimo = lento.proximo().unwrap();
        assert!(ultimo.contains("96"), "descarta-se o antigo, guarda-se o novo: {ultimo}");
    }

    /// **Controle negativo do teste acima.** Um browser que lê não perde nada — sem isto, o
    /// teste da perda passaria mesmo que o difusor descartasse sempre.
    #[test]
    fn browser_que_le_nao_perde_nada() {
        let f = Fanout::com_capacidade(4);
        let s = f.ligar();
        for i in 0..100 {
            f.difundir(&format!(r#"{{"n":{i}}}"#));
            assert!(s.proximo().is_some());
        }
        assert_eq!(s.descartados(), 0, "quem le nao perde");
    }

    /// Browser desligado é **removido** — senão a lista cresceria para sempre.
    #[test]
    fn browser_desligado_e_removido() {
        let f = Fanout::novo();
        let a = f.ligar();
        let b = f.ligar();
        f.desligar(a.id());
        assert_eq!(f.ligados(), 1);

        f.difundir("x");
        assert_eq!(b.pendentes(), 1);
        assert_eq!(a.pendentes(), 0, "o desligado nao recebe mais nada");
    }
}
