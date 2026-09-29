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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    ///
    /// Devolve **quantos** descartou nesta entrega, para o [`Fanout`] os somar ao contador
    /// global (ADR-0026 §13-bis).
    fn entrega(&self, linha: &str) -> u64 {
        let mut f = self.fila.lock().expect("fila");
        let mut descartou = 0;
        while f.len() >= self.capacidade {
            f.pop_front();
            self.descartados.fetch_add(1, Ordering::Relaxed);
            descartou += 1;
        }
        f.push_back(linha.to_string());
        descartou
    }
}

/// O difusor: recebe **um** fluxo do daemon e entrega a N browsers.
#[derive(Debug, Default)]
pub struct Fanout {
    subs: Mutex<Vec<Arc<Subscriber>>>,
    proximo_id: AtomicU64,
    capacidade: usize,
    /// Quantas vezes o console subscreveu no daemon **desde sempre**. Cumulativo: numa
    /// reconexão legítima cresce, porque houve de facto uma subscrição nova.
    subscricoes_ipc: AtomicU64,
    /// A **reivindicação**: alguém detém o direito de ser o subscritor. Privada de propósito
    /// — serve a exclusão mútua, e não é o que o mundo lá fora quer saber.
    reivindicada: AtomicBool,
    /// A subscrição está **estabelecida** (o `subscribe` foi aceite)? É este o medidor
    /// público.
    ///
    /// Os dois não são o mesmo, e confundi-los custou um teste intermitente: entre
    /// reivindicar e o daemon aceitar há uma janela, e durante ela dizer "viva" é dizer que
    /// há um fluxo que ainda não existe. Quem observar `subscricao_viva()` e difundir um
    /// evento nessa janela perde-o.
    estabelecida: AtomicBool,
    /// Tentativas de ligação, com ou sem sucesso. Torna o backoff **observável**.
    tentativas: AtomicU64,
    /// `console.dropped` (ADR-0026 §13-bis): eventos descartados **em todos os browsers**,
    /// desde o arranque desta instância. Só cresce — **não** desce quando um browser se
    /// desliga, ao contrário de [`Fanout::descartados_totais`], que soma só os ligados.
    descartados_desde_arranque: AtomicU64,
    /// O arranque desta instância do console, em ms Unix. Fixado **uma vez**, na construção.
    /// É o que o cliente usa para detetar um reinício (§13-bis) — nunca a direção do contador.
    arranque_ms: u64,
}

/// Agora, em milissegundos Unix. Um relógio antes de 1970 é um relógio partido, e dá 0.
fn agora_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A prova de posse da subscrição upstream. Enquanto existir, `subscricao_viva()` é `true`.
///
/// O `Drop` liberta — e é isso que faz o invariante sobreviver a `break`, a `?` e a `panic!`.
#[derive(Debug)]
pub struct GuardaSubscricao<'a> {
    fanout: &'a Fanout,
}

impl GuardaSubscricao<'_> {
    /// O daemon **aceitou** o `subscribe`. Só a partir daqui existe fluxo, e só a partir
    /// daqui `subscricao_viva()` diz `true`.
    pub fn estabelecida(&self) {
        self.fanout.subscricoes_ipc.fetch_add(1, Ordering::Relaxed);
        self.fanout.estabelecida.store(true, Ordering::Release);
    }
}

impl Drop for GuardaSubscricao<'_> {
    fn drop(&mut self) {
        // A ordem importa: primeiro deixa de haver fluxo, só depois se abre a porta a outro
        // subscritor. Ao contrário, haveria um instante em que dois se julgariam vivos.
        self.fanout.estabelecida.store(false, Ordering::Release);
        self.fanout.reivindicada.store(false, Ordering::Release);
    }
}

impl Fanout {
    pub fn novo() -> Self {
        Self::com_arranque(FILA_POR_BROWSER, agora_unix_ms())
    }

    pub fn com_capacidade(capacidade: usize) -> Self {
        Self::com_arranque(capacidade, agora_unix_ms())
    }

    /// Com o instante de arranque **explícito** — para os testes poderem simular um reinício
    /// sem depender de o relógio avançar um milissegundo.
    pub fn com_arranque(capacidade: usize, arranque_ms: u64) -> Self {
        Self { capacidade: capacidade.max(1), arranque_ms, ..Default::default() }
    }

    /// `console.dropped` (ADR-0026 §13-bis): descartes em todos os browsers desde o arranque.
    /// Monotónico dentro de uma instância.
    pub fn descartados_desde_arranque(&self) -> u64 {
        self.descartados_desde_arranque.load(Ordering::Relaxed)
    }

    /// O arranque desta instância do console, em ms Unix (`since` em `/api/dropped`).
    pub fn arranque_ms(&self) -> u64 {
        self.arranque_ms
    }

    /// Regista que uma subscrição foi **efetivamente estabelecida** — depois de o
    /// `subscribe` ter sido aceite, nunca antes.
    ///
    /// A distinção não é cosmética: contar no momento em que a subscrição é *reivindicada*
    /// faria cada tentativa falhada contar como subscrição, e um console contra um daemon
    /// ausente reportaria subscrições que nunca existiram. Uma reivindicação não é uma
    /// subscrição — foi um teste da F5 que apanhou exatamente isso.
    ///
    /// Chamado só pelo supervisor — nunca por browser.
    pub fn marcar_subscricao_ipc(&self) {
        self.subscricoes_ipc.fetch_add(1, Ordering::Relaxed);
    }

    /// Quantas subscrições houve **desde sempre**. É **cumulativo**, não um medidor: numa
    /// reconexão legítima cresce, porque houve de facto uma subscrição nova.
    ///
    /// Para saber se existe uma **agora**, use [`Fanout::subscricao_viva`]. Confundir os dois
    /// é fácil e caro, e foi por isso que este parágrafo existe.
    pub fn subscricoes_ipc(&self) -> u64 {
        self.subscricoes_ipc.load(Ordering::Relaxed)
    }

    /// **Existe uma subscrição upstream neste momento?** `false` enquanto o daemon está em
    /// baixo — que é a verdade, e não um zero fabricado.
    pub fn subscricao_viva(&self) -> bool {
        self.estabelecida.load(Ordering::Acquire)
    }

    /// `1` se existe subscrição upstream agora, `0` se não. É o **medidor** em forma de
    /// número, para os testes poderem afirmar "nunca passa de um" sem traduzir booleanos.
    ///
    /// Não pode devolver 2: o valor vem de um `bool`, e é isso que torna o invariante uma
    /// propriedade do **tipo** em vez de uma esperança.
    pub fn subscricoes_simultaneas(&self) -> u64 {
        u64::from(self.subscricao_viva())
    }

    /// Quantas **tentativas** de ligação o supervisor já fez. Existe para o backoff ser
    /// observável: sem isto, um busy-loop e um backoff correto são indistinguíveis de fora.
    pub fn tentativas_de_ligacao(&self) -> u64 {
        self.tentativas.load(Ordering::Relaxed)
    }

    /// Conta uma tentativa de ligação, tenha ela sucesso ou não.
    pub fn marcar_tentativa(&self) {
        self.tentativas.fetch_add(1, Ordering::Relaxed);
    }

    /// **Reivindica** a subscrição upstream. Devolve `None` se já houver uma viva.
    ///
    /// É isto que torna "duas subscrições ao mesmo tempo" **não representável**, em vez de
    /// proibido por convenção: quem não conseguir a reivindicação não tem por onde abrir a
    /// segunda. O `compare_exchange` é a exclusão; o `Drop` da guarda é a libertação.
    ///
    /// Mesma disciplina do resto do projeto: quando dá, garantir por construção em vez de
    /// verificar em runtime.
    pub fn reivindicar_subscricao(&self) -> Option<GuardaSubscricao<'_>> {
        self.reivindicada
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| GuardaSubscricao { fanout: self })
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
        let mut descartou = 0;
        for s in self.subs.lock().expect("subs").iter() {
            descartou += s.entrega(linha);
        }
        if descartou > 0 {
            self.descartados_desde_arranque.fetch_add(descartou, Ordering::Relaxed);
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
    /// **ADR-0026 §13-bis.** `console.dropped` conta os descartes de TODOS os browsers.
    #[test]
    fn console_dropped_conta_os_descartes_de_todos_os_browsers() {
        let f = Fanout::com_capacidade(4);
        let _a = f.ligar();
        let _b = f.ligar();
        assert_eq!(f.descartados_desde_arranque(), 0, "sem perdas, zero — nao um valor fabricado");
        for i in 0..100 {
            f.difundir(&format!(r#"{{"n":{i}}}"#));
        }
        // Dois browsers parados, capacidade 4: cada um perde 96.
        assert_eq!(f.descartados_desde_arranque(), 192, "a perda de CADA browser entra no total");
    }

    /// **ADR-0026 §13-bis.** Monotónico dentro da instância: um browser que se desliga leva
    /// consigo o seu contador, mas as perdas dele **não** saem de `console.dropped`.
    #[test]
    fn console_dropped_nao_desce_quando_o_browser_se_desliga() {
        let f = Fanout::com_capacidade(4);
        let lento = f.ligar();
        for i in 0..100 {
            f.difundir(&format!(r#"{{"n":{i}}}"#));
        }
        assert_eq!(f.descartados_desde_arranque(), 96);
        f.desligar(lento.id());
        assert_eq!(f.descartados_totais(), 0, "a soma dos LIGADOS desce — e por isso nao serve");
        assert_eq!(
            f.descartados_desde_arranque(),
            96,
            "console.dropped DESCEU quando o browser saiu — deixou de ser monotonico"
        );
    }

    /// **ADR-0026 §13-bis.** `since` é fixado na construção e não muda com a atividade.
    #[test]
    fn o_arranque_e_fixado_uma_vez() {
        let f = Fanout::com_arranque(4, 1_790_000_000_000);
        let _a = f.ligar();
        for i in 0..50 {
            f.difundir(&format!(r#"{{"n":{i}}}"#));
        }
        assert_eq!(f.arranque_ms(), 1_790_000_000_000);
    }

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
