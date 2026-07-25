# ADR-0007 — Seams canônicos certificados por SemVer

- **Status:** aceito
- **Data original:** 2026-06-28 (`led-core/src/contract_version.rs`); a política
  "seam muda em um lugar" existe desde a fundação (2026-06-03)
- **Fonte:** CLAUDE.md changelog 2026-06-28; contract_version.rs; KB-012

## Contexto e problema
O workspace tem 12+ crates que dependem de um núcleo neutro (`led-core`) via
tipos-seam: `ProtocolOutput`, `DeviceDriver`, `IDevice`, `LogicalFrame`,
`AudioFeatures`, `CompiledLayout`, `UniverseData`, `Provenance`,
`MusicalSection`. Uma mudança acidental na assinatura de um desses tipos
quebra silenciosamente todos os consumidores — e sem um contrato explícito,
ninguém sabe *quais* tipos são intocáveis e *quais* podem evoluir.

## Decisão
Certificar os seams com uma política SemVer explícita em `contract_version.rs`:
- `ContractStability`: **Frozen** (ProtocolOutput, DeviceDriver, IDevice,
  CompiledLayout, UniverseData — nunca mudam de assinatura), **Stable**
  (LogicalFrame, AudioFeatures, Provenance, MusicalSection — evoluem com bump),
  **Evolving**.
- `certified_contracts()` = 9 contratos versionados; `LED_CORE_CONTRACT_VERSION`.
- Regra de processo: mudança de seam edita `led-core` **em um lugar**, atualiza
  os dois lados e o `CLAUDE.md`, e **bumpa a versão**. O `semver-guardian`
  (`lumyx_guardian.sh`) faz snapshot da superfície pública de `led-core` e
  bloqueia um diff sem bump de versão.

## Consequências
**Boas:** um breaking change acidental vira um gate vermelho, não um bug em
produção; o baseline `.lumyx-guardian/led-core-api.txt` é o contrato versionado.
Distingue explicitamente o que é intocável (Frozen) do que evolui (Stable).
Controle negativo provado: injetar um item de seam sem bump → BLOCK.
**Ruins/custos:** disciplina de processo — toda mudança de seam custa um bump +
atualização de baseline. `AudioFeatures` tem duas versões (v0 em led-core, v1 em
audio-core) reconciliadas no `led-bridge` — divergência de contrato aceita e
documentada, não silenciosamente unificada.

## Alternativas rejeitadas
- **Sem certificação, confiar em revisão humana** — KB-012: gates que não
  exercitam a propriedade passam falso-verde; revisão humana esquece.
- **`cargo-semver-checks`** — ferramenta externa não instalada; o snapshot da
  superfície pública + diff dá 90% do valor com zero dependências.
- **Congelar tudo** — impediria a evolução legítima de `AudioFeatures`/
  `LogicalFrame`; por isso a distinção Frozen vs Stable.
