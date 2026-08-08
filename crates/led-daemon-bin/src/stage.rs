//! GS4.2 — **o palco**: o único sítio do daemon que põe bytes no fio.
//!
//! ## Um só caminho de saída
//!
//! `run` e `run_with_control` chamam [`Stage::on_tick`] e mais nada. Não há um caminho para o
//! modo com IPC e outro para o modo sem — dois caminhos seriam duas coisas para divergir, e a
//! que divergisse seria a que ninguém testa. O heartbeat também passa por aqui: ele **não**
//! corre numa thread própria, porque uma segunda thread a enviar seria exatamente o caminho
//! paralelo que se quer evitar (e faria o determinismo do laço depender do escalonador).
//!
//! ## Transporte não apaga o palco
//!
//! Em `Playing` sai o quadro da posição. Em `Paused`/`Stopped`/`Finished`/`Ready` sai o
//! **último quadro válido**, reenviado a cada [`HEARTBEAT_MS`] — nunca zeros. É a decisão 3
//! do ADR-0023 (*"Stop/Pause NÃO apagam o palco"*) e o invariante do heartbeat do
//! `LUMYX_GOSL`, e aqui são a mesma linha de código.
//!
//! ## Falhar, registar e prosseguir
//!
//! Um erro de envio **não** derruba o laço: é contado em [`OutputStats`], devolvido ao
//! chamador e escrito no journal. A regra de degradação segura do `control-protocol.md` diz
//! que o show continua — o que é proibido é falhar em **silêncio**.

use crate::loader::LoadError;
use crate::output::{OutputConfig, OutputManager};
use crate::source::FrameSource;
use led_daemon::State;
use led_hal::Heartbeat;

/// Período do keep-alive, em ms.
///
/// **Não é um número novo.** É o mesmo do `LUMYX_GOSL` e o mesmo que o `led-protocols` já
/// usava; há um teste que falha se as duas fontes divergirem, em vez de as deixar apodrecer
/// em paralelo. Um `HardwareProfile` pode declarar o seu (`Transport::heartbeat_ms`) — este
/// é o valor quando não há profile.
pub const HEARTBEAT_MS: u64 = led_protocols::HEARTBEAT_MS;

/// O que o tick pôs (ou não pôs) no fio. **Observável de propósito**: sem isto, "não enviou
/// nada" e "enviou e falhou" seriam indistinguíveis no journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageTick {
    /// Saiu o quadro do show, na posição pedida.
    Sent { position_ms: u64 },
    /// Saiu o **último quadro válido**, reenviado pelo keep-alive.
    Held,
    /// Nada saiu, e está certo: ou não é hora do keep-alive, ou ainda não houve 1.º quadro.
    Quiet,
    /// O envio falhou. O laço **continua**; o erro fica contado e registado.
    Failed(String),
}

/// A saída do daemon, do `.lumyx` ao datagrama.
pub struct Stage {
    source: FrameSource,
    output: OutputManager,
    heartbeat: Heartbeat,
    /// Instante do último envio bem-sucedido, no relógio **injetado** do laço.
    last_sent_ms: Option<u64>,
}

impl Stage {
    /// Abre a fonte e a saída para um show concreto.
    ///
    /// O `pixel_count` vem do **show**, não da CLI: a saída é dimensionada pelo artefato que
    /// vai tocar, e um número escrito à mão seria mais uma oportunidade de discordar dele.
    /// `profile` **não é opcional**: sem ele o daemon teria de adivinhar ordem de canais,
    /// universos e MTU — e adivinhar errado acende a cor errada na fita.
    pub fn open(
        show_path: &str,
        output: &str,
        profile: &led_hardware_profile::HardwareProfile,
    ) -> Result<Self, String> {
        let source = FrameSource::open(show_path).map_err(|e: LoadError| e.to_string())?;
        let cfg = OutputConfig::resolve(profile, output, source.pixel_count as usize, 1)?;
        let output = OutputManager::open(cfg).map_err(|e| format!("saída: {e}"))?;
        Ok(Self { source, output, heartbeat: Heartbeat::new(), last_sent_ms: None })
    }

    pub fn output(&self) -> &OutputManager {
        &self.output
    }

    /// **Chamado uma vez por tick, pelo laço.** É a única entrada de dados para o fio.
    pub fn on_tick(&mut self, state: State, position_ms: u64, now_ms: u64) -> StageTick {
        if state == State::Playing {
            match self.source.frame_at(position_ms) {
                Ok(Some(frame)) => {
                    // Registar **antes** de enviar: um quadro que o show produziu é o último
                    // válido mesmo que o envio falhe — senão uma falha de rede transitória
                    // faria o keep-alive regredir para um quadro mais antigo.
                    self.heartbeat.record(&frame);
                    return match self.output.send(&frame) {
                        Ok(()) => {
                            self.last_sent_ms = Some(now_ms);
                            StageTick::Sent { position_ms }
                        }
                        Err(e) => StageTick::Failed(format!("{e:?}")),
                    };
                }
                Ok(None) => {} // show sem quadros: cai no keep-alive
                Err(e) => return StageTick::Failed(format!("fonte: {e}")),
            }
        }

        // Fora de `Playing` — e também num `Playing` sem quadro — mantém o palco vivo.
        let devido = match self.last_sent_ms {
            Some(t) => now_ms.saturating_sub(t) >= HEARTBEAT_MS,
            None => false, // nunca houve quadro válido: não há o que reenviar
        };
        if !devido {
            return StageTick::Quiet;
        }
        match self.heartbeat.beat(&self.output) {
            Ok(true) => {
                self.last_sent_ms = Some(now_ms);
                StageTick::Held
            }
            Ok(false) => StageTick::Quiet, // NUNCA fabrica um quadro preto
            Err(e) => StageTick::Failed(format!("{e:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::PixelColor;
    use led_show_recorder::{ShowRecord, ShowWriter};
    use std::net::UdpSocket;

    fn escrever(nome: &str, frames: &[(u64, u8)], px: u32) -> String {
        let path = std::env::temp_dir().join(nome);
        let f = std::fs::File::create(&path).unwrap();
        let mut w = ShowWriter::new(f, px).unwrap();
        for &(ts, v) in frames {
            w.write_frame(&ShowRecord {
                timestamp_ms: ts,
                pixels: vec![PixelColor { r: v, g: 0, b: 0 }; px as usize],
                audio: None,
            })
            .unwrap();
        }
        w.flush().unwrap();
        path.to_str().unwrap().to_string()
    }

    fn palco(nome: &str, frames: &[(u64, u8)]) -> (Stage, UdpSocket, String) {
        let path = escrever(nome, frames, 4);
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_millis(200))).unwrap();
        let perfil = crate::output::profile_by_name("esp32-poe-wled-ddp").unwrap();
        let st =
            Stage::open(&path, &sock.local_addr().unwrap().to_string(), &perfil).unwrap();
        (st, sock, path)
    }

    /// Recebe tudo o que estiver no socket e devolve quantos datagramas eram.
    fn drenar(sock: &UdpSocket) -> usize {
        let mut n = 0;
        let mut buf = [0u8; 2048];
        while sock.recv(&mut buf).is_ok() {
            n += 1;
        }
        n
    }

    /// **Uma só verdade sobre o período do keep-alive.** Se alguém mudar um dos números sem
    /// mudar o outro, é aqui que se sabe — não no palco.
    #[test]
    fn o_periodo_do_keepalive_concorda_com_as_outras_fontes_do_repo() {
        assert_eq!(HEARTBEAT_MS, led_protocols::HEARTBEAT_MS, "led-protocols");
        assert_eq!(HEARTBEAT_MS, 800, "LUMYX_GOSL");
        assert!(
            HEARTBEAT_MS < led_hardware_profile::Transport::MAX_GAP_MS as u64,
            "o período tem de caber com folga no teto de 2400 ms"
        );
    }

    #[test]
    fn em_playing_o_quadro_da_posicao_sai_no_fio() {
        let (mut st, sock, p) = palco("stage_a.lumyx", &[(0, 7), (100, 9)]);
        assert_eq!(st.on_tick(State::Playing, 0, 0), StageTick::Sent { position_ms: 0 });
        assert_eq!(st.on_tick(State::Playing, 150, 20), StageTick::Sent { position_ms: 150 });
        assert_eq!(drenar(&sock), 2, "dois ticks a tocar, dois datagramas");
        assert_eq!(st.output().stats().frames(), 2);
        assert_eq!(st.output().stats().errors(), 0);
        let _ = std::fs::remove_file(p);
    }

    /// **Pausar não apaga o palco** (ADR-0023 §3): o último quadro continua a ser reenviado.
    #[test]
    fn pausado_o_palco_continua_vivo_e_nunca_recebe_zeros() {
        let (mut st, sock, p) = palco("stage_b.lumyx", &[(0, 200)]);
        st.on_tick(State::Playing, 0, 0);
        let mut tocado = [0u8; 2048];
        let n_tocado = sock.recv(&mut tocado).expect("o tick a tocar envia");
        drenar(&sock);

        // Antes de vencer o período do keep-alive, silêncio é o correto.
        assert_eq!(st.on_tick(State::Paused, 0, 100), StageTick::Quiet);
        assert_eq!(drenar(&sock), 0);

        // Vencido o período, o ÚLTIMO QUADRO VÁLIDO volta ao fio.
        assert_eq!(st.on_tick(State::Paused, 0, HEARTBEAT_MS), StageTick::Held);
        let mut buf = [0u8; 2048];
        let n = sock.recv(&mut buf).expect("o keep-alive tem de enviar");

        // **Comparar com o que foi tocado, não com um literal.** A versão anterior procurava
        // o byte `200` — o valor lógico do quadro — e passou a falhar quando a calibração do
        // preset (gamma 2.2, ADR-0019 Emenda 1) o converteu em 152 no fio. O invariante nunca
        // foi "o byte 200 aparece": é "sai o mesmo quadro que estava a tocar, e não zeros".
        // Comparar o payload com o do tick anterior afirma exatamente isso, e continua a
        // valer seja qual for a calibração do nó.
        assert_eq!(
            &buf[10..n],
            &tocado[10..n_tocado],
            "o keep-alive tem de reenviar o MESMO quadro que estava no fio"
        );
        assert!(
            buf[10..n].iter().any(|&b| b != 0),
            "o keep-alive NUNCA envia zeros — apagaria o palco"
        );
        let _ = std::fs::remove_file(p);
    }

    /// O mesmo vale para `Stopped` e `Finished` — transporte não é saída.
    #[test]
    fn parado_e_terminado_tambem_mantem_o_palco() {
        for estado in [State::Stopped, State::Finished, State::Ready] {
            let (mut st, sock, p) = palco("stage_c.lumyx", &[(0, 42)]);
            st.on_tick(State::Playing, 0, 0);
            drenar(&sock);
            assert_eq!(st.on_tick(estado, 0, HEARTBEAT_MS), StageTick::Held, "{estado:?}");
            assert_eq!(drenar(&sock), 1, "{estado:?}");
            let _ = std::fs::remove_file(p);
        }
    }

    /// **O gap nunca excede o limite duro.** Simula 10 s parado e mede o maior intervalo.
    #[test]
    fn o_intervalo_entre_envios_nunca_passa_do_limite_do_gosl() {
        const CRIT_GAP_MS: u64 = 2_400;
        let (mut st, sock, p) = palco("stage_d.lumyx", &[(0, 5)]);
        st.on_tick(State::Playing, 0, 0);
        let mut ultimo = 0u64;
        let mut maior = 0u64;
        for tick in 1..=500u64 {
            let now = tick * 20; // laço a 50 Hz, 10 s
            if st.on_tick(State::Paused, 0, now) == StageTick::Held {
                maior = maior.max(now - ultimo);
                ultimo = now;
            }
        }
        assert!(maior > 0, "houve keep-alive");
        assert!(maior < CRIT_GAP_MS, "maior intervalo {maior} ms >= limite {CRIT_GAP_MS} ms");
        drenar(&sock);
        let _ = std::fs::remove_file(p);
    }

    /// **Sem primeiro quadro não há keep-alive** — e sobretudo não há quadro preto fabricado.
    #[test]
    fn antes_do_primeiro_quadro_nada_sai() {
        let (mut st, sock, p) = palco("stage_e.lumyx", &[(0, 1)]);
        for t in [0, 800, 5_000, 100_000] {
            assert_eq!(st.on_tick(State::Ready, 0, t), StageTick::Quiet);
        }
        assert_eq!(drenar(&sock), 0, "nunca se inventa um quadro para um palco que não tocou");
        let _ = std::fs::remove_file(p);
    }

    /// Erro de envio é **contado e devolvido**, e o palco continua utilizável.
    #[test]
    fn erro_de_envio_nao_derruba_o_palco() {
        let path = escrever("stage_f.lumyx", &[(0, 3)], 4);
        // Porta 1 em loopback: `sendto` falha de forma reprodutível em macOS/Linux.
        let perfil = crate::output::profile_by_name("esp32-poe-wled-ddp").unwrap();
        let mut st = Stage::open(&path, "127.0.0.1:1", &perfil).unwrap();
        let r = st.on_tick(State::Playing, 0, 0);
        assert!(matches!(r, StageTick::Failed(_)) || matches!(r, StageTick::Sent { .. }));
        // Seja qual for o veredito do SO, a contabilidade fecha: nada desaparece.
        assert_eq!(st.output().stats().frames() + st.output().stats().errors(), 1);
        let _ = std::fs::remove_file(path);
    }
}
