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

/// **Fonte única** das versões que este daemon fala (ADR-0031, decisão 2).
///
/// `PROTOCOL_V` e o `accepts` do `hello` **derivam daqui** — nenhum dos dois volta a ser um
/// literal independente. É o precedente do `OutputProtocol::ALL` (ADR-0024): a lista deriva do
/// sítio que já define o comportamento, e por isso **não pode divergir dele**. O `accepts:"[1]"`
/// escrito à mão que vivia em `server.rs` era exactamente o defeito que isto fecha — um literal
/// **sem um único leitor**, como o ADR o nomeou.
///
/// **Continua com um só elemento.** Esta fatia fixa a *derivação*, não o *conteúdo*: não cria a
/// v2, não a torna operacional, e não implementa a negociação por ligação (decisão 3).
pub const SUPORTADAS: &[u64] = &[1];

/// Versão do protocolo. Uma versão desconhecida é **recusada explicitamente**, nunca
/// degradada — a mesma regra que o `schema_version` do ADR-0018 já aplica.
///
/// Derivada de [`SUPORTADAS`]: enquanto houver uma só versão suportada, ela **é** o dialecto.
/// No dia em que houver duas, esta constante deixa de poder significar «a versão da ligação» e
/// é aí que a decisão 3 do ADR-0031 tem de ser implementada — o que **não** acontece aqui.
pub const PROTOCOL_V: u64 = SUPORTADAS[0];

/// Lista de versões suportadas, em JSON, derivada **exclusivamente** de [`SUPORTADAS`].
///
/// Existe para que a lista não volte a ser escrita à mão no sítio onde é emitida. Um literal ali
/// é indistinguível do valor correcto enquanto houver uma só versão — e passa a estar errado,
/// **em silêncio**, no dia em que houver duas.
pub fn accepts_json() -> String {
    let mut s = String::from("[");
    for (i, v) in SUPORTADAS.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&v.to_string());
    }
    s.push(']');
    s
}

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
                // ADR-0031 decisão 2: a lista é **derivada** de `SUPORTADAS`, nunca escrita à
                // mão. A forma anterior construía a lista a partir do **escalar** `PROTOCOL_V`
                // — que é só `SUPORTADAS[0]` — e ficava desactualizada no instante em que
                // houvesse uma segunda versão. Reusa `accepts_json()` em vez de um segundo
                // formatador: é o mesmo texto que o `hello` já publica.
                format!("v={v}; este daemon aceita {}", accepts_json()),
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

    /// **Fatia 1-B** — a recusa por versão nomeia a lista **derivada** de `SUPORTADAS`.
    ///
    /// *Que falsidade este teste impede?* Que a mensagem volte a construir a lista a partir de
    /// `PROTOCOL_V` (um escalar) e fique a mentir sobre o que o daemon aceita no dia em que
    /// `SUPORTADAS` ganhar uma versão.
    ///
    /// **Limite honesto:** com uma só versão suportada, «derivado» e «primeiro elemento escrito
    /// à mão» produzem **o mesmo texto** — este teste só discrimina sob a mutação
    /// `SUPORTADAS = &[1,2]`. É a mesma indistinguibilidade do ADR-0029 §8. O gate estrutural
    /// abaixo é que fecha a janela no estado de repouso.
    #[test]
    fn a_recusa_de_versao_nomeia_a_lista_derivada_de_suportadas() {
        assert!(!SUPORTADAS.is_empty(), "SUPORTADAS vazio tornaria este teste vacuoso");

        // Construído AQUI a partir da fonte, e deliberadamente **não** por `accepts_json()`:
        // chamá-la compararia a função consigo própria e passaria com qualquer valor.
        let esperado = format!(
            "[{}]",
            SUPORTADAS.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
        );

        // Uma versão garantidamente fora do conjunto, seja ele qual for — para o teste
        // continuar a exercitar o ramo da recusa sob a mutação.
        let fora = SUPORTADAS.iter().max().expect("não vazio") + 1;
        let e = req(&format!(r#"{{"v":{fora},"id":7,"cmd":"ping"}}"#)).unwrap_err();

        assert_eq!(e.err.code, code::UNSUPPORTED_VERSION);
        assert!(
            e.err.detail.contains(&esperado),
            "a recusa tem de nomear a lista derivada de SUPORTADAS; detail={:?}, esperava conter {:?}",
            e.err.detail,
            esperado
        );
    }

    /// **Fatia 1-B, gate estrutural** — ninguém volta a escrever a lista à mão no caminho da
    /// recusa. Discrimina **sempre**, incluindo com uma só versão, que é onde o teste
    /// comportamental acima é cego.
    ///
    /// Corta em `mod tests` pelo precedente de `led-console-bin` (`main.rs:254`,
    /// `surface_gate.rs:51`): um gate não pode ser o sítio onde o proibido é escrito — e este
    /// ficheiro escreve `[{PROTOCOL_V}]` no doc-comment do teste anterior.
    #[test]
    fn a_lista_de_versoes_nao_e_construida_a_mao_em_producao() {
        const FONTE: &str = include_str!("proto.rs");
        let producao = FONTE.split("mod tests").next().expect("há código antes dos testes");

        assert!(
            producao.contains("accepts_json()"),
            "extração vazia seria um gate vacuoso (KB-012): a produção tem de chamar accepts_json()"
        );

        let proibido = format!("[{{{}}}]", "PROTOCOL_V");
        assert!(
            !producao.contains(&proibido),
            "a produção de proto.rs constrói a lista de versões à mão ({proibido}); \
             deriva-a de SUPORTADAS via accepts_json()"
        );
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

#[cfg(test)]
mod contrato_id_nulo {
    use super::*;
    use crate::json::parse;

    /// Uma recusa não-atribuível é uma **resposta**, não um evento — e o que a torna
    /// distinguível é a *presença da chave* `id`, não o seu valor.
    ///
    /// O `ledctl` e o `led-console-bin` decidem com `parse(l).get("id").is_some()`. Num
    /// `{"id":null,…}` isso devolve `Some(Json::Null)` — logo, resposta. Se `err_line`
    /// alguma vez passasse a **omitir** a chave em vez de a pôr a `null`, os dois clientes
    /// passariam a lê-la como evento assíncrono e ficariam à espera para sempre de uma
    /// resposta que já tinha passado. Este teste é o que impede essa mudança silenciosa.
    #[test]
    fn id_nulo_continua_a_ser_uma_resposta_e_nao_um_evento() {
        let l = err_line(None, &ProtoError::new(code::BAD_REQUEST, "linha demasiado longa"));
        let j = parse(&l).expect("a recusa tem de ser JSON válido");

        assert!(
            j.get("id").is_some(),
            "a chave `id` tem de EXISTIR (mesmo a null), senão o cliente lê isto como evento: {l}"
        );
        assert_eq!(j.get("id"), Some(&Json::Null), "e o seu valor é null: {l}");

        // O contraste que dá sentido ao anterior: um evento **não tem** a chave.
        let ev = event_line(r#"{"event":"position_changed"}"#);
        let je = parse(&ev).expect("evento válido");
        assert!(je.get("id").is_none(), "um evento não pode ter a chave `id`: {ev}");
    }
}
