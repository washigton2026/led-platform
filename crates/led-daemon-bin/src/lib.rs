//! # led-daemon-bin — o **processo** daemon (GS2)
//!
//! O [`led_daemon`] define o contrato (ADR-0023, congelado na GS1.6). Este crate dá-lhe um
//! processo: relógio, laço, carregamento de `.lumyx`, journal e encerramento.
//!
//! **A separação é o ponto.** O `led-daemon` continua **sem dependências** — nem `led-core` —
//! e esta fatia **não altera uma linha** dele. Relógio, I/O e CLI vivem aqui.
//!
//! ## O que este processo NÃO faz (GS2)
//!
//! **Não tem saída.** Nenhum frame deixa o processo: não há HAL, dispositivos, socket nem
//! rede. IPC é GS3, Ethernet é GS4. O daemon avança o transporte e regista o que aconteceu.
//!
//! Isto tem uma consequência de honestidade que está no código, não só aqui: os gates de
//! pré-voo `network_ok` e `devices_present` são **vacuosamente** verdadeiros — não se pode
//! enviar por WiFi o que não se envia. Ver [`run::preflight_for_no_output`].
//!
//! ## Encerramento
//!
//! Duas vias, ambas limpas: `--max-ticks N` (execução limitada) e a linha `shutdown` no
//! stdin. **Não há tratamento de `SIGINT`/`SIGTERM`** — exigiria uma dependência de sinais, e
//! o `shutdown` por IPC é entrega do GS3. Ctrl-C termina o processo, mas **abruptamente**:
//! sem a linha final de estado nem o *flush* do journal.

pub mod json;
pub mod journal;
pub mod loader;
pub mod output;
pub mod pacer;
pub mod proto;
pub mod run;
pub mod preflight;
pub mod source;
pub mod stage;
#[cfg(unix)]
pub mod server;

pub use journal::Journal;
pub use loader::{descriptor_from_path, descriptor_from_reader, Integrity, LoadError};
pub use output::{OutputConfig, OutputManager, OutputProtocol};
pub use pacer::{Pacer, SystemPacer};
pub use source::FrameSource;
pub use stage::{Stage, StageTick};
pub use run::{run, Config, ExitReason, Outcome};
#[cfg(unix)]
pub use server::{ControlPlane, Server, Snapshot};
