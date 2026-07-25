# LUMYX — Production Certification Ledger

Programa: **LUMYX Production Certification** · Atualizado: 2026-07-12
Regra: um critério só vira ✅ com **evidência verificável** (arquivo, log ou
comando reproduzível). Sem evidência = ⏳, sem exceções.

## HARDWARE

| Critério | Status | Evidência / próximo passo |
|---|---|---|
| WLED validado | ⏳ bloqueado no rig | Comando pronto e ensaiado em simulador idêntico: `cargo run -p led-player -- robot_sequence.lumyx --ddp 192.168.2.156 --metrics 9464`. Rig re-verificado offline em 2026-07-11. |
| Falcon validado | ⏳ requer hardware Falcon | Caminho sACN unicast/multicast pronto (`SacnDevice`, 69 testes de protocolo); falta o controlador físico. |
| FPP validado | ⏳ requer hardware FPP | FPP aceita DDP/E1.31 — ambos implementados e testados em loopback; falta o dispositivo. |

## CONFIABILIDADE

| Critério | Status | Evidência |
|---|---|---|
| Burn-in 1h (software, pré-gate) | ✅ | `burnin-20260711-083048.jsonl` — **30 passes, 0 aborts**, show real 6.200px, hash estável em todos os passes |
| Burn-in 72h | ⏳ pronto p/ lançar | Harness + `scripts/com.lumyx.burnin.ready.plist` (launchd) prontos. Precisa de contexto persistente: `launchctl load ~/Library/LaunchAgents/com.lumyx.burnin.plist`. Processo `nohup` dentro de sessão de ferramenta não persiste 72h — deve ser lançado pelo usuário |
| Burn-in 168h | ⏳ | Editar `72`→`168` no plist; após 72h aprovado |
| Chaos físico | 🟡 parcial | Wire-level real: `integration-tests/tests/udp_chaos.rs` (5 testes — perda 30%, outage 100%, heal, latência, determinismo). Literal (puxar cabo): runbook = burn-in rodando + desconectar ETH do controlador; requer rig |
| Failover / Hot-join / Clock drift | ✅ | `two_node_cluster.rs` (6 testes) + `net_time` (5 testes) + Phase 11 do e2e PASS |

## SEGURANÇA

| Critério | Status | Evidência |
|---|---|---|
| Cosign ativo | ✅ | cosign v3.1.1; `scripts/release_sign.sh` executado — `led-player.cosign.bundle`, `led-demo.cosign.bundle`, `verify-blob: Verified OK` (verificação faz parte do pipeline) |
| SBOM ativo | ✅ | `release/sbom.cdx.json` (CycloneDX 1.5, 158 componentes, purl+licença) via `scripts/generate_sbom.py` |
| Attestation ativa | ✅ | `*.sbom.bundle` — attest-blob tipo cyclonedx sobre cada binário de release |
| Ed25519 (replay+snapshot) | ✅ | `led-show-recorder/src/signing.rs`, 12 testes (tamper/wrong-key/sidecar); sidecars `.sig/.pub` nos 3 artefatos |
| **RT-001 — autenticidade da assinatura (achado red-team CRITICAL)** | ✅ MITIGADO | `verify_manifest` confiava na chave embutida (tamper re-assinado passava). Correção: `verify_manifest_pinned` + `led-player --verify-key`. **Provado e2e**: atacante re-assina com chave `34b4…` → palco verifica com chave do estúdio → `SIG VERIFY FAILED` exit 1. Ledger: `docs/red-team/findings.md` |
| Red Team (auditoria adversarial) | ✅ | `scripts/lumyx_red_team.sh` — 5 probes, 0 achados CRITICAL/HIGH abertos; RT-001 mitigado, RT-002/003 aceitos/parciais, RT-004/005 rastreados |

## DETERMINISMO

| Plataforma | Status | Evidência |
|---|---|---|
| macOS (arm64) — referência | ✅ | Goldens pinados 2026-07-09 (`determinism_vector.rs`): intent `0x12ce2cfdf90ff176`, plasma `0x1ed5508a56d0b0bc`; 3 testes PASS em todo e2e |
| ARM | ✅ (arm64 via referência) | A máquina de referência É arm64 (Apple Silicon) |
| Linux | ⏳ execução pendente | Probe pronto: `./scripts/determinism_probe.sh` em qualquer Linux com Rust (1 comando, gera o artefato de evidência). Container local indisponível (sem Docker nesta máquina) |
| Windows | ⏳ execução pendente | Mesmo probe via WSL/Git-Bash |

## OBSERVABILIDADE

| Critério | Status | Evidência |
|---|---|---|
| Prometheus | ✅ | `led_hal::prometheus` (4 testes) + `--metrics` no player; **scrape ao vivo em show real**: p50=0,5ms p99=4,1ms 0 drops |
| Grafana | ✅ | `docs/observability/grafana-lumyx.json` + compose com provisioning automático |
| Alertas | ✅ | `docs/observability/alerts.yml` — 5 regras (fast/slow burn, p99, show stalled, exporter down) montadas no Prometheus do compose |
| SLOs | ✅ | `docs/observability/SLO.md` — 4 SLOs formais + error budgets + política de congelamento |

## GOVERNANÇA

| Critério | Status | Evidência |
|---|---|---|
| Gates C1–C11 | ✅ | e2e 2026-07-11: **15 fases, 708 testes, 0 falhas, exit 0** (`ALL SYSTEMS NOMINAL`) |
| Anti-regressão (toda alteração/PR/release) | ✅ | `scripts/lumyx_guardian.sh` — 6 guardiões mecânicos em ~8,6s, exit 0. Negative control provado: injeção de item de seam sem bump → BLOCK |
| Agentes: 4 times completos | ✅ | `.claude/agents/` — **27 definições**: BUILDER/Sonnet +7, GUARDIAN/Haiku +6, VALIDATOR/Sonnet +5, RED-TEAM/Sonnet +5. Política de modelo respeitada (Haiku=repetitivo/gates, Sonnet=decisão/segurança/chaos) |
| Cadeia Builder→Validator→Guardian→RedTeam | ✅ | `xlights-export` (workbook 6 seções → `lumyx_builder.sh check` APROVADA → `lumyx_validator.sh` PASS 11/1 → Guardian 0 regressões) + RED-TEAM achou e fechou RT-001 CRITICAL (exploit provado e2e) |
| Export xLights (migração bidirecional) | ✅ | `led-xlights::export` — roundtrip campo-a-campo, gate no próprio output, negative control (export adulterado É pego). 647 testes no workspace |
| SemVer | ✅ | `led-core` 1.2.0; contratos Frozen/Stable inalterados (contract_version.rs, 9 certificados) |
| Replay | ✅ | 3 gravações reais verificadas por hash (demo `0x2fb1…`, robot_show `0xda96…`, robot_sequence `0xd8f1…`) + gate P10/P15 |
| Provenance | ✅ | `led-core/provenance.rs` end-to-end + JSON corrigido (KB-014) |

## RESUMO EXECUTIVO

- **Certificado hoje (evidência completa)**: Segurança (incl. achado red-team
  RT-001 fechado), Observabilidade, Governança, Determinismo (plataforma de
  referência), Confiabilidade lógica, 4 times de agentes operacionais.
- **Em curso**: burn-in software (24+ passes, 0 aborts na última janela).
- **Aguardando recurso externo**: hardware (rig/Falcon/FPP — rig offline),
  Linux/Windows (probe de 1 comando pronto), chaos físico literal, burn-in 72h
  persistente (launchd — processo de sessão não sobrevive 72h).
- Nenhum critério certificado sem artefato. Este ledger é atualizado a cada
  evidência nova; discrepância entre ledger e realidade é bug P0.

## O QUE FALTA PARA "PRODUCTION CERTIFICATION COMPLETE"

Todos os itens restantes dependem de **recurso externo**, não de software:

| Bloqueador | Quem destrava | Comando |
|---|---|---|
| WLED validado | ligar o robô | `led-player robot_sequence.lumyx --ddp 192.168.2.156 --metrics 9464` |
| Falcon/FPP | ter o controlador | mesmo player, `--artnet`/`--ddp` no IP do device |
| Burn-in 72h/168h | lançar fora da sessão | `launchctl load ~/Library/LaunchAgents/com.lumyx.burnin.plist` |
| Chaos físico | rig + puxar cabo | burn-in rodando + desconectar ETH do controlador |
| Determinismo Linux/Windows | máquina/CI | `./scripts/determinism_probe.sh` (1 comando, gera evidência) |

A plataforma (software) está **certificada e pronta**; a certificação de
**produção** aguarda o hardware ligado.
