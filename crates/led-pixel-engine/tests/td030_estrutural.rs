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
    let base_r = "impl S { pub fn scalars(&self) -> u8 { self.scalars_com_tentativas(1) } \
                  fn scalars_com_tentativas(&self, k: u32) -> u8 { match self.leitor.ler(k) { Some(_) => descodificar(), None => self.fallback() } } \
                  fn fallback(&self) -> u8 { self.f.fetch_add(1, O); escolher_fallback() } } \
                  fn escolher_fallback() -> u8 { 0 } fn descodificar() -> u8 { seccao_do_codigo() } fn seccao_do_codigo() -> u8 { 0 }";
    let base_s = "impl L { fn ler(&self, k: u32) -> u8 { self.tentar() } fn tentar(&self) -> u8 { 0 } }";
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
