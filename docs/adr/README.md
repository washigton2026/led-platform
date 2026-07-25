# Architecture Decision Records — LUMYX

ADRs no formato [MADR](https://adr.github.io/madr/) (Markdown Any Decision
Record): contexto → decisão → consequências → alternativas rejeitadas.

Estes ADRs são **retroativos** (2026-07-12): as decisões já haviam sido tomadas
e vividas no código; aqui o "porquê" foi extraído das fontes primárias —
`CLAUDE.md` (Session changelog), `LUMYX_GOSL.md` e `docs/knowledge-base.md` — e
não inventado. A data de cada ADR é a data original aproximada da decisão,
recuperada do changelog.

| ADR | Título | Status | Data original |
|---|---|---|---|
| [0001](0001-replay-deterministico.md) | Replay determinístico via hash de pixels | aceito | 2026-06-26 |
| [0002](0002-arcswap-substitui-mutex.md) | ArcSwap substitui Mutex no AudioShare | aceito | 2026-06-19 |
| [0003](0003-ddp-protocolo-preferencial.md) | DDP como protocolo de saída preferencial | aceito | 2026-06-26 |
| [0004](0004-ed25519-pinned-verification.md) | Verificação de assinatura com chave fixada | aceito | 2026-07-12 |
| [0005](0005-wifi-proibido-producao.md) | WiFi proibido para shows ao vivo | aceito | 2026-06-03 |
| [0006](0006-showintent-sem-ia-runtime.md) | ShowIntent: IA só em design-time, nunca em runtime | aceito | 2026-06-28 |
| [0007](0007-semver-certified-seams.md) | Seams canônicos certificados por SemVer | aceito | 2026-06-28 |
| [0008](0008-triple-buffer.md) | Triple buffer entre render e send | aceito | 2026-06-03 |
| [0009](0009-chaos-framework.md) | Chaos framework determinístico com baseline | aceito | 2026-06-28 |
| [0010](0010-cluster-failover.md) | Failover de cluster por saúde de segmento | aceito | 2026-06-28 |
| [0011](0011-colorformat-rgbw-no-mapper.md) | `ColorFormat` no mapper: suporte RGBW/4-canais aditivo | aceito | 2026-07-25 |
| [0012](0012-unificacao-saida-fanout-paralelo.md) | Unificação da saída: fan-out paralelo adiado; serialização já é fonte única | aceito (impl. adiada) | 2026-07-25 |

## Quando escrever um novo ADR
Uma mudança merece ADR quando altera um **seam**, um **invariante**, ou uma
**escolha estrutural difícil de reverter** (protocolo, modelo de concorrência,
fronteira de confiança). Correções e features aditivas normais vão no changelog.
