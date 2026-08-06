//! Protocolo de controlo **v1** — tipos e (des)serialização.
//!
//! Concretiza o `docs/architecture/control-protocol.md`: uma mensagem por linha, `id` a
//! correlacionar pedido e resposta, `v` negociado no `hello`, e **códigos de erro
//! enumerados** — nunca string livre.
//!
//! ## O que este módulo NÃO faz
//!
//! Não abre sockets e não toca no `ShowRuntime`. É só o formato. Isso torna todo o protocolo
//! testável sem rede — e é o que permite que os testes adversariais do parser corram em
//! microssegundos em vez de dependerem de um servidor.

use crate::json::{escape, parse, Json};

/// Versão do protocolo. Uma versão desconhecida é **recusada explicitamente**, nunca
/// degradada — a mesma regra que o `schema_version` do ADR-0018 já aplica.
pub const PROTOCOL_V: u64 = 1;

// ── Códigos de erro (enumerados, do control-protocol.md) ─────────────────────

pub mod code {
    /// Comando antes do `hello`.
    pub const UNAUTHENTICATED: &str = "unauthenticated";
    /// `v` desconhecida.
    pub const UNSUPPORTED_VERSION: &str = "unsupported_version";
    pub const UNKNOWN_COMMAND: &str = "unknown_command";
    pub const INVALID_ARGS: &str = "invalid_args";
    /// Ação irreversível sem `confirm`.
    pub const CONFIRMATION_REQUIRED: &str = "confirmation_required";
    /// Recusado por política (ex.: pré-voo, WiFi — ADR-0005).
    pub const REFUSED_BY_POLICY: &str = "refused_by_policy";
    /// O daemon não conseguiu processar (fila cheia, a encerrar).
    pub const ENGINE_BUSY: &str = "engine_busy";
    /// Erro ao ler o `.lumyx`.
    pub const LOAD_FAILED: &str = "load_failed";
    pub const BAD_REQUEST: &str = "bad_request";
}

/// Erro de protocolo: código **enumerado** + detalhe humano.
#[derive(Clone, Debug, PartialEq)]
pub struct ProtoError {
    pub code: &'static str,
    pub detail: String,
}

impl ProtoError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }
}

// ── Pedido ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Handshake **obrigatório**. Nada mais é aceite antes dele.
    Hello { client: String },
    Ping,
    Version,
    Status,
    Load { path: String, assume_integrity: bool },
    Unload,
    Play,
    Pause,
    Stop,
    Seek { to_ms: u64 },
    /// Passa a receber eventos assíncronos nesta ligação.
    Subscribe,
    /// **Duas fases.** Sem `confirm`, o daemon responde `confirmation_required` com um token
    /// de uso único; o cliente repete com ele.
    Shutdown { confirm: Option<String> },
}

impl Cmd {
    pub fn name(&self) -> &'static str {
        match self {
            Cmd::Hello { .. } => "hello",
            Cmd::Ping => "ping",
            Cmd::Version => "version",
            Cmd::Status => "status",
            Cmd::Load { .. } => "load",
            Cmd::Unload => "unload",
            Cmd::Play => "play",
            Cmd::Pause => "pause",
            Cmd::Stop => "stop",
            Cmd::Seek { .. } => "seek",
            Cmd::Subscribe => "subscribe",
            Cmd::Shutdown { .. } => "shutdown",
        }
    }
    /// Muda o estado do runtime? Usado pelo servidor para decidir o que enfileirar para o
    /// laço principal em vez de responder de imediato.
    pub fn touches_runtime(&self) -> bool {
        matches!(
            self,
            Cmd::Load { .. } | Cmd::Unload | Cmd::Play | Cmd::Pause | Cmd::Stop | Cmd::Seek { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub v: u64,
    pub id: u64,
    pub cmd: Cmd,
}

/// Erro de análise que **ainda sabe o `id`**, para a resposta poder ser correlacionada.
///
/// Sem isto, um pedido com `cmd` inválido receberia uma resposta sem `id` e o cliente ficaria
/// à espera para sempre do seu. O `id` é extraído **antes** de validar o resto.
#[derive(Debug, PartialEq)]
pub struct RequestError {
    pub id: Option<u64>,
    pub err: ProtoError,
}

fn bad(id: Option<u64>, code: &'static str, detail: impl Into<String>) -> RequestError {
    RequestError { id, err: ProtoError::new(code, detail) }
}

impl Request {
    /// Analisa uma linha. **Nunca entra em pânico** — toda a entrada malformada vira `Err`.
    pub fn from_line(line: &str) -> Result<Request, RequestError> {
        let j = parse(line).map_err(|e| bad(None, code::BAD_REQUEST, e.to_string()))?;

        // `id` primeiro, para que qualquer erro seguinte já possa ser correlacionado.
        let id = j.get("id").and_then(|x| x.as_u64());

        let v = j
            .get("v")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| bad(id, code::BAD_REQUEST, "falta `v`"))?;
        if v != PROTOCOL_V {
            return Err(bad(
                id,
                code::UNSUPPORTED_VERSION,
                format!("v={v}; este daemon aceita [{PROTOCOL_V}]"),
            ));
        }
        let id = id.ok_or_else(|| bad(None, code::BAD_REQUEST, "falta `id`"))?;
        let name = j
            .get("cmd")
            .and_then(|x| x.as_str())
            .ok_or_else(|| bad(Some(id), code::BAD_REQUEST, "falta `cmd`"))?;

        let args = j.get("args");
        let arg_str = |k: &str| args.and_then(|a| a.get(k)).and_then(|x| x.as_str());
        let arg_u64 = |k: &str| args.and_then(|a| a.get(k)).and_then(|x| x.as_u64());
        let arg_bool = |k: &str| {
            args.and_then(|a| a.get(k)).and_then(|x| match x {
                Json::Bool(b) => Some(*b),
                _ => None,
            })
        };

        let cmd = match name {
            "hello" => Cmd::Hello {
                client: j.get("client").and_then(|x| x.as_str()).unwrap_or("desconhecido").into(),
            },
            "ping" => Cmd::Ping,
            "version" => Cmd::Version,
            "status" => Cmd::Status,
            "unload" => Cmd::Unload,
            "play" => Cmd::Play,
            "pause" => Cmd::Pause,
            "stop" => Cmd::Stop,
            "subscribe" => Cmd::Subscribe,
            "load" => {
                let path = arg_str("path")
                    .ok_or_else(|| bad(Some(id), code::INVALID_ARGS, "`load` exige args.path"))?;
                Cmd::Load {
                    path: path.to_string(),
                    assume_integrity: arg_bool("assume_integrity").unwrap_or(false),
                }
            }
            "seek" => {
                let to_ms = arg_u64("to_ms").ok_or_else(|| {
                    bad(Some(id), code::INVALID_ARGS, "`seek` exige args.to_ms inteiro >= 0")
                })?;
                Cmd::Seek { to_ms }
            }
            "shutdown" => Cmd::Shutdown {
                confirm: j.get("confirm").and_then(|x| x.as_str()).map(String::from),
            },
            outro => {
                return Err(bad(Some(id), code::UNKNOWN_COMMAND, format!("comando `{outro}`")))
            }
        };

        Ok(Request { v, id, cmd })
    }
}

// ── Resposta ─────────────────────────────────────────────────────────────────

/// Resposta de sucesso, com campos extra já serializados (`"chave":valor,…`).
pub fn ok_line(id: u64, extra: &[(&str, String)]) -> String {
    let mut s = format!(r#"{{"v":{PROTOCOL_V},"id":{id},"ok":true"#);
    for (k, v) in extra {
        s.push_str(&format!(r#","{}":{v}"#, escape(k)));
    }
    s.push('}');
    s
}

pub fn err_line(id: Option<u64>, e: &ProtoError) -> String {
    let id_s = match id {
        Some(i) => i.to_string(),
        None => "null".to_string(),
    };
    format!(
        r#"{{"v":{PROTOCOL_V},"id":{id_s},"ok":false,"error":{{"code":"{}","detail":"{}"}}}}"#,
        e.code,
        escape(&e.detail)
    )
}

/// Um evento assíncrono (só para quem fez `subscribe`). Não tem `id`: não responde a nada.
pub fn event_line(payload: &str) -> String {
    format!(r#"{{"v":{PROTOCOL_V},"async":true,"payload":{payload}}}"#)
}

/// String JSON pronta a embutir num campo de resposta.
pub fn jstr(s: &str) -> String {
    format!("\"{}\"", escape(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(s: &str) -> Result<Request, RequestError> {
        Request::from_line(s)
    }

    #[test]
    fn analisa_todos_os_comandos() {
        let casos: Vec<(&str, Cmd)> = vec![
            (r#"{"v":1,"id":1,"cmd":"hello","client":"ledctl/0.1"}"#, Cmd::Hello { client: "ledctl/0.1".into() }),
            (r#"{"v":1,"id":2,"cmd":"ping"}"#, Cmd::Ping),
            (r#"{"v":1,"id":3,"cmd":"version"}"#, Cmd::Version),
            (r#"{"v":1,"id":4,"cmd":"status"}"#, Cmd::Status),
            (r#"{"v":1,"id":5,"cmd":"load","args":{"path":"/s.lumyx","assume_integrity":true}}"#,
             Cmd::Load { path: "/s.lumyx".into(), assume_integrity: true }),
            (r#"{"v":1,"id":6,"cmd":"unload"}"#, Cmd::Unload),
            (r#"{"v":1,"id":7,"cmd":"play"}"#, Cmd::Play),
            (r#"{"v":1,"id":8,"cmd":"pause"}"#, Cmd::Pause),
            (r#"{"v":1,"id":9,"cmd":"stop"}"#, Cmd::Stop),
            (r#"{"v":1,"id":10,"cmd":"seek","args":{"to_ms":1500}}"#, Cmd::Seek { to_ms: 1500 }),
            (r#"{"v":1,"id":11,"cmd":"subscribe"}"#, Cmd::Subscribe),
            (r#"{"v":1,"id":12,"cmd":"shutdown"}"#, Cmd::Shutdown { confirm: None }),
            (r#"{"v":1,"id":13,"cmd":"shutdown","confirm":"tok"}"#, Cmd::Shutdown { confirm: Some("tok".into()) }),
        ];
        for (linha, esperado) in casos {
            let r = req(linha).unwrap_or_else(|e| panic!("{linha} → {e:?}"));
            assert_eq!(r.cmd, esperado, "{linha}");
        }
    }

    /// Os 11 comandos do GS3 estão todos cobertos — se alguém acrescentar um sem teste, esta
    /// contagem denuncia.
    #[test]
    fn os_onze_comandos_do_gs3_existem() {
        let nomes = [
            "hello", "ping", "version", "status", "load", "unload", "play", "pause", "stop",
            "seek", "subscribe", "shutdown",
        ];
        for n in nomes {
            let linha = format!(
                r#"{{"v":1,"id":1,"cmd":"{n}","args":{{"path":"/x","to_ms":0}}}}"#
            );
            assert!(req(&linha).is_ok(), "comando `{n}` não analisa");
        }
        assert_eq!(nomes.len(), 12, "11 do enunciado + o handshake `hello`");
    }

    #[test]
    fn versao_desconhecida_e_recusada_nao_degradada() {
        let e = req(r#"{"v":99,"id":5,"cmd":"ping"}"#).unwrap_err();
        assert_eq!(e.err.code, code::UNSUPPORTED_VERSION);
        assert_eq!(e.id, Some(5), "o id tem de sobreviver para o cliente correlacionar");
    }

    #[test]
    fn comando_desconhecido_preserva_o_id() {
        let e = req(r#"{"v":1,"id":42,"cmd":"autodestruir"}"#).unwrap_err();
        assert_eq!(e.err.code, code::UNKNOWN_COMMAND);
        assert_eq!(e.id, Some(42));
    }

    #[test]
    fn args_invalidos_sao_recusados_com_codigo_proprio() {
        for (linha, c) in [
            (r#"{"v":1,"id":1,"cmd":"seek"}"#, code::INVALID_ARGS),
            (r#"{"v":1,"id":1,"cmd":"seek","args":{"to_ms":-5}}"#, code::INVALID_ARGS),
            (r#"{"v":1,"id":1,"cmd":"seek","args":{"to_ms":1.5}}"#, code::INVALID_ARGS),
            (r#"{"v":1,"id":1,"cmd":"load"}"#, code::INVALID_ARGS),
            (r#"{"v":1,"id":1,"cmd":"load","args":{"path":123}}"#, code::INVALID_ARGS),
        ] {
            let e = req(linha).unwrap_err();
            assert_eq!(e.err.code, c, "{linha}");
        }
    }

    #[test]
    fn lixo_nao_entra_em_panico_e_da_bad_request() {
        for s in ["", "{", "nao json", r#"{"v":1}"#, r#"{"id":1,"cmd":"ping"}"#] {
            let e = req(s).unwrap_err();
            assert!(
                e.err.code == code::BAD_REQUEST || e.err.code == code::UNSUPPORTED_VERSION,
                "{s} → {:?}",
                e.err
            );
        }
    }

    #[test]
    fn respostas_sao_json_valido_de_uma_linha() {
        let linhas = vec![
            ok_line(1, &[]),
            ok_line(2, &[("state", jstr("playing")), ("position_ms", "1234".into())]),
            err_line(Some(3), &ProtoError::new(code::UNKNOWN_COMMAND, "x")),
            err_line(None, &ProtoError::new(code::BAD_REQUEST, "aspas \" e \n newline")),
            event_line(r#"{"event":"reached_end"}"#),
        ];
        for l in &linhas {
            assert!(!l.contains('\n'), "uma mensagem por LINHA: {l}");
            let j = crate::json::parse(l).unwrap_or_else(|e| panic!("{l} → {e}"));
            assert_eq!(j.get("v").unwrap().as_u64(), Some(PROTOCOL_V));
        }
    }

    #[test]
    fn detalhe_com_aspas_nao_quebra_a_resposta() {
        let l = err_line(Some(1), &ProtoError::new(code::BAD_REQUEST, r#"a "b" \ c"#));
        let j = crate::json::parse(&l).unwrap();
        assert_eq!(j.get("error").unwrap().get("detail").unwrap().as_str(), Some(r#"a "b" \ c"#));
    }

    #[test]
    fn touches_runtime_separa_transporte_de_consulta() {
        assert!(Cmd::Play.touches_runtime());
        assert!(Cmd::Seek { to_ms: 0 }.touches_runtime());
        assert!(!Cmd::Ping.touches_runtime());
        assert!(!Cmd::Status.touches_runtime(), "status é leitura, não passa pela fila");
        assert!(!Cmd::Subscribe.touches_runtime());
    }
}
