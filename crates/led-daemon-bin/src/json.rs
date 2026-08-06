//! Parser JSON mínimo — **a superfície exposta ao socket**.
//!
//! O workspace já emite JSON à mão (`ReadModel::to_json`, `MetricsEmitter`) e já tem
//! precedente de parser próprio (`led-xlights` faz XML). O que faltava era **ler**.
//!
//! ## Isto lê bytes de um socket, e é escrito com essa consciência
//!
//! Um parser de protocolo é código adversarial por definição: quem escreve do outro lado
//! pode não ser amigável. Daí três decisões deliberadas:
//!
//! 1. **Limite de profundidade** ([`MAX_DEPTH`]). Sem ele, `[[[[[…` recursivo estoura a pilha
//!    do processo — um cliente derrubaria o daemon com uma linha de texto.
//! 2. **Limite de tamanho** aplicado pelo chamador (ver `server`), não aqui: o parser não
//!    decide política de rede.
//! 3. **Nunca entra em pânico.** Toda a entrada malformada vira `Err`. Um `unwrap` neste
//!    ficheiro seria um vetor de negação de serviço.

use std::collections::BTreeMap;
use std::fmt;

/// Profundidade máxima de aninhamento. O protocolo v1 usa 2 (objeto + `args`); 16 dá folga
/// enorme para evolução e continua muito longe de estourar a pilha.
pub const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Números são guardados como `f64`, como manda o JSON. [`Json::as_u64`] faz a conversão
    /// **com verificação** — inteiro exato, não-negativo e dentro de alcance.
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// `BTreeMap` e não `Vec<(String, Json)>`: chaves ordenadas dão serialização estável e
    /// lookup O(log n). Chave repetida fica com o **último** valor, como a maioria dos
    /// parsers.
    Obj(BTreeMap<String, Json>),
}

#[derive(Debug, PartialEq)]
pub struct ParseError {
    pub at: usize,
    pub what: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON inválido na posição {}: {}", self.at, self.what)
    }
}

impl Json {
    pub fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(k),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Inteiro não-negativo **exato**. Recusa fracionários, negativos, `NaN`, infinitos e
    /// tudo acima de `u64::MAX` — um `as u64` cru aceitaria `1.9` como `1` e `-1.0` como um
    /// número enorme.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) => {
                if !n.is_finite() || *n < 0.0 || n.fract() != 0.0 || *n > u64::MAX as f64 {
                    None
                } else {
                    Some(*n as u64)
                }
            }
            _ => None,
        }
    }
}

// ── Serialização ─────────────────────────────────────────────────────────────

/// Escapa uma string para JSON, incluindo os controlos que **têm** de ser escapados.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{b}"),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{n}")
                }
            }
            Json::Str(s) => write!(f, "\"{}\"", escape(s)),
            Json::Arr(a) => {
                f.write_str("[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Json::Obj(m) => {
                f.write_str("{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "\"{}\":{v}", escape(k))?;
                }
                f.write_str("}")
            }
        }
    }
}

// ── Parser ───────────────────────────────────────────────────────────────────

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

pub fn parse(s: &str) -> Result<Json, ParseError> {
    let mut p = P { b: s.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        // Lixo depois do valor é erro, não é ignorado: `{"a":1} DROP TABLE` não pode passar
        // como um objeto válido.
        return Err(p.err("lixo depois do valor"));
    }
    Ok(v)
}

impl<'a> P<'a> {
    fn err(&self, what: &'static str) -> ParseError {
        ParseError { at: self.i, what }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn lit(&mut self, s: &str) -> bool {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("aninhamento demasiado profundo"));
        }
        match self.peek().ok_or_else(|| self.err("fim inesperado"))? {
            b'n' if self.lit("null") => Ok(Json::Null),
            b't' if self.lit("true") => Ok(Json::Bool(true)),
            b'f' if self.lit("false") => Ok(Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' => self.array(depth),
            b'{' => self.object(depth),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.err("token inesperado")),
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1; // "
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("string não terminada"))?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("escape truncado"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let h = self
                                .b
                                .get(self.i..self.i + 4)
                                .ok_or_else(|| self.err("\\u truncado"))?;
                            let s = std::str::from_utf8(h).map_err(|_| self.err("\\u inválido"))?;
                            let n =
                                u32::from_str_radix(s, 16).map_err(|_| self.err("\\u não-hex"))?;
                            self.i += 4;
                            // Substitutos isolados viram U+FFFD em vez de erro: são JSON
                            // legal e recusar a mensagem inteira seria mais frágil.
                            out.push(char::from_u32(n).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("escape desconhecido")),
                    }
                }
                c if c < 0x20 => return Err(self.err("controlo cru na string")),
                c => {
                    // Reconstrói UTF-8 multi-byte a partir do primeiro byte.
                    let extra = match c {
                        0x00..=0x7F => 0,
                        0xC0..=0xDF => 1,
                        0xE0..=0xEF => 2,
                        0xF0..=0xF7 => 3,
                        _ => return Err(self.err("UTF-8 inválido")),
                    };
                    let start = self.i - 1;
                    self.i += extra;
                    let raw = self
                        .b
                        .get(start..self.i)
                        .ok_or_else(|| self.err("UTF-8 truncado"))?;
                    out.push_str(std::str::from_utf8(raw).map_err(|_| self.err("UTF-8 inválido"))?);
                }
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')) {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("número"))?;
        s.parse::<f64>().map(Json::Num).map_err(|_| ParseError { at: start, what: "número inválido" })
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.i += 1;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Arr(out));
        }
        loop {
            self.ws();
            out.push(self.value(depth + 1)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(out));
                }
                _ => return Err(self.err("esperava ',' ou ']'")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.i += 1;
        let mut m = BTreeMap::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Obj(m));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("chave tem de ser string"));
            }
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.err("esperava ':'"));
            }
            self.i += 1;
            self.ws();
            let v = self.value(depth + 1)?;
            m.insert(k, v);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                _ => return Err(self.err("esperava ',' ou '}'")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseia_o_shape_do_protocolo() {
        let v = parse(r#"{"v":1,"id":7,"cmd":"seek","args":{"to_ms":1000}}"#).unwrap();
        assert_eq!(v.get("v").unwrap().as_u64(), Some(1));
        assert_eq!(v.get("id").unwrap().as_u64(), Some(7));
        assert_eq!(v.get("cmd").unwrap().as_str(), Some("seek"));
        assert_eq!(v.get("args").unwrap().get("to_ms").unwrap().as_u64(), Some(1000));
    }

    #[test]
    fn as_u64_recusa_o_que_nao_e_inteiro_nao_negativo() {
        assert_eq!(parse("1.9").unwrap().as_u64(), None, "fracionário");
        assert_eq!(parse("-1").unwrap().as_u64(), None, "negativo");
        assert_eq!(parse("1e400").unwrap().as_u64(), None, "infinito");
        assert_eq!(parse(r#""7""#).unwrap().as_u64(), None, "string não é número");
        assert_eq!(parse("0").unwrap().as_u64(), Some(0));
    }

    /// **O guarda que impede um cliente de derrubar o daemon com uma linha de texto.**
    #[test]
    fn aninhamento_profundo_e_recusado_sem_estourar_a_pilha() {
        let fundo = "[".repeat(MAX_DEPTH + 5) + &"]".repeat(MAX_DEPTH + 5);
        assert!(parse(&fundo).is_err(), "tem de recusar, não entrar em pânico");
        let raso = "[".repeat(3) + &"]".repeat(3);
        assert!(parse(&raso).is_ok());
    }

    #[test]
    fn entradas_malformadas_nunca_entram_em_panico() {
        for s in [
            "", "{", "}", "[", r#"{"a"}"#, r#"{"a":}"#, r#"{a:1}"#, r#""nao terminada"#,
            r#""\q""#, r#""\u12""#, "{\"a\":\"\x01\"}", "tru", "01x", "--1", "{} lixo",
            r#"{"a":1,}"#, "[1,]", "\u{feff}{}",
        ] {
            let _ = parse(s); // o contrato é: devolve, nunca entra em pânico
            assert!(parse(s).is_err(), "devia recusar: {s:?}");
        }
    }

    #[test]
    fn lixo_depois_do_valor_e_recusado() {
        assert!(parse(r#"{"a":1} DROP TABLE"#).is_err());
    }

    #[test]
    fn roundtrip_preserva_o_conteudo() {
        for s in [
            r#"{"a":1,"b":"x","c":true,"d":null,"e":[1,2,3]}"#,
            r#"{"nested":{"deep":{"ok":1}}}"#,
        ] {
            let v = parse(s).unwrap();
            let out = v.to_string();
            assert_eq!(parse(&out).unwrap(), v, "roundtrip falhou: {out}");
        }
    }

    #[test]
    fn escape_cobre_controlos_e_aspas() {
        let v = Json::Str("a\"b\\c\nd\te\u{1}".into());
        let s = v.to_string();
        assert!(!s[1..s.len() - 1].contains('\n'), "newline cru quebraria o enquadramento");
        assert!(s.contains("\\u0001"));
        assert_eq!(parse(&s).unwrap(), v);
    }

    #[test]
    fn utf8_atravessa_intacto() {
        let v = parse(r#"{"s":"robô ★ ação"}"#).unwrap();
        assert_eq!(v.get("s").unwrap().as_str(), Some("robô ★ ação"));
        assert_eq!(parse(&v.to_string()).unwrap(), v);
    }
}
