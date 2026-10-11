//! TD-030 (accept v2, §3.6 e §2.1): estruturais sobre o texto-fonte do caminho de `scalars()`.
//!
//! - **P3:** nenhum corpo do caminho do render adquire lock (`lock(`, `.read(`, `.write(`,
//!   `Mutex`, `RwLock`).
//! - **Escritor único:** `Escritor` não deriva nem implementa `Clone`/`Copy`, e `novo` é o
//!   único sítio que constrói um `Escritor`.
//!
//! Cada assinatura procurada tem de existir EXATAMENTE uma vez no fonte: zero ou duas
//! ocorrências reprovam (uma isca com o mesmo nome não pode esconder a verdadeira — lição do
//! FALSE_GREEN C2/C3 do R8.5c).

const REACTIVE: &str = include_str!("../src/reactive.rs");
const SEQLOCK: &str = include_str!("../src/seqlock.rs");

/// Remove comentários de linha (`//`) e de bloco (`/* */`) para que texto em comentário não
/// conte nem esconda nada.
fn sem_comentarios(fonte: &str) -> String {
    let mut out = String::with_capacity(fonte.len());
    let b = fonte.as_bytes();
    let (mut i, mut em_str) = (0, false);
    while i < b.len() {
        if em_str {
            if b[i] == b'\\' && i + 1 < b.len() {
                out.push(b[i] as char);
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if b[i] == b'"' {
                em_str = false;
            }
            out.push(b[i] as char);
            i += 1;
        } else if b[i] == b'"' {
            em_str = true;
            out.push('"');
            i += 1;
        } else if fonte[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if fonte[i..].starts_with("/*") {
            let fim = fonte[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(b.len());
            i = fim;
        } else {
            let c = fonte[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// O corpo (`{ … }`) da ÚNICA ocorrência de `assinatura`. Zero ou mais de uma → falha.
fn corpo_unico(fonte: &str, assinatura: &str) -> String {
    let n = fonte.matches(assinatura).count();
    assert_eq!(n, 1, "`{assinatura}` tem de aparecer exatamente 1 vez (fora de comentários); aparece {n}");
    let inicio = fonte.find(assinatura).unwrap();
    let abre = fonte[inicio..].find('{').unwrap() + inicio;
    let mut prof = 0usize;
    for (i, c) in fonte[abre..].char_indices() {
        match c {
            '{' => prof += 1,
            '}' => {
                prof -= 1;
                if prof == 0 {
                    return fonte[abre..=abre + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("corpo de `{assinatura}` sem fecho");
}

const PROIBIDO_NO_RENDER: [&str; 5] = ["lock(", ".read(", ".write(", "Mutex", "RwLock"];

#[test]
fn caminho_de_scalars_sem_lock() {
    let r = sem_comentarios(REACTIVE);
    let s = sem_comentarios(SEQLOCK);
    let corpos = [
        ("scalars", corpo_unico(&r, "pub fn scalars(&self) -> AudioScalars")),
        ("scalars_com_tentativas", corpo_unico(&r, "fn scalars_com_tentativas(&self")),
        ("fallback", corpo_unico(&r, "fn fallback(&self) -> AudioScalars")),
        ("escolher_fallback", corpo_unico(&r, "fn escolher_fallback(")),
        ("descodificar", corpo_unico(&r, "fn descodificar(")),
        ("Leitor::ler", corpo_unico(&s, "pub(crate) fn ler(&self")),
        ("Leitor::tentar", corpo_unico(&s, "pub(crate) fn tentar(&self")),
    ];
    for (nome, corpo) in &corpos {
        for p in PROIBIDO_NO_RENDER {
            assert!(!corpo.contains(p), "{nome} contém `{p}` — lock no caminho do render (RT-LOCK-RENDER-001)");
        }
    }
    // scalars() lê pelo latch, com K = TENTATIVAS_DO_LEITOR, e só por ele.
    assert!(corpos[0].1.contains("self.scalars_com_tentativas(TENTATIVAS_DO_LEITOR)"), "scalars() tem de usar K");
    assert!(corpos[1].1.contains("self.leitor.ler(k)"), "scalars_com_tentativas tem de ler o latch");
}

/// Só o código de produção: corta no primeiro módulo de testes.
fn producao(fonte: &str) -> &str {
    let corte = fonte.find("#[cfg(all(test").expect("o seqlock tem módulos de teste");
    &fonte[..corte]
}

#[test]
fn escritor_unico_por_estrutura() {
    let limpo = sem_comentarios(SEQLOCK);
    let s = producao(&limpo);
    let decl = "pub(crate) struct Escritor<";
    assert_eq!(s.matches(decl).count(), 1, "uma só declaração de Escritor");
    // Os atributos imediatamente antes da declaração não podem derivar Clone/Copy.
    let antes = &s[..s.find(decl).unwrap()];
    let atributos: String = antes.lines().rev().take_while(|l| l.trim().starts_with("#[") || l.trim().is_empty()).collect();
    assert!(!atributos.contains("Clone") && !atributos.contains("Copy"), "Escritor não pode derivar Clone/Copy: {atributos}");
    assert!(!s.contains("Clone for Escritor") && !s.contains("Copy for Escritor"), "Escritor não pode implementar Clone/Copy");
    // Um só sítio constrói um Escritor: o `novo`.
    assert_eq!(s.matches("Escritor { latch").count(), 1, "só `novo` constrói um Escritor");
    let novo = corpo_unico(s, "pub(crate) fn novo<");
    assert!(novo.contains("Escritor { latch"), "o Escritor é construído em `novo`");
    // A escrita só existe no Escritor e exige &mut.
    assert_eq!(s.matches("fn escrever(").count(), 1);
    assert!(s.contains("pub(crate) fn escrever(&mut self"), "escrever exige &mut self");
    assert!(!s.contains(".store(") || {
        // todos os `.store(` estão dentro de `escrever`
        let esc = corpo_unico(s, "pub(crate) fn escrever(&mut self");
        s.matches(".store(").count() == esc.matches(".store(").count()
    }, "só `escrever` faz store no latch");
}

/// Controlo negativo dos próprios detetores (KB-012): uma isca e um lock plantados TÊM de
/// ser vistos.
#[test]
fn os_detetores_veem_o_que_foi_plantado() {
    let isca = "pub fn scalars(&self) -> AudioScalars { 1 }\n/* pub fn scalars(&self) -> AudioScalars { 2 } */\npub fn scalars(&self) -> AudioScalars { self.m.lock() }";
    let limpo = sem_comentarios(isca);
    assert_eq!(limpo.matches("pub fn scalars(&self) -> AudioScalars").count(), 2, "comentário removido, duas reais contam");
    let r = std::panic::catch_unwind(|| corpo_unico(&limpo, "pub fn scalars(&self) -> AudioScalars"));
    assert!(r.is_err(), "duas ocorrências têm de reprovar");
    let um = sem_comentarios("pub fn scalars(&self) -> AudioScalars { self.m.lock().x }");
    let corpo = corpo_unico(&um, "pub fn scalars(&self) -> AudioScalars");
    assert!(PROIBIDO_NO_RENDER.iter().any(|p| corpo.contains(p)), "lock plantado tem de ser visto");
}
