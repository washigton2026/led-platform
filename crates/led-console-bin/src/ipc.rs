//! ADR-0026 §1–3 — o cliente do IPC v1. **Duas ligações, e nada de domínio.**
//!
//! # Nenhuma segunda implementação do protocolo
//!
//! As linhas são construídas e lidas com o `proto`/`json` do `led-daemon-bin` — o mesmo que
//! o servidor usa e que o `ledctl` já exercita. Um segundo codificador seria uma segunda
//! coisa para divergir do fio que já funciona.

#![cfg(unix)]

use crate::limits::MAX_BODY;
use led_daemon_bin::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

/// O que correu mal ao falar com o daemon. **Os códigos do daemon atravessam intactos**
/// dentro de [`Erro::Recusado`]; o console nunca os reinterpreta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Erro {
    /// Não foi possível sequer falar com o daemon. É o `OFFLINE` do ADR-0026 §7.
    Offline(String),
    /// O daemon respondeu, e recusou. `code` é **o código dele**, verbatim.
    Recusado { code: String, detail: String },
    /// A resposta não era analisável, ou a ligação fechou a meio.
    Protocolo(String),
    /// O **pedido** excede o que o protocolo v1 aceita, e não chega a sair.
    ///
    /// Separado de [`Erro::Protocolo`] de propósito: culpar o daemon (502) por um corpo que
    /// o browser mandou grande demais atribui a falha ao lado errado, e é o operador que
    /// depois vai procurar o defeito no sítio errado.
    PedidoDemasiadoGrande { bytes: usize, limite: usize },
}

impl Erro {
    /// O código a pôr no corpo HTTP. Os do console são prefixados, para nunca se confundirem
    /// com os do daemon.
    pub fn code(&self) -> &str {
        match self {
            Erro::Offline(_) => "console.daemon_offline",
            Erro::Recusado { code, .. } => code,
            Erro::Protocolo(_) => "console.bad_response",
            Erro::PedidoDemasiadoGrande { .. } => "console.request_too_large",
        }
    }

    /// O estado HTTP — **transporte apenas**. O significado está no `code`.
    pub fn http_status(&self) -> u16 {
        match self {
            Erro::Offline(_) => 503,
            Erro::Protocolo(_) => 502,
            // 413, e não 400: o pedido é bem formado — é só grande demais.
            Erro::PedidoDemasiadoGrande { .. } => 413,
            Erro::Recusado { code, .. } if code == led_daemon_bin::proto::code::BAD_REQUEST => 400,
            Erro::Recusado { code, .. } if code == led_daemon_bin::proto::code::INVALID_ARGS => 400,
            Erro::Recusado { code, .. } if code == led_daemon_bin::proto::code::ENGINE_BUSY => 504,
            // Tudo o resto é "nao se aplica ao estado atual" — e QUAL estado vem do `code`.
            Erro::Recusado { .. } => 409,
        }
    }
}

/// Uma ligação ao socket do daemon. Duas destas por console: comando e eventos.
pub struct Ligacao {
    stream: UnixStream,
    leitor: BufReader<UnixStream>,
    proximo_id: u64,
}

impl Ligacao {
    /// Abre e faz o handshake **obrigatório**. Nada mais é aceite antes dele.
    pub fn abrir(socket: &str, cliente: &str) -> Result<Self, Erro> {
        let stream = UnixStream::connect(socket)
            .map_err(|e| Erro::Offline(format!("{socket}: {e}")))?;
        let leitor = BufReader::new(
            stream.try_clone().map_err(|e| Erro::Offline(e.to_string()))?,
        );
        let mut l = Ligacao { stream, leitor, proximo_id: 1 };
        l.pedir("hello", &format!(r#","client":"{}""#, json::escape(cliente)))?;
        Ok(l)
    }

    /// Envia um comando e devolve a linha de resposta crua.
    ///
    /// `args_extra` é um fragmento JSON já escapado (ex.: `,"args":{"to_ms":4000}`) — o
    /// console não constrói tipos de comando próprios, escreve o que o protocolo v1 define.
    pub fn pedir(&mut self, cmd: &str, args_extra: &str) -> Result<String, Erro> {
        let id = self.proximo_id;
        self.proximo_id += 1;
        let linha = format!(r#"{{"v":1,"id":{id},"cmd":"{cmd}"{args_extra}}}"#);
        if linha.len() > MAX_BODY {
            return Err(Erro::PedidoDemasiadoGrande { bytes: linha.len(), limite: MAX_BODY });
        }
        self.stream
            .write_all(linha.as_bytes())
            .and_then(|_| self.stream.write_all(b"\n"))
            .and_then(|_| self.stream.flush())
            .map_err(|e| Erro::Offline(e.to_string()))?;
        self.ler_resposta()
    }

    /// Lê uma **resposta** (tem `id`), saltando eventos assíncronos (que não têm).
    ///
    /// É o mesmo critério que o `ledctl` usa: os eventos distinguem-se pela **ausência** de
    /// `id`, sem campo extra.
    fn ler_resposta(&mut self) -> Result<String, Erro> {
        loop {
            let l = self.ler_linha()?;
            let tem_id = json::parse(&l).ok().and_then(|j| j.get("id").cloned()).is_some();
            if !tem_id {
                continue; // evento: não é a resposta deste pedido
            }
            let j = json::parse(&l).map_err(|e| Erro::Protocolo(format!("{e:?}")))?;
            let ok = j.get("ok").map(|v| *v == json::Json::Bool(true)).unwrap_or(false);
            if ok {
                return Ok(l);
            }
            let code = j
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(|c| c.as_str().map(String::from))
                .unwrap_or_else(|| "console.bad_response".to_string());
            let detail = j
                .get("error")
                .and_then(|e| e.get("detail"))
                .and_then(|c| c.as_str().map(String::from))
                .unwrap_or_default();
            return Err(Erro::Recusado { code, detail });
        }
    }

    /// Lê a próxima linha, seja resposta ou evento. Usada pela ligação de eventos.
    pub fn ler_linha(&mut self) -> Result<String, Erro> {
        let mut l = String::new();
        match self.leitor.read_line(&mut l) {
            Ok(0) => Err(Erro::Offline("o daemon fechou a ligacao".into())),
            Ok(_) => Ok(l.trim().to_string()),
            Err(e) => Err(Erro::Offline(e.to_string())),
        }
    }

    /// Transforma esta ligação no fluxo de eventos. **Chamado uma vez por console.**
    pub fn subscrever(&mut self) -> Result<(), Erro> {
        self.pedir("subscribe", "").map(|_| ())
    }
}
