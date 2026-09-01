//! O **dono único** da aritmética de repartição (ADR-0030 §6).
//!
//! Havia duas repartições possíveis no projecto e elas não podiam virar duas implementações
//! da mesma regra:
//!
//! ```text
//! show ──repartir()──▶ Alvo{pixel_offset, pixel_count}      [ADR-0029, nó]
//!                          │
//!                          └──repartir()──▶ Porta{...}      [ADR-0030, porta]
//! ```
//!
//! ## É o mesmo núcleo, e NÃO é a mesma função
//!
//! O cálculo dos intervalos — `inicio = i × tecto`, `conta = min(total − inicio, tecto)` — é
//! idêntico nos dois níveis. **Duas regras não transferem**, e o §6 diz porquê:
//!
//! | | Nó (ADR-0029) | Porta (ADR-0030) |
//! |---|---|---|
//! | Unidade sem píxeis | **recusa** — o operador escolheu os endereços | **válida** (§4-ter) — `ports` vem do hardware |
//! | Fronteira de universo | não se aplica | **obrigatória** (§4-bis) — a porta arranca em canal 0 |
//!
//! Por isso o núcleo é **parametrizado** ([`UnidadeVazia`] entra como dado) e o alinhamento
//! de universo é uma **camada por cima**, em [`repartir_portas`]. Copiar o núcleo com as
//! regras trocadas seria a segunda implementação que o §6 existe para impedir; chamá-lo sem
//! parametrizar recusaria hardware normal.
//!
//! ## O que este módulo NÃO faz
//!
//! Não endereça píxeis a universo/canal — isso é [`crate::compile_layout`], e é outro eixo.
//! Não abre sockets, não conhece protocolos e não sabe o que é um `Alvo`: o vocabulário de
//! cada consumidor fica no consumidor.

use crate::HardwareProfile;

/// O que fazer com uma unidade que fica sem píxeis. **Entra como dado** (ADR-0030 §6) porque
/// é exactamente aqui que a regra do nó e a da porta divergem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnidadeVazia {
    /// **Recusa** — a regra do nó (ADR-0029). Cinco endereços para um show que cabe em dois
    /// significa que o operador está enganado, e abrir um socket que nunca envia esconde isso
    /// até alguém reparar no palco.
    Recusa,
    /// **Aceita** — a regra da porta (ADR-0030 §4-ter). Aqui o operador não escolheu nada:
    /// `ports` vem do hardware. Um show mais pequeno que o nó é normal, e aplicar a regra do
    /// nó exigiria ≥ 15 301 px num Falcon de 16 portas — recusaria o show real de 6 200 px.
    Aceita,
}

/// Uma fatia: onde começa e quantos píxeis leva. **Derivado, nunca declarado** (§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fatia {
    pub pixel_offset: u32,
    pub pixel_count: usize,
}

/// Uma fatia de porta: a [`Fatia`] mais o universo onde a porta arranca (§4-bis).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FatiaDePorta {
    pub index: u16,
    pub pixel_offset: u32,
    pub pixel_count: usize,
    /// O universo onde esta porta começa, **sempre em canal 0** (§4-bis). Uma porta pode
    /// acabar dentro do seu último universo, mas nunca o partilha com a porta seguinte.
    pub universe_start: u16,
}

/// Porque a repartição foi recusada. Cada variante carrega os números — sem eles o chamador
/// não sabe se acrescenta unidades ou encurta o show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepartirError {
    /// Zero unidades: não há por onde repartir.
    SemUnidades,
    /// O tecto por unidade é zero. O validador já emite `ZeroLimit`/`NoPhysicalPorts` e o
    /// ADR-0024 impede a saída de abrir; aqui é guarda contra divisão por zero, **não** uma
    /// segunda política.
    TectoZero,
    /// O total não cabe na capacidade declarada.
    NaoCabe { total: usize, tecto: usize, unidades: usize, capacidade: usize },
    /// Uma unidade ficaria sem píxeis, e a política é [`UnidadeVazia::Recusa`].
    UnidadeSemPixeis { indice: usize, total: usize, tecto: usize, unidades: usize, cabem_em: usize },
    /// O universo derivado não cabe em `u16`.
    ///
    /// É guarda de **overflow aritmético**, não a regra de 15 bits do ADR-0029 §7 — essa é do
    /// daemon, e duplicá-la aqui criaria a segunda fonte que o §6 recusa.
    UniversoForaDeFaixa { porta: u16, universo: u32 },
}

/// O **núcleo**. Reparte `total` píxeis por `unidades`, cada uma com `tecto` no máximo.
///
/// # Porque o resto fica na ÚLTIMA unidade
///
/// Encher cada unidade até ao tecto e deixar o resto na última torna a repartição **função só
/// do tecto e da ordem**: acrescentar uma unidade no fim não mexe nas fatias das anteriores.
/// Distribuir o resto por igual faria a fatia do robô 1 mudar quando alguém ligasse o robô 6.
pub fn repartir(
    total: usize,
    tecto: usize,
    unidades: usize,
    vazia: UnidadeVazia,
) -> Result<Vec<Fatia>, RepartirError> {
    if unidades == 0 {
        return Err(RepartirError::SemUnidades);
    }
    if tecto == 0 {
        return Err(RepartirError::TectoZero);
    }
    let capacidade = tecto.saturating_mul(unidades);
    if total > capacidade {
        return Err(RepartirError::NaoCabe { total, tecto, unidades, capacidade });
    }

    let mut fatias = Vec::with_capacity(unidades);
    for i in 0..unidades {
        let inicio = i * tecto;
        // `saturating_sub`: quando a unidade começa **para lá** do fim do show, a subtracção
        // simples estoura. Dá 0, que é a condição tratada logo abaixo.
        let conta = total.saturating_sub(inicio).min(tecto);
        if conta == 0 && vazia == UnidadeVazia::Recusa {
            return Err(RepartirError::UnidadeSemPixeis {
                indice: i,
                total,
                tecto,
                unidades,
                cabem_em: inicio.div_ceil(tecto),
            });
        }
        fatias.push(Fatia { pixel_offset: inicio as u32, pixel_count: conta });
    }
    Ok(fatias)
}

/// Reparte os píxeis **de um nó** pelas suas portas físicas (ADR-0030 §§4, 4-bis, 4-ter).
///
/// Chama o mesmo [`repartir`] com [`UnidadeVazia::Aceita`] e acrescenta o alinhamento de
/// universo por cima. A capacidade por porta é `max_pixels / ports` — **derivada, nunca
/// declarada** (§4).
///
/// # A pendência que este código NÃO decide
///
/// Quando `max_pixels % ports != 0` sobra capacidade declarada e não endereçável. O ADR-0030
/// §5 regista isso **sem política**: recusar, avisar ou não emitir achado são três respostas
/// com consequências diferentes e **nenhuma está decidida**. Esta função aplica a fórmula do
/// §4 tal como escrita e **não emite achado nenhum** — não porque isso seja a política, mas
/// porque inventar uma seria pior. Gatilho registado: o primeiro preset com divisão inexacta.
/// Os dois candidatos de hoje dividem exacto (Falcon 16384/16, Advatek 16320/16).
pub fn repartir_portas(
    profile: &HardwareProfile,
    pixeis_do_no: usize,
    first_universe: u16,
) -> Result<Vec<FatiaDePorta>, RepartirError> {
    let portas = profile.capabilities.ports as usize;
    if portas == 0 {
        // O validador já recusa por `Finding::NoPhysicalPorts` e o ADR-0024 impede a saída de
        // abrir. Aqui é guarda contra divisão por zero, não uma segunda política.
        return Err(RepartirError::SemUnidades);
    }
    let ppu = profile.limits.pixels_per_universe as usize;
    if ppu == 0 {
        return Err(RepartirError::TectoZero);
    }
    let capacidade_por_porta = profile.limits.max_pixels as usize / portas;

    let fatias = repartir(pixeis_do_no, capacidade_por_porta, portas, UnidadeVazia::Aceita)?;

    // §4-bis: o `ceil` é o que IMPEDE a partilha de universo, não o que a causa — a porta
    // seguinte arranca **para lá** do último universo da anterior, em canal 0.
    let universos_por_porta = capacidade_por_porta.div_ceil(ppu);

    fatias
        .into_iter()
        .enumerate()
        .map(|(k, f)| {
            let universo = first_universe as u32 + (k * universos_por_porta) as u32;
            let universe_start = u16::try_from(universo).map_err(|_| {
                RepartirError::UniversoForaDeFaixa { porta: k as u16, universo }
            })?;
            Ok(FatiaDePorta {
                index: k as u16,
                pixel_offset: f.pixel_offset,
                pixel_count: f.pixel_count,
                universe_start,
            })
        })
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HardwareRegistry;

    fn perfil(nome: &str) -> HardwareProfile {
        HardwareRegistry::with_builtin().profile(nome).expect("preset embutido")
    }

    /// **A prova do dono único (ADR-0030 §6, invariante 1).**
    ///
    /// Repartir por nós e depois por portas tem de dar o mesmo que **uma** chamada com
    /// `N = nós × portas`. Se alguém escrever uma segunda implementação para o nível da porta,
    /// os dois caminhos divergem e este teste apanha-o — é a razão de ele existir.
    ///
    /// A equivalência vale porque o tecto do nó é `portas × capacidade_por_porta`; é isso que
    /// faz o índice global `i × portas + k` cair no mesmo offset pelos dois caminhos.
    #[test]
    fn repartir_por_nos_e_depois_por_portas_e_uma_so_regra() {
        let (cap_porta, portas, nos) = (1_024usize, 16usize, 3usize);
        let tecto_no = cap_porta * portas;
        let total = tecto_no * 2 + 5_000; // enche dois nós e deixa resto no terceiro

        // Caminho A: nó → porta.
        let mut aninhado = Vec::new();
        for no in repartir(total, tecto_no, nos, UnidadeVazia::Aceita).unwrap() {
            for p in
                repartir(no.pixel_count, cap_porta, portas, UnidadeVazia::Aceita).unwrap()
            {
                aninhado.push(Fatia {
                    pixel_offset: no.pixel_offset + p.pixel_offset,
                    pixel_count: p.pixel_count,
                });
            }
        }

        // Caminho B: uma chamada, N = nós × portas.
        let plano = repartir(total, cap_porta, nos * portas, UnidadeVazia::Aceita).unwrap();

        assert_eq!(aninhado, plano, "dois caminhos, uma só aritmética (§6)");
    }

    /// Falcon: o discriminante **obrigatório** do invariante 9 — `1024 / 170` NÃO é inteiro,
    /// portanto o último universo de cada porta é parcial. Com um preset de divisão exacta
    /// este teste passaria sem provar nada.
    #[test]
    fn falcon_reparte_1024_por_porta_e_arranca_de_7_em_7_universos() {
        let p = perfil("falcon-f16v3-sacn");
        assert_eq!(p.capabilities.ports, 16);
        assert_eq!(p.limits.max_pixels, 16_384);

        let fatias = repartir_portas(&p, 16_384, 0).unwrap();
        assert_eq!(fatias.len(), 16);
        assert!(fatias.iter().all(|f| f.pixel_count == 1_024), "16_384 / 16 = 1_024");

        // 1024 / 170 = 6.02… → 7 universos por porta. O universo 6 leva os píxeis 1020–1023
        // e os canais 12–509 ficam sem atribuição (§4-bis).
        assert_eq!(fatias[0].universe_start, 0);
        assert_eq!(fatias[1].universe_start, 7, "a porta 1 arranca PARA LÁ do universo 6");
        assert_eq!(fatias[15].universe_start, 105);
    }

    /// Advatek: o **controlo exacto** — `1020 / 170 = 6` sem resto. Ao lado do Falcon, mostra
    /// que o `ceil` do §4-bis só acrescenta universo quando a divisão sobra.
    #[test]
    fn advatek_reparte_1020_por_porta_e_arranca_de_6_em_6_universos() {
        let p = perfil("advatek-pixlite16-sacn");
        assert_eq!(p.capabilities.ports, 16);
        assert_eq!(p.limits.max_pixels, 16_320);

        let fatias = repartir_portas(&p, 16_320, 0).unwrap();
        assert!(fatias.iter().all(|f| f.pixel_count == 1_020), "16_320 / 16 = 1_020");
        assert_eq!(fatias[1].universe_start, 6, "divisão exacta: 6 universos, sem folga");
        assert_eq!(fatias[15].universe_start, 90);
    }

    /// §4-bis: duas portas **nunca** partilham universo. Controlo negativo do teste anterior —
    /// trocar o `ceil` por divisão inteira faz a porta 1 entrar no universo 6 do Falcon.
    #[test]
    fn duas_portas_nunca_partilham_universo() {
        let p = perfil("falcon-f16v3-sacn");
        let ppu = p.limits.pixels_per_universe as usize;
        let fatias = repartir_portas(&p, 16_384, 0).unwrap();

        for par in fatias.windows(2) {
            let (a, b) = (par[0], par[1]);
            let ultimo_de_a =
                a.universe_start as usize + a.pixel_count.div_ceil(ppu).saturating_sub(1);
            assert!(
                (b.universe_start as usize) > ultimo_de_a,
                "porta {} acaba no universo {ultimo_de_a} e a porta {} arranca em {}",
                a.index,
                b.index,
                b.universe_start
            );
        }
    }

    /// §4-ter: uma porta sem píxeis é **válida**. O show real do rig (6 200 px) num Falcon de
    /// 16 portas deixa as portas 7–15 vazias, e o profile não é recusado.
    #[test]
    fn porta_sem_pixeis_e_valida_e_o_show_real_do_rig_compila() {
        let p = perfil("falcon-f16v3-sacn");
        let fatias = repartir_portas(&p, 6_200, 0).unwrap();

        assert_eq!(fatias.len(), 16, "as 16 portas continuam a existir");
        // 6 portas cheias (6 × 1024 = 6144) + 56 na porta 6 → 9 vazias (7–15).
        assert_eq!(fatias[5].pixel_count, 1_024);
        assert_eq!(fatias[6].pixel_count, 56, "o resto fica na última porta que recebe algo");
        assert_eq!(fatias.iter().filter(|f| f.pixel_count == 0).count(), 9);
        assert_eq!(fatias.iter().map(|f| f.pixel_count).sum::<usize>(), 6_200);
        // E o `pixel_offset` continua derivado mesmo nas vazias.
        assert_eq!(fatias[15].pixel_offset, 15 * 1_024);
    }

    /// A regra do nó **não** foi contagiada pela da porta: com `Recusa`, uma unidade vazia
    /// continua a ser erro. Sem este teste, trocar a política por `Aceita` no consumidor do
    /// daemon passaria despercebida.
    #[test]
    fn a_politica_do_zero_e_dado_e_as_duas_regras_nao_se_confundem() {
        assert_eq!(
            repartir(1_000, 1_000, 3, UnidadeVazia::Recusa),
            Err(RepartirError::UnidadeSemPixeis {
                indice: 1,
                total: 1_000,
                tecto: 1_000,
                unidades: 3,
                cabem_em: 1,
            })
        );
        let aceite = repartir(1_000, 1_000, 3, UnidadeVazia::Aceita).unwrap();
        assert_eq!(aceite.len(), 3);
        assert_eq!(aceite[1].pixel_count, 0, "a mesma entrada, a outra regra");
    }
}
