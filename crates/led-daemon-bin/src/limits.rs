//! Limites do IPC partilhados com o `led-console-bin` (ADR-0026 §11–12) — **sem `cfg`**.
//!
//! Vivem fora do `server` porque o `server` é `#[cfg(unix)]` (UDS, ADR-0014) e o console é
//! compilado em todas as plataformas da matriz: um consumidor cross-plataforma não pode
//! derivar de um módulo que só existe em unix (TD-021). O `server` reexporta-os, portanto
//! `led_daemon_bin::server::{MAX_LINE, REPLY_TIMEOUT}` continua a ser o mesmo objeto.

use std::time::Duration;

/// Tamanho máximo de uma linha de pedido. Sem isto, um cliente que nunca envie `\n` faz o
/// daemon crescer sem limite — negação de serviço com um byte por segundo.
pub const MAX_LINE: usize = 64 * 1024;

/// Quanto tempo uma ligação espera pela resposta do laço antes de desistir.
///
/// **Público desde o ADR-0026** para que o `led-console-bin` possa *derivar* dele o seu
/// timeout HTTP em vez de escrever um segundo número. Se os dois empatassem, o browser
/// receberia "falhou" enquanto o daemon ainda aplicaria o comando.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
