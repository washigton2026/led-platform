//! C4 — 9.º check do `HardwareProfileGuardian`: **a porta não vaza para o runtime** (ADR-0030 §7).
//!
//! O §7 diz que a porta é resolvida **uma vez, no arranque**, e que **nunca** é consultada
//! durante a renderização — nunca por quadro, nunca no caminho
//! `Show → Logical Pixels → ProtocolOutput → HAL → DeviceDriver`.
//!
//! ## Porque este gate é estrutural e não textual
//!
//! A primeira tentativa foi um scanner de texto sobre `led-hal/src` e `led-pixel-engine/src`,
//! à procura do vocabulário de porta. Foi **abandonada por ser frágil**, e as duas razões
//! ficam escritas para ninguém a repetir:
//!
//! 1. **Colisão de vocabulário.** `led-hal/src/network_guard.rs` usa `Port`/`ports` no sentido
//!    do `networksetup` do macOS — *hardware ports* são interfaces de rede, e o ficheiro nem
//!    sequer está no caminho de renderização (o `NetworkGuard` corre uma vez, no arranque;
//!    há um teste com `CountingGuard` que o prova).
//! 2. **Literais multi-linha.** Aquele mesmo ficheiro tem strings com continuação `"\`, que
//!    quebram qualquer remoção de literais feita linha a linha e deixam `Hardware Port:`
//!    visível ao scanner.
//!
//! Um gate que produz falso BLOCK é um gate que acaba enfraquecido. A forma estrutural não
//! tem esse modo de falha.
//!
//! ## O que fecha o §7, e porque estes dois checks bastam
//!
//! Só há **dois** caminhos para um valor de porta chegar ao render path:
//!
//! | Caminho | Quem o fecha |
//! |---|---|
//! | (a) `led-hal` / `led-pixel-engine` passarem a depender de `led-hardware-profile` | **este ficheiro** |
//! | (b) um campo de porta entrar num tipo do `led-core` (`PixelPhysical`, `CompiledLayout`, `UniverseData`) | **SemVer Guardian** — é o invariante 8 do ADR-0030, textualmente: *«`led-core` intocado — o SemVer Guardian já o faz»* |
//!
//! Duplicar o (b) aqui criaria uma segunda regra para o mesmo facto, que é precisamente o que
//! o §6 do mesmo ADR recusa. Este gate cobre o (a) e **compõe** com o que já existe.
//!
//! É a mesma mecânica do check 4 do guardião (*«o crate do profile não depende do HAL»*),
//! aplicada na direcção oposta: o HAL não depende do profile.

/// O crate que **possui** o conceito de porta. Se ele entrar no grafo de um crate de runtime,
/// `ports` passa a ser alcançável a partir do caminho de renderização.
const CRATE_DA_PORTA: &str = "led-hardware-profile";

/// Os manifestos dos crates que executam o caminho `ProtocolOutput → HAL → DeviceDriver` e o
/// render. Lidos em tempo de compilação: se algum for movido ou renomeado, isto **não compila**
/// — que é o modo de falha certo para um gate. Um `read_dir` em runtime falharia em silêncio.
const MANIFESTOS: &[(&str, &str)] = &[
    ("led-hal", include_str!("../../crates/led-hal/Cargo.toml")),
    ("led-pixel-engine", include_str!("../../crates/led-pixel-engine/Cargo.toml")),
];

/// ADR-0030 §7 — nenhum crate do caminho de runtime pode depender do crate da porta.
///
/// Não distingue `[dependencies]` de `[dev-dependencies]` de propósito: uma dev-dependency
/// deixaria um teste dentro do `led-hal` consultar `ports`, e isso é o conceito a atravessar
/// a fronteira na mesma. O crate não tem nada que o conhecer, em nenhum perfil de compilação.
#[test]
fn nenhum_crate_de_runtime_depende_do_crate_da_porta() {
    for (crate_name, manifesto) in MANIFESTOS {
        // KB-012: um manifesto vazio faria este teste passar sem verificar nada. Ler zero é
        // falha, nunca "nada a verificar" — a lição do Miri N=0.
        assert!(
            manifesto.contains("[dependencies]"),
            "{crate_name}: manifesto sem secção [dependencies] — o gate não leu o que julga ter lido"
        );

        assert!(
            !manifesto.contains(CRATE_DA_PORTA),
            "ADR-0030 §7 VIOLADO: `{crate_name}` passou a depender de `{CRATE_DA_PORTA}`.\n\
             A porta é resolvida no arranque e desaparece; o caminho de renderização não a \
             pode alcançar.\n\
             Se a intenção for passar dados compilados ao runtime, passe o `CompiledLayout` — \
             que é o que o profile produz — e não o profile."
        );
    }
}

/// Controlo negativo do anterior. Sem isto, uma constante `MANIFESTOS` vazia — ou um
/// `include_str!` a apontar para o ficheiro errado — faria o teste acima passar vacuamente.
///
/// Afirma que os manifestos lidos são mesmo os dos crates de runtime, e que o gate está a
/// olhar para conteúdo real.
#[test]
fn o_gate_leu_mesmo_os_manifestos_dos_crates_de_runtime() {
    assert_eq!(MANIFESTOS.len(), 2, "os dois crates do caminho de runtime");

    for (crate_name, manifesto) in MANIFESTOS {
        assert!(
            manifesto.contains(&format!("name = \"{crate_name}\"")),
            "o manifesto lido não é o do `{crate_name}` — `include_str!` aponta para o sítio errado"
        );
        // Ambos dependem do `led-core` e é isso que os torna caminho de runtime. Se esta
        // linha desaparecer, o crate mudou de natureza e este gate precisa de ser revisto.
        assert!(
            manifesto.contains("led-core"),
            "`{crate_name}` deixou de depender de `led-core` — o caminho de runtime mudou de forma"
        );
    }
}
