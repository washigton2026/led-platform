//! **ADR-0026** — a ponte console↔daemon.
//!
//! # O que este crate é
//!
//! Um **tradutor de transporte**. Fala IPC v1 com o daemon (como o `ledctl`) e prepara a
//! superfície que o browser vai consumir. Não é uma autoridade de domínio.
//!
//! # O que este crate **nunca** contém
//!
//! Máquina de estados · regras de hardware · `Calibration`/LUT · MTU · `refresh_hz` ·
//! `HardwareProfile` · serialização canónica. Tudo isso vive a montante e é **transportado**.
//! Há um gate estrutural (`tests/surface_gate.rs`) que reprova se algum deles aparecer
//! aqui — porque um tradutor que ganha opiniões é uma segunda fonte de verdade a nascer.
//!
//! # A regra que este crate existe sobretudo para proteger
//!
//! **Observabilidade não é evidência física.** `frames_sent` a crescer diz que o `sendto`
//! local teve sucesso — e um `sendto` para um destino inexistente **também** tem sucesso
//! local. Ver [`truth`].

pub mod contract;
pub mod fanout;
pub mod limits;
pub mod metrics;
pub mod surface;
pub mod truth;

#[cfg(unix)]
pub mod http;
#[cfg(unix)]
pub mod ipc;

pub use fanout::{Fanout, Subscriber};
pub use metrics::{buscar, ErroMetricas, MetricsBrutas};
pub use limits::{bind_permitido, http_timeout, MARGEM_HTTP, MAX_BODY, MAX_JSON_DEPTH};
pub use surface::{Rota, Verbo, ROTAS};
pub use truth::{Elo, EstadoUi, Evidencia, Instantaneo};
