//! Playback **em fluxo** com pacing por **instante absoluto** (FASE F2 — ADR-0022 D1/D3).
//!
//! Dois problemas, um módulo. Ambos foram achados por leitura do código existente, não
//! presumidos.
//!
//! ## Problema 1 — o show inteiro em RAM
//!
//! [`play`](crate::play) e [`play_instrumented`](crate::play_instrumented) recebem
//! `&[ShowRecord]`, e o binário faz `collect_all()`. Para desktop está **certo**, e a razão
//! está registrada: permite verificar o manifesto antes do 1º quadro.
//!
//! Para um traje, não serve. Um ESP32 tem ~520 kB de SRAM; um artefato de 4 min × 400 px
//! tem ~11 MB. O [`ShowReader`] **já** transmite quadro a quadro — quem não sabia consumir
//! em fluxo era o player. [`play_streaming_unverified`] é essa ponte: pico de memória de **um quadro**,
//! independente da duração.
//!
//! ## Problema 2 — o pacing atual acumula erro
//!
//! `play_instrumented` (`src/lib.rs:112-120`) dorme o **intervalo** entre quadros, não até um
//! **instante alvo**:
//!
//! ```text
//! gap = ts[i] - ts[i-1];  sleep(gap / fator);   // depois: envia, e o envio custa tempo
//! ```
//!
//! Duas propriedades estruturais disso:
//!
//! 1. **Livre-corrente.** O erro de granularidade do escalonador em cada `sleep` não é
//!    corrigido no quadro seguinte — **soma**.
//! 2. **O custo do envio não é descontado.** O período real é `gap + custo_de_envio`, e
//!    `t0.elapsed()` é medido para métrica mas nunca subtraído do próximo `sleep`. Não é
//!    jitter: é **viés sistemático de lentidão**, monotônico ao longo do número.
//!
//! Para o rig cabeado nunca importou — um player só, e a percepção é relativa. Para N trajes
//! independentes é exatamente o mecanismo que os separa.
//!
//! [`Pacing::Absolute`] dorme **até `epoch + timestamp`** contra um [`SharedClock`], e quando
//! está atrasado **não empurra o erro para a frente**: descarta o quadro vencido. O desvio
//! passa a ser limitado pela exatidão do relógio, não pelo acúmulo do escalonador.
//!
//! `Speed::Factor` **não é removido**: continua correto para bancada e re-verificação de
//! integridade. Este modo é **aditivo**.
//!
//! ## 🔴 O que este módulo **não** faz — leia antes de usar
//!
//! **Não há autenticação nem verificação de integridade antes do primeiro quadro.**
//! O binário `led-player` faz isso e faz certo (`main.rs:175→218→226→412`): materializa,
//! constrói o `ReplayManifest`, compara com o sidecar e chama `verify_manifest_pinned`
//! **antes** de tocar. [`play_streaming_unverified`] **não faz nada disso** — lê um quadro e
//! envia. É por isso que o nome carrega `_unverified`: o risco viaja para todo call-site e
//! todo `grep`, em vez de ficar escondido numa doc que ninguém lê.
//!
//! Some-se a isso que um artefato de `bake` tem manifesto e hashes **novos** (ver TD-013): a
//! assinatura do show de origem não o cobre. Portanto, hoje, este caminho é **bancada e
//! teste**. Não é modo de traje e não é modo de produção.
//!
//! A cobertura correta é a próxima fatia obrigatória do F2, e reusa o que já existe
//! (ADR-0004): mesmo `ReplayManifest`, mesma `ShowSigner`, mesmo `verify_manifest_pinned`,
//! mesmo sidecar. Nenhuma segunda assinatura, nenhum formato paralelo.
//!
//! Também **não** mede deriva de oscilador real — isso é o gate **G6**, em bancada, com
//! hardware. Aqui se prova a **política de escalonamento**, que é o que o ADR-0022 D3 afirma.
//! Nenhum número de ppm aparece neste arquivo.

use std::io::Read;
use std::time::Duration;

use led_core::{LogicalFrame, OutputError, ProtocolOutput};
use led_hal::SharedClock;
use led_show_recorder::{ReadError, ShowReader};


/// Como espaçar os quadros.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pacing {
    /// Dorme até **`epoch_ms + timestamp_ms`** medido no [`SharedClock`]. Quadro vencido é
    /// **descartado**, nunca atrasa o próximo — é isso que impede o erro de acumular.
    Absolute { epoch_ms: u64 },
    /// Sem dormir. Para bancada e verificação de integridade.
    Max,
}

/// Resultado de um playback em fluxo **não autenticado**.
///
/// **Deliberadamente NÃO carrega `manifest_hash`**, ao contrário de
/// [`PlaybackReport`](crate::PlaybackReport). Um player em fluxo não conhece o hash do show
/// antes de tocá-lo — conhecê-lo exige uma **primeira passada** sobre o arquivo. Preencher o
/// campo com o hash do que já foi lido daria a impressão de uma verificação que não houve.
///
/// A verificação antes do 1º quadro é uma propriedade de segurança real do player atual
/// (ADR-0004 / `--verify-key`) e **continua sendo requisito** para o traje: a forma correta é
/// uma passada de verificação sobre a flash **antes** de começar, e ela é a próxima fatia
/// obrigatória (**TD-013**) — não algo que este relatório possa fingir.
///
/// Enquanto essa fatia não existir, **um `StreamReport` bem-sucedido não significa que o show
/// era autêntico**. Significa apenas que os bytes lidos foram enviados.
#[derive(Debug)]
pub struct StreamReport {
    pub frames_played: u64,
    pub frames_failed: u64,
    pub duration_ms: u64,
    pub pixel_count: u32,
    /// Quadros descartados por já terem vencido quando chegou a vez deles.
    pub frames_late: u64,
    /// Pior atraso observado, em ms. **Medido, não estimado.**
    pub worst_late_ms: u64,
}

/// Erros que interrompem um playback em fluxo.
#[derive(Debug)]
pub enum StreamError {
    Read(ReadError),
    Output(OutputError),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Read(e) => write!(f, "leitura do show: {e:?}"),
            StreamError::Output(e) => write!(f, "saída: {e:?}"),
        }
    }
}

impl std::error::Error for StreamError {}

/// Toca um show **direto do leitor**, sem materializá-lo — **sem autenticar nada**.
///
/// Contraste com [`play`](crate::play): aqui o pico de memória é um quadro. É a forma que um
/// traje vai precisar (ADR-0022 D1) e a única que funciona quando o artefato não cabe na RAM.
///
/// # 🔴 Não use em palco
///
/// Emite o primeiro quadro **sem** verificar assinatura nem integridade — ao contrário do
/// caminho do binário `led-player`, que exige `verify_manifest_pinned` antes de tocar. O
/// sufixo `_unverified` é o contrato: enquanto a **TD-013** estiver aberta, este é um caminho
/// de **bancada e teste**, não de traje nem de produção.
pub fn play_streaming_unverified<R: Read>(
    mut reader: ShowReader<R>,
    output: &dyn ProtocolOutput,
    pacing: Pacing,
    clock: &SharedClock,
) -> Result<StreamReport, StreamError> {
    let mut played = 0u64;
    let mut failed = 0u64;
    let mut frames_late = 0u64;
    let mut worst_late_ms = 0u64;
    // Período inferido do próprio show: é o que define quando um quadro foi SUPERADO.
    let mut prev_ts: Option<u64> = None;
    let mut period_ms: u64 = 0;
    let pixel_count = reader.pixel_count;
    let start = clock.now_ms();

    loop {
        let rec = match reader.next_frame() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => return Err(StreamError::Read(e)),
        };

        if let Some(p) = prev_ts {
            period_ms = rec.timestamp_ms.saturating_sub(p).max(period_ms);
        }
        prev_ts = Some(rec.timestamp_ms);

        if let Pacing::Absolute { epoch_ms } = pacing {
            let target = epoch_ms.saturating_add(rec.timestamp_ms);
            let now = clock.now_ms();
            if now < target {
                std::thread::sleep(Duration::from_millis(target - now));
            } else if now > target {
                let late = now - target;
                worst_late_ms = worst_late_ms.max(late);
                frames_late += 1;
                // Descartar só quando o quadro já foi **superado** — isto é, quando o
                // atraso passou de um período inteiro e o quadro seguinte já deveria estar
                // no ar. Um atraso menor que isso é enviado assim mesmo: mostrar o quadro
                // 3 ms tarde é muito melhor que não mostrar. O que NÃO acontece em nenhum
                // dos dois casos é empurrar o atraso para o quadro seguinte — o alvo do
                // próximo continua sendo `epoch + ts`, e é isso que impede o acúmulo.
                if period_ms > 0 && late >= period_ms {
                    continue;
                }
            }
        }

        let frame = LogicalFrame::new(rec.pixels, rec.timestamp_ms);
        match output.send_frame(&frame) {
            Ok(()) => played += 1,
            // Show parcial é melhor que palco escuro — mesmo precedente do player atual.
            Err(_) => failed += 1,
        }
    }

    let duration_ms = clock.now_ms().saturating_sub(start);
    Ok(StreamReport {
        frames_played: played,
        frames_failed: failed,
        duration_ms,
        pixel_count,
        frames_late,
        worst_late_ms,
    })
}

// ── Política de pacing, isolada para ser provável sem relógio de parede ────────

/// As duas políticas, como funções puras sobre um modelo de tempo. Existe para que o gate
/// de deriva seja **determinístico**: medir `sleep` real seria instável sob carga e não
/// provaria a propriedade estrutural que o ADR-0022 D3 afirma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacingPolicy {
    /// O que `play_instrumented` faz hoje: dorme o intervalo, não até o alvo.
    Incremental,
    /// O que este módulo faz: dorme até `epoch + timestamp`.
    Absolute,
}

/// Simula `frames` quadros e devolve o **desvio do último quadro** em relação ao ideal, em
/// ms — positivo = atrasado.
///
/// Modelo: cada quadro custa `send_cost_ms` para enviar e o escalonador acorda
/// `sched_slop_ms` tarde. Nenhum dos dois é medido aqui; são **entradas** do modelo, e é
/// justamente por isso que o resultado é uma prova sobre a *política*, não sobre hardware.
pub fn simulate_drift_ms(
    policy: PacingPolicy,
    frames: u64,
    period_ms: u64,
    send_cost_ms: u64,
    sched_slop_ms: u64,
) -> i64 {
    let mut now: u64 = 0;
    for i in 0..frames {
        let ideal = i * period_ms;
        match policy {
            PacingPolicy::Incremental => {
                // Dorme o INTERVALO a partir de onde estiver — o erro de onde estava
                // permanece, e o custo de envio é somado depois sem nunca ser descontado.
                if i > 0 {
                    now += period_ms + sched_slop_ms;
                }
            }
            PacingPolicy::Absolute => {
                // Dorme até o ALVO. Se já passou, não recua; se falta, acorda com folga.
                let target = ideal;
                now = if now < target { target + sched_slop_ms } else { now };
            }
        }
        now += send_cost_ms;
        if i + 1 == frames {
            return now as i64 - (ideal + send_cost_ms) as i64;
        }
    }
    0
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::PixelColor;
    use led_show_recorder::{finalise_seekable, ShowRecord, ShowWriter};
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    fn show(px: usize, frames: u64, period_ms: u64) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = ShowWriter::new(&mut buf, px as u32).unwrap();
            for f in 0..frames {
                w.write_frame(&ShowRecord {
                    timestamp_ms: f * period_ms,
                    pixels: vec![PixelColor::rgb(f as u8, 0, 0); px],
                    audio: None,
                })
                .unwrap();
            }
            finalise_seekable(&mut w).unwrap();
        }
        buf.into_inner()
    }

    #[derive(Default)]
    struct Spy {
        seen: Mutex<Vec<u64>>,
    }
    impl ProtocolOutput for Spy {
        fn send_frame(&self, f: &LogicalFrame) -> Result<(), OutputError> {
            self.seen.lock().unwrap().push(f.timestamp_ms);
            Ok(())
        }
        fn universe_count(&self) -> u16 {
            1
        }
    }

    #[test]
    fn streaming_playback_delivers_every_frame_without_materialising() {
        let data = show(64, 200, 25);
        let reader = ShowReader::new(Cursor::new(data)).unwrap();
        let spy = Arc::new(Spy::default());
        let clock = SharedClock::new();

        let rep = play_streaming_unverified(reader, spy.as_ref(), Pacing::Max, &clock).unwrap();
        assert_eq!(rep.frames_played, 200);
        assert_eq!(rep.frames_failed, 0);

        let seen = spy.seen.lock().unwrap();
        assert_eq!(seen.len(), 200);
        assert_eq!(seen[0], 0);
        assert_eq!(seen[199], 199 * 25, "ordem e timestamps preservados");
    }

    #[test]
    fn absolute_pacing_on_schedule_reports_no_lateness() {
        // Relógio novo começa em ~0, então época 0 é AGORA: o playback consegue cumprir
        // os alvos e nada pode ser reportado como atrasado.
        let data = show(8, 6, 25);
        let reader = ShowReader::new(Cursor::new(data)).unwrap();
        let spy = Arc::new(Spy::default());
        let rep = play_streaming_unverified(
            reader,
            spy.as_ref(),
            Pacing::Absolute { epoch_ms: 0 },
            &SharedClock::new(),
        )
        .unwrap();
        assert_eq!(rep.frames_played, 6);
        assert_eq!(rep.frames_late, 0, "no horário: nada pode contar como atrasado");
    }

    #[test]
    fn absolute_pacing_with_an_epoch_already_past_reports_lateness_and_never_stalls() {
        // `with_offset` põe o relógio bem à frente ⇒ TODO quadro já venceu. O modo não pode
        // travar: tem de correr, CONTAR o atraso e descartar os quadros superados.
        let data = show(8, 50, 25);
        let reader = ShowReader::new(Cursor::new(data)).unwrap();
        let spy = Arc::new(Spy::default());
        let clock = SharedClock::with_offset(60_000); // 60 s à frente do show
        let rep =
            play_streaming_unverified(reader, spy.as_ref(), Pacing::Absolute { epoch_ms: 0 }, &clock).unwrap();

        assert_eq!(rep.frames_late, 50, "todo quadro venceu — e isso tem de ser contado");
        assert!(rep.worst_late_ms >= 59_000, "atraso real, não arredondado: {}", rep.worst_late_ms);
        // Descartados por superação: só o primeiro passa (antes dele não há período conhecido).
        assert_eq!(rep.frames_played, 1, "quadros superados são descartados, não enfileirados");
    }

    /// A regra de descarte tem de ser **superação**, não impaciência: um atraso menor que um
    /// período ainda vai ao ar. Mostrar o quadro 3 ms tarde é muito melhor que não mostrar.
    #[test]
    fn a_frame_late_by_less_than_one_period_is_still_shown() {
        let data = show(8, 4, 1000); // período de 1 s: folga enorme
        let reader = ShowReader::new(Cursor::new(data)).unwrap();
        let spy = Arc::new(Spy::default());
        // 10 ms à frente: todo quadro fica atrasado, mas MUITO menos que um período.
        let clock = SharedClock::with_offset(10);
        let rep =
            play_streaming_unverified(reader, spy.as_ref(), Pacing::Absolute { epoch_ms: 0 }, &clock).unwrap();
        assert_eq!(rep.frames_played, 4, "atraso sub-período não pode apagar o quadro");
        assert!(rep.frames_late >= 1, "e ainda assim o atraso é reportado");
    }

    #[test]
    fn a_failing_output_does_not_stop_the_show() {
        struct Bad;
        impl ProtocolOutput for Bad {
            fn send_frame(&self, _: &LogicalFrame) -> Result<(), OutputError> {
                Err(OutputError::Transport("nope".into()))
            }
            fn universe_count(&self) -> u16 {
                1
            }
        }
        let data = show(4, 10, 25);
        let reader = ShowReader::new(Cursor::new(data)).unwrap();
        let rep = play_streaming_unverified(reader, &Bad, Pacing::Max, &SharedClock::new()).unwrap();
        assert_eq!(rep.frames_played, 0);
        assert_eq!(rep.frames_failed, 10, "conta a falha e segue — palco escuro é pior");
    }

    // ── O gate de deriva (ADR-0022 D3) ────────────────────────────────────────

    /// A propriedade que o ADR-0022 D3 afirma: o pacing absoluto **não acumula**.
    #[test]
    fn absolute_pacing_does_not_accumulate_error() {
        // 4 min a 40 fps = 9600 quadros; 1 ms de custo de envio, 1 ms de folga do
        // escalonador — números modestos e realistas para um MCU.
        let d = simulate_drift_ms(PacingPolicy::Absolute, 9600, 25, 1, 1);
        assert!(
            d.unsigned_abs() <= 25,
            "absoluto tem de ficar dentro de UM quadro (25 ms); ficou {d} ms"
        );
    }

    /// **Controle negativo (KB-012).** O gate acima só significa alguma coisa se a política
    /// atual **reprovar** nele. Se ambas passassem, o gate não mediria acúmulo nenhum.
    #[test]
    fn negative_control_current_incremental_pacing_fails_the_same_gate() {
        let frames = 9600;
        let incremental = simulate_drift_ms(PacingPolicy::Incremental, frames, 25, 1, 1);
        let absolute = simulate_drift_ms(PacingPolicy::Absolute, frames, 25, 1, 1);

        assert!(
            incremental.unsigned_abs() > 25,
            "se o pacing incremental passasse no gate, o gate não provaria nada (deu {incremental} ms)"
        );
        assert!(
            incremental > absolute * 100,
            "o acúmulo tem de ser ordens de grandeza pior: incremental {incremental} ms vs absoluto {absolute} ms"
        );
    }

    /// O acúmulo é **linear no número de quadros** — é isso que o torna fatal num número
    /// longo e invisível num teste curto.
    #[test]
    fn incremental_error_grows_with_the_show_length() {
        let short = simulate_drift_ms(PacingPolicy::Incremental, 100, 25, 1, 1);
        let long = simulate_drift_ms(PacingPolicy::Incremental, 10_000, 25, 1, 1);
        assert!(long > short * 50, "curto {short} ms, longo {long} ms — tem de crescer");

        // E o absoluto NÃO cresce: é a diferença estrutural entre as duas políticas.
        let a_short = simulate_drift_ms(PacingPolicy::Absolute, 100, 25, 1, 1);
        let a_long = simulate_drift_ms(PacingPolicy::Absolute, 10_000, 25, 1, 1);
        assert_eq!(a_short, a_long, "absoluto: o desvio não depende do comprimento do show");
    }

    /// Sem custo de envio nem folga de escalonador as duas políticas coincidem — o que
    /// confirma que a divergência vem **exatamente** dessas duas fontes, e não do modelo.
    #[test]
    fn with_a_perfect_scheduler_both_policies_agree() {
        assert_eq!(
            simulate_drift_ms(PacingPolicy::Incremental, 5000, 25, 0, 0),
            simulate_drift_ms(PacingPolicy::Absolute, 5000, 25, 0, 0)
        );
    }
}
