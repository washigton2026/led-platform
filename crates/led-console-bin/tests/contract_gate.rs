//! ADR-0027 — **o caminho B**: o contrato gerado é confrontado com o **texto-fonte** Rust.
//!
//! # Porque este ficheiro não usa `EstadoUi::ALL`
//!
//! De propósito. O gerador (`src/contract.rs`) usa os valores compilados; se este gate
//! usasse os mesmos, os dois partilhariam o **mesmo engano**: uma variante acrescentada ao
//! `enum` e esquecida no `ALL` sairia em falta do TypeScript, e o ficheiro versionado —
//! gerado pelo mesmo caminho — concordaria. Verde, e errado.
//!
//! Este gate lê o **texto** dos ficheiros `.rs` e extrai os literais dos `match` exaustivos
//! (que o compilador obriga a manter completos) e das `const`. É o **controlo negativo** do
//! gerador. Mesma técnica do `surface_gate.rs` e do
//! `nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida` (GS4.4).
//!
//! # A regra que impede este gate de ser falso-verde
//!
//! Uma extração que devolva **zero** literais é tratada como **falha**, nunca como "nada a
//! verificar" — a forma mais barata do KB-012, e a que já mordeu este repo (Miri N=0).

use led_console_bin::contract::gerar_typescript;

const TRUTH_RS: &str = include_str!("../src/truth.rs");
const SURFACE_RS: &str = include_str!("../src/surface.rs");
const DAEMON_RS: &str = include_str!("../../led-daemon/src/lib.rs");
const PROTO_RS: &str = include_str!("../../led-daemon-bin/src/proto.rs");
/// O **produtor** do corpo de `/api/state`: o arm `Cmd::Status` escreve os campos aqui.
const SERVER_RS: &str = include_str!("../../led-daemon-bin/src/server.rs");
/// O artefacto versionado que o frontend importa.
const TS_VERSIONADO: &str = include_str!("../contract/lumyx-contract.generated.ts");

/// Linhas de código — comentários podem citar qualquer coisa legitimamente.
fn codigo(fonte: &str) -> impl Iterator<Item = &str> {
    fonte.lines().map(|l| l.trim()).filter(|t| !t.starts_with("//") && !t.starts_with('*'))
}

/// Extrai os literais de um `match` exaustivo `Tipo::Variante => "valor"`.
///
/// O `match` é a estrutura certa para ler: o compilador **obriga-o** a cobrir todas as
/// variantes, portanto uma variante nova aparece aqui quer o autor se lembre do `ALL` ou não.
fn valores_do_match(fonte: &str, tipo: &str) -> Vec<String> {
    let marca = format!("{tipo}::");
    codigo(fonte)
        .filter(|l| l.contains(&marca) && l.contains("=> \""))
        .filter_map(|l| {
            let i = l.find("=> \"")? + 4;
            let resto = &l[i..];
            let j = resto.find('"')?;
            Some(resto[..j].to_string())
        })
        .collect()
}

/// Extrai os valores de `pub const NOME: &str = "valor";` dentro de um módulo.
fn valores_das_consts(fonte: &str) -> Vec<String> {
    codigo(fonte)
        .filter(|l| l.starts_with("pub const ") && l.contains(": &str = \""))
        .filter_map(|l| {
            let i = l.find(": &str = \"")? + 10;
            let resto = &l[i..];
            let j = resto.find('"')?;
            Some(resto[..j].to_string())
        })
        .collect()
}

/// Extrai os `caminho: "..."` da tabela `ROTAS`.
fn caminhos_das_rotas(fonte: &str) -> Vec<String> {
    codigo(fonte)
        .filter(|l| l.starts_with("caminho: \""))
        .filter_map(|l| {
            let resto = &l["caminho: \"".len()..];
            let j = resto.find('"')?;
            Some(resto[..j].to_string())
        })
        .collect()
}

/// Nenhuma extração pode devolver vazio. Extrair zero e "não há divergência" seriam
/// indistinguíveis, e o gate passaria sem ter verificado nada.
fn nao_vazio(v: Vec<String>, o_que: &str) -> Vec<String> {
    assert!(
        !v.is_empty(),
        "extraí ZERO de `{o_que}` — o formato da fonte mudou e este gate deixou de verificar \
         o que diz que verifica. Um gate que passa sem correr e pior que um gate que falha."
    );
    v
}

/// O TypeScript emite cada valor como `"valor"`; procuramos com as aspas para que
/// `not_armed` não case dentro de `not_armed_extra`.
fn contem(ts: &str, valor: &str) -> bool {
    ts.contains(&format!("\"{valor}\""))
}

// ── 1. O artefacto versionado está sincronizado ──────────────────────────────

/// **O `.ts` versionado é byte a byte o que o gerador produz.**
///
/// Apanha as duas metades do mesmo defeito: alguém editou o `.ts` à mão, ou alguém mudou o
/// Rust e não regenerou. É o gate que sustenta a decisão do ADR-0016 — sem ele, "React" e
/// "sem enum paralelo" seriam incompatíveis.
#[test]
fn o_typescript_versionado_e_exatamente_o_gerado() {
    let gerado = gerar_typescript();
    if gerado != TS_VERSIONADO {
        let g: Vec<&str> = gerado.lines().collect();
        let v: Vec<&str> = TS_VERSIONADO.lines().collect();
        let primeira = (0..g.len().max(v.len()))
            .find(|&i| g.get(i) != v.get(i))
            .unwrap_or(0);
        panic!(
            "o contrato versionado DIVERGE do Rust.\n\
             \n  primeira diferenca na linha {}:\n\
             \n  Rust gera : {:?}\n  ficheiro  : {:?}\n\
             \n  Se mudou o Rust: regenere com\n    \
             cargo run -p led-console-bin --example gerar_contrato\n  \
             Se editou o .ts a mao: nao o faca — o Rust e a fonte de verdade (ADR-0027).",
            primeira + 1,
            g.get(primeira),
            v.get(primeira),
        );
    }
}

// ── 2. Nada do Rust falta no TypeScript (caminho B) ──────────────────────────

/// **Todo estado da UI que existe no Rust existe no contrato.**
///
/// Lê o `match` de `EstadoUi::as_str` — que o compilador mantém exaustivo — em vez de
/// `EstadoUi::ALL`, que é uma lista à mão e pode ficar para trás.
#[test]
fn nenhum_estado_do_rust_falta_no_typescript() {
    let do_rust = nao_vazio(valores_do_match(TRUTH_RS, "EstadoUi"), "EstadoUi::as_str");
    for v in &do_rust {
        assert!(
            contem(TS_VERSIONADO, v),
            "o estado `{v}` existe no Rust e NAO esta no contrato TypeScript.\n  \
             Provavel causa: variante nova no enum, esquecida em `EstadoUi::ALL`.\n  \
             O gerador nao a viu; este gate viu, porque le a fonte."
        );
    }
    assert!(do_rust.len() >= 9, "esperava >= 9 estados, extrai {}", do_rust.len());
}

/// **Todo elo da cadeia de evidência existe no contrato.**
#[test]
fn nenhum_elo_do_rust_falta_no_typescript() {
    let do_rust = nao_vazio(valores_do_match(TRUTH_RS, "Elo"), "Elo::as_str");
    for v in &do_rust {
        assert!(contem(TS_VERSIONADO, v), "o elo `{v}` existe no Rust e falta no contrato");
    }
    assert!(do_rust.len() >= 5, "esperava >= 5 elos, extrai {}", do_rust.len());
}

/// **Todo estado do daemon existe no contrato.** (`led-daemon` está congelado — por isso a
/// leitura é do texto, e não de uma anotação que teria de lá ser posta.)
#[test]
fn nenhum_estado_do_daemon_falta_no_typescript() {
    let do_rust = nao_vazio(valores_do_match(DAEMON_RS, "State"), "State::as_str");
    for v in &do_rust {
        assert!(contem(TS_VERSIONADO, v), "o estado do daemon `{v}` falta no contrato");
    }
    assert!(do_rust.len() >= 8, "esperava >= 8 estados, extrai {}", do_rust.len());
}

/// **Nenhum código de erro perde informação ao atravessar.**
///
/// Cobre os dois conjuntos: os do protocolo (`const` em `proto::code`) e os de recusa do
/// runtime (`match` de `Rejected::code`). Um código que o backend emite e o contrato não
/// conhece torna-se, no frontend, um erro sem nome.
#[test]
fn nenhum_codigo_de_erro_do_rust_falta_no_typescript() {
    let protocolo = nao_vazio(valores_das_consts(PROTO_RS), "proto::code");
    for v in &protocolo {
        assert!(
            contem(TS_VERSIONADO, v),
            "o codigo de protocolo `{v}` existe no Rust e falta no contrato TypeScript"
        );
    }

    let runtime = nao_vazio(valores_do_match(DAEMON_RS, "Rejected"), "Rejected::code");
    for v in &runtime {
        assert!(
            contem(TS_VERSIONADO, v),
            "o codigo de recusa `{v}` atravessa VERBATIM (GS1.6) e falta no contrato"
        );
    }
    assert!(protocolo.len() >= 9 && runtime.len() >= 7, "cobertura menor que a esperada");
}

/// **Toda rota da superfície existe no contrato, e nenhuma inventada aparece nele.**
///
/// Os dois sentidos: uma rota que o console serve e o cliente não conhece é uma
/// funcionalidade morta; uma rota que o cliente conhece e o console não serve é um **comando
/// inexistente** — um 404 em produção, escrito com confiança.
#[test]
fn as_rotas_do_contrato_sao_exatamente_as_do_console() {
    let do_rust = nao_vazio(caminhos_das_rotas(SURFACE_RS), "ROTAS");
    for c in &do_rust {
        assert!(contem(TS_VERSIONADO, c), "a rota `{c}` existe no console e falta no contrato");
    }
    // E o sentido inverso: nenhum `caminho: "/api/..."` no TS que o Rust não sirva.
    for linha in TS_VERSIONADO.lines().filter(|l| l.contains("caminho: \"")) {
        let i = linha.find("caminho: \"").unwrap() + 10;
        let resto = &linha[i..];
        let caminho = &resto[..resto.find('"').unwrap()];
        assert!(
            do_rust.iter().any(|c| c == caminho),
            "o contrato declara a rota `{caminho}`, que o console NAO serve — \
             um comando inexistente no cliente"
        );
    }
}

/// **Todo campo que o `status` põe no fio existe no contrato.**
///
/// Caminho B para o corpo de `/api/state`: extrai os nomes do arm `Cmd::Status` de
/// `server.rs` — o **produtor real** — e confronta-os com o TypeScript gerado.
///
/// Sem isto, um campo acrescentado ao `Snapshot` e ao `Cmd::Status` sairia em falta do
/// contrato, e o frontend leria `undefined` num campo que o backend enviou. O gerador
/// sozinho não o apanharia: ele emite o que lhe escreveram, e o ficheiro versionado —
/// gerado pelo mesmo caminho — concordaria com a omissão.
#[test]
fn nenhum_campo_do_status_falta_no_contrato() {
    // O arm começa em `Cmd::Status =>` e acaba no arm seguinte.
    let arm = SERVER_RS
        .split("Cmd::Status =>")
        .nth(1)
        .expect("o arm `Cmd::Status` tem de existir em server.rs")
        .split("Cmd::")
        .next()
        .expect("corpo do arm");

    // Cada campo é o 1.º elemento de um tuplo, e aparece em **duas** formas no código:
    // `("nome", valor)` numa linha, e `"nome",` sozinho quando o tuplo é multi-linha (é o
    // caso do `show_id`). A 1.ª versão deste gate só apanhava a primeira forma e extraía 4
    // de 5 campos — passava a verificar menos do que dizia. Foi o `assert` de contagem
    // abaixo que o apanhou, e é por isso que ele existe.
    let campos: Vec<String> = arm
        .lines()
        .filter_map(|l| {
            let t = l.trim().trim_start_matches('(');
            let resto = t.strip_prefix('"')?;
            let fim = resto.find('"')?;
            // Só conta se for mesmo o 1.º elemento de um tuplo: `"nome",`
            resto[fim + 1..].trim_start().starts_with(',').then(|| resto[..fim].to_string())
        })
        .collect();

    let campos = nao_vazio(campos, "campos do Cmd::Status");
    for c in &campos {
        assert!(
            TS_VERSIONADO.contains(&format!("readonly {c}:")),
            "o campo `{c}` e escrito no fio por `Cmd::Status` e NAO esta no contrato \
             TypeScript. Um campo que o backend envia e o contrato desconhece chega ao \
             frontend como `undefined`."
        );
    }
    assert!(
        campos.len() >= 5,
        "esperava pelo menos os 5 campos do snapshot, extrai {}: {campos:?}",
        campos.len()
    );

    // E o envelope, que vem do `ok_line` e não do arm.
    for c in ["v", "id", "ok"] {
        assert!(
            TS_VERSIONADO.contains(&format!("readonly {c}:")),
            "o envelope do IPC v1 tem `{c}`, e o contrato tem de o descrever"
        );
    }
}

/// **`show_id` é anulável, e isso é semântica — não conveniência.**
///
/// O Rust tem `Option<u64>` e o fio escreve `null` quando não há show. `number | null`
/// preserva a distinção; `number` sozinho obrigaria o frontend a inventar um sentinela, e
/// `0` é um `ShowId` legítimo.
#[test]
fn show_id_nulavel_sobrevive_ate_ao_contrato() {
    assert!(
        SERVER_RS.contains("s.show_id.map(|i| i.to_string()).unwrap_or_else(|| \"null\".into())"),
        "o produtor mudou a forma como escreve `show_id`; este gate ficou desatualizado"
    );
    assert!(
        TS_VERSIONADO.contains("readonly show_id: number | null;"),
        "`show_id` tem de ser `number | null` — `null` e SEM SHOW, nunca 0"
    );
}

// ── 3. A semântica de opcional sobrevive ─────────────────────────────────────

/// **`Option<T>` atravessa como `| null` explícito, nunca como campo ausente.**
///
/// A distinção é semântica e já tem histórico: `stale_ms()` devolve `Option<u64>`
/// precisamente para que "nunca houve instantâneo" não se confunda com "idade zero"; e
/// `"id": null` é a recusa não-atribuível, que os clientes distinguem de um evento pela
/// **presença da chave**. Um contrato que emitisse `staleMs?: number` apagaria as duas.
#[test]
fn os_opcionais_mantem_a_semantica_de_nulo() {
    for (campo, esperado) in
        [("staleMs", "staleMs: number | null"), ("dado", "dado: T | null"), ("id", "id: number | null")]
    {
        assert!(
            TS_VERSIONADO.contains(esperado),
            "o campo opcional `{campo}` perdeu a semantica: esperava `{esperado}`.\n  \
             `T | null` e `campo?: T` NAO sao a mesma coisa — um campo ausente e um campo a \
             null sao indistinguiveis para quem le com `?.`"
        );
        assert!(
            !TS_VERSIONADO.contains(&format!("{campo}?:")),
            "`{campo}?:` torna o campo OPCIONAL em vez de nulavel — perde a distincao"
        );
    }
}

/// **O ficheiro declara-se gerado.** Sem esta marca, a próxima pessoa edita-o de boa-fé.
#[test]
fn o_artefacto_diz_que_e_gerado_e_como_o_regenerar() {
    assert!(TS_VERSIONADO.contains("GERADO"), "o cabecalho tem de dizer que e gerado");
    assert!(
        TS_VERSIONADO.contains("NAO EDITAR") || TS_VERSIONADO.contains("NÃO EDITAR"),
        "o cabecalho tem de proibir a edicao manual"
    );
    assert!(
        TS_VERSIONADO.contains("gerar_contrato"),
        "o cabecalho tem de dizer QUAL o comando que regenera"
    );
}

/// O gerador é **determinístico** — senão o gate do ficheiro versionado seria instável e
/// alguém acabaria por o desligar.
#[test]
fn o_gerador_e_deterministico() {
    assert_eq!(gerar_typescript(), gerar_typescript());
}
