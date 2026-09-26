//! `control-protocol.md:119` — **o p99 de `send_frame` não é inflado por comandos em
//! trânsito**, provado pelo MECANISMO e não por um número.
//!
//! # Porque não há aqui um limiar de latência
//!
//! Medir p99 sob carga concorrente daria um número **ruidoso e dependente da máquina**, e
//! qualquer limiar seria arbitrário — exactamente o erro que o TD-006 já custou a este
//! repositório. O que se pode provar de forma determinística é a **ausência da causa**:
//! comandos só poderiam inflar o p99 do envio por duas vias, e as duas estão fechadas.
//!
//! 1. **Lock partilhado** — se a superfície de controlo adquirisse um lock que o envio
//!    também adquire, um comando podia fazer um frame esperar. Este ficheiro prova que ela
//!    não adquire lock nenhum.
//! 2. **Alocação** — coberta pelo `no_alloc_canal.rs`, que mede zero.
//!
//! E há uma terceira razão, estrutural e anterior às duas: **há um só aplicador** (GS3). O
//! servidor IPC **enfileira** (`server.rs:363`); quem aplica é o laço (`run.rs:352`). Um
//! comando nunca corre *durante* um envio — corre no limite do tick, entre envios.
//!
//! # A fronteira, escrita para não ser arredondada
//!
//! Isto prova que **o mecanismo de inflação não existe**. **Não** mede p99, e não afirma um
//! número. Se algum dia alguém quiser o número, é medição própria, com plataforma declarada.
//!
//! # Precedente
//!
//! Um detector estrutural sobre o texto-fonte é o padrão que o TD-002 já usou para o
//! `ArcSwap` (*«grep `read()`/`write()`/`lock()` em `reactive.rs` → ZERO em `scalars()`»*).
//! Reusar em vez de inventar um segundo mecanismo para o mesmo fim.

/// O texto-fonte do módulo que este ficheiro vigia.
const FONTE: &str = include_str!("../src/output.rs");

/// Extrai o corpo de `pub fn <nome>` por emparelhamento de chavetas.
///
/// Devolve `None` se a função não existir — e o teste trata isso como **falha**, nunca como
/// «nada a verificar». Uma extracção vazia e «não há lock» seriam indistinguíveis, que é o
/// KB-012 na sua forma mais barata.
fn corpo_de(nome: &str) -> Option<String> {
    let assinatura = format!("pub fn {nome}");
    let inicio = FONTE.find(&assinatura)?;
    let abre = FONTE[inicio..].find('{')? + inicio;
    let mut prof = 0usize;
    for (i, c) in FONTE[abre..].char_indices() {
        match c {
            '{' => prof += 1,
            '}' => {
                prof -= 1;
                if prof == 0 {
                    return Some(FONTE[abre..=abre + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// **A superfície de controlo do blackout não adquire lock nenhum.**
///
/// É isto que impede um comando de fazer um frame esperar. Falsificável: pôr uma aquisição
/// de lock em qualquer das três funções reprova, nomeando qual.
#[test]
fn a_superficie_de_controlo_nao_adquire_lock_algum() {
    // A marca é montada em pedaços de propósito: escrita inteira, este ficheiro conteria o
    // que proíbe, e um `grep` externo sobre os testes acusaria o próprio gate. É a armadilha
    // que a lista de palavras do ADR-0017 já apanhou duas vezes neste repositório.
    let marca = format!(".{}()", "lock");

    let vigiadas = ["blackout_accionar", "blackout_levantar", "blackout_activo"];
    let mut vistas = 0;

    for nome in vigiadas {
        let corpo = corpo_de(nome)
            .unwrap_or_else(|| panic!("`pub fn {nome}` NAO existe em output.rs — o gate \
                 deixou de vigiar o que afirma vigiar, e isso e falha, nao ausencia de defeito"));
        assert!(
            !corpo.contains(&marca),
            "`{nome}` adquire um lock. Um comando passa a poder fazer um frame esperar, e o \
             p99 do envio deixa de ser independente do canal de controlo:\n{corpo}"
        );
        vistas += 1;
    }

    // Guarda contra o falso-verde: zero funções vigiadas passaria trivialmente.
    assert_eq!(vistas, 3, "as tres funcoes do controlo tem de ser vigiadas, vi {vistas}");
}

/// **Controlo negativo — o detector sabe ver um lock.**
///
/// Sem isto, o teste acima poderia estar a passar por a marca nunca casar com nada (regex
/// errada, extracção vazia, marca mal montada) em vez de por ausência real de locks. O
/// caminho de envio **tem** locks, e o detector tem de os encontrar lá.
#[test]
fn o_detector_encontra_locks_onde_eles_de_facto_existem() {
    let marca = format!(".{}()", "lock");
    let envio = corpo_de("send").expect("`pub fn send` existe");
    // O `send` delega no `enviar`; a prova mais directa é o ficheiro inteiro, que contém os
    // dois locks reais do caminho de saída (`corrigidos` e `fatia`).
    assert!(
        FONTE.contains(&marca),
        "o detector nao encontra locks em lado nenhum — a marca esta errada e o outro \
         teste esta a passar vacuamente"
    );
    assert!(!envio.is_empty(), "extracao vazia e falha, nao sucesso");
}
