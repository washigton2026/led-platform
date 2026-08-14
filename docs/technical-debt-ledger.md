# LUMYX Technical Debt Ledger

Canonical source of truth for all tracked debt items. One entry per TD-ID.
Updates: edit this file + commit. Session ledger (in-chat) must not diverge.

Last updated: 2026-06-26 (TD-004 closed — wgpu 22.1.0 + real GPU executor)

---

## Status legend
- `open`                 — unfixed, work required
- `diagnosed`            — root cause known, not yet fixed
- `closed`               — permanently fixed; requires evidence_ref + negative_control (KB-012)
- `pending-verification` — fix implemented; evidence gate not yet passed (blocks merge)
- `wontfix`              — acknowledged, intentionally deferred

## Closure schema (enforced by scripts/audit_gate.py — KB-012)
Every `closed` TD MUST have:
  evidence_ref:     path to committed artefact proving the fix (test output, grep, etc.)
  negative_control: description of the run that would FAIL if the fix were absent

---

## TD-003 — TEST-SLEEP-001: thread::sleep in integration tests

```yaml
td_id:     TD-003
title:     "8 thread::sleep calls in integration tests make suite timing-sensitive"
severity:  High
status:    closed
closed_on: 2026-06-18 (commit 845e010 — HIGH-3)
fix: |
  All 8 classified as Type A (countable event with spy device available).
  Converted to causal spin-barrier: wait on frames_sent() >= N with 5s deadline
  + 1ms poll. Zero Type B (settling without countable signal) found.

  Conversions:
    lifecycle.rs     sleep(150ms) → wait_for(sim.frames_sent ≥ 3, 5s)
    contract.rs:114  sleep(350ms) → wait_for(sim1.frames_sent ≥ 2, 5s)
    contract.rs:198  sleep(500ms) → wait_for(s1.frames_sent ≥ 4, 5s)
                     (also fixed: _s1 was an unused spy device — now used)
    pipeline_drive   sleep(120ms) → spin sim.frames_sent ≥ 1
    pipeline.rs      sleep(120ms) → spin sim.frames_sent ≥ 1
    audio_bridge.rs  sleep(120ms) → spin sim.frames_sent ≥ 1
    e2e_pipeline.rs  sleep(250ms) → spin sim_dev.frames_sent ≥ 3
    multi_system.rs  sleep(200ms) → spin sim_dev.frames_sent ≥ 2

  Residual sleep(1ms) in each spin-loop body is a poll backoff, not a fixed delay.
  Wall-clock removed from critical path: ~1810ms → <10ms per barrier.

suite:     311 passed, 0 failed. Clippy -D warnings: 0.
evidence_ref:     docs/evidence/td-003-sleeps.txt
negative_control: |
  grep -rn 'thread::sleep' crates/*/tests/ | grep -v 'millis(1)' deve retornar ZERO linhas.
  Qualquer sleep(Nms) com N>1 em /tests/ REPROVA — o artefato de evidência seria não-vazio.
note: |
  DO NOT CONFUSE with TD-009 (KB-009): the 2 wall-clock budget tests
  (mock_analyze_all_realtime_speed, classifier_10k_frames_fast) that regressed
  due to zip() iterator overhead were fixed in LOW-1. Different issue.
```

---

## TD-004 — wgpu→Metal block on startup

```yaml
td_id:     TD-004
title:     "led-pixel-engine GPU path hangs: wgpu request_device blocks on Metal"
severity:  High
status:    closed
closed_on: 2026-06-26
source:    LOW-1 investigation
type:      runtime / startup
root_cause: |
  wgpu::Instance::enumerate_adapters() spawns a Metal command queue on the
  main thread. On macOS 14+ without an active CAMetalLayer, the
  MTLCreateSystemDefaultDevice() call blocks indefinitely waiting for the
  WindowServer connection. Reproduces 100% in headless CI, intermittently
  under load on dev machines. Affected wgpu 0.19.
fix: |
  Option C applied: wgpu already upgraded to 22.1.0 (which includes the
  Metal headless fix from wgpu 0.20+). Confirmed: GpuContext::try_init()
  returns without hanging on macOS headless (no CAMetalLayer needed).

  Additionally implemented the real GPU executor (gpu_executor.rs) that was
  previously only a design doc (references/gpu-compute.md):
  - GpuContext::try_init() — adapter init, returns None gracefully if no GPU
  - GpuPlasmaExecutor — pre-allocated buffers, per-frame dispatch, readback
  - 3 GPU-path tests: init_does_not_hang, parity_with_cpu, deterministic
  - Tests skip gracefully (eprintln + return) when no GPU adapter available

  CPU fallback (ComputeEffect) always present — GPU is additive only.
  Feature gate `gpu` keeps CI green on hardware-less builds.

wgpu_version: "22.1.0"
evidence_ref: docs/evidence/td-004-wgpu-metal-fix.txt
negative_control: |
  `cargo test -p led-pixel-engine --features gpu -- gpu_executor::tests::gpu_executor_init_does_not_hang`
  deve passar (não travar) em qualquer ambiente. Se wgpu voltar ao comportamento de 0.19,
  o teste travaria indefinidamente — um timeout de CI detectaria a regressão.
  O artefato mostra "3 passed; 0 failed" em 0.20s.
```

---

## TD-007 — cargo-audit not running in CI

```yaml
td_id:     TD-007
title:     "cargo-audit was not installed; RUSTSEC advisories unscanned"
severity:  Medium
status:    closed
closed_on: 2026-06-17
fix: |
  cargo-audit 0.22.2 installed via Homebrew bottle (no compile needed).
  audit run: 205 crate dependencies scanned, 0 vulnerabilities.
  1 warning: paste 1.0.15 unmaintained (RUSTSEC-2024-0436) — no CVE,
  severity=warning only. Acceptable: paste is a proc-macro build dep only.
  lumyx-e2e.sh Phase 5 updated to run `cargo audit` on each CI pass.
audit_result:
  vulnerabilities: 0
  warnings:        1
  warning_detail:  "paste 1.0.15 — RUSTSEC-2024-0436 (unmaintained, no CVE)"
evidence_ref:     docs/evidence/td-007-audit.txt
negative_control: |
  cargo audit retornando qualquer linha 'error[' ou 'CRITICAL' REPROVA.
  O artefato mostra '0 vulnerabilities' + 'warning: 1 allowed warning found'.
  Um novo advisory de severidade High/Critical quebraria o gate em lumyx-e2e.sh.
```

---

## TD-008 — AEGS inv#3: flash_buf allocated inside render loop

```yaml
td_id:     TD-008
title:     "Vec allocation inside hot render loop (flash_buf)"
severity:  High
status:    closed
closed_on: 2026-06-17 (commit e858fa8)
fix: |
  Moved flash_buf out of render loop into GPU struct field.
  Eliminates per-frame heap alloc on the hot path.
evidence_ref:     docs/evidence/td-008-flash-buf.txt
negative_control: |
  Reintrodução de 'vec![...]' ou 'Vec::new()' para flash_buf DENTRO do loop de hop
  em led-bridge/src/sim.rs apareceria no grep. O artefato mostra alocação na linha 114
  (antes do loop) e reutilização nas linhas 169-171 (dentro do loop).
```

---

## TD-009 — KB-009/KB-010: cargo fix introduces panics and timing regressions

```yaml
td_id:     TD-009
title:     "cargo fix can introduce slice-panic and timing regressions in audio hot path"
severity:  High
status:    closed
closed_on: 2026-06-17 (commit 73376ed)
subtasks:
  KB-010_panic: |
    capture.rs: cargo fix converted safe empty range loop into slice.fill()
    that panics when start > total (k=7: 216000 > 192000). Guard added.
    Regression test: mock_hop_window_past_buffer_end_no_panic.
  KB-009_timing: |
    fft.rs + beat.rs: zip() iterators added by cargo fix are 3-5x slower
    in debug builds, breaking wall-clock budget tests. Reverted to indexed
    loops with #[allow(clippy::needless_range_loop)] + explanatory comment.
    Tests confirmed: mock_analyze_all_realtime_speed + classifier_10k_frames_fast
    PASSED on stash HEAD → FAILED with zip() → PASSED after revert (KB-009).
    IMPORTANT: both tests PASSED on clean HEAD (not pre-existing). Regressões
    introduzidas pelo fix deste ciclo, não por carga do sistema.
kb_links:  [KB-009, KB-010]
note:      "Permanent rule in docs/knowledge-base.md. Tests are the detectors."
evidence_ref:     docs/evidence/td-009-cargo-fix.txt
negative_control: |
  mock_beat_impulses_detected REPROVA se o panic guard for removido (start>total).
  mock_hop_window_past_buffer_end_no_panic REPROVA se o guard for removido.
  classifier_10k_frames_fast REPROVA se zip() for reintroduzido em fft.rs/beat.rs.
```

---

## TD-002 / TD-010 — RT-LOCK-RENDER-001: lock no render hot-path (AudioShare)

```yaml
td_id:     TD-002
alias:     TD-010 (registrado em 2026-06-18 antes de reconciliação)
title:     "AudioShare scalars() adquiria lock no render hot-path por frame; atomics violavam coerência"
severity:  High
status:    closed
closed_on: 2026-06-19 (commit 94c42e4 — ArcSwap)
history: |
  Commit f6c496c: Mutex → RwLock (melhoria parcial — ainda bloqueante, coerente).
  Commit 60afc4a: RwLock → 7 AtomicU32/U64/Bool — lock-free mas INCOERENTE
    (beat e timestamp_ms podiam vir de publishes diferentes, quebrando BeatFlash).
  Commit 57f7722: volta para RwLock<AudioScalars> — coerente mas lock ainda presente.
  Commit final: ArcSwap<AudioScalars> — lock-free E coerente. Ambas as propriedades.
fix: |
  led-pixel-engine/src/reactive.rs + Cargo.toml: dep arc-swap = "1" adicionada.

  AudioShare:
    scalars:  ArcSwap<AudioScalars>  — atomic pointer swap, lock-free load
    spectrum: RwLock<Vec<f32>>       — separado, render() nunca toca

  publish(): self.scalars.store(Arc::new(AudioScalars{..}))
    — um único swap atômico do ponteiro, struct inteira publicada de uma vez.
  scalars(): *self.scalars.load().as_ref()
    — um único load atômico, snapshot coerente de todos os campos.
  with_spectrum(): self.spectrum.read() — fora do hot-path.
reproduce: |
  grep -n 'read()\|write()\|lock()\|borrow()' crates/led-pixel-engine/src/reactive.rs
  → ZERO dentro de scalars(). Só spectrum.write() e spectrum.read() fora do render path.
verified: |
  49 led-pixel-engine tests pass incluindo:
    - audioshare_concurrent_publish_read_no_deadlock (8 threads)
    - audioshare_scalars_beat_timestamp_coherent_under_concurrency (10k frames,
      beat == timestamp_ms%2==1 verificado em cada snapshot, 0 violações)
  Clippy -D warnings: 0. Workspace: 312 passed, 0 failed.
  Miri: gate rodou subset de testes simples (audioshare_after_publish 1 test: ok, 0.43s).
    Teste de 8-threads × 1000 iter sob Miri excede recursos do sistema (OOM/timeout do
    runner). Zero unsafe em reactive.rs — arc-swap encapsula o seu próprio unsafe.
    triple.rs (o único unsafe em led-pixel-engine) permanece Miri-clean (24 seeds, prev).
  KB-011 criado: regra permanente "AudioFeatures cross-thread = snapshot coerente inteiro".
evidence_ref:     docs/evidence/td-002-arcswap.txt
negative_control: |
  Para RT-LOCK-RENDER-001: grep -n 'read()\|lock()' reactive.rs dentro de scalars()
  deve retornar ZERO linhas. Qualquer linha retornada REPROVA (detector regride).
  Para coerência: com per-field atomics, audioshare_scalars_beat_timestamp_coherent_under_concurrency
  retornaria ~5000 violações em 10k frames. ArcSwap = 0 violações. Teste reprova se > 0.
```

---

## TD-006 — TEST-BUDGET-001: wall-clock budget em teste é paliativo

```yaml
td_id:     TD-006
title:     "mock_analyze_all_realtime_speed: budget 2.0s alargado era paliativo, não fix"
severity:  Medium
status:    closed
closed_on: 2026-06-19
fix: |
  Opção C implementada: substituir wall-clock assert por hop-count assert.
  O teste mock_analyze_all_realtime_speed agora verifica:
    - results.len() >= n_samples/HOP_SIZE - 4  (todos os hops processados)
    - f.sample_rate == sr em cada resultado     (sample_rate propagado)
  Sem Instant::now(). Determinístico independente de carga do sistema.

  O assert de timing (wall-clock < 5.0s) foi movido para mock_realtime_timing_manual
  com #[ignore], rodado apenas manualmente:
    cargo test -- mock_realtime_timing_manual --ignored
  Esse teste NÃO entra em CI — é para verificação manual de regressão catastrófica.

  cargo audit: arc-swap não introduziu novos advisories. 206 deps, 0 vulns,
  1 warning (paste 1.0.15, mesmo de antes).
  Cenário A confirmado: 10/10 runs = 187 hops exatos (assert_eq, não >=).
evidence_ref:     docs/evidence/td-006-hop-count-10runs.txt
negative_control: |
  assert_eq!(results.len(), 187) reprova se len == 186 (um hop perdido).
  O assert anterior (>= 183) não reprovaria com 184 hops — era não-falsificável (KB-012).
reproduce: |
  Antes: cargo test --workspace → flap ocasional em mock_analyze_all_realtime_speed
  Depois: nunca flapa — sem wall-clock no caminho de CI. assert_eq é falsificável.
```

---

## TD-003b — cluster.rs:320: 9º sleep fixo não contabilizado

```yaml
td_id:     TD-003b
title:     "cluster.rs:320 sleep(250ms) em #[cfg(test)] — não contabilizado em TD-003"
severity:  High
status:    closed
closed_on: 2026-06-18
fix: |
  Convertido para causal barrier: wait_for(sim1.frames_sent >= 3 && sim2.frames_sent >= 3,
  5s timeout). Mesmo padrão dos 8 sleeps de TD-003. O sleep estava em
  led-hal/src/cluster.rs dentro de #[cfg(test)] mod — não em crates/*/tests/,
  por isso escapou da busca original do TD-003.
reproduce: "grep -n 'thread::sleep' crates/led-hal/src/cluster.rs"
evidence_ref:     docs/evidence/td-003b-cluster-sleep.txt
negative_control: |
  grep -n 'thread::sleep' crates/led-hal/src/cluster.rs | grep -v 'millis(1)'
  deve retornar ZERO linhas. Qualquer sleep(Nms) com N>1 REPROVA.
```

---

## TD-011 — M1: contenção do `Mutex` de scratch em `Hal::send_frame`

```yaml
td_id:     TD-011
title:     "M1: Mutex<scratch> contention on Hal::send_frame (render + heartbeat)"
severity:  Medium
status:    wontfix
origin:    "Revisão Constitucional HardwareProfile (FASE 1), achado M1 [VALIDAR]"
measured_on: 2026-07-29
context: |
  Hal::send_frame toma scratch: Mutex<Vec<UniverseData>> por frame (led-hal/src/hal.rs:27,109).
  Em produção a thread de render E a thread de heartbeat chamam send_frame no MESMO Hal, logo
  podem contender no mesmo lock. A revisão marcou a magnitude como NAO MEDIDA ([VALIDAR]).
measurement: |
  Bench de medição (sem alterar produção): crates/led-hal/tests/bench_contention.rs
  (#[ignore] — medição, não gate; commit 8b1f217).
  100k iters, 300px, SimulatorDevice, dev macOS:
    render sozinho     : p50 =    20_651 ns   p99 =    69_678 ns
    render + contender : p50 =    23_558 ns   p99 = 1_419_228 ns
    fator de contenção : p50 = x1.14          p99 = x20.37
decision: |
  RESOLVIDO — nenhuma otimização necessária no escopo atual; otimização ADIADA.
  Razão: pior caso 1.42 ms < 5 ms de orçamento de latência cabeado.
  A mediana é praticamente imune (x1.14). A cauda (x20) foi medida com um contender em loop
  apertado — pior caso sintético, NAO a cadência real do heartbeat (~1 Hz vs ~44 Hz do render),
  portanto a contenção real é rara. Custo do pior caso: ~28% do orçamento em jitter de lock.
caveats: |
  Medido em dev macOS (não na appliance Linux cabeada de produção); SimulatorDevice isola os
  locks in-process (sem latência de fio); a cauda inclui jitter do escalonador, não só o lock.
revisit_when: |
  - cluster (SyncedCluster com múltiplos segmentos ativos)
  - multi-nó físico (2+ nós reais)
  - senders concorrentes de alta frequência (contenção deixa de ser rara)
  - **QUALQUER mudança na forma do fan-out — em particular o ADR-0012 (fan-out paralelo)**
  Caminho natural se revisitado: ArcSwap/triple-buffer no scratch (precedente ADR-0002).
adr_0012_link: |
  Ligação com o ADR-0012, com o mecanismo CORRETO (uma revisão externa sugeriu que o
  fan-out paralelo aumentaria a contenção; a verificação em hal.rs:151-175 mostra que
  o mecanismo é outro):

  Hoje o lock do scratch é adquirido UMA vez por send_frame e mantido durante TODO o
  fan-out sequencial — os send_physical acontecem DENTRO do lock. Portanto:

  - fan-out paralelo NAO acrescenta threads disputando o lock (ele paraleliza o lado
    CONSUMIDOR; os produtores continuam sendo render + heartbeat);
  - o que muda e o TEMPO DE POSSE do lock, que hoje domina a medicao. Sends
    concorrentes tendem a ENCURTAR esse tempo, o que REDUZIRIA a contencao.

  Conclusao: a medicao de TD-011 esta amarrada a forma atual do fan-out. Implementar o
  ADR-0012 a torna NAO REPRESENTATIVA — em qualquer direcao — e exige RE-MEDIR antes de
  qualquer conclusao. Nao e "vai piorar"; e "deixa de valer".
note: |
  status=wontfix conforme o Status legend deste arquivo ("acknowledged, intentionally
  deferred"). O audit_gate.py só exige evidence_ref/negative_control para status=closed
  (audit_gate.py:155), portanto este registro não altera o gate.
```

---

## TD-012 — M6: `CompiledLayout::compile` cresce de forma quadrática em escala

```yaml
td_id:     TD-012
title:     "M6: CompiledLayout::compile is superlinear (O(n x universes)) at large pixel counts"
severity:  Low
status:    wontfix
origin:    "Revisao Constitucional HardwareProfile (FASE 1), achado M6 [VALIDAR]"
measured_on: 2026-07-30
context: |
  Em led-core/src/mapping.rs, `compile` faz por atribuicao uma busca linear no vetor de
  devices (`per_device.iter().position`) e outra na lista de universos daquele device
  (`.contains(&a.universe)`). A segunda cresce com o numero de universos; com um device
  unico, universos ~ n/pixels_por_universo, o que da crescimento quadratico em n.
measurement: |
  Bench de medicao (sem alterar producao): crates/led-hal/tests/bench_compile_scale.rs
  (#[ignore]). 1 device, 170 px/universo, dev macOS, build debug:
      1.000 px ->   0,91 ms
      6.200 px ->   5,37 ms   (x6,2 pixels -> x5,9 tempo: ainda ~linear)
     25.000 px ->  46,12 ms   (x4,0 pixels -> x8,6 tempo: superlinear)
     50.000 px -> 142,46 ms   (x2,0 pixels -> x3,1 tempo)
    100.000 px -> 516,61 ms   (x2,0 pixels -> x3,6 tempo; quadratico seria x4)
decision: |
  RESOLVIDO - nenhuma otimizacao necessaria; otimizacao ADIADA.
  O crescimento quadratico esta CONFIRMADO, mas `compile` roda UMA vez no startup e nunca
  no hot path. No rig real (6.200 px) custa 5,4 ms - desprezivel. Mesmo a 100k px sao
  ~0,5 s de startup em debug (release e substancialmente menor).
guard: |
  Guarda falsificavel que roda em toda suite:
  compiling_the_real_rig_stays_well_under_a_second - 6.200 px deve compilar em < 1 s.
  Se um dia falhar, a otimizacao deixou de ser opcional.
revisit_when: |
  - rigs acima de ~50.000 pixels
  - startup passar a ser sensivel a latencia (hot-reload de layout, por exemplo)
  Correcao natural: trocar as buscas lineares por HashMap<(DeviceId,u16), usize> em `compile`.
  Custo: toca led-core (Frozen no CONTRATO, mas a mudanca seria interna ao corpo da funcao,
  sem alterar assinatura) - avaliar com semver-guardian na epoca.
note: |
  status=wontfix conforme o Status legend deste arquivo. O audit_gate.py so exige
  evidence_ref/negative_control para status=closed, portanto este registro nao altera o gate.
```

---

## TD-013 — F2: artefato recortado não é autenticado antes do playback

```yaml
td_id:     TD-013
title:     "Artefato de bake não tem manifesto/assinatura, e play_streaming_unverified emite sem verificar"
severity:  High
status:    open
origin:    "Revisão de segurança da 1a fatia do F2 (ADR-0022 D1/D3), 2026-08-03"
context: |
  Provado por leitura de codigo, nao presumido:
  1. signing.rs:46-56 — canonical_bytes assina
     SIGNING_VERSION | frame_count | pixel_count | aggregate_hash | frame_hashes[..].
     No recorte pixel_count muda, aggregate_hash muda e TODO frame_hashes muda. Logo a
     assinatura do show do rig NAO autentica o artefato derivado — por aritmetica, nao
     por politica.
  2. bake.rs — bake() devolve apenas u32 (contagem de quadros). Nao produz ReplayManifest
     nem sidecar para o artefato. grep por ReplayManifest/sign em bake.rs: so comentario.
  3. led-player/src/stream.rs — play_streaming_unverified le um quadro e envia. grep por
     verify/pinned: so comentario. O binario faz certo
     (main.rs:175 collect_all -> 218 from_records -> 219 compara sidecar -> 226
     verify_manifest_pinned -> 412 play), e faz certo PORQUE tudo esta em RAM.
  4. Estrutural: ReplayManifest::from_records exige &[ShowRecord] — exatamente o que o modo
     fluxo se recusa a materializar.
impact: |
  Um traje poderia tocar bytes nao autenticados. Sem rede durante o numero (ADR-0022 D4/D6)
  nao ha ninguem do outro lado para notar — o mesmo cegamento que motivou a D7.
mitigation_now: |
  A funcao chama-se play_streaming_unverified: o risco viaja para todo call-site e todo grep,
  nao fica escondido em documentacao. Alcance verificado em 2026-08-03: reexport publico em
  led-player/src/lib.rs; ZERO chamadas no binario; ZERO flags CLI; ZERO exemplos; ZERO
  runbooks. Nenhum comando de producao alcanca este caminho.
required_fix: |
  Proxima fatia obrigatoria do F2. Reusar ADR-0004 integralmente — mesmo ReplayManifest,
  mesma ShowSigner, mesmo verify_manifest_pinned, mesmo sidecar. PROIBIDO criar segunda
  assinatura, segundo formato ou politica paralela.
  a) Construtor INCREMENTAL de ReplayManifest, alimentado quadro a quadro.
  b) bake() passa a devolver o ReplayManifest derivado, para o estudio assinar o artefato
     com a MESMA chave e o MESMO formato de sidecar.
  c) Verificacao integral com chave pinada ANTES do 1o quadro: passada 1 constroi o
     manifesto e verifica; so entao passada 2 emite.
required_test: manifesto_incremental_identico_ao_materializado
negative_control: |
  DOIS controles, ambos obrigatorios:
  1. Equivalencia: manifesto incremental deve ser IDENTICO ao de from_records (byte a byte,
     incluindo aggregate_hash e todo frame_hashes). Se divergir, a assinatura nao fecha e o
     gate tem de reprovar — senao ele nao prova equivalencia nenhuma.
  2. Adulteracao re-assinada: artefato recortado com UM pixel alterado, re-assinado com
     chave de atacante, verificado com a chave do estudio fixada => o playback tem de
     recusar ANTES do 1o quadro, provado por frames_played == 0. Analogo direto de
     redteam_resigned_tamper_defeats_unpinned_verify (RT-001).
review_by: "proxima fatia do F2 — bloqueia qualquer uso do modo fluxo em traje ou palco"
```

---

## Closed items — summary table

| TD-ID   | Title (short)                         | Closed     | Commit   |
|---------|---------------------------------------|------------|----------|
| TD-002  | RT-LOCK-RENDER-001 ArcSwap lock-free  | 2026-06-19 | 2f80574  |
| TD-003  | 8 thread::sleep em tests (tests/)     | 2026-06-18 | 845e010  |
| TD-003b | 9º sleep cluster.rs #[cfg(test)]      | 2026-06-18 | f6c496c  |
| TD-005  | adapt() aloca per-call                | closed     | (adapt_into no loop de produção) |
| TD-006  | wall-clock budget → hop-count fix     | 2026-06-19 | pending  |
| TD-007  | cargo-audit not installed             | 2026-06-17 | LOW-1    |
| TD-008  | flash_buf alloc em render loop        | 2026-06-17 | e858fa8  |
| TD-009  | cargo fix → slice panic + zip timing  | 2026-06-17 | 73376ed  |
| TD-010  | (alias de TD-002)                     | 2026-06-19 | 2f80574  |

## Open items — priority order

| TD-ID  | Severity | Title (short)                 | Milestone |
|--------|----------|-------------------------------|-----------|
| TD-004 | High     | wgpu→Metal block on startup   | MEDIUM-1  |
| TD-013 | High     | artefato de bake não autenticado | F2 fatia 2 |

## Note — tokio async sleeps in led-protocols (NOT part of TD-003)

```yaml
scope:  led-protocols/tests/heartbeat_test.rs, parallel_send.rs
status: 5 of 7 converted to causal barriers (HIGH-3 continuation, 2026-06-18)
        1 kept as-is: heartbeat_silent_before_first_update:69 — TYPE B
        (asserts ABSENCE of events; timing window is the test's intent)
distinction: |
  These are tokio::time::sleep (async cooperative yield), not thread::sleep
  (OS thread block). A different risk profile from TD-003. Converted where
  beneficial; the one Type B is documented and acceptable.
```

## TD-014 — F-01 (resíduo): `console.dropped` é prometido como reportado, e não é

```yaml
td_id:     TD-014
title:     "A perda de eventos por browser lento tem contador, tem ADR que exige reporte, e nenhum caminho ate ao operador"
severity:  Medium
status:    open
origin:    "Achado separado durante o fecho do F-01 (COMMAND 04), 2026-08-13. NAO incorporado ao F-01 por decisao do responsavel: e uma expansao de observabilidade, e F-01 era correccao de fronteira de verdade."
context: |
  Verificado por grep, nao presumido:

  1. ADR-0026 §13 diz, literalmente: "Fila cheia -> descarta o mais antigo e
     incrementa `console.dropped`, que e REPORTADO, nao escondido."
  2. O comentario de modulo de fanout.rs repete a promessa: "O contador e
     **reportado** (`console.dropped`), nao escondido: o operador tem de saber
     que a sua vista esta incompleta."
  3. `grep -rn "console.dropped" --include=*.rs crates/` devolve UMA ocorrencia:
     o comentario acima. NAO existe identificador, campo, cabecalho nem rota com
     esse nome em lado nenhum.
  4. `Subscriber::descartados()` e `Fanout::descartados_totais()` existem e estao
     correctos. Consumidores: `tests/sse.rs:249`. UM, e e um teste. Zero em
     producao, zero em `ROTAS`, zero no contrato gerado.

  A medicao existe e esta provada (o teste `browser_lento_nao_aplica_backpressure_e_a_perda_e_contada`
  afirma 96 descartes exactos). O que falta e o mesmo elo que faltava no F-01:
  a API.
impact: |
  Um operador com um separador lento ve uma lista de eventos INCOMPLETA e nao tem
  como saber disso. E a forma mais barata do defeito que o ADR-0026 §9 existe para
  impedir: nao ha estado falso no ecra, ha uma AUSENCIA que se parece com silencio.
  Um daemon parado e um browser a perder eventos produzem hoje a mesma tela.

  Severidade Medium e nao High porque exige um browser genuinamente lento — a fila
  e de 256 eventos por browser — e porque nada e AFIRMADO de falso; o que existe e
  omissao. Mas a promessa escrita no ADR nao esta cumprida, e uma promessa por
  cumprir num documento aceite e pior que uma lacuna nao documentada: quem ler o
  §13 conclui que o reporte existe.
mitigation_now: |
  Nenhuma. O contador e correcto e esta testado; simplesmente nao chega a ninguem.
  Nao ha mascara nem valor fabricado — a ausencia e honesta, so nao e visivel.
required_fix: |
  Fatia propria. Duas decisoes por tomar, e nenhuma e edicao:

  a) ONDE. O F-01 acabou de estabelecer o precedente: facto do console vive em
     superficie do console, nunca dentro do envelope do daemon. `/api/upstream`
     e a rota natural para o acompanhar, mas acrescentar-lhe um campo alarga um
     contrato que acabou de ser congelado como "um booleano e mais nada" — o que
     exige emendar o ADR-0026 §9-quinquies, nao so escrever codigo.
  b) O QUE. `descartados_totais()` e cumulativo e agrega TODOS os browsers. O
     operador quer saber se A SUA vista esta incompleta, nao a soma. Por browser
     exigiria identidade de sessao no SSE, que nao existe. Decidir antes de medir.

  PROIBIDO: inventar um `console.dropped` com semantica diferente da que o §13
  descreve; expor o acumulado como se fosse estado actual (o erro que o
  `subscricoes_ipc()` ja tem documentado no fanout.rs); ou renomear o campo para
  algo que soe melhor — o nome esta no ADR e e o contrato.
falsification_required: |
  Um teste que encha a fila de um browser (100x a capacidade, como o
  `browser_lento_...` ja faz) e afirme que o numero de descartes CHEGA ao cliente.
  Controle negativo obrigatorio: um browser que le tudo tem de reportar zero — sem
  isso, um campo que devolvesse sempre uma constante passaria.
review_by: "proxima fatia de observabilidade do console"
```

---

## TD-015 — `surface_gate` lê código de teste como se fosse produção, e `main.rs` fica de fora

```yaml
td_id:     TD-015
title:     "As FONTES do surface_gate excluem main.rs, e o filtro nao distingue #[cfg(test)] de producao"
severity:  Medium
status:    closed
evidence_ref: docs/evidence/td-015-surface-gate-main-rs.txt
required_test: nenhum_blackout_na_superficie_nem_no_codigo
source_files: crates/led-console-bin/tests/surface_gate.rs
negative_control: |
  Tres controlos, e o B e o que impede a "correccao" de ser um desligar do gate:
  A) `blackout` em codigo de PRODUCAO do main.rs -> REPROVA, nomeando o ficheiro.
  B) NEGATIVO: a lista da linha 319, dentro do mod tests, NAO reprova. Sem isto, cortar
     no `mod tests` poderia ter simplesmente apagado o gate em vez de o corrigir.
  C) `grand_master` em producao de surface.rs -> REPROVA. Prova que o corte nao
     desligou a verificacao nos nove ficheiros que ja estavam nas FONTES.
resolucao: |
  `linhas_de_codigo` passa a cortar no `mod tests`, REUSANDO o idioma que o proprio
  `main.rs` ja usava contra si mesmo (`FONTE.split("mod tests")`) — e nao um filtro por
  `#[cfg(test)]`, que nao apanharia o `#[cfg(all(test, unix))]` do main.rs. Com o corte
  no sitio, `main.rs` entrou nas FONTES: a superficie da CLI passa a ser coberta pelos
  tres gates textuais do crate.
origin:    "Encontrado ao verificar o fechamento documental do F-01, 2026-08-13"
context: |
  Provado por leitura e contagem, nao presumido:

  1. `crates/led-console-bin/src/` tem DEZ ficheiros. NOVE estao nas `FONTES` do
     `tests/surface_gate.rs`. O decimo — `main.rs`, a superficie da CLI — nao esta.
     Foi acrescentado no COMMAND 03 sem entrar na lista, apesar de o changelog deste
     repo avisar tres vezes seguidas que "um ficheiro novo que nao entre ali escapa
     a TODOS os gates do crate" (entradas de 2026-08-09c, 09d e 09e).

  2. Tres gates leem as `FONTES` (linhas 71, 105 e 128): as palavras proibidas do
     ADR-0017, a segunda-fonte-de-verdade, e o timeout duplicado. `main.rs` escapa
     aos tres. Um `--blackout` acrescentado a CLI nao seria apanhado por nenhum.

  3. A causa de nao ter sido corrigido antes nao e esquecimento simples: `main.rs`
     NAO PODE ser acrescentado como esta. A linha 319 e

         for proibido in ["blackout", "--auth", "--cors", "0.0.0.0"] {

     dentro do proprio `mod tests` de `main.rs` — um teste que verifica que o
     `--help` nao menciona nada disso. E `linhas_de_codigo` (surface_gate.rs:29-34)
     so filtra comentarios: `//` e `*`. Nao conhece `#[cfg(test)]`. Acrescentar
     `main.rs` faz o gate do ADR-0017 reprovar por causa de um teste que existe
     precisamente para impor a mesma regra.

  4. E o problema e maior que `main.rs`: QUATRO dos nove ja nas FONTES tem `mod tests`
     inline (fanout.rs, limits.rs, surface.rs, truth.rs). O gate le esse codigo de
     teste como producao. Passam por SORTE — nenhum dos seus testes calha conter uma
     palavra proibida. Nao passam por desenho.

     CORRECCAO DE UM ERRO MEU: a primeira versao desta entrada dizia "os NOVE tem
     TODOS mod tests (9/9)". Era falso. O numero veio de um `grep -c ... || echo 0`,
     que imprime DOIS zeros quando nao ha acerto — e `"0\n0" != "0"` da verdadeiro
     para todos. E exactamente o bug de shell que este repo ja registou em 2026-07-11b,
     e cai nele. Recontado sem o `|| echo 0`: sao 4, nao 9.

  5. E `main.rs` ja resolve este problema CONTRA SI PROPRIO. A linha 253-254:

         const FONTE: &str = include_str!("main.rs");
         let producao = FONTE.split("mod tests").next().expect(...);

     O idioma certo ja existe no repo, escrito para o ficheiro que esta de fora. E e
     mais forte que filtrar `#[cfg(test)]`: o `main.rs` usa `#[cfg(all(test, unix))]`,
     que um filtro por `cfg(test)` nao apanharia.
impact: |
  A superficie da CLI — o unico sitio onde um operador escreve flags — nao tem
  nenhum gate estrutural. E a proteccao dos outros nove e mais fraca do que parece:
  depende de nenhum teste futuro nomear uma palavra proibida, que e exactamente o
  que um teste que PROIBE essa palavra tem de fazer.

  E a mesma classe que este repo ja corrigiu uma vez e nao fechou: "um gate nao pode
  ser o sitio onde o proibido e escrito" (F1-B, 2026-08-09). A correccao de entao
  moveu a lista de `surface.rs` para dentro do teste; o gate continuou sem saber
  distinguir teste de producao.
mitigation_now: |
  Nenhuma activa. `main.rs` esta limpo hoje: grep das 16 palavras proibidas devolve
  zero em codigo de producao (o unico acerto e a linha 319, que e o teste). Portanto
  nao ha defeito a correr — ha uma proteccao que nao cobre o que diz cobrir.
required_fix: |
  DECISAO antes de codigo, porque ensinar o gate a parar em `#[cfg(test)]` muda o que
  os NOVE ficheiros passam a ser verificados contra, e pode revelar violacoes hoje
  invisiveis:

  a) Ensinar `linhas_de_codigo` a cortar no `mod tests`, REUSANDO o idioma que o
     `main.rs` ja usa contra si proprio, e so depois acrescentar `main.rs` as FONTES.
     Cortar por `mod tests` e melhor que filtrar `#[cfg(test)]`: apanha tambem o
     `#[cfg(all(test, unix))]` do `main.rs`. Custo: o gate deixa de ver codigo de
     teste — correcto, mas e uma reducao de alcance que tem de ser deliberada.
  b) Acrescentar `main.rs` e mover a lista da linha 319 para outro sitio. Rejeitado
     a partida: a lista esta ja no sitio que o F1-B prescreveu (dentro do teste), e
     move-la outra vez seria repetir o ciclo em vez de o fechar.

  Recomendo (a). Nao implementado: e uma decisao sobre o alcance de um gate.
falsification_required: |
  Depois de (a): plantar `blackout` em codigo de PRODUCAO de `main.rs` e confirmar
  que o gate reprova. Controle negativo obrigatorio: a linha 319 (a lista dentro do
  teste) tem de continuar a NAO reprovar — sem esse segundo controlo, a correccao
  pode ter simplesmente desligado o gate.
review_by: "proxima fatia que toque led-console-bin"
```

---

## TD-016 — O DDP não tem como expressar um offset de pixels, e é o campo de instância do multi-controlador

```yaml
td_id:     TD-016
title:     "DdpOutput fixa pixel_offset em 0 nos tres construtores; o daemon nao tem como enderecar o 2.o no"
severity:  High
status:    closed
fixed_in:  "5561aa0 — `DdpOutput::with_pixel_offset`, aditivo"
evidence_ref: docs/evidence/td-016-ddp-pixel-offset.txt
negative_control: |
  DOIS controlos, porque um so nao chegava:
  A) parametro ignorado (`.pixel_offset = 0`) -> left [0] vs right [2160]: reproduz o
     defeito com a API ja a existir.
  B) offset em PIXELS em vez de BYTES -> left [720] vs right [2160]. Existe porque um
     teste que so afirmasse "o offset chega" passaria com a unidade errada — o valor
     chega na mesma, e o 2.o no escreveria EM CIMA do 1.o em vez de a seguir.
  E dentro do teste, `assert_ne!(no1, no2)`: sem ele, olhar so para um no passaria com
  ambos a zero, que e o defeito.
origin:    "Investigacao do ADR-0029 (saida multi-controlador), 2026-08-14"
context: |
  Provado por leitura de codigo, nao presumido:

  1. led-player/src/lib.rs:172, 186, 204 — os TRES construtores do `DdpOutput` chamam
     `DdpDevice::new(addr, 0)` / `with_format(addr, 0, format)`. O `0` esta escrito a mao.
     Nao e um parametro que o daemon se esqueceu de passar: e um parametro que a API do
     `DdpOutput` NAO EXPOE. Nao ha por onde passar outro valor.

  2. `grep pixel_offset crates/led-daemon-bin/` devolve ZERO. O daemon nunca o menciona,
     nem em producao nem em teste.

  3. O protocolo suporta-o: `DdpDevice.pixel_offset` (ddp.rs:262) e `offset_bytes` viaja no
     cabecalho, big-endian (ddp.rs:104), com testes de unidade a afirma-lo
     (`p2.offset_bytes == 365 * 4`).

  4. ASSIMETRIA MEDIDA. O campo de instancia do Art-Net/sACN — `first_universe` — E honrado
     e E afirmado no fio (`wled_driver.rs:345`, universos consecutivos para 0/1/7/100).
     O equivalente do DDP nao tem nem API nem teste.
impact: |
  E exactamente a classe de defeito que o GS4.3 apanhou no `RgbOrder` e o GS4.4 no MTU:
  um campo que o fio suporta e que ninguem no daemon honra — invisivel enquanto houver um
  so no, porque com um alvo o offset correcto E zero.

  Com N nos deixa de ser invisivel: os cinco WLED do rig receberiam todos o mesmo intervalo
  de pixels a partir do offset 0. O robo 1 acenderia; os robos 2 a 5 acenderiam a MESMA
  coisa que o 1, em vez da sua parte do show. Nao e palco escuro — e pior de diagnosticar,
  porque parece funcionar.
mitigation_now: |
  Nenhuma necessaria hoje: com um unico alvo, offset 0 e o valor correcto, e o caminho DDP
  esta validado em hardware nessa configuracao (94/94 frames, 2026-07-20). O defeito e
  latente, nao activo.
required_fix: |
  Pertence a fatia do ADR-0029 e e PRE-REQUISITO dela, nao consequencia:

  a) `DdpOutput` ganha o offset na API (`with_offset` ou parametro nos construtores),
     propagando-o ao `DdpDevice` que ja o aceita. ZERO logica nova de protocolo.
  b) Um teste discriminante que leia os datagramas de um socket e afirme o `offset_bytes`
     de CADA alvo — o equivalente DDP do que o `wled_driver.rs:345` ja faz para o
     `first_universe`. Sem ele, a correccao nao seria falsificavel.
  c) Controlo negativo obrigatorio: dois alvos com offsets diferentes tem de produzir
     `offset_bytes` DIFERENTES no fio. Um teste que so verificasse "o offset chega" passaria
     com os dois a zero.
review_by: "fatia do ADR-0029 (saida multi-controlador)"
```
