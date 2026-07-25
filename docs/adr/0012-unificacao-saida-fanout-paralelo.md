# ADR-0012 — Unificação da saída: fan-out paralelo adiado; serialização já é fonte única

- **Status:** aceito (decisão) — **implementação adiada até 2º nó físico**
- **Data original:** 2026-07-25
- **Fonte:** Revisão Constitucional HardwareProfile, achados H1/H2 + FASE 2 (revisão crítica das
  próprias conclusões)

## Contexto e problema
A Revisão Constitucional levantou **H1** ("três serializadores sACN — duplicação") e **H2**
("fan-out sequencial do HAL — gargalo de escala"). Ao aprofundar (FASE 2, "não assuma que suas
conclusões estão corretas"):

- **H1 está superdimensionado.** A serialização E1.31 já é **fonte única**: `SacnDevice`,
  `SacnBackend` e `ParallelSender` chamam o **mesmo** `packet::build_data_packet`
  (`device.rs:126`, `router.rs:74`, `sender.rs:168`). Não existem três serializadores — existem
  **três modelos de entrega** distintos e legítimos: síncrono single-socket (`SacnDevice`),
  por-universo sob roteador multi-protocolo (`SacnBackend`), e assíncrono persistente
  (`ParallelSender`, 1 task tokio/universo via `watch`). Resíduo real e **menor**: `SacnBackend`
  fixa CID `"LUMYX Router"`/priority 100 (`router.rs:72-74`) em vez de parametrizar.
- **H2 está confirmado.** `Hal::send_frame` faz fan-out **sequencial** (`hal.rs:115`): os
  `send_physical` de N devices serializam numa thread. Vira gargalo a partir de ~100
  controladores.

## Decisão
1. **Não reescrever o hot-path do HAL agora.** O `ParallelSender` já existe para escala; ligá-lo
   ao HAL substitui o modelo `DeviceDriver` (que `SimulatorDevice` e a suíte de testes usam) por
   um modelo watch-por-universo — uma **re-arquitetura real** do estágio de saída. Com **1 nó
   físico** (bancada validada 2026-07-20), o fan-out sequencial cabe folgado no budget de
   latência. Adiar segue o precedente explícito do **ADR-0010** (lock-free de cluster adiado
   "até existir um segundo nó físico — evita overengineering") e o próprio achado da revisão
   ("SIMPLIFICAR, focar no 1x real").
2. **Gatilho de reativação:** ao existir um **2º nó** / **> ~50 controladores**, ligar o HAL ao
   `ParallelSender` (fan-out paralelo, sockets dedicados por universo) mantendo `DeviceDriver`
   para simulador/testes; e parametrizar CID/priority do `SacnBackend`. **Gate obrigatório:**
   `no_alloc` + wire-test + `parallel_send` + bench de N universos dentro do budget de 1 ms UDP.
3. **H1 rebaixado** de HIGH para **LOW** (cleanup de CID/priority), pois a duplicação alegada não
   existe na serialização.

## Consequências
**Boas:** evita re-arquitetar o hot-path para uma escala que ainda não existe; a decisão e seu
gatilho ficam auditáveis; corrige de forma honesta uma conclusão superdimensionada da própria
revisão (FASE 2 funcionando). **Ruins/custos:** o gargalo H2 permanece **latente** — mitigado por
estar documentado e gated por escala (não esquecido). O resíduo de CID/priority do `SacnBackend`
fica em aberto (LOW).

## Alternativas rejeitadas
- **Reescrever já para fan-out paralelo** — overengineering a 1 nó; mesma lógica que adiou o
  lock-free no ADR-0010.
- **Colapsar os três wrappers num só** — perderia modelos de entrega legítimos (sync/roteado/
  assíncrono); a serialização de fio já é única (`build_data_packet`), então não há o ganho
  alegado.
