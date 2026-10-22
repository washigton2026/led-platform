//! TD-030 (accept v3): estruturais por AST (`syn`) sobre `src/reactive.rs` e `src/seqlock.rs`.
//!
//! - **P3, sem lock nem alocação no render:** o FECHO DE CHAMADAS a partir de `AudioShare::scalars` (todas as funções
//!   e métodos destes dois ficheiros alcançáveis a partir dele) não contém: métodos de lock (`lock`, `read`, `write`,
//!   `try_lock`…), operações de spinlock (`swap`, `compare_exchange*`, `fetch_*` — só o contador do `fallback` pode
//!   usar `fetch_add`), tipos de lock (`Mutex`, `RwLock`, `Condvar`, `Once*`), nem alocação (`vec!`, `format!`,
//!   `Box::new`, `Vec::…`, `String`, `to_string`, `to_owned`, `collect`).
//! - **Escritor único:** no ficheiro INTEIRO de `seqlock.rs` (incluindo os módulos de teste), `Escritor { … }` é
//!   construído exatamente uma vez, dentro de `novo`; nenhum `impl` do `Leitor` faz `store`/`swap`/`fetch_*`/
//!   `compare_exchange*`; não há macros ao nível de item. O «não é Clone/Copy» é verificado pelo compilador no teste
//!   `escritor_nao_e_clone_nem_copy` de `reactive.rs`.
//!
//! Os comentários não existem na árvore do `syn`; os argumentos das macros são analisados como expressões quando
//! possível. Cada detetor tem um controlo negativo em `os_detetores_veem_o_que_foi_plantado` (KB-012).

use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;

const REACTIVE: &str = include_str!("../src/reactive.rs");
const SEQLOCK: &str = include_str!("../src/seqlock.rs");

fn tokens(t: &impl ToTokens) -> String {
    t.to_token_stream().to_string()
}

fn e_cfg_teste(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg") && tokens(&a.meta).contains("test"))
}

/// Todas as funções (livres e métodos) de produção de um ficheiro, por nome simples → corpos (um nome pode ter mais
/// de um corpo: todos contam).
fn funcoes(f: &syn::File) -> BTreeMap<String, Vec<syn::Block>> {
    struct V(BTreeMap<String, Vec<syn::Block>>);
    impl<'ast> Visit<'ast> for V {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            if !e_cfg_teste(&m.attrs) {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
            self.0.entry(i.sig.ident.to_string()).or_default().push((*i.block).clone());
            syn::visit::visit_item_fn(self, i);
        }
        fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
            self.0.entry(i.sig.ident.to_string()).or_default().push(i.block.clone());
            syn::visit::visit_impl_item_fn(self, i);
        }
    }
    let mut v = V(BTreeMap::new());
    v.visit_file(f);
    v.0
}

/// O que um corpo chama (nomes simples de funções e métodos) e o que nele é proibido no render.
#[derive(Default)]
struct Corpo {
    chamadas: BTreeSet<String>,
    proibidos: Vec<String>,
}

const METODOS_PROIBIDOS: [&str; 14] = ["lock", "try_lock", "read", "write", "try_read", "try_write", "swap",
    "compare_exchange", "compare_exchange_weak", "fetch_update", "to_string", "to_owned", "collect", "into_boxed_slice"];
const CAMINHOS_PROIBIDOS: [&str; 12] = ["Mutex", "RwLock", "Condvar", "Once", "OnceLock", "OnceCell", "LazyLock", "Box",
    "Vec", "String", "HashMap", "BTreeMap"];
const MACROS_PROIBIDAS: [&str; 3] = ["vec", "format", "println"];

fn analisar(b: &syn::Block) -> Corpo {
    struct V(Corpo);
    impl<'ast> Visit<'ast> for V {
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*c.func {
                if let Some(s) = p.path.segments.last() {
                    self.0.chamadas.insert(s.ident.to_string());
                }
            }
            syn::visit::visit_expr_call(self, c);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            let n = m.method.to_string();
            self.0.chamadas.insert(n.clone());
            if METODOS_PROIBIDOS.contains(&n.as_str()) || (n.starts_with("fetch_") && n != "fetch_add") {
                self.0.proibidos.push(format!(".{n}("));
            }
            if n == "fetch_add" {
                self.0.proibidos.push(".fetch_add( (só permitido no fallback)".into());
            }
            syn::visit::visit_expr_method_call(self, m);
        }
        fn visit_path(&mut self, p: &'ast syn::Path) {
            for s in &p.segments {
                if CAMINHOS_PROIBIDOS.contains(&s.ident.to_string().as_str()) {
                    self.0.proibidos.push(format!("caminho {}", tokens(p)));
                }
            }
            syn::visit::visit_path(self, p);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            let n = tokens(&m.path).replace(' ', "");
            if MACROS_PROIBIDAS.contains(&n.as_str()) {
                self.0.proibidos.push(format!("{n}!"));
            }
            // os argumentos de qualquer macro, se forem expressões, também são analisados
            if let Ok(args) = m.parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated) {
                for a in &args {
                    self.visit_expr(a);
                }
            }
        }
        fn visit_item(&mut self, i: &'ast syn::Item) {
            // itens aninhados num corpo (fn, static, thread_local!…) também contam
            if let syn::Item::Static(s) = i {
                self.0.proibidos.push(format!("static aninhado {}", s.ident));
            }
            syn::visit::visit_item(self, i);
        }
    }
    let mut v = V(Corpo::default());
    v.visit_block(b);
    v.0
}

/// O fecho de chamadas a partir de `raiz`, sobre as funções dos ficheiros dados: nome → proibidos encontrados.
fn fecho(fontes: &[&str], raiz: &str) -> BTreeMap<String, Vec<String>> {
    let mut todas: BTreeMap<String, Vec<syn::Block>> = BTreeMap::new();
    for f in fontes {
        for (n, c) in funcoes(&syn::parse_file(f).expect("fonte válido")) {
            todas.entry(n).or_default().extend(c);
        }
    }
    let mut visitadas = BTreeMap::new();
    let mut pendentes = vec![raiz.to_string()];
    while let Some(n) = pendentes.pop() {
        if visitadas.contains_key(&n) {
            continue;
        }
        let Some(corpos) = todas.get(&n) else { continue };
        let mut proibidos = Vec::new();
        for c in corpos {
            let a = analisar(c);
            proibidos.extend(a.proibidos);
            pendentes.extend(a.chamadas.into_iter().filter(|c| todas.contains_key(c)));
        }
        visitadas.insert(n, proibidos);
    }
    visitadas
}

fn verificar_render(fontes: &[&str]) -> Vec<String> {
    let f = fecho(fontes, "scalars");
    assert!(f.contains_key("scalars"), "a raiz `scalars` não existe");
    let mut erros = Vec::new();
    for (n, ps) in &f {
        for p in ps {
            let permitido = n == "fallback" && p.starts_with(".fetch_add(");
            if !permitido {
                erros.push(format!("{n}: {p}"));
            }
        }
    }
    erros
}

// ── O4 (ronda 2): LISTA DE PERMITIDOS no fecho de chamadas de `scalars` ──
// A lista de proibidos acima fica como segunda camada. Aqui, tudo o que o fecho faz e não é uma função do crate (essas
// resolvem-se pelo fecho e NÃO entram nas listas) tem de estar numa lista fechada, com uma justificação por entrada.
// Cada erro começa pela regra que o deu (R1–R7, U, T), para os controlos negativos afirmarem a regra esperada.

/// Métodos externos permitidos no fecho. `fetch_add` só no `fallback` (R1).
const METODOS_PERMITIDOS: &[(&str, &str)] = &[
    ("load", "`AtomicU64::load` da versão e dos slots do latch (`tentar`): leitura atómica, sem lock nem alocação"),
    ("iter", "iteração por referência sobre o array fixo de um slot (`tentar`): sem alocação"),
    ("iter_mut", "iteração mutável sobre o array local `palavras` (`tentar`): sem alocação"),
    ("zip", "emparelha os dois iteradores de arrays de tamanho fixo (`tentar`): sem alocação"),
    ("then_some", "`bool::then_some` do snapshot por valor (`tentar`): sem alocação"),
    ("with", "`LocalKey::with` sobre `ULTIMO_VALIDO` (init `const`, sem destrutor): sem alocação"),
    ("set", "`Cell::set` do último válido (`Copy`): sem alocação"),
    ("get", "`Cell::get` do último válido (`Copy`): sem alocação"),
    ("is_none", "`Option::is_none` no `fallback`"),
    ("unwrap_or_default", "`Option<AudioScalars>::unwrap_or_default`: `AudioScalars` é `Copy` e o `Default` não aloca"),
    ("fetch_add", "SÓ no `fallback`: os dois contadores de fallback (`Relaxed`); em qualquer outra função é R1"),
];
/// Caminhos externos permitidos, em posição de chamada (R3) ou de valor (R4).
const CAMINHOS_PERMITIDOS: &[(&str, &str)] = &[
    ("Some", "construtor de `Option` por valor: sem alocação"),
    ("Ok", "construtor de `Result` por valor (o `ler` da ronda 2): sem alocação"),
    ("Err", "construtor de `Result` por valor (`Esgotado`, `Copy`): sem alocação"),
    ("f32::from_bits", "reinterpretação de bits (`descodificar`): pura, sem alocação"),
    ("fence", "`std::sync::atomic::fence(Acquire)` do protocolo do leitor (`tentar`): sem lock"),
    ("Ordering::Acquire", "ordering do protocolo (carga da versão e cerca do leitor)"),
    ("Ordering::Relaxed", "ordering do protocolo (slots, 2.ª carga da versão) e dos contadores do fallback"),
    ("MusicalSection::Intro", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Verse", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Chorus", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Bridge", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Drop", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Build", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Outro", "variante sem dados (`seccao_do_codigo`)"),
    ("MusicalSection::Unknown", "variante sem dados (`seccao_do_codigo`)"),
];
/// `static`s que o fecho pode referir (R5).
const STATICS_PERMITIDOS: &[(&str, &str)] = &[
    ("ULTIMO_VALIDO", "`thread_local!` com init `const` e sem destrutor: o último snapshot válido desta thread (TD-030)"),
];
/// Macros permitidas no fecho (R6).
/// Vazia: o C0 não usa nenhuma macro no fecho.
const MACROS_PERMITIDAS: &[(&str, &str)] = &[];
/// Segmentos de caminho (de tipo ou de expressão) que alocam ou bloqueiam (R7).
const TIPOS_PROIBIDOS: [&str; 10] = ["Box", "Vec", "String", "Rc", "Arc", "Mutex", "RwLock", "Condvar", "Once", "Lazy"];
/// Raízes de caminho cujos itens não podem ser renomeados (`use … as`, U) nem aliasados (`type`, T).
const RAIZES_SENSIVEIS: [&str; 7] = ["std::sync", "std::cell", "std::alloc", "core::sync", "core::cell", "parking_lot", "spin"];

fn permitido(lista: &[(&str, &str)], n: &str) -> bool {
    lista.iter().any(|(e, _)| *e == n)
}

fn caminho(p: &syn::Path) -> String {
    tokens(p).replace(' ', "")
}

/// O índice do crate nas fontes dadas (fora dos módulos `cfg(test)`).
#[derive(Default)]
struct Indice {
    livres: BTreeMap<String, Vec<syn::Block>>,
    /// métodos com receptor (`self`): os únicos que uma chamada de método pode resolver
    metodos: BTreeMap<String, Vec<syn::Block>>,
    /// todas as funções de `impl` (com ou sem `self`): resolvem `Self::x` / `Tipo::x`
    associadas: BTreeMap<String, Vec<syn::Block>>,
    tipos: BTreeSet<String>,
    statics: BTreeSet<String>,
    /// campo → último segmento do tipo declarado, em todas as structs das fontes (R1: recetor `self.<campo>`)
    campos: BTreeMap<String, Vec<String>>,
    /// nomes trazidos por `use` em QUALQUER sítio do ficheiro (módulo, função, testes); globs não enumeráveis (R4)
    importados: BTreeSet<String>,
}

fn indexar(fontes: &[&str]) -> Indice {
    struct V<'a>(&'a mut Indice);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            if !e_cfg_teste(&m.attrs) {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
            self.0.livres.entry(i.sig.ident.to_string()).or_default().push((*i.block).clone());
            syn::visit::visit_item_fn(self, i);
        }
        fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
            let n = i.sig.ident.to_string();
            self.0.associadas.entry(n.clone()).or_default().push(i.block.clone());
            if i.sig.receiver().is_some() {
                self.0.metodos.entry(n).or_default().push(i.block.clone());
            }
            syn::visit::visit_impl_item_fn(self, i);
        }
        fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
            self.0.tipos.insert(s.ident.to_string());
            for c in &s.fields {
                if let Some(n) = &c.ident {
                    let t = match &c.ty {
                        syn::Type::Path(tp) => tp.path.segments.last().map(|g| g.ident.to_string()).unwrap_or_default(),
                        outro => tokens(outro),
                    };
                    self.0.campos.entry(n.to_string()).or_default().push(t);
                }
            }
            syn::visit::visit_item_struct(self, s);
        }
        fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
            self.0.tipos.insert(e.ident.to_string());
            syn::visit::visit_item_enum(self, e);
        }
        fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
            self.0.statics.insert(s.ident.to_string());
            syn::visit::visit_item_static(self, s);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            // `static` declarados dentro de macros (`thread_local!`, …)
            let t = tokens(&m.tokens);
            let ws: Vec<&str> = t.split_whitespace().filter(|w| *w != "mut").collect();
            for w in ws.windows(2) {
                if w[0] == "static" {
                    self.0.statics.insert(w[1].to_string());
                }
            }
        }
    }
    /// Os nomes importados vêm do ficheiro INTEIRO, incluindo os módulos de teste e os `use` dentro de funções.
    struct U<'a>(&'a mut BTreeSet<String>);
    impl<'ast> Visit<'ast> for U<'_> {
        fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
            fn arvore(t: &syn::UseTree, out: &mut BTreeSet<String>) {
                match t {
                    syn::UseTree::Path(p) => arvore(&p.tree, out),
                    syn::UseTree::Name(n) if n.ident != "self" => { out.insert(n.ident.to_string()); }
                    syn::UseTree::Rename(r) => { out.insert(r.rename.to_string()); }
                    syn::UseTree::Group(g) => g.items.iter().for_each(|i| arvore(i, out)),
                    _ => {}
                }
            }
            arvore(&u.tree, self.0);
        }
    }
    let mut ix = Indice::default();
    for f in fontes {
        let arvore = syn::parse_file(f).expect("fonte válido");
        V(&mut ix).visit_file(&arvore);
        U(&mut ix.importados).visit_file(&arvore);
    }
    ix
}

/// R8: um `for` só se aceita sobre um intervalo ou um iterador de array (`.iter()`/`.iter_mut()` sobre um caminho,
/// campo ou índice, e `zip` de dois destes).
fn iterador_de_array(e: &syn::Expr) -> bool {
    fn simples(e: &syn::Expr) -> bool {
        match e {
            syn::Expr::Path(p) => p.qself.is_none(),
            syn::Expr::Field(f) => simples(&f.base),
            syn::Expr::Index(i) => simples(&i.expr),
            syn::Expr::Reference(r) => simples(&r.expr),
            syn::Expr::Paren(p) => simples(&p.expr),
            _ => false,
        }
    }
    match e {
        syn::Expr::Range(_) => true,
        syn::Expr::MethodCall(m) if m.method == "iter" || m.method == "iter_mut" => m.args.is_empty() && simples(&m.receiver),
        syn::Expr::MethodCall(m) if m.method == "zip" => {
            m.args.len() == 1 && iterador_de_array(&m.receiver) && iterador_de_array(&m.args[0])
        }
        _ => false,
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Alvo {
    Livre(String),
    Metodo(String),
    Associada(String),
}

impl Alvo {
    fn nome(&self) -> &str {
        match self {
            Alvo::Livre(n) | Alvo::Metodo(n) | Alvo::Associada(n) => n,
        }
    }
}

struct Analise<'a> {
    ix: &'a Indice,
    fn_atual: String,
    alvos: Vec<Alvo>,
    erros: Vec<String>,
}

impl Analise<'_> {
    /// Um caminho com prefixo do próprio crate (`Self::x`, `crate::…::x`, `seqlock::x`, `Tipo::x`) → a função do crate.
    fn do_crate(&self, p: &syn::Path) -> Option<Alvo> {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        let ultimo = segs.last()?.clone();
        if segs.len() == 1 {
            return self.ix.livres.contains_key(&ultimo).then_some(Alvo::Livre(ultimo));
        }
        let raiz = segs[0].as_str();
        let crate_ish = matches!(raiz, "Self" | "crate" | "super" | "self" | "seqlock" | "reactive")
            || self.ix.tipos.contains(&segs[segs.len() - 2]);
        if !crate_ish {
            return None;
        }
        if self.ix.associadas.contains_key(&ultimo) {
            Some(Alvo::Associada(ultimo))
        } else if self.ix.livres.contains_key(&ultimo) {
            Some(Alvo::Livre(ultimo))
        } else {
            None
        }
    }

    /// R1: uma chamada de método só resolve para o crate se o recetor for `self`, ou `self.<campo>` cujo tipo declarado
    /// (em TODAS as structs das fontes que têm esse campo) seja um tipo do crate.
    fn recetor_do_crate(&self, r: &syn::Expr) -> bool {
        let e_self = |e: &syn::Expr| matches!(e, syn::Expr::Path(p) if p.qself.is_none() && p.path.is_ident("self"));
        match r {
            syn::Expr::Field(f) => e_self(&f.base) && matches!(&f.member, syn::Member::Named(n)
                if self.ix.campos.get(&n.to_string()).is_some_and(|ts| !ts.is_empty() && ts.iter().all(|t| self.ix.tipos.contains(t)))),
            outro => e_self(outro),
        }
    }

    fn erro(&mut self, regra: &str, o_que: String) {
        self.erros.push(format!("{regra}: {} — {o_que}", self.fn_atual));
    }
}

impl<'ast> Visit<'ast> for Analise<'_> {
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let n = m.method.to_string();
        if self.ix.metodos.contains_key(&n) && self.recetor_do_crate(&m.receiver) {
            self.alvos.push(Alvo::Metodo(n));
        } else if !permitido(METODOS_PERMITIDOS, &n) {
            self.erro("R1", format!("método `.{n}(` fora de METODOS_PERMITIDOS"));
        } else if n == "fetch_add" && self.fn_atual != "fallback" {
            self.erro("R1", "`.fetch_add(` fora do `fallback`".into());
        }
        syn::visit::visit_expr_method_call(self, m);
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        match &*c.func {
            syn::Expr::Path(p) if p.qself.is_none() => {
                if let Some(a) = self.do_crate(&p.path) {
                    self.alvos.push(a);
                } else if !permitido(CAMINHOS_PERMITIDOS, &caminho(&p.path)) {
                    self.erro("R3", format!("chamada a `{}` não resolve nem para o crate nem para CAMINHOS_PERMITIDOS",
                                            caminho(&p.path)));
                }
                self.visit_path(&p.path); // R7 sobre os segmentos (o callee não é um «valor»: sem R4)
            }
            syn::Expr::Path(p) => {
                self.erro("R3", format!("chamada por caminho qualificado `{}`", tokens(p)));
                syn::visit::visit_expr(self, &c.func);
            }
            outro => {
                self.erro("R2", format!("chamada sobre algo que não é um caminho: `{}`", tokens(outro)));
                syn::visit::visit_expr(self, &c.func);
            }
        }
        for a in &c.args {
            self.visit_expr(a);
        }
    }
    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        let c = caminho(&p.path);
        let ultimo = p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if p.qself.is_some() {
            self.erro("R4", format!("caminho qualificado como valor `{}`", tokens(p)));
        } else if self.ix.statics.contains(&ultimo) {
            if !permitido(STATICS_PERMITIDOS, &ultimo) {
                self.erro("R5", format!("`static` `{c}` fora de STATICS_PERMITIDOS"));
            }
        } else if self.do_crate(&p.path).is_some() {
            self.erro("R4", format!("função do crate `{c}` usada como valor"));
        } else if p.path.segments.len() > 1 && !permitido(CAMINHOS_PERMITIDOS, &c) {
            self.erro("R4", format!("caminho `{c}` usado como valor fora de CAMINHOS_PERMITIDOS"));
        } else if p.path.segments.len() == 1 && self.ix.importados.contains(&ultimo) && !permitido(CAMINHOS_PERMITIDOS, &c) {
            self.erro("R4", format!("nome importado `{c}` usado como valor fora de CAMINHOS_PERMITIDOS"));
        }
        syn::visit::visit_expr_path(self, p);
    }
    fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
        self.erro("R8", format!("`while` no fecho: `while {}`", tokens(&e.cond)));
        syn::visit::visit_expr_while(self, e);
    }
    fn visit_expr_loop(&mut self, e: &'ast syn::ExprLoop) {
        self.erro("R8", "`loop` no fecho".into());
        syn::visit::visit_expr_loop(self, e);
    }
    fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
        if !iterador_de_array(&e.expr) {
            self.erro("R8", format!("`for` sobre algo que não é intervalo nem iterador de array: `{}`", tokens(&e.expr)));
        }
        syn::visit::visit_expr_for_loop(self, e);
    }
    fn visit_path(&mut self, p: &'ast syn::Path) {
        for s in &p.segments {
            let n = s.ident.to_string();
            if TIPOS_PROIBIDOS.iter().any(|t| n == *t || (matches!(*t, "Once" | "Lazy") && n.starts_with(t))) {
                self.erro("R7", format!("segmento `{n}` em `{}`", caminho(p)));
            }
        }
        syn::visit::visit_path(self, p);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let n = caminho(&m.path);
        if !permitido(MACROS_PERMITIDAS, &n) {
            self.erro("R6", format!("macro `{n}!` fora de MACROS_PERMITIDAS"));
        }
        if let Ok(args) = m.parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated) {
            for a in &args {
                self.visit_expr(a);
            }
        }
    }
    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.erro("R5", format!("`static` aninhado `{}`", s.ident));
        syn::visit::visit_item_static(self, s);
    }
}

/// Raiz de cada `use … as …` e de cada `extern crate … as …` do ficheiro inteiro.
fn renomeacoes(f: &syn::File) -> Vec<String> {
    fn arvore(prefixo: &str, t: &syn::UseTree, out: &mut Vec<String>) {
        match t {
            syn::UseTree::Path(p) => arvore(&format!("{prefixo}{}::", p.ident), &p.tree, out),
            syn::UseTree::Rename(r) => out.push(format!("{prefixo}{} as {}", r.ident, r.rename)),
            syn::UseTree::Group(g) => g.items.iter().for_each(|i| arvore(prefixo, i, out)),
            _ => {}
        }
    }
    struct V(Vec<String>, Vec<String>);
    impl<'ast> Visit<'ast> for V {
        fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
            arvore("", &u.tree, &mut self.0);
        }
        fn visit_item_extern_crate(&mut self, e: &'ast syn::ItemExternCrate) {
            if let Some((_, r)) = &e.rename {
                self.0.push(format!("{} as {r}", e.ident));
            }
        }
        fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
            self.1.push(format!("type {} = {}", t.ident, tokens(&t.ty).replace(' ', "")));
            syn::visit::visit_item_type(self, t);
        }
        fn visit_impl_item_type(&mut self, t: &'ast syn::ImplItemType) {
            self.1.push(format!("type {} = {}", t.ident, tokens(&t.ty).replace(' ', "")));
            syn::visit::visit_impl_item_type(self, t);
        }
    }
    let mut v = V(vec![], vec![]);
    v.visit_file(f);
    let mut erros = Vec::new();
    for r in v.0 {
        if RAIZES_SENSIVEIS.iter().any(|raiz| r.starts_with(raiz)) || r.starts_with("std ") || r.starts_with("core ")
            || r.starts_with("alloc") {
            erros.push(format!("U: `use {r}` renomeia um item de sincronização/alocação"));
        }
    }
    for t in v.1 {
        let ty = t.split_once(" = ").map_or("", |(_, ty)| ty);
        let sensivel = RAIZES_SENSIVEIS.iter().any(|raiz| ty.contains(raiz))
            || ["Mutex", "RwLock", "Condvar", "Cell", "Once", "Lazy", "Arc", "Rc", "Box", "Vec", "String", "alloc"]
                .iter().any(|s| ty.contains(s));
        if sensivel {
            erros.push(format!("T: `{t}` é um alias sobre um item de sincronização/alocação"));
        }
    }
    erros
}

/// O4: (funções alcançadas a partir de `AudioShare::scalars`, erros).
fn verificar_permitidos(fontes: &[&str]) -> (BTreeSet<String>, Vec<String>) {
    let ix = indexar(fontes);
    let mut erros = Vec::new();
    for f in fontes {
        erros.extend(renomeacoes(&syn::parse_file(f).expect("fonte válido")));
    }
    let mut visitados: BTreeSet<Alvo> = BTreeSet::new();
    let mut pendentes = vec![Alvo::Metodo("scalars".into())];
    while let Some(a) = pendentes.pop() {
        if !visitados.insert(a.clone()) {
            continue;
        }
        let corpos = match &a {
            Alvo::Livre(n) => ix.livres.get(n),
            Alvo::Metodo(n) => ix.metodos.get(n),
            Alvo::Associada(n) => ix.associadas.get(n),
        };
        for c in corpos.into_iter().flatten() {
            let mut an = Analise { ix: &ix, fn_atual: a.nome().to_string(), alvos: vec![], erros: vec![] };
            an.visit_block(c);
            erros.extend(an.erros);
            pendentes.extend(an.alvos);
        }
    }
    (visitados.iter().map(|a| a.nome().to_string()).collect(), erros)
}

/// Os erros de uma regra (o prefixo `Rn:`/`U:`/`T:`).
fn da_regra(erros: &[String], regra: &str) -> usize {
    erros.iter().filter(|e| e.starts_with(&format!("{regra}:"))).count()
}

#[test]
fn render_sem_lock_nem_alocacao_no_fecho_de_chamadas() {
    let f = fecho(&[REACTIVE, SEQLOCK], "scalars");
    let alcancadas: Vec<&String> = f.keys().collect();
    eprintln!("FECHO de scalars: {alcancadas:?}");
    for esperada in ["scalars", "scalars_com_tentativas", "ler", "tentar", "descodificar", "seccao_do_codigo", "fallback",
                     "escolher_fallback"] {
        assert!(f.contains_key(esperada), "o fecho tem de incluir {esperada}: {alcancadas:?}");
    }
    let erros = verificar_render(&[REACTIVE, SEQLOCK]);
    assert!(erros.is_empty(), "lock/alocação no caminho do render: {erros:?}");
    // O4: a lista de permitidos, com o seu próprio fecho (resolvido por caminho)
    let (alcancadas, erros) = verificar_permitidos(&[REACTIVE, SEQLOCK]);
    eprintln!("FECHO (permitidos) de scalars: {alcancadas:?}");
    for esperada in ["scalars", "scalars_com_tentativas", "ler", "tentar", "descodificar", "seccao_do_codigo", "fallback",
                     "escolher_fallback"] {
        assert!(alcancadas.contains(esperada), "o fecho (permitidos) tem de incluir {esperada}: {alcancadas:?}");
    }
    assert!(erros.is_empty(), "fora da lista de permitidos no caminho do render: {erros:#?}");
}

// ── O2 (ronda 2): o corpo de `AudioShare::scalars` é UMA expressão: `self.scalars_com_tentativas(TENTATIVAS_DO_LEITOR)` ──
fn verificar_scalars_canonico(reactive: &str) -> Vec<String> {
    let f = syn::parse_file(reactive).expect("reactive válido");
    let mut corpos = Vec::new();
    for item in &f.items {
        if let syn::Item::Impl(i) = item {
            if i.trait_.is_none() && tokens(&i.self_ty).replace(' ', "") == "AudioShare" {
                for ii in &i.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        if m.sig.ident == "scalars" {
                            corpos.push(m.block.clone());
                        }
                    }
                }
            }
        }
    }
    if corpos.len() != 1 {
        return vec![format!("AudioShare::scalars encontrado {} vez(es)", corpos.len())];
    }
    let e_caminho = |e: &syn::Expr, nome: &str| {
        matches!(e, syn::Expr::Path(p) if p.attrs.is_empty() && p.qself.is_none() && p.path.is_ident(nome))
    };
    let canonico = match corpos[0].stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::MethodCall(m), None)] => {
            m.attrs.is_empty() && m.turbofish.is_none() && e_caminho(&m.receiver, "self")
                && m.method == "scalars_com_tentativas" && m.args.len() == 1
                && e_caminho(&m.args[0], "TENTATIVAS_DO_LEITOR")
        }
        _ => false,
    };
    if canonico {
        vec![]
    } else {
        vec![format!("o corpo de scalars não é `self.scalars_com_tentativas(TENTATIVAS_DO_LEITOR)`: {}", tokens(&corpos[0]))]
    }
}

#[test]
fn scalars_e_uma_unica_chamada_com_tentativas_do_leitor() {
    let erros = verificar_scalars_canonico(REACTIVE);
    assert!(erros.is_empty(), "{erros:?}");
}

// ── O2-bis (ronda 2, adenda 1a): `Leitor::ler` conta as tentativas ──
// Um ÚNICO ciclo `for _ in 0..k` sobre o parâmetro `k`; um contador `let mut c = 0` incrementado (`c += 1`) em CADA
// iteração, como instrução de topo do ciclo e junto à única chamada a `tentar()`; e `Esgotado { tentativas: c }`,
// nunca `k` nem um literal.
fn verificar_ler_canonico(seqlock: &str) -> Vec<String> {
    let f = syn::parse_file(seqlock).expect("seqlock válido");
    let mut corpos = Vec::new();
    for item in &f.items {
        if let syn::Item::Impl(i) = item {
            if tokens(&i.self_ty).replace(' ', "").starts_with("Leitor") {
                for ii in &i.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        if m.sig.ident == "ler" {
                            corpos.push(m.block.clone());
                        }
                    }
                }
            }
        }
    }
    if corpos.len() != 1 {
        return vec![format!("Leitor::ler encontrado {} vez(es)", corpos.len())];
    }
    let corpo = &corpos[0];
    struct V { ciclos: Vec<syn::ExprForLoop>, estruturas: Vec<syn::ExprStruct>, incrementos: usize, tentar: usize }
    impl<'ast> Visit<'ast> for V {
        fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
            self.ciclos.push(e.clone());
            syn::visit::visit_expr_for_loop(self, e);
        }
        fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
            self.ciclos.push(syn::parse_quote!(for _ in 0..0 {})); // qualquer outro ciclo conta como um a mais
            syn::visit::visit_expr_while(self, e);
        }
        fn visit_expr_loop(&mut self, e: &'ast syn::ExprLoop) {
            self.ciclos.push(syn::parse_quote!(for _ in 0..0 {}));
            syn::visit::visit_expr_loop(self, e);
        }
        fn visit_expr_struct(&mut self, e: &'ast syn::ExprStruct) {
            if e.path.segments.last().is_some_and(|s| s.ident == "Esgotado") {
                self.estruturas.push(e.clone());
            }
            syn::visit::visit_expr_struct(self, e);
        }
        fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
            if matches!(e.op, syn::BinOp::AddAssign(_)) {
                self.incrementos += 1;
            }
            syn::visit::visit_expr_binary(self, e);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            if m.method == "tentar" {
                self.tentar += 1;
            }
            syn::visit::visit_expr_method_call(self, m);
        }
    }
    let mut v = V { ciclos: vec![], estruturas: vec![], incrementos: 0, tentar: 0 };
    v.visit_block(corpo);
    let mut erros = Vec::new();
    let e_caminho = |e: &syn::Expr, n: &str| matches!(e, syn::Expr::Path(p) if p.qself.is_none() && p.path.is_ident(n));
    let e_lit = |e: &syn::Expr, x: &str| matches!(e, syn::Expr::Lit(l) if tokens(&l.lit).trim_end_matches("u32") == x);
    // o contador: `let mut <c> = 0[u32];` ao nível do corpo
    let contador = corpo.stmts.iter().find_map(|st| match st {
        syn::Stmt::Local(l) => match (&l.pat, &l.init) {
            (syn::Pat::Ident(pi), Some(init)) if pi.mutability.is_some() && e_lit(&init.expr, "0") => Some(pi.ident.to_string()),
            (syn::Pat::Type(pt), Some(init)) if e_lit(&init.expr, "0") => match &*pt.pat {
                syn::Pat::Ident(pi) if pi.mutability.is_some() => Some(pi.ident.to_string()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    });
    let Some(c) = contador else { return vec!["sem contador `let mut c = 0`".into()] };
    if v.ciclos.len() != 1 {
        erros.push(format!("{} ciclo(s); tem de haver um só `for _ in 0..k`", v.ciclos.len()));
    }
    if v.incrementos != 1 || v.tentar != 1 {
        erros.push(format!("{} incremento(s) e {} chamada(s) a tentar(); tem de haver um de cada", v.incrementos, v.tentar));
    }
    if let Some(ciclo) = v.ciclos.first() {
        let faixa_ok = matches!(&*ciclo.expr, syn::Expr::Range(r)
            if matches!(r.limits, syn::RangeLimits::HalfOpen(_))
                && r.start.as_deref().is_some_and(|a| e_lit(a, "0"))
                && r.end.as_deref().is_some_and(|b| e_caminho(b, "k")));
        if !matches!(&*ciclo.pat, syn::Pat::Wild(_)) || !faixa_ok {
            erros.push(format!("o ciclo não é `for _ in 0..k`: `for {} in {}`", tokens(&ciclo.pat), tokens(&ciclo.expr)));
        }
        // o incremento é uma instrução de topo do ciclo, adjacente à instrução que chama tentar()
        let st = &ciclo.body.stmts;
        let e_incremento = |s: &syn::Stmt| matches!(s, syn::Stmt::Expr(syn::Expr::Binary(b), Some(_))
            if matches!(b.op, syn::BinOp::AddAssign(_)) && e_caminho(&b.left, &c) && e_lit(&b.right, "1"));
        let chama_tentar = |s: &syn::Stmt| tokens(s).contains("tentar ()");
        let junto = st.windows(2).any(|w| (e_incremento(&w[0]) && chama_tentar(&w[1])) || (chama_tentar(&w[0]) && e_incremento(&w[1])));
        if !junto {
            erros.push(format!("`{c} += 1` não é uma instrução do ciclo junto à chamada a tentar()"));
        }
    }
    if v.estruturas.is_empty() {
        erros.push("nenhum `Esgotado { … }`".into());
    }
    for e in &v.estruturas {
        let campo_ok = e.fields.len() == 1 && e.rest.is_none() && e.fields.iter().all(|fv|
            matches!(&fv.member, syn::Member::Named(n) if n == "tentativas") && e_caminho(&fv.expr, &c));
        if !campo_ok {
            erros.push(format!("`{}` não leva o contador `{c}`", tokens(e)));
        }
    }
    erros
}

#[test]
fn ler_conta_as_tentativas_por_estrutura() {
    let erros = verificar_ler_canonico(SEQLOCK);
    assert!(erros.is_empty(), "{erros:?}");
}

fn verificar_escritor_unico(seqlock: &str) -> Vec<String> {
    let f = syn::parse_file(seqlock).expect("seqlock válido");
    struct V { construcoes: Vec<String>, dentro_de: Vec<String>, escritas_no_leitor: Vec<String>, macros_item: usize,
               fn_atual: Vec<String>, em_leitor: bool }
    impl<'ast> Visit<'ast> for V {
        fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
            self.fn_atual.push(i.sig.ident.to_string());
            syn::visit::visit_item_fn(self, i);
            self.fn_atual.pop();
        }
        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            let alvo = tokens(&i.self_ty).replace(' ', "");
            let antes = self.em_leitor;
            self.em_leitor = alvo.ends_with("Leitor<W>") || alvo.contains("Leitor");
            syn::visit::visit_item_impl(self, i);
            self.em_leitor = antes;
        }
        fn visit_expr_struct(&mut self, e: &'ast syn::ExprStruct) {
            if e.path.segments.last().is_some_and(|s| s.ident == "Escritor") {
                self.construcoes.push(tokens(e));
                self.dentro_de.push(self.fn_atual.last().cloned().unwrap_or_default());
            }
            syn::visit::visit_expr_struct(self, e);
        }
        fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
            let n = m.method.to_string();
            if self.em_leitor && (n == "store" || n == "swap" || n.starts_with("fetch_") || n.starts_with("compare_exchange")) {
                self.escritas_no_leitor.push(n);
            }
            syn::visit::visit_expr_method_call(self, m);
        }
        fn visit_item_macro(&mut self, _: &'ast syn::ItemMacro) {
            self.macros_item += 1;
        }
    }
    let mut v = V { construcoes: vec![], dentro_de: vec![], escritas_no_leitor: vec![], macros_item: 0, fn_atual: vec![],
                    em_leitor: false };
    v.visit_file(&f);
    let mut erros = Vec::new();
    if v.construcoes.len() != 1 || v.dentro_de != ["novo"] {
        erros.push(format!("Escritor construído {} vez(es), em {:?}", v.construcoes.len(), v.dentro_de));
    }
    if !v.escritas_no_leitor.is_empty() {
        erros.push(format!("o Leitor escreve: {:?}", v.escritas_no_leitor));
    }
    if v.macros_item != 0 {
        erros.push(format!("{} macro(s) ao nível de item", v.macros_item));
    }
    erros
}

#[test]
fn escritor_unico_por_estrutura() {
    let erros = verificar_escritor_unico(SEQLOCK);
    assert!(erros.is_empty(), "{erros:?}");
}

/// Controlo negativo dos próprios detetores (KB-012): as formas L1–L5 e W3/W4/W6/W7 do falsificador TÊM de ser vistas.
#[test]
fn os_detetores_veem_o_que_foi_plantado() {
    let base_r = "struct S { leitor: L, f: u8 } impl S { pub fn scalars(&self) -> u8 { self.scalars_com_tentativas(1) } \
                  fn scalars_com_tentativas(&self, k: u32) -> u8 { match self.leitor.ler(k) { Some(_) => descodificar(), None => self.fallback() } } \
                  fn fallback(&self) -> u8 { self.f.fetch_add(1, O); escolher_fallback() } } \
                  fn escolher_fallback() -> u8 { 0 } fn descodificar() -> u8 { seccao_do_codigo() } fn seccao_do_codigo() -> u8 { 0 }";
    let base_s = "struct L {} impl L { fn ler(&self, k: u32) -> u8 { self.tentar() } fn tentar(&self) -> u8 { 0 } }";
    assert!(verificar_render(&[base_r, base_s]).is_empty(), "a base limpa passa");
    for (nome, r, s) in [
        ("L1 lock via auxiliar", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { espectro() } fn espectro() -> u8 { X.read().unwrap(); 0 }"), base_s.to_string()),
        ("L2 Mutex estático", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { static M: std::sync::Mutex<u8> = std::sync::Mutex::new(0); 0 }"), base_s.to_string()),
        ("L3 spinlock", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { while self.b.swap(true, O) {} 0 }")),
        ("L4 vec no fallback", base_r.replace("escolher_fallback() }", "let _v = vec![0u8; 64]; escolher_fallback() }"), base_s.to_string()),
        ("L5 alocação por secção", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { let _ = Box::new(1); 0 }"), base_s.to_string()),
        ("fetch_add fora do fallback", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { self.n.fetch_add(1, O); 0 }")),
        ("format! escondido noutra macro", base_r.replace("fn escolher_fallback() -> u8 { 0 }", "fn escolher_fallback() -> u8 { assert!(format!(\"{}\", 1).len() > 0); 0 }"), base_s.to_string()),
    ] {
        assert!(!verificar_render(&[&r, &s]).is_empty(), "não visto: {nome}");
    }
    // O4: a mesma base passa na lista de permitidos; cada forma reprova pela regra esperada.
    let (_, erros_base) = verificar_permitidos(&[base_r, base_s]);
    assert!(erros_base.is_empty(), "a base limpa passa na lista de permitidos: {erros_base:?}");
    let trava = "static TRAVA: u8 = 0; static TRAVA2: u8 = 0; static GIRO: u8 = 0;";
    for (nome, regra, r, s) in [
        ("X4 Mutex renomeado, UFCS", "R3", format!("use std::sync::Mutex as Trinco; {trava} {}", base_r.replace("fn scalars_com_tentativas(&self, k: u32) -> u8 { ", "fn scalars_com_tentativas(&self, k: u32) -> u8 { let _t = Trinco::lock(&TRAVA); ")), base_s.to_string()),
        ("X4 o `use … as`", "U", format!("use std::sync::Mutex as Trinco; {base_r}"), base_s.to_string()),
        ("X5 spinlock UFCS", "R3", format!("{trava} {}", base_r.replace("fn scalars_com_tentativas(&self, k: u32) -> u8 { ", "fn scalars_com_tentativas(&self, k: u32) -> u8 { while std::sync::atomic::AtomicBool::swap(&GIRO, true, O) { std::hint::spin_loop(); } ")), base_s.to_string()),
        ("X6 to_vec no fallback", "R1", base_r.replace("escolher_fallback() }", "let _ = std::hint::black_box([0u8; 64].to_vec()); escolher_fallback() }"), base_s.to_string()),
        ("X10 fn do crate como valor", "R4", format!("{trava} fn tranca(_: &S) -> bool {{ TRAVA2.lock().is_ok() }} {}", base_r.replace("fn scalars_com_tentativas(&self, k: u32) -> u8 { ", "fn scalars_com_tentativas(&self, k: u32) -> u8 { let _ = Some(self).map(tranca); ")), base_s.to_string()),
        ("X10b ponteiro de função", "R3", format!("{trava} fn tranca(_: &S) -> bool {{ TRAVA2.lock().is_ok() }} {}", base_r.replace("fn scalars_com_tentativas(&self, k: u32) -> u8 { ", "fn scalars_com_tentativas(&self, k: u32) -> u8 { let trancar: fn(&S) -> bool = tranca; let _ = trancar(self); ")), base_s.to_string()),
        ("X10b a fn como valor", "R4", format!("{trava} fn tranca(_: &S) -> bool {{ TRAVA2.lock().is_ok() }} {}", base_r.replace("fn scalars_com_tentativas(&self, k: u32) -> u8 { ", "fn scalars_com_tentativas(&self, k: u32) -> u8 { let trancar: fn(&S) -> bool = tranca; let _ = trancar(self); ")), base_s.to_string()),
        ("R2 chamada sobre um campo", "R2", base_r.replace("escolher_fallback() }", "(self.g)(1); escolher_fallback() }"), base_s.to_string()),
        ("R2 closure chamada", "R2", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { (|| 0)() }")),
        ("R5 static ao nível do módulo", "R5", format!("{trava} {}", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { GIRO }")), base_s.to_string()),
        ("R5 static aninhado", "R5", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { static M: u8 = 0; M }"), base_s.to_string()),
        ("R5 static de thread_local!", "R5", format!("thread_local! {{ static OUTRO: u8 = const {{ 0 }}; }} {}", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { OUTRO.with(|c| *c) }")), base_s.to_string()),
        ("R6 macro (L4 vec!)", "R6", base_r.replace("escolher_fallback() }", "let _v = vec![0u8; 64]; escolher_fallback() }"), base_s.to_string()),
        ("R7 tipo que aloca", "R7", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { let _: Option<Box<u8>> = None; 0 }"), base_s.to_string()),
        ("L1 lock via auxiliar", "R1", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { espectro() } fn espectro() -> u8 { X.read().unwrap(); 0 }"), base_s.to_string()),
        ("L2 Mutex estático aninhado", "R5", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { static M: std::sync::Mutex<u8> = std::sync::Mutex::new(0); 0 }"), base_s.to_string()),
        ("L3 spinlock", "R1", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { while self.b.swap(true, O) {} 0 }")),
        ("L5 alocação por secção", "R7", base_r.replace("fn seccao_do_codigo() -> u8 { 0 }", "fn seccao_do_codigo() -> u8 { let _ = Box::new(1); 0 }"), base_s.to_string()),
        ("fetch_add fora do fallback", "R1", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { self.n.fetch_add(1, O); 0 }")),
        ("U use … as num grupo", "U", format!("use std::sync::{{atomic::AtomicBool as Bandeira, Arc}}; {base_r}"), base_s.to_string()),
        ("T type sobre um lock", "T", format!("type Trinco = std::sync::Mutex<u8>; {base_r}"), base_s.to_string()),
        // endurecimento do operador antes do commit (c)
        ("R1 método do crate homónimo num campo externo", "R1", base_r.to_string(), base_s.replace("struct L {}", "struct L { b: std::sync::Barrier } struct Inocuo; impl Inocuo { fn wait(&self) {} }").replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { self.b.wait(); 0 }")),
        ("R4 nome importado como valor", "R4", format!("use std::thread::sleep; {}", base_r.replace("fn scalars_com_tentativas(&self, k: u32) -> u8 { ", "fn scalars_com_tentativas(&self, k: u32) -> u8 { let _ = Some(1u64).map(sleep); ")), base_s.to_string()),
        ("R8 while no tentar", "R8", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { while self.ocupado.load(Ordering::Acquire) {} 0 }")),
        ("R8 loop no tentar (extra)", "R8", base_r.to_string(), base_s.replace("fn tentar(&self) -> u8 { 0 }", "fn tentar(&self) -> u8 { loop { break; } 0 }")),
    ] {
        let (_, erros) = verificar_permitidos(&[&r, &s]);
        assert!(da_regra(&erros, regra) > 0, "{nome}: não reprovou pela regra {regra}: {erros:?}");
    }
    // O2: o corpo canónico passa; o literal 16, outra constante, ou mais de uma expressão, reprovam
    let canon = "impl AudioShare { pub fn scalars(&self) -> u8 { self.scalars_com_tentativas(TENTATIVAS_DO_LEITOR) } }";
    assert!(verificar_scalars_canonico(canon).is_empty(), "o corpo canónico passa");
    for (nome, f) in [
        ("X2 u32::MAX", canon.replace("TENTATIVAS_DO_LEITOR)", "u32::MAX)")),
        ("literal 16", canon.replace("TENTATIVAS_DO_LEITOR)", "16)")),
        ("outra constante", canon.replace("TENTATIVAS_DO_LEITOR)", "OUTRA)")),
        ("caminho qualificado", canon.replace("TENTATIVAS_DO_LEITOR)", "crate::reactive::TENTATIVAS_DO_LEITOR)")),
        ("uma instrução antes", canon.replace("{ self.scalars", "{ let _ = 0; self.scalars")),
        ("outro método", canon.replace("scalars_com_tentativas(", "scalars_outra(")),
    ] {
        assert!(!verificar_scalars_canonico(&f).is_empty(), "O2 não visto: {nome}");
    }
    // O2-bis: o `ler` canónico passa; X3, X3b e as outras formas reprovam
    let ler = "impl<const W: usize> Leitor<W> { fn ler(&self, k: u32) -> Result<u8, Esgotado> { \
               let mut tentativas = 0u32; for _ in 0..k { tentativas += 1; if let Some(p) = self.tentar() { return Ok(p); } } \
               Err(Esgotado { tentativas }) } }";
    assert!(verificar_ler_canonico(ler).is_empty(), "o ler canónico passa: {:?}", verificar_ler_canonico(ler));
    for (nome, f) in [
        ("X3 0..2k", ler.replace("0..k", "0..k.saturating_mul(2)")),
        ("X3b 0..2k com tentativas: k", ler.replace("0..k", "0..k.saturating_mul(2)").replace("Esgotado { tentativas }", "Esgotado { tentativas: k }")),
        ("tentativas: k", ler.replace("Esgotado { tentativas }", "Esgotado { tentativas: k }")),
        ("tentativas literal", ler.replace("Esgotado { tentativas }", "Esgotado { tentativas: 16 }")),
        ("sem incremento", ler.replace("tentativas += 1; ", "")),
        ("incremento condicional", ler.replace("tentativas += 1; ", "if k > 1 { tentativas += 1; } ")),
        ("incremento longe do tentar", ler.replace("tentativas += 1; if let Some(p) = self.tentar() { return Ok(p); }", "tentativas += 1; let _ = 0; if let Some(p) = self.tentar() { return Ok(p); }")),
        ("incremento de 2", ler.replace("tentativas += 1", "tentativas += 2")),
        ("dois ciclos", ler.replace("Err(Esgotado", "for _ in 0..k {} Err(Esgotado")),
        ("ciclo while", ler.replace("for _ in 0..k {", "while tentativas < k {")),
        ("0..=k", ler.replace("0..k", "0..=k")),
    ] {
        assert!(!verificar_ler_canonico(&f).is_empty(), "O2-bis não visto: {nome}");
    }
    let base_esc = "pub(crate) struct Escritor<const W: usize> { latch: u8 } struct Leitor { } \
                    pub(crate) fn novo<const W: usize>() -> Escritor<W> { Escritor { latch: 0 } } \
                    impl Leitor { fn tentar(&self) { self.x.load(O); } }";
    assert!(verificar_escritor_unico(base_esc).is_empty(), "a base limpa passa");
    for (nome, f) in [
        ("W3 o Leitor constrói um Escritor", base_esc.replace("fn tentar(&self) { self.x.load(O); }", "fn promover(&self) -> Escritor<4> { Escritor {latch: 0} }")),
        ("W4 o Leitor escreve por swap", base_esc.replace("self.x.load(O);", "self.x.swap(1, O);")),
        ("W4 o Leitor escreve por fetch_add", base_esc.replace("self.x.load(O);", "self.x.fetch_add(1, O);")),
        ("W7 2.º construtor depois dos testes", format!("{base_esc} #[cfg(test)] mod tests {{}} fn outro() -> Escritor<1> {{ Escritor {{ latch: 1 }} }}")),
        ("W2 macro de item", format!("{base_esc} macro_rules! m {{ () => {{}} }}")),
    ] {
        assert!(!verificar_escritor_unico(&f).is_empty(), "não visto: {nome}");
    }
}
