//! ADR-0026 §9-bis — `/api/metrics` como **proxy read-only** do exporter que já existe.
//!
//! # O que este módulo é
//!
//! Um **cano**. Faz `GET /metrics` ao `led_hal::serve_metrics` e devolve o corpo tal como
//! veio, com o `Content-Type` tal como veio.
//!
//! # O que este módulo **nunca** faz
//!
//! Não calcula, não agrega, não soma, não converte unidades e não reescreve o formato de
//! exposição. Um proxy que agregasse seria uma **segunda fonte de verdade** sobre
//! observabilidade — a classe que o §15 proíbe — e a divergência entre o que o Prometheus
//! raspa e o que o browser vê seria invisível dos dois lados.
//!
//! # Porque existe
//!
//! O exporter vive noutro processo. Sem este cano, o browser teria de lhe falar
//! **diretamente**, e passaria a ter **duas origens**: o console e o exporter. A segunda não
//! atravessa o tradutor, portanto nada do que este crate garante se aplicaria a ela.
//!
//! # O que passar por aqui **não** muda
//!
//! Continua a ser **observabilidade, não evidência física** (§9). `lumyx_frames_total` a
//! crescer diz que o `sendto` local teve sucesso — e um `sendto` para um destino inexistente
//! também tem. Ver [`crate::truth`].

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

/// A resposta do exporter, **como veio**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsBrutas {
    /// O corpo, byte a byte. Nunca reformatado.
    pub corpo: String,
    /// O `Content-Type` que o exporter declarou. Repassado sem interpretação.
    pub content_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErroMetricas {
    /// Não foi possível falar com o exporter. É **estado**, não erro do browser.
    ExporterOffline(String),
    /// O exporter respondeu algo que não é uma resposta HTTP utilizável.
    RespostaInvalida(String),
    /// O exporter respondeu com um estado que não é 200.
    ExporterRecusou(u16),
}

impl ErroMetricas {
    /// O código HTTP com que o console responde ao browser.
    ///
    /// `502` e não `500`: a falha é do **componente a montante**, e dizer 500 mandaria o
    /// operador procurar o defeito no console. É a mesma correção de *blame* que o
    /// `PedidoDemasiadoGrande` já levou (413, não 502).
    pub fn http_status(&self) -> u16 {
        match self {
            ErroMetricas::ExporterOffline(_) => 503,
            ErroMetricas::RespostaInvalida(_) | ErroMetricas::ExporterRecusou(_) => 502,
        }
    }
}

/// O caminho que o console **pede ao exporter**. Não é o caminho que o browser usa.
const CAMINHO_EXPORTER: &str = "/metrics";

/// Teto de leitura da resposta do exporter.
///
/// Deriva de [`crate::limits::MAX_BODY`] em vez de ser um número novo: o corpo que o console
/// aceita do browser e o que aceita do exporter são o **mesmo** limite, pela mesma razão —
/// um par que nunca fecha faz o processo crescer sem limite. Foi exatamente esse o defeito
/// que o GS3 tinha e que a correção do `MAX_LINE` fechou.
const TETO_RESPOSTA: usize = crate::limits::MAX_BODY;

/// Busca as métricas ao exporter e devolve-as **inalteradas**.
///
/// `exporter` é **dado injetado** — o console não descobre nem adivinha onde o exporter
/// está. Quem monta o processo é que sabe.
pub fn buscar(exporter: SocketAddr) -> Result<MetricsBrutas, ErroMetricas> {
    let mut stream = TcpStream::connect_timeout(&exporter, crate::limits::http_timeout())
        .map_err(|e| ErroMetricas::ExporterOffline(e.to_string()))?;
    stream
        .set_read_timeout(Some(crate::limits::http_timeout()))
        .map_err(|e| ErroMetricas::ExporterOffline(e.to_string()))?;

    let pedido = format!(
        "GET {CAMINHO_EXPORTER} HTTP/1.1\r\nHost: {exporter}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(pedido.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| ErroMetricas::ExporterOffline(e.to_string()))?;

    // O exporter fecha a ligação no fim (`Connection: close`), por isso ler até ao EOF é o
    // fim da resposta — mas com teto, nunca "até acabar".
    let mut bruto = Vec::new();
    let mut janela = [0u8; 4096];
    loop {
        match stream.read(&mut janela) {
            Ok(0) => break,
            Ok(n) => {
                bruto.extend_from_slice(&janela[..n]);
                if bruto.len() > TETO_RESPOSTA {
                    return Err(ErroMetricas::RespostaInvalida(
                        "o exporter respondeu acima do teto".into(),
                    ));
                }
            }
            Err(e) => return Err(ErroMetricas::ExporterOffline(e.to_string())),
        }
    }

    separar(&bruto)
}

/// Parte a resposta HTTP em estado + cabeçalhos + corpo. **Não toca no corpo.**
fn separar(bruto: &[u8]) -> Result<MetricsBrutas, ErroMetricas> {
    let texto = String::from_utf8_lossy(bruto);
    let corte = texto
        .find("\r\n\r\n")
        .ok_or_else(|| ErroMetricas::RespostaInvalida("sem fim de cabecalhos".into()))?;
    let (cabecalhos, resto) = texto.split_at(corte);
    let corpo = &resto[4..];

    let mut linhas = cabecalhos.lines();
    let estado = linhas
        .next()
        .ok_or_else(|| ErroMetricas::RespostaInvalida("sem linha de estado".into()))?;
    let codigo: u16 = estado
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| ErroMetricas::RespostaInvalida(format!("estado ilegivel: {estado}")))?;
    if codigo != 200 {
        return Err(ErroMetricas::ExporterRecusou(codigo));
    }

    let content_type = linhas
        .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
        .map(|l| l["content-type:".len()..].trim().to_string())
        .unwrap_or_default();

    Ok(MetricsBrutas { corpo: corpo.to_string(), content_type })
}
