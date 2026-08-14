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
| [0013](0013-engine-daemon-separado.md) | Engine headless em daemon separado; UI é cliente (output não compartilha processo de falha) | aceito (pré-impl.) | 2026-07-26 |
| [0014](0014-ipc-seguranca-ui-engine.md) | IPC + segurança UI↔engine: UDS owner-only / token-mTLS por interface; nunca 0.0.0.0 | aceito (pré-impl.) | 2026-07-26 |
| [0015](0015-preview-lossy-fora-hot-path.md) | Preview: cópia downsampled/rate-limited/lossy fora do hot-path; UI nunca lê o triple buffer | aceito (pré-impl.) | 2026-07-26 |
| [0016](0016-stack-console-provisorio.md) | Stack do console: web DOM+WebGPU + **React/TypeScript**, com gate obrigatório de tipos gerados | **aceito** | 2026-08-09 |
| [0017](0017-blackout-intencional-vs-heartbeat.md) | Blackout intencional × invariante do heartbeat — decisão adiada | proposto (adiado) | 2026-07-26 |
| [0018](0018-hardwareprofile-capacidades-design-time.md) | `HardwareProfile`: descritor de capacidades em design-time (presets são dado; compila para os seams) | aceito (pré-impl.) | 2026-07-29 |
| [0019](0019-calibracao-por-output-no-hal.md) | Calibração por-output (gamma+brightness) aplicada no HAL, por device, sem tocar contrato Frozen | aceito | 2026-07-29 |
| [0020](0020-whitemode-subtrativo.md) | `WhiteMode::MinSubtract`: derivação subtrativa do branco (RGBW deixa de somar ~4x de corrente) | aceito | 2026-08-02 |
| [0021](0021-efeitos-funcoes-puras-estado-derivado.md) | Efeito é função pura de `(tempo, posição, índice)`; aleatoriedade é hash, nunca fluxo; estado é derivado, nunca armazenado | aceito | 2026-08-03 |
| [0022](0022-wearable-playback-autonomo-sync-deterministico.md) | Traje de LED: playback autônomo + sincronização por relógio comum (o traje é *player*, não *device*); ADR-0005 intacto | aceito | 2026-08-03 |
| [0023](0023-superficie-de-transporte-do-engine.md) | Superfície de transporte do engine (estado de show em runtime) | aceito | 2026-08-05 |
| [0024](0024-fronteira-de-validacao-do-hardwareprofile.md) | A fronteira de validação do `HardwareProfile` | aceito | 2026-08-07 |
| [0025](0025-refresh-hz-e-a-cadencia-pedida.md) | `refresh_hz` é um limite, e o daemon recusa ultrapassá-lo | aceito | 2026-08-07 |
| [0026](0026-console-daemon-boundary.md) | A fronteira console↔daemon: o console é cliente do IPC v1, e traduz sem interpretar | aceito | 2026-08-07 |
| [0027](0027-contrato-tipos-rust-typescript.md) | O contrato TypeScript é **gerado** do Rust; dois caminhos independentes provam que não divergiu | aceito | 2026-08-09 |
| [0028](0028-web-platform-topology-and-state-boundary.md) | Topologia da Web Platform (`console-web/`) e a **fronteira de estado** que ela não pode atravessar | aceito | 2026-08-10 |
| [0029](0029-saida-multi-controlador.md) | Saída multi-controlador: N nós, um mapa, um só caminho — DDP primeiro, sequencial | aceito | 2026-08-14 |

## Quando escrever um novo ADR
Uma mudança merece ADR quando altera um **seam**, um **invariante**, ou uma
**escolha estrutural difícil de reverter** (protocolo, modelo de concorrência,
fronteira de confiança). Correções e features aditivas normais vão no changelog.
