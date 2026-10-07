# LUMYX Technical Debt Ledger

Canonical source of truth for all tracked debt items. One entry per TD-ID.
Updates: edit this file + commit. Session ledger (in-chat) must not diverge.

Last updated: 2026-10-07 (schema alinhado com o audit_gate R4.1 — `watched:` e região de prova)

---

## Status legend
- `open`                 — unfixed, work required
- `diagnosed`            — root cause known, not yet fixed
- `closed`               — permanently fixed; requires evidence_ref + negative_control (KB-012)
- `pending-verification` — fix implemented; evidence gate not yet passed (OK dentro do `review_by`; Critical depois)
- `wontfix`              — acknowledged, intentionally deferred

## Closure schema (enforced by scripts/audit_gate.py — KB-012)
Every `closed` TD MUST have:
  evidence_ref:     path to committed artefact proving the fix (test output, grep, etc.)
  negative_control: description of the run that would FAIL if the fix were absent
If it has `source_files:`, the evidence pins EACH of them by content, one line each, at the
start of the line: `watched: <path> sha256:<hex>` (computed when the evidence is generated).
If it has `required_test:`, the evidence declares ONE proof region (`--- prova ---` …
`--- fim da prova ---`) with a structured result line for that test (`test [path::]<name> ... ok`
or `<name>: N passed; 0 failed`, N>0), no FAILED for it, and an `N passed; 0 failed` summary.
The authoritative rules are the docstring of scripts/audit_gate.py.

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

## Open items — a fonte é o ficheiro, não uma tabela

Havia aqui uma tabela mantida à mão. Foi **apagada em 2026-09-13**, e a razão é medida:
listava `TD-004` como `High/open` quando o ficheiro diz `status: closed`, e continha **2**
entradas quando o ficheiro tinha **7** `status: open` (TD-013, TD-014, TD-017, TD-018,
TD-019, TD-020, TD-021) — errava nos dois sentidos, e só o TD-013 estava certo.

Era uma **vista derivada** de dados que o `scripts/audit_gate.py` já imprime, entrada a
entrada e com o status de cada uma, em **cada commit**. Reconciliá-la seria reiniciar o
relógio até apodrecer outra vez. A regra da casa é *reutilizar, não duplicar* — e uma
segunda fonte de verdade que diverge em silêncio é precisamente o que este ficheiro existe
para registar, não para praticar.

**Para ver o que está aberto:** `python3 scripts/audit_gate.py`, ou
`grep -B4 '^status:    open' docs/technical-debt-ledger.md`.

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
status:    pending-verification
fix_ref:   "branch fix/td-014 (2026-09-27): GET /api/dropped -> {dropped, since}, ADR-0026 §13-bis. Decisoes do operador: canal novo GET (nao SSE, /api/upstream intocado), contador GLOBAL cumulativo, delta no cliente, produtor = console, `since` = arranque do console."
pending_gate: |
  1. O PR de fix/td-014 MERGEADO na main, com a CI lida no log.
  2. O OPERADOR ve o contador no console com uma perda INDUZIDA (um separador que abre
     o SSE e nao le, e uma enchente de eventos): a seccao DROPPED mostra `dropped` a
     crescer e `+N` em "Since last read". Visual = NOT_MEASURED ate la; nenhum teste
     automatico o substitui.
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
  FEITO em fix/td-014: `perda_induzida_faz_dropped_crescer_na_rota` (cresce) e
  `browser_que_le_tudo_reporta_zero` (zero). Mutacao «rota devolve constante 96» ->
  os dois reprovam; revertida -> verdes.
# review_by retirado em 2026-09-27: era texto ("proxima fatia de observabilidade do
# console"), que num pending-verification o audit_gate trata como Critical. Esta e essa
# fatia; nenhum prazo novo foi decidido, e nao se inventa uma data.
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

## TD-017 — Daemon e player divergem na politica do universo fora da faixa de 15 bits

```yaml
td_id:     TD-017
title:     "O daemon RECUSA um universo fora da faixa; o led-player AVISA e prossegue, e o pacote sai mascarado"
severity:  Medium
status:    open
origin:    "Revisao A1 (porta de saida multi-controlador), 2026-08-17"
context: |
  Provado por leitura de codigo, nao presumido:

  1. led-protocols/src/artnet.rs, `build_art_dmx` — o Net e escrito com
     `buf[15] = ((universe >> 8) & 0x7F)`. O `& 0x7F` deita fora o bit 15, portanto
     qualquer universo acima de 32767 e MASCARADO em silencio: 40000 (0x9C40) sai como
     0x1C40 = 7232. Nao ha erro, nao ha aviso no fio, e o pacote e valido.

  2. led-daemon-bin/src/output.rs:376 — a fronteira do ADR-0029 §7 RECUSA:
     "universo {u} fora da faixa do {proto} ({min}..={max})". O daemon esta protegido.

  3. led-player/src/main.rs:395 — o outro binario que fala Art-Net apenas AVISA
     ("likely a typo") e PROSSEGUE. O comentario acima declara a intencao:
     "Warn loudly; do not block (some rigs legitimately use high universes)".

  A divergencia e o problema, nao cada uma das metades. O argumento que o ADR-0029 §7
  usa para recusar — a bancada de 2026-07-23 mostrou que o universo errado DESLOCA A
  FITA sem erro nenhum — aplica-se ao player exactamente como ao daemon. E o player e
  o binario que fez a primeira luz e o burn-in, ou seja e o que esteve mais perto de
  hardware real.

  O ADR-0029 §7.1 ja nomeia a correccao NA ORIGEM (o mascaramento em `build_art_dmx`)
  como fatia propria. O que NAO estava nomeado em lado nenhum, e e o que esta entrada
  regista, e que os dois binarios aplicam politicas diferentes ao mesmo perigo fisico.
  Divergencias entre binarios apodrecem em silencio: ninguem as ve porque cada metade,
  lida sozinha, parece deliberada.

  Nao corrigido nesta revisao de proposito: escolher entre "o player passa a recusar"
  e "a origem passa a recusar e os dois herdam" e decisao de arquitectura, nao edicao.
review_by: "antes de qualquer uso do led-player contra o rig fisico (GS4.5)"
```

## TD-018 — A sintaxe obrigatoria do universo nao esta no `--help` do daemon

```yaml
td_id:     TD-018
title:     "O ADR-0029 §7 tornou `IP@UNIVERSO` obrigatorio em Art-Net/sACN, e o --help nao o menciona"
severity:  Low
status:    open
origin:    "Revisao A1 (porta de saida multi-controlador), 2026-08-17"
context: |
  Medido, nao presumido: `led-daemon --help` tem ZERO ocorrencias de `@`, e a unica
  linha com a palavra "universo" descreve o que vem do profile, nao a especificacao do
  endereco. Mas `OutputConfig` EXIGE o universo nos protocolos que o usam — sem ele o
  palco nao abre.

  Consequencia: o operador que leia a ajuda antes de correr nao descobre a sintaxe. So
  a aprende falhando. Isto e a superficie de descoberta desactualizada face ao codigo,
  e o `--help` e literalmente o unico sitio onde um operador escreve flags (foi esse o
  argumento que abriu o TD-015).

  Muito mitigado, e por isso Low: a mensagem de recusa e explicita e ensina a sintaxe —
  "o preset `{modelo}` usa {proto} e exige o universo — escreva `{endereco}@N`. Nao ha
  omissao: a bancada de 2026-07-23 mostrou que o universo errado desloca a fita sem
  erro nenhum". Quem falha uma vez fica a saber, e a saber PORQUE.

  Nao corrigido aqui porque a revisao A1 e read-only sobre produccao; e uma linha de
  texto no `--help`, e cabe na fatia que fechar o TD-017.
review_by: "fatia do TD-017"
```

## TD-019 — `linear_assignments` tem 170 e 3 canais escritos à mão, e é o caminho Art-Net/sACN do daemon

```yaml
td_id:     TD-019
title:     "O `led-player` sem `--profile` ainda endereca com `170`/`x3` e RGB a mao — e o fecho do lado do daemon (C2b) nunca foi capturado como evidencia"
severity:  Low
status:    open
origin:    "Inspeccao para o ADR-0030 (portas fisicas), 2026-08-30. Registado como DL-2 nesse ADR."
adr:       "ADR-0030 §Dividas registadas / DL-2"
evidence_ref: docs/evidence/td-019-enderecamento-no-fio-2026-09-13.md
negative_control: |
  M2 (output.rs:615 `color: ColorFormat::Rgb(self.rgb_order())`) reprova o teste no pixel 0:
  "esperava os 4 canais [50, 150, 0, 50], veio [100, 200, 50, 100]". ANTES do commit 09c135e
  a mesma mutacao passava os 18 testes — a assercao era `d.len() >= 126`, cega ao formato.
  M1 (output.rs:611 `pixels_per_universe: 170`) reprova na fronteira de universo.
  ATENCAO: o criterio B do closure_criteria abaixo esta STALE — manda repor PX_PER_UNIVERSE
  no led-player, constante que o daemon deixou de usar no C2b; mutar essa constante NAO pode
  reprovar este teste. Os controlos validos sao o M1/M2 acima.
context: |
  RECONCILIACAO 2026-09-13 — LEIA ISTO ANTES DO CORPO HISTORICO ABAIXO.

  O corpo original (preservado a partir de "SINTOMA") descreve o mundo ANTES do C2b da
  FASE C (2026-09-01). Foi medido hoje, ponto a ponto, e a maior parte ja nao se aplica.
  O historico fica porque explica PORQUE foi classificado High; o que muda e o presente.

  MEDIDO HOJE:

  a) Ponto 3 do corpo — FALSO hoje. `grep linear_assignments crates/led-daemon-bin/src/output.rs`
     devolve UMA linha, e e um COMENTARIO (`output.rs:599`) que documenta o proprio fecho:
     "Ate ao C2b o daemon chamava `led_player::linear_assignments`...". O daemon deixou de
     chamar a funcao; o endereçamento e pedido ao `led-hardware-profile` (ADR-0030 §8).

  b) Ponto 4 — a funcao sobrevive, o defeito nao. `rgb_order()` ainda faz
     `Rgbw(o, _) => o` (`output.rs:629-634`), mas tem ZERO chamadores em producao: o unico
     e `output.rs:1249`, e o `mod tests` abre em `:1031`. O colapso RGBW->RGB nao pode
     acontecer no fio por esta rota.

  c) Ponto 5 — a assimetria "a tres linhas de distancia" foi resolvida pelo C2b: os tres
     protocolos pedem o endereçamento ao dono. O SEGUNDO EIXO do `severity_rationale`
     ("os dois binarios passam a ter semanticas diferentes") caiu com ela.

  d) Ponto 6 — a alcancabilidade via `generic-sk6812-rgbw-sacn` dependia do
     `linear_assignments` no arm do sACN. Removido esse, o mecanismo desapareceu.

  e) Ponto 7 — a FASE C afirma que o gate passou a cobrir, COM falsificacao medida
     (reintroduziu o defeito; reprovaram dois testes, incluindo o gate estrutural da GS4.4).
     Nao verificado hoje: e afirmacao do changelog, nao artefacto.

  O QUE CONTINUA ABERTO, e e outra coisa: o ramo `None` do `led-player`
  (`main.rs:303-308`) — sem `--profile`, ainda usa
  `linear_assignments(px, 0, first_universe, RgbOrder::Rgb)`, ou seja `170`/`x3` a mao e
  RGB forcado. O changelog da FASE C ja o tinha nomeado como "fora do que o TD-019
  descreve". A entrada passa a descrever ISTO.

  E UMA CORRECCAO A DUAS FONTES: o changelog da FASE C escreve "TD-019 no `led-player`
  SEM `--profile`" — a flag EXISTE (`main.rs:158`) e o ramo `Some` honra-a ponta a ponta
  (`:303` devolve `(layout, calibration, profile_color)`, `:310` usa `led_hardware_profile`,
  `:322` valida, `:333` chama `hwp::compile_layout`). O que nao honra e o ramo `None`.

  PORQUE CONTINUA `open` E NAO `closed`: o `audit_gate.py` exige, para `closed`, um
  `evidence_ref` que aponte para um ficheiro EXISTENTE com `N passed; 0 failed` e `N > 0`
  (regras 2 e 6 do docstring de `scripts/audit_gate.py`), mais linhas `watched:` que fixam o
  conteudo dos source_files (R4.1; o `git-hash:` deixou de servir de frescura).
  [2026-10-07] Esta frase sobre a falta de artefacto e anterior a 2026-09-13: hoje existem
  `docs/evidence/td-019-*.md` e o `evidence_ref` acima aponta para um deles. A correccao aterrou
  em codigo; a PROVA nunca foi capturada. Isto e o gate a funcionar, nao uma omissao:
  neste repositorio `closed` significa "provado fechado com artefacto", nao "acreditamos
  que esta corrigido".

  ESTADO DOS CAMPOS ABAIXO — todos escritos pre-C2b, e nenhum reescrito de propósito
  (o historico explica o raciocinio; este bloco diz o que dele caiu):

  - `mitigation_now`: descreve quando o defeito "acorda" NO DAEMON — caminho que ja nao
    existe. A mitigacao real hoje e outra: o daemon recusa arrancar sem `--profile`.
  - `not_fixed_because` (c): dizia que a correccao certa era consequencia do ADR-0030 §6,
    "quando a reparticao tiver um so dono". SATISFEITO — o C2a poe o nucleo em
    `led-hardware-profile::reparticao` e o daemon passa a chamador.
  - `required_fix` 1: SATISFEITO (§6 implementado). `required_fix` 2: satisfeito PARA O
    DAEMON (deixou de chamar a funcao); a funcao continua no `led-player` com a mesma
    assinatura. `required_fix` 3: NAO FEITO — o gate textual da GS4.4 continua a ler tres
    ficheiros (`output.rs`, `stage.rs`, `run.rs`) e `led-player/src/lib.rs` fica de fora.
    E este o unico item do `required_fix` que sobrevive inteiro.
  - `closure_criteria` A–D: continuam validos como critérios, mas foram escritos contra o
    defeito do daemon. O (C) — comparar o endereçamento do player com o do daemon para o
    mesmo profile — foi coberto pela mutacao cruzada do C2a, sem artefacto capturado.

  ---- CORPO HISTORICO (pre-C2b, 2026-08-30) — preservado, ja nao descreve o presente ----

  SINTOMA. O `led-player` tem valores fisicos escritos a mao no caminho de saida, e o
  daemon usa esse caminho para Art-Net e sACN. Dois campos que o `HardwareProfile`
  declara — `pixels_per_universe` e `color` — nao chegam ao fio por esta rota.

  Provado por leitura de codigo, nao presumido, e nao executado:

  1. crates/led-player/src/lib.rs:297-312 — `linear_assignments`:
       `const PX_PER_UNIVERSE: usize = 170;  // 510 / 3`
       `universe: first_universe + (i / PX_PER_UNIVERSE) as u16`
       `channel:  ((i % PX_PER_UNIVERSE) * 3) as u16`
       `format:   order.into()`
     O 170 e o *3 estao escritos a mao. E a assinatura recebe `order: RgbOrder` — nao
     `ColorFormat`. Nao e um argumento que o chamador se esqueceu de passar: e um
     parametro que a API NAO EXPOE. Mesma forma do TD-016.

  2. crates/led-core/src/types.rs:141-146 — `impl From<RgbOrder> for ColorFormat` devolve
     sempre `ColorFormat::Rgb(o)`. Logo `format: order.into()` e SEMPRE tres canais.

  3. crates/led-daemon-bin/src/output.rs:28 — `use led_player::linear_assignments;`
     Chamado em :713 (Art-Net) e :724 (sACN), com `cfg.rgb_order()` como quarto argumento.

  4. crates/led-daemon-bin/src/output.rs:569-574 — `rgb_order()` faz
     `ColorFormat::Rgbw(o, _) => o`: descarta o RGBW E o `WhiteMode`.

  5. CONTRASTE DENTRO DO MESMO `match`. O arm DDP (:700-707) passa `cfg.color` inteiro
     a `DdpOutput::with_limits`, mais `cfg.pixels_per_universe`. O DDP honra; o
     Art-Net/sACN nao. A assimetria esta a tres linhas de distancia.

  6. ALCANCAVEL, nao hipotetico. O preset `generic-sk6812-rgbw-sacn`
     (presets.rs:178-197) declara `protocol: Sacn`, `color: Rgbw(Grb, MinSubtract)` e
     `pixels_per_universe: 128`. O validador do ADR-0018 so recusa RGBW quando o
     protocolo e DDP (validate.rs:181, `caps.protocol == Protocol::Ddp`) — nao ha guarda
     nenhuma para RGBW sobre sACN. O preset valida limpo e chega ao arm do sACN.

  7. O GATE NAO COBRE. `nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida`
     (led-daemon-bin/tests/wled_driver.rs:395) le exactamente tres ficheiros —
     `output.rs`, `stage.rs`, `run.rs` (linhas 397-399). O `led-player/src/lib.rs` esta
     fora, e e onde os numeros vivem. O gate criado na GS4.4 para impedir esta classe
     nao a ve.
impact: |
  Com `generic-sk6812-rgbw-sacn` o daemon produziria, no fio: tres canais por pixel em
  vez de quatro, e 170 pixels por universo em vez dos 128 declarados. O endereco de
  cada pixel a partir do primeiro fica errado, e o die branco da fita nunca acende.

  E a mesma familia do `RgbOrder` (GS4.3), do MTU (GS4.4) e do TD-016: campo declarado
  que o fio ignora. E, como esses, NAO da palco escuro — da uma fita que acende com as
  cores e as posicoes trocadas, que e mais caro de diagnosticar porque parece funcionar.

  Segundo eixo, e e o que o torna High e nao Medium: os dois binarios que falam com
  hardware passam a ter semanticas diferentes para o mesmo profile. O DDP honra o
  `ColorFormat` e o `pixels_per_universe`; o Art-Net/sACN nao. Divergencias entre
  caminhos apodrecem em silencio — cada metade, lida sozinha, parece deliberada.
severity_rationale: |
  RECLASSIFICADO 2026-09-13: High -> Low. Os DOIS eixos do argumento original cairam,
  e nao por opiniao — por medicao (ver RECONCILIACAO em `context`).

  - Eixo 1 ("valor fisico a mao num caminho de saida do daemon"): o daemon deixou de
    chamar `linear_assignments` (`output.rs:599` e so um comentario), e `rgb_order()`
    tem zero chamadores em producao. O caminho descrito nao existe.
  - Eixo 2 ("os dois binarios com semanticas diferentes para o mesmo profile"): resolvido
    pelo C2b — os tres protocolos pedem o endereçamento ao dono.

  Comparado pelo mesmo metodo do ledger, contra o que RESTA (o ramo `None` do
  `led-player`):

  = TD-018 (Low), e na verdade MELHOR mitigado. O TD-018 e Low porque a mensagem de
    recusa ensina o operador. Aqui a mitigacao e mais forte: o `led-daemon` — o binario
    que faz show — RECUSA ARRANCAR sem `--profile` desde a GS4.4 (exit 2). Chegar ao
    defeito exige usar o binario legado E omitir a flag.
  < TD-017 (Medium): la a divergencia daemon/player esta VIVA. Aqui o lado do daemon
    esta corrigido em codigo; sobra o fallback opcional do binario legado.

  NAO e zero, e e por isso que nao proponho `closed` nem apagar: um `RgbOrder` errado e
  SILENCIOSO — vermelho acende verde, sem erro nenhum. E a classe que a FASE C encontrou,
  agora reduzida a um caminho que o operador tem de escolher explicitamente.

  ---- ARGUMENTO ORIGINAL (pre-C2b) — preservado, sustentava o High que ja nao se aplica ----

  Classificado por comparacao com o ledger, nao por intuicao.

  = TD-016 (High): valor fisico escrito a mao num caminho de saida, num parametro que a
    API nao expoe, latente hoje e produtor de saida errada-mas-plausivel quando activa.
    A forma e a mesma; aqui sao DOIS campos declarados em vez de um.
  > TD-017 (Medium): la a divergencia daemon/player e uma escolha DELIBERADA e
    documentada no codigo ("Warn loudly; do not block"). Aqui nada e deliberado — e uma
    constante escrita a mao que ignora um campo declarado.
  > TD-018 (Low): nao ha mitigacao. O TD-018 e Low porque a mensagem de recusa ensina o
    operador. Aqui nao ha recusa, nao ha aviso, e nada no ecra o denuncia.
mitigation_now: |
  Nenhuma necessaria hoje, e o defeito e LATENTE, nao activo:

  - os quatro presets validados ou usados em hardware — `esp32-devkit-wled-artnet`,
    `esp32-poe-wled-ddp`, `falcon-f16v3-sacn`, `advatek-pixlite16-sacn` — declaram todos
    `pixels_per_universe: 170` e cor RGB, que e exactamente o que o codigo assume;
  - o unico preset RGBW+sACN do catalogo (`generic-sk6812-rgbw-sacn`) nunca foi corrido
    contra hardware;
  - o caminho DDP, que e o validado em hardware (94/94 frames, 2026-07-20), NAO passa
    por aqui.

  Ou seja: o defeito so acorda quando alguem correr o daemon com um preset cujo
  `pixels_per_universe` seja diferente de 170, ou com RGBW em Art-Net/sACN.
not_fixed_because: |
  Tres razoes, e nenhuma e falta de tempo:

  a) E ANTERIOR a FASE C. Nao foi introduzido pelo ADR-0030; foi encontrado por ele.
     Corrigi-lo dentro da etapa documental misturaria duas preocupacoes no mesmo diff.
  b) E CODIGO DE PRODUCAO, e a etapa que o encontrou era read-only por directiva.
  c) A CORRECCAO CERTA E CONSEQUENCIA DO ADR-0030 §6, nao independente dele. Quando a
     reparticao tiver um so dono (`led-hardware-profile`), o 170 escrito a mao deixa de
     ter onde viver. Corrigi-lo agora, isolado, criaria uma segunda implementacao da
     regra que o §6 existe para unificar — exactamente o que este repositorio recusa.
required_fix: |
  Pre-condicoes, por ordem:

  1. ADR-0030 §6 implementado: `led-hardware-profile` como fonte normativa unica do
     enderecamento, e o daemon a consumi-lo em vez de construir o layout inline.
  2. `linear_assignments` deixa de receber `RgbOrder` e passa a receber o que o profile
     declara — ou desaparece, absorvida pelo `compile_layout`. A segunda hipotese e a
     preferivel, e e a que o §6 aponta.
  3. O gate `nenhum_valor_fisico_esta_escrito_a_mao_no_caminho_da_saida` passa a incluir
     `led-player/src/lib.rs` nas suas FONTES. Sem isto a correccao nao fica protegida
     contra reincidencia, e foi a ausencia deste ficheiro que deixou o defeito crescer.
closure_criteria: |
  Fecha quando TODAS as quatro se verificarem:

  A) Um teste discriminante le os datagramas de um socket real com o preset
     `generic-sk6812-rgbw-sacn` e afirma QUATRO canais por pixel e 128 pixels por
     universo. Contar so canais nao chega — tem de afirmar tambem a fronteira do
     universo, senao passa com o 170 ainda la.
  B) CONTROLO NEGATIVO OBRIGATORIO: uma mutacao no ponto que o daemon REALMENTE usa tem de
     REPROVAR esse teste. Dois, ambos medidos em 2026-09-13 (ver `evidence_ref`):
       M1 · output.rs:611 `pixels_per_universe: 170`        -> reprova na fronteira
       M2 · output.rs:615 `color: Rgb(self.rgb_order())`    -> reprova no pixel 0
     Um teste que so afirmasse "sai RGBW" passaria com a fronteira de universo errada; um
     que so afirmasse a fronteira passa com o formato colapsado — foi esse o falso-verde
     fechado no commit 09c135e. Sao precisos os dois.

     REENDERECADO 2026-09-13, e a exigencia NAO foi baixada. A redaccao anterior mandava
     repor o `PX_PER_UNIVERSE = 170` (ou o `* 3`) do `led-player`. Depois do C2b o daemon
     deixou de chamar `linear_assignments`, logo essa mutacao NAO pode reprovar o teste do
     daemon: um controlo negativo que nao reprova nao e um controlo, e seguir a instrucao a
     letra produzia um verde vazio (KB-012) — a forma exacta do defeito que este TD regista.
     O `led-player` continua coberto, pelo criterio (D), que e onde essa constante vive.
  C) Um teste que compare o enderecamento produzido pelo `led-player` e pelo
     `led-daemon-bin` para o MESMO profile e o MESMO show, e que reprove se divergirem.
     E o gate do ADR-0030 §8, e e o que impede a divergencia de voltar.
  D) O gate textual da GS4.4 cobre `led-player/src/lib.rs` e reprova com `170` ou `* 3`
     fora de comentario nesse ficheiro.

  Mais `evidence_ref` e `negative_control` no schema de fecho (KB-012), como qualquer
  entrada `closed` deste ledger.
review_by: |
  ACTUALIZADO 2026-09-13 — a proibicao anterior mudou de NATUREZA, nao caiu.

  Dizia: "NAO correr o daemon contra hardware com um preset de pixels_per_universe != 170
  ou com RGBW em Art-Net/sACN". A razao era que o daemon produziria bytes errados. Essa
  razao acabou: o C2b poe o endereçamento no `led-hardware-profile` e o daemon honra
  `pixels_per_universe` e `ColorFormat` nos tres protocolos.

  O que RESTA nao e um defeito de software, e a AUSENCIA DE VALIDACAO FISICA — e a FASE C
  diz isso por escrito: "NAO esta provado que um controlador real os aceita... Validacao
  fisica PENDENTE", com o rig offline. Portanto:

  - `pixels_per_universe != 170` e RGBW sobre Art-Net/sACN passam a ser SUPORTADOS EM
    SOFTWARE e NUNCA OBSERVADOS EM HARDWARE. Correr isso contra o rig e uma PRIMEIRA VEZ,
    com o risco de uma primeira vez — nao a repeticao de um defeito conhecido.
  - Continua PROIBIDO usar o `led-player` sem `--profile` contra uma fita GRB ou RGBW: e o
    ramo `None` (`main.rs:303-308`), que forca `RgbOrder::Rgb`, e a falha e SILENCIOSA.
  - O gate da GS4.4 continua a nao cobrir `led-player/src/lib.rs` (`required_fix` 3), logo
    nada impede a reincidencia nesse ficheiro.
```

## TD-020 — A guarda de monotonia do `SharedClock` nao e atomica, e o relogio do show pode recuar

```yaml
td_id:     TD-020
title:     "`now_ms` faz load-calcula-store em vez de `fetch_max`: uma perda de actualizacao deixa um leitor observar o relogio a andar para tras"
severity:  High
status:    closed
closed_by: "13d2f41 (2026-09-17) — `load`/`max`/`store` substituido por `fetch_max(AcqRel)`; detector endurecido no mesmo ficheiro."
evidence_ref: docs/evidence/td-020-reverificacao-2026-09-26.md
required_test: concurrent_readers_never_see_rewind_during_correction
source_files: crates/led-hal/src/shared_clock.rs
negative_control: |
  DOIS controlos, e o segundo e o que impede o detector de ser teatro.

  A) O DEFEITO REPOSTO. Com a versao de tres operacoes (`load`/`max`/`store`) e o detector
     endurecido: 10 execucoes, 10 VERMELHAS. Mensagem verbatim —
     "ronda 2: um leitor viu o relogio RECUAR 1 ms sob correccao concorrente
     (8 leitoras x 20000 leituras)". Com `fetch_max`: 10 execucoes, 10 VERDES, mesmos
     parametros. Isto satisfaz o criterio A do `review_by` (o teste tem de reprovar de
     forma FIAVEL com o defeito presente — 1-em-30 nao serve).

  B) A AMPLITUDE DESLIGADA, COM O DEFEITO PRESENTE. Com a versao de tres operacoes E
     `AMPLITUDE_MS = 0` (a thread de correccao continua a girar, mas o offset nunca muda):
     10 execucoes, **0 vermelhas**. Sem este controlo, "o detector dispara" e "o detector
     tem 3,2 milhoes de iteracoes" seriam indistinguiveis, e ninguem saberia se a thread de
     correccao faz trabalho ou e decoracao. Ela faz: e a alternancia do offset que impede
     `adjusted` de voltar a dominar `prev` e mascarar a escrita obsoleta.

  UMA PREVISAO MINHA FALSIFICADA, registada em vez de apagada: eu previa que o recuo teria
  a magnitude de `AMPLITUDE_MS`. O recuo medido e de **1 ms** — a granularidade de `wall`.
  A amplitude nao cria magnitude; cria a condicao em que a perda aflora. O comentario do
  codigo foi reescrito para o mecanismo medido ANTES de o fix ser aplicado.
origin:    "Observado na fatia 1-A do ADR-0031 (2026-09-10). NAO e regressao dessa fatia — ver PROVA DE ALHEAMENTO."
context: |
  SINTOMA. `cargo test --workspace` reprovou numa de duas medicoes, em
  `shared_clock::tests::concurrent_readers_never_see_rewind_during_correction`
  (`crates/led-hal/src/shared_clock.rs:198`), com a mensagem
  "a reader observed a backward jump under concurrency". A segunda medicao deu
  1131 passed / 0 failed. Em isolamento: 10 execucoes, 10 verdes.

  CAUSA, lida no codigo e nao inferida do sintoma. `crates/led-hal/src/shared_clock.rs`,
  `pub fn now_ms` (linhas 78-81):

      let prev = self.last_now.load(Ordering::Acquire);
      let next = adjusted.max(prev);
      self.last_now.store(next, Ordering::Release);
      next

  Sao tres operacoes atomicas separadas, nao um read-modify-write. E a perda de
  actualizacao classica: a thread A le `prev`, a thread B le o MESMO `prev`, B calcula
  um `next` maior e guarda-o, e A guarda por cima o seu `next` menor. Quem ja devolveu
  o valor maior le na iteracao seguinte um `last_now` rebaixado e devolve um valor
  MENOR que o anterior — o recuo que a assercao apanha.

  `AtomicU64::fetch_max` (estavel desde o Rust 1.45) faz a mesma coisa num so RMW e
  fecha a janela. NAO foi aplicado: e `led-hal`, e fora do escopo da fatia que o
  encontrou.

  PRECONDICAO DO DEFEITO, medida e nao suposta. O recuo exige que `adjusted` DESCA,
  ou seja um `set_offset_ms` para tras concorrente com leituras. Sem isso `adjusted`
  e monotono por thread e `max(adjusted, prev)` nunca regride, mesmo com o `store` a
  ser pisado. Por isso o defeito e probabilistico e so aparece sob contencao — foi
  visto sob `cargo test --workspace` (paralelo) e nunca em isolamento.

  PROVA DE ALHEAMENTO a fatia 1-A do ADR-0031 (tres factos independentes):

  1. O diff da fatia toca 0 ficheiros em `crates/led-hal/`.
  2. `crates/led-hal/Cargo.toml` NAO declara `led-daemon-bin` nem `led-console-bin`
     (grep exit 1). Nao existe caminho de dependencia por onde a alteracao pudesse
     alcancar este crate.
  3. O mesmo teste passou na baseline `e6096ed` (suite serial, 1128 passed).

  POR QUE `High`, e o que baixaria a classificacao. A propriedade que o codigo declara
  — o comentario diz "Monotonicity: never go backward", e o teste chama-lhe "the show
  clock never rewinds" — NAO e garantida pela implementacao. `SharedClock` e o relogio
  usado pelo `net_time` (sincronizacao multi-no) e pelo `Pacing::Absolute` do
  `led-player`, e `net_time::sync_to` e exactamente o caminho que aplica correccoes
  para tras. A correccao e de uma linha.

  ALCANCABILIDADE EM PRODUCAO: **NAO MEDIDA**. O rig multi-no nunca foi energizado, e
  num show de um so no `set_offset_ms` nao e chamado durante a reproducao. Se a
  auditoria decidir que "declarado mas nao alcancavel hoje" pesa mais que "invariante
  declarado e falso", isto desce a Medium. Registado a High para que a decisao seja
  tomada por alguem e nao por omissao.

closure_criteria: |
  Fecha quando as tres se verificarem:

  A) `now_ms` usa um read-modify-write atomico unico (`fetch_max`) em vez de
     load/max/store.
  B) CONTROLO NEGATIVO OBRIGATORIO: repor o `load`+`store` tem de REPROVAR
     `concurrent_readers_never_see_rewind_during_correction`. Como o defeito e
     probabilistico, o controlo tem de ser repetido N vezes e reprovar em pelo menos
     uma — e o N usado tem de ficar escrito na evidencia. Um controlo que corra uma
     vez e passe NAO prova nada (KB-012).
  C) O teste corre em `cargo test --workspace` (paralelo, com contencao), nao so
     isolado — foi a contencao que o revelou, e um gate que so corre sozinho voltaria
     a nao o ver.

  Mais `evidence_ref` e `negative_control` no schema de fecho, como qualquer entrada
  `closed` deste ledger.
review_by: "antes de qualquer trabalho que dependa de sincronizacao multi-no (net_time / SyncedCluster), e antes do G4 com mais de um controlador energizado."
```

## TD-021 — O `led-console-bin` nao compila em Windows: importa um modulo `#[cfg(unix)]` do daemon

```yaml
td_id:     TD-021
title:     "`limits.rs` deriva `MAX_BODY` e `BACKOFF_MAX` de `led_daemon_bin::server`, que so existe em unix (IPC sobre UDS) — E0433 no Windows"
severity:  Medium
status:    open
origin:    "Observado no job `windows (allow-failure)` do PR #5, run 34683654404 (SHA 51d8241, 2026-09-12). NAO e regressao desse PR — ver PROVA DE ALHEAMENTO."
context: |
  SINTOMA, lido do log da CI e nao inferido:

      error[E0433]: cannot find `server` in `led_daemon_bin`
      crates/led-console-bin/src/limits.rs:8, :36, :47

  CAUSA, lida no codigo. `crates/led-daemon-bin/src/lib.rs:37-38`:

      #[cfg(unix)]
      pub mod server;

  e o mesmo gate no re-export da linha 48. As duas constantes que o console consome vivem
  DENTRO desse modulo — `MAX_LINE` em `server.rs:35` e `REPLY_TIMEOUT` em `server.rs:42` —
  e nenhuma tem `cfg` proprio: herdam o do modulo. O gate e correcto na origem: o `server`
  e o IPC sobre **UDS**, que nao existe em Windows. O que nao e cross-plataforma e o
  **consumidor**: o `led-console-bin` nao declara gate nenhum e e compilado em todas as
  plataformas da matriz.

  EXTENSAO MEDIDA (grep, nao estimada). Cinco referencias a `led_daemon_bin::server` em
  `limits.rs`, das quais o compilador nomeia **tres** — `:8`, `:36`, `:47`. As outras duas
  (`:74`, `:88`) estao dentro do `mod tests`, que comeca em `:67`. Mais **4 ficheiros de
  teste** do mesmo crate importam `server::{ControlPlane, Server}` sem gate: `sse.rs`,
  `http_server.rs`, `ipc_contra_o_daemon.rs`, `sse_reconnect.rs`. Total: **1 ficheiro de
  producao + 4 de teste**.

  IMPACTO. O crate do console nao compila em Windows, logo nada dele corre la: nem os
  testes, nem o binario. Isso toca duas coisas ja escritas no roadmap — o **G5**
  (determinismo Linux/Windows; o probe `scripts/determinism_probe.sh` existe e nunca correu
  em Windows) e o **D8 / H1** (empacotamento desktop com webview do SO).

  A RAIZ E O ADR-0014, NAO O `limits.rs` — e esta e a parte que nao pode ser arredondada.
  O `E0433` e um **sintoma de superficie**: o import cruza um `cfg` que nao devia cruzar.
  A causa a montante e a decisao de transporte do IPC — **UDS owner-only** (ADR-0014), que
  o Windows nao tem. Prova de que o sintoma nao e o problema: o gate esta em
  `led-daemon-bin`, e o proprio `led-daemon` **tambem nao serve** em Windows pela mesma
  razao. Fazer o `led-console-bin` compilar la produziria **um crate que compila e um
  console que nao fala com daemon nenhum**.

  CONSEQUENCIA PARA O G5 E O D8/H1: os dois **herdam uma decisao de arquitectura de
  transporte IPC**, nao um fix de import. Quem os abrir tem de decidir primeiro por onde
  o console fala com o daemon numa plataforma sem UDS — e isso e emenda ao ADR-0014, com
  o ADR-0026 §11-12 (limites **derivados**, nunca reescritos) como restricao a preservar.
  Planear o G5 ou o D8 a contar com um fix de uma linha aqui seria planear contra o facto.

  PROVA DE ALHEAMENTO ao PR #5 (tres factos independentes):

  1. `git blame`: a linha `:8` e de `4455a908` (2026-08-09) e a `:36` de `ffd82774`
     (2026-08-10). As duas nascem com a fundacao do `led-console-bin`, ha ~34 dias.
  2. `git log -1 -- crates/led-console-bin/src/limits.rs` = `ffd82774`. O ficheiro **nao e
     tocado** desde 2026-08-10; o PR #5 nao lhe mexeu numa linha.
  3. O changelog de 2026-08-10c ja registava o job Windows vermelho, e o de 2026-09-08
     nomeia esta causa por escrito. E defeito conhecido, nunca aberto como divida.

  EVIDENCIA PARA A CLASSIFICACAO — os dois lados, e o veredito no fim.

  Puxa para CIMA: o crate que nao compila e **o console**, que e exactamente o que o D8
  empacota; o G5 exige correr em Windows e nao consegue; e a divida esta silenciosa ha 34
  dias precisamente porque o job e `allow-failure` — um vermelho permanente ensina a nao
  olhar, e foi o que aconteceu.

  Puxa para BAIXO: o Windows e `continue-on-error` **por decisao de arquitectura**
  (`ci.yml:3-4`, ADR-0013: *«Windows e suporte e NAO orienta a arquitectura»*); nada hoje
  depende dele; e a raiz e o UDS do ADR-0014, portanto corrigir este E0433 **nao** daria um
  LUMYX funcional em Windows — daria um crate que compila.

  VEREDITO: **Medium** (decidido pelo responsavel em 2026-09-12, sobre esta evidencia).
  Nao `High` porque a **alcancabilidade nao se cumpre** — e o mesmo argumento que derrubou
  o `High` do TD-019, e aplica-lo aqui mantem a escala coerente. Nao `Low` porque o `Low`
  deste ledger e o TD-018, uma lacuna de `--help` mitigada pela mensagem de recusa; isto e
  uma plataforma inteira que nao compila, no caminho escrito do G5/D8. `Medium` e a forma
  do TD-017 e do TD-014: defeito real e nomeado, sem consumidor bloqueado hoje, que exige
  decisao **antes** do marco que depende dele.

  NENHUM FIX PROPOSTO, por instrucao. Registo so a restricao que qualquer correccao futura
  tera de respeitar, e que e o que a torna decisao e nao edicao: o ADR-0026 §11-12 exige que
  estes limites sejam **derivados** do daemon, *«nunca reescritos»* — o proprio gate
  `os_limites_sao_os_do_gs3_e_nao_copias` existe para impedir uma segunda copia. Qualquer
  saida tem de preservar isso.
review_by: "antes de abrir trabalho no G5 (determinismo Linux/Windows) ou no D8 (empacotamento desktop) — o que vier primeiro. Nao bloqueia nada antes disso."
```

---

## TD-022 — `o_daemon_recusa_a_linha_longa_por_si_proprio` faz `unwrap()` numa escrita que o daemon esta correcto em interromper

```yaml
td_id:     TD-022
title:     "O teste mede uma corrida entre a sua propria escrita de 64 KiB e o fecho do daemon; o `unwrap()` do `writeln!` transforma o comportamento DESEJADO da F1-B num vermelho intermitente do gate"
severity:  Medium
status:    closed
closed_on: 2026-09-26
closed_by: "3c60ab8 (correcao do lado do teste) + sonda Linux run 36245228354: o writeln! interrompido devolve BrokenPipe (errno 32) em Linux, 3/3 — o mesmo que em macOS. Conjunto aceite NAO alargado."
evidence_ref: docs/evidence/td-022-reverificacao-2026-10-07.md
required_test: o_daemon_recusa_a_linha_longa_por_si_proprio
source_files: crates/led-console-bin/tests/ipc_contra_o_daemon.rs
negative_control: |
  As tres falsificacoes do `falsification_required`, RE-EXECUTADAS em 2026-09-26 sobre
  57cf21d (3 execucoes cada, exit lido sem pipe, vermelhos de TESTE — 0 `error[E`):
  F1) interrupcao FORCADA + `unwrap()` cru reposto -> 101 x3, panico com BrokenPipe.
      Controlo F1c: a mesma interrupcao forcada com o codigo actual -> 0 x3 (verde).
  F2) o `writeln!` trocado por um erro ConnectionReset -> 101 x3: um erro nao-EPIPE
      continua a reprovar ("tolerar EPIPE" != "ignorar erros de escrita").
  F3) daemon mutado para NAO recusar a linha longa -> 101 x3, reprovado em `:206`.
      Achado: a resposta do daemon mutado AINDA traz `bad_request`; quem discrimina e a
      assercao "demasiado longa". A sonda tem o seu proprio controlo (fase 3 anulada ->
      INTERRUPCAO_NAO_OBSERVADA, vermelho).
origin:    "Primeira observacao 2026-08-13c (CLAUDE.md:629), segunda 2026-09-01 (CLAUDE.md:346), terceira 2026-09-22 com o panic capturado inteiro. Diagnosticado desde a primeira, NUNCA promovido a TD — por isso o audit_gate nunca o viu e foi redescoberto do zero tres vezes."
context: |
  MEDIDO HOJE, nao inferido. `scripts/baseline_watch.sh` (instrumento novo, escreve o
  output para ficheiro e le o `$?` sem pipe — KB-013) apanhou-o a primeira passagem:

      EXIT_CARGO=101
      thread 'o_daemon_recusa_a_linha_longa_por_si_proprio' (89574) panicked at
      crates/led-console-bin/tests/ipc_contra_o_daemon.rs:178:82:
      called `Result::unwrap()` on an `Err` value:
        Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
      test result: FAILED. 7 passed; 1 failed

  Prova preservada em /tmp/baseline_RED_20260922_140420.log (21321 bytes).

  LOCALIZACAO REAL: `ipc_contra_o_daemon.rs:178`. O changelog de 2026-08-13c diz `:177`
  — a citacao DERIVOU uma linha em cinco semanas. E o caso exacto que a lumyx-next-steps
  §2 avisa: `file:line` envelhece, reverificar antes de afirmar.

  CAUSA, lida no codigo dos dois lados:

  1. `led-daemon-bin/src/server.rs:264-274` — assim que `n > MAX_LINE` e a linha nao
     termina em `\n`, o daemon escreve a recusa (`:266`), faz `flush` (`:267`) e
     `break` (`:274`), FECHANDO sem drenar. O comentario in-loco explica porque:
     drenar e ler uma quantidade que o atacante escolhe, e prosseguir sem drenar
     deixaria o resto da linha gigante ser analisado como pedido novo. **Fechar e a
     decisao correcta da F1-B, e esta documentada como tal.**

  2. `ipc_contra_o_daemon.rs:178` — o teste ainda esta a escrever `MAX_BODY + 10` bytes
     quando esse fecho acontece, e o `writeln!(...).unwrap()` apanha EPIPE.

  O teste e o daemon estao numa corrida: quem chega primeiro ao fim da escrita. Sob
  carga (suite completa do workspace, 4 cores) o daemon ganha e o teste entra em panico.
  Isolado, o teste ganha — medido: 0 falhas em 54 execucoes apos um vermelho.

  O QUE O TESTE QUER PROVAR, lido do doc-comment `:164-166`: *«O daemon tambem recusa —
  a guarda do console nao e a unica defesa»*. Isso esta nas assercoes `:181-182`
  (`bad_request` + `demasiado longa`), e **essas nunca chegam a correr** quando o panic
  dispara. O `unwrap()` da linha 178 nao afirma nada sobre o daemon: afirma que a
  escrita do proprio teste coube antes do fecho, que e ruido de escalonador.
impact: |
  E um FALSO-VERMELHO, nunca um falso-verde: quando dispara, dispara alto, e nenhum
  defeito real fica escondido por ele. E isso que limita a severidade a Medium.

  O custo e outro e e de processo: `cargo test --workspace` e o gate de entrada de toda
  a missao neste repositorio, e um gate que falha 1-em-N e passa no rerun ensina a
  re-executar em vez de ler. No dia em que uma regressao a serio aparecer, o primeiro
  reflexo treinado sera correr outra vez. Ja aconteceu uma vez em 2026-09-01: duas
  falhas da suite foram reportadas como possivel regressao e a causa era carga.

  Custo medido nesta sessao: um turno inteiro gasto a re-diagnosticar do zero um defeito
  que ja estava escrito no CLAUDE.md, porque prosa de changelog nao tem `td_id` e o
  `scripts/audit_gate.py` so ve o ledger.
mitigation_now: |
  Nenhuma automatica. Na pratica, quem apanha o vermelho re-executa e passa — que e
  exactamente o habito que esta entrada existe para nomear como custo.
required_fix: |
  DO LADO DO TESTE, nunca do daemon. O daemon esta correcto e o ADR da F1-B fixa esse
  fecho como decisao; mexer em `server.rs` para acomodar um teste seria inverter a
  hierarquia (a lumyx-next-steps §3 regra 4 proibe).

  Tolerar EPIPE — e SO EPIPE — no `writeln!` da linha 178, mantendo intactas as duas
  asserçoes que provam a recusa. E seguro porque o daemon escreve a recusa e faz `flush`
  ANTES de fechar (`server.rs:266-267`), logo a resposta ja esta no buffer de recepcao
  do teste quando o EPIPE acontece: `read_line` continua a devolve-la.

  PROIBIDO: apagar as asserçoes `:181-182`; marcar o teste `#[ignore]`; tolerar qualquer
  `io::Error` em vez de so `BrokenPipe` (mascararia um erro de transporte a serio);
  drenar ou adiar o fecho no daemon.

  RISCO CONHECIDO E DELIBERADAMENTE NAO MITIGADO — a assercao fixa o errno.
  `assert_eq!(e.kind(), BrokenPipe)` afirma um valor que o kernel escolhe. Este
  repositorio ja mediu divergencia macOS/Linux desta classe exacta: 2026-08-17 (C0)
  encontrou o mesmo alvo a falhar no `connect` em Linux e no `send` em macOS, e o mesmo
  endereco a dar `PermissionDenied` numa plataforma e `BrokenPipe` na outra — e concluiu
  por escrito que *«nenhum teste pode afirmar o errno»*. A falsificacao determinista
  desta correcao correu SO em macOS.

  Alargar o conjunto aceite (p.ex. incluir `ConnectionReset`) foi CONSIDERADO E
  REJEITADO: seria escolher um errno que nao consigo observar — nao ha Linux nesta
  maquina (medido em 2026-08-13d: sem docker/colima/podman/lima/vagrant/multipass) — e
  acrescentaria um ramo que nenhum teste alcanca. Preferir o estrito: se o Linux
  divergir, a mensagem `:193` imprime `{e:?}` e NOMEIA o errno real. Falha ruidosa e
  diagnostica vale mais que tolerancia especulativa. E por isso que o status e
  `pending-verification` e nao `closed`.
pending_gate: |
  O job `test (ubuntu-latest)` verde num PR que contenha esta correcao, LIDO NO LOG e
  nao no simbolo de check — o precedente e a F7.2 (PR #4, 2026-09-08), onde o veredito
  vinculante foi a linha `test result` do log.

  O que o gate tem de decidir: se o `writeln!` interrompido em Linux devolve `BrokenPipe`
  (⇒ a correcao esta completa, promover a `closed` com este run como `evidence_ref`) ou
  outro errno (⇒ a mensagem de `:193` nomeia-o, e SO entao se decide o conjunto aceite,
  com o valor medido em vez de adivinhado).

  No log do ubuntu, distinguir panico em :190 (errno do writeln!) de reprovacao em :198
  (recusa ausente) — correccoes diferentes.
falsification_required: |
  Repor o `unwrap()` cru na escrita tem de reproduzir o vermelho com `BrokenPipe` sob
  carga de workspace. Controlo negativo obrigatorio: um erro de I/O que NAO seja EPIPE
  tem de continuar a reprovar — sem isso, "tolerar EPIPE" e "ignorar erros de escrita"
  ficam indistinguiveis, que e a forma do KB-012.

  E a asserçao de recusa tem de continuar a discriminar: um daemon que aceitasse a linha
  longa tem de reprovar em `:181`.
review_by: 2026-10-07
```

---

## TD-023 — Os gates de alocacao contam as alocacoes de todas as threads (led-hal, led-sequencer, audio-core, led-pixel-engine)

```yaml
td_id:     TD-023
title:     "Os gates `tests/no_alloc.rs` do led-hal, led-sequencer, audio-core e led-pixel-engine usam um contador GLOBAL: alocacoes do libtest noutras threads entram na janela e o gate reprova sem o caminho quente ter alocado"
severity:  Medium
status:    open
origin:    "PR #8, run 36245590122 tentativa 2 (job macOS 108673138958, SHA 57cf21d, so docs): «calibrated hot path allocated 7 time(s) over 10000 frames». Mesma classe no led-sequencer: PR #20, run 37236057484, job 111535264226 (diff = so o ledger): «timeline render allocated 2 time(s) over 10000 frames» (180 vs 182). led-hal de novo no PR #17 (232 vs 237)."
required_test: ruido_de_fundo_noutra_thread_nao_reprova
source_files: crates/led-hal/tests/no_alloc.rs, crates/led-sequencer/tests/no_alloc.rs, crates/audio-core/tests/no_alloc.rs, crates/led-pixel-engine/tests/no_alloc.rs
context: |
  Os quatro ficheiros tinham `static ALLOCS: AtomicUsize` incrementado pelo alocador em
  QUALQUER thread (led-hal :10, led-sequencer :12, audio-core :13, led-pixel-engine :17 em
  79e52e2). `ALLOC_GATE` (led-hal :37, led-pixel-engine :44) so serializa os corpos dos testes
  do ficheiro; nao exclui as threads do libtest. O led-sequencer reprovou com UM so #[test] no
  binario: a contaminacao nao vem de testes vizinhos, logo nenhum Mutex a resolve.
  Consolida o TD-DRAFT-no-alloc-contador-global (2026-10-04), que nao entrou no ledger.
  O led-protocols ja atribui por thread desde a F7.2 (950a497) e e a referencia.
  Tambem com contador global (janela `MEDINDO`), sem flake observado, FORA deste TD por
  ordem do operador: led-daemon-bin/tests/custo_do_fanout.rs:69 e no_alloc_canal.rs:66.
  led-daemon-bin/tests/ipc_line_limit.rs mede memoria viva de OUTRA thread: nao e desta classe.
impact: |
  Falso-vermelho intermitente no job bloqueante `test (macos-latest)`. O contador antigo nao gera
  falso-verde: um contador que soma todas as threads nunca conta menos do que a thread do teste
  alocou. O contador por thread tem um limite proprio: ver LIMITE em required_fix.
mitigation_now: |
  Nenhuma automatica; re-run (1 por falha, regra de repeticao de jobs do Gauntlet).
required_fix: |
  Atribuir por thread, como a F7.2 fez em crates/led-protocols/tests/no_alloc.rs
  (`E_A_THREAD_DO_TESTE`, `FORA_DA_THREAD`, janela `MEDINDO`), replicado nos quatro ficheiros
  (um #[global_allocator] nao se partilha entre binarios; nao ha crate de utilitarios de teste).
  Correcao proposta no ramo fix/td-023-contador-por-thread (R3.4, 2026-10-05).
  LIMITE (aceite, como na F7.2): o gate passa a provar «zero alocacoes NA THREAD QUE EXECUTA o
  caminho quente». Uma alocacao por frame delegada num worker persistente deixa de ser vista
  (falsificador R3.4, ataque b2: verde; com o contador global: «9921 time(s)»). Um spawn por
  frame continua a ser visto (o proprio spawn aloca na thread chamadora). Hoje nenhum caminho
  medido delega noutra thread: FORA_DA_THREAD = 0 em todas as janelas dos gates principais
  (medido em macOS; Linux = CI do PR).
  PROIBIDO: alargar a tolerancia, #[ignore], correr o teste isolado na CI.
falsification_required: |
  Por ficheiro: (1) `ruido_de_fundo_noutra_thread_nao_reprova_*` — thread de fundo a alocar
  em ciclo; a janela so fecha depois de >= 1000 alocacoes dela la dentro; exige contador
  global >= 1000 E por thread == 0. Mutar `registar` para devolver sempre true (= contador
  global) -> vermelho. (2) `o_contador_ainda_ve_o_que_e_alocado_na_thread_do_teste` (alloc,
  alloc_zeroed e realloc, uma assercao cada) — mutar para devolver sempre false -> vermelho.
  (3) Alocacao injetada no caminho quente -> vermelho.
  Ao fechar: o rename/remocao destes testes nao fica vermelho em nenhum gate (o e2e Inv3/C3 aceita
  0 testes; required_test exige linha estruturada que passou desde o R4.1 (#26), mas o audit_gate
  so o verifica com o TD `closed`) — fixar N
  por binario e os nomes completos na evidencia.
review_by: 2026-10-12
```

---

## TD-024 — `speed_factor_paces_playback` afirma um limite de relogio de parede

```yaml
td_id:     TD-024
title:     "`speed_factor_paces_playback` mede tempo de parede (<99 ms) que um runner partilhado pode exceder: falso-vermelho da classe do TD-006"
severity:  Medium
status:    open
origin:    "PR #8, run 36245590122 tentativa 1 (job macOS 108414070581, SHA 57cf21d, so docs): «10x must be faster than real time, got 99ms»."
required_test: speed_factor_paces_playback
source_files: crates/led-player/src/lib.rs
context: |
  crates/led-player/src/lib.rs:361-368 — `play(..., Speed::Factor(10.0))` (:365) medido com
  relogio real; assercoes `elapsed >= 8` (:367) e `elapsed < 99` (:368).
impact: |
  Falso-vermelho intermitente num job bloqueante; o que o teste quer provar (10x acelera)
  fica misturado com a latencia do SO.
mitigation_now: |
  Nenhuma; re-run.
required_fix: |
  Por decidir (nao implementar antes): tempo injetado (relogio logico, como o Pacer do
  daemon) ou afirmar as esperas calculadas em vez do tempo decorrido.
  PROIBIDO: alargar a tolerancia (subir o 99 ou baixar o 8), #[ignore], retry automatico.
falsification_required: |
  Speed::Factor(10.0) a comportar-se como 1x (mutacao) -> vermelho; com a correcao -> verde
  sob carga e sem depender do relogio real.
review_by: 2026-10-12
```

---

## TD-025 — O hook de pre-commit julgava o worktree, nao o indice

```yaml
td_id:     TD-025
title:     "O hook de pre-commit corria o debt gate sobre o WORKTREE: validava um estado que nao ia ser commitado, e nao via o commit que torna uma evidencia stale"
severity:  High
status:    closed
closed_on: 2026-10-01
closed_by: "eb791fe (correcao, PR #11) + 7ee2f89 (o teste passa a emitir «test_pre_commit_hook: N passed; M failed» dos contadores reais). Re-medido sobre 7ee2f89: 4 passed; 0 failed, exit 0."
evidence_ref: docs/evidence/td-025-hook-julga-o-indice-2026-10-07.md
required_test: test_pre_commit_hook
origin:    "Descrito na mensagem de eb791fe: D1 — o gate lia o ledger do worktree (indice 19 TD / worktree 20 -> «20 OK»); D2 — o stale usava git log <hash>..HEAD, cego as alteracoes em stage (o commit C4, 320ff94, passou o hook)."
source_files: scripts/pre-commit-hook.sh
context: |
  Ate 2026-10-01 estava pending-verification: a evidencia de 2026-09-29 so tinha a linha
  «cenarios: passou=N falhou=M», que o regex de scripts/audit_gate.py:84 nao aceita.
  Correcao: PR #11 (eb791fe), mergeado na main em 0f7857e (2026-09-29): o indice vira um commit
  candidato num worktree temporario e o gate corre la.
  Medido 2026-09-29 com tests/test_pre_commit_hook.sh em origin/main 445d296 (os dois ficheiros
  nao mudaram desde eb791fe):
    hook de origin/main: S1-S4 PASS, «cenarios: passou=4 falhou=0», exit 0
      (~/lumyx-evidence/2026-09-29/td025-hook-test-origin-main.txt)
  .git/hooks/pre-commit reinstalado a partir de scripts/pre-commit-hook.sh da main; igual byte a
  byte (sha256 f89f1fa9..., ~/lumyx-evidence/2026-09-29/pre-commit.origin-main).
  Hoje o gate so aceita evidencia no formato do `cargo test` («N passed; 0 failed» ou
  «test result: ok. N passed»).
negative_control: |
  O mesmo teste contra o hook antigo (57cf21d), re-medido sobre 7ee2f89: S1 FAIL, S2 FAIL,
  S3 FAIL, S4 PASS, «test_pre_commit_hook: 1 passed; 3 failed», exit 1. Com essa saida como
  unica evidencia (TD-025 closed num clone temporario) o audit_gate sai com exit 1:
  «CRITICAL TD-025: evidence_ref contains no 'N passed; 0 failed' line at all».
  Ambos em docs/evidence/td-025-hook-julga-o-indice-2026-10-01.md, secs. 2 e 3.
```

---

## TD-026 — O `audit_gate` ignora o returncode do `git log`: em clone raso o stale fica verde

```yaml
td_id:     TD-026
title:     "`files_changed_since` nao le o returncode do `git log`: num clone raso o hash da evidencia nao existe, o `git log` falha e o detector de stale devolve «nao mudou»"
severity:  High
status:    closed
closed_on: 2026-10-06
closed_by: "R4.1 (ramo ci/audit-gate-integridade): o stale passa a comparar o sha256 do CONTEUDO fixado na evidencia (linhas watched:) com o conteudo do workspace; o git deixa de ser usado. tests/test_audit_gate.py 34 passed; 0 failed. Apos 4 rondas do falsificador o required_test so conta dentro da REGIAO DE PROVA declarada na evidencia (--- prova --- / --- fim da prova ---): lista positiva, sem lista negra de formas NEG; pins duplicados/absolutos/com ../por symlink sao Critical."
evidence_ref: docs/evidence/td-026-028-audit-gate-conteudo-2026-10-06.md
required_test: test_r41_td026_clone_raso_igual_a_completo
origin:    "Encontrado em 2026-09-26 ao desenhar o job debt gate (PR #9)."
source_files: scripts/audit_gate.py, tests/test_audit_gate.py
context: |
  scripts/audit_gate.py:101-113 — `subprocess.run(['git','log','--oneline',
  f'{git_hash}..HEAD','--',p], capture_output=True)` (:107-108): so `stdout` e lido, o
  `returncode` nunca; `except Exception: pass` (:112) engole o resto.
  Medido 2026-09-29 num clone `--depth 1` de 84c146e: `git log --oneline e46151b..HEAD --
  crates/led-console-bin/tests/ipc_contra_o_daemon.rs` -> exit 128 («fatal: bad revision»);
  o gate sai 0 e da TD-022 `[OK]` sem ter podido verificar o stale.
  O job de CI contorna-o com `fetch-depth: 0` (PR #9); fora dessa configuracao o gate e fragil.
  CI, medido 2026-09-29: .github/workflows/ci.yml:134-136 — o job `debt-gate` faz
  `actions/checkout@v4` com `fetch-depth: 0` explicito. No log desse job na main 84c146e
  (run 36617074872): a entrada do checkout mostra `fetch-depth: 0`, o `git fetch` corre sem
  `--depth`, e nao ha nenhuma linha `fatal`/`bad revision`. O gate captura o stderr do `git log`
  (audit_gate.py:108), portanto uma falha dele nao apareceria nesse log.
impact: |
  Falso-verde do debt gate (a forma do KB-012) em qualquer checkout raso.
mitigation_now: |
  `fetch-depth: 0` no job `debt gate (audit_gate.py)` da CI.
required_fix: |
  Candidata, nao implementada: `git log` com returncode != 0 passa a Critical («nao
  verificavel»), nunca «nao mudou».
falsification_required: |
  Clone raso -> exit != 0 com «nao verificavel»; clone completo -> igual a hoje.
negative_control: |
  Os mesmos testes contra o audit_gate.py antigo (f318c97): test_r41_td026_clone_raso_igual_a_completo
  FAILED — num clone `--depth 1` o hash da evidencia nao existe, o `git log` falha calado e o gate
  antigo da verde com o ficheiro vigiado MUDADO. Com o gate novo o veredito do clone raso e igual
  ao do completo (stale). Ficheiro vigiado inexistente -> Critical «not verifiable». Evidencia, sec. 2.
review_by: 2026-10-12
```

---

## TD-027 — O `unsafe` do `audio-core` nao e exercido pelo Miri em nenhum gate versionado

```yaml
td_id:     TD-027
title:     "As 3 construcoes `unsafe` do `audio-core` (ring buffer SPSC) nao estao sob Miri na CI; a unica via e um script opt-in fora do repositorio"
severity:  Medium
status:    open
origin:    "Lacuna de cobertura registada em 2026-09-20 (CLAUDE.md, contagem de unsafe) e confirmada em 2026-09-29."
source_files: crates/audio-core/src/ring_buffer.rs
context: |
  crates/audio-core/src/ring_buffer.rs:24 (`unsafe impl Sync`), :55 e :75 (blocos `unsafe`).
  .github/workflows/ci.yml:202 — o job `miri` corre so `cargo +nightly-2026-06-02 miri test
  -p led-triple`. O `audio-core` so entra no laco do `~/lumyx-e2e.sh --miri` (linha 102),
  que e opt-in e nao versionado. Nenhum artefacto em docs/evidence/ regista uma execucao
  do Miri sobre o `audio-core`.
impact: |
  Uma regressao de memoria no ring buffer SPSC nao e apanhada por nenhum gate que corra
  sozinho.
mitigation_now: |
  Nenhuma automatica.
required_fix: |
  Por decidir: estender o job `miri` da CI ao `audio-core` (com a mesma exigencia N > 0) ou
  outra via versionada.
falsification_required: |
  Com o gate ligado: violar a disciplina SPSC do ring buffer (mutacao) -> Miri reporta UB e
  o job reprova; revertida -> verde com N > 0.
review_by: 2026-10-12
```

---

## TD-028 — O detector de stale julga pelo commit no `git log`, nao pelo conteudo

```yaml
td_id:     TD-028
title:     "`files_changed_since` usa `git log <hash>..HEAD -- <ficheiro>`: um merge que nao muda o conteudo do ficheiro torna a evidencia «stale» (falso-vermelho)"
severity:  Medium
status:    closed
closed_on: 2026-10-06
closed_by: "R4.1 (ramo ci/audit-gate-integridade): o stale passa a comparar o sha256 do CONTEUDO fixado na evidencia (linhas watched:) com o conteudo do workspace; o git deixa de ser usado. tests/test_audit_gate.py 34 passed; 0 failed. Apos 4 rondas do falsificador o required_test so conta dentro da REGIAO DE PROVA declarada na evidencia (--- prova --- / --- fim da prova ---): lista positiva, sem lista negra de formas NEG; pins duplicados/absolutos/com ../por symlink sao Critical."
evidence_ref: docs/evidence/td-026-028-audit-gate-conteudo-2026-10-06.md
required_test: test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde
origin:    "PR #7, run 36475926021 (job 109109510154, merge ref do PR): «TD-022: evidence is stale — source files changed after evidence was generated (hash 57cf21d): ['crates/led-console-bin/tests/ipc_contra_o_daemon.rs']», com o ficheiro inalterado."
source_files: scripts/audit_gate.py, tests/test_audit_gate.py
context: |
  scripts/audit_gate.py:101-113 (`files_changed_since`) decide por existencia de commits no
  `git log`, nao por diferenca de conteudo.
  Reproduzido em 2026-09-29 sobre a main 84c146e:
    git log --oneline 57cf21d..84c146e -- crates/led-console-bin/tests/ipc_contra_o_daemon.rs
      -> b86464b (merge do PR #6)
    git diff --quiet 57cf21d 84c146e -- (o mesmo ficheiro) -> exit 0 (conteudo igual)
  No PR #7 foi contornado com evidencia re-medida (docs/evidence/td-022-reverificacao-2026-09-28.md,
  git-hash e46151b); o gate nao foi alterado.
  O caso simetrico (conteudo mudado e depois revertido -> falso-verde?) nao foi medido.
impact: |
  Falso-vermelho do debt gate em merges que tocam a historia do ficheiro sem lhe mudar o
  conteudo; obriga a re-medir evidencia valida.
mitigation_now: |
  Re-medir a evidencia sobre um commit posterior (como no PR #7).
required_fix: |
  Por decidir; candidata: comparar conteudo (`git diff --quiet <hash> HEAD -- <ficheiros>`).
  Coordenar com o TD-026 (mesma funcao).
falsification_required: |
  Merge sem mudanca de conteudo -> OK; mudanca real no ficheiro -> Critical stale.
negative_control: |
  Os mesmos testes contra o audit_gate.py antigo (f318c97): test_r41_td028_historia_sem_mudanca_de_conteudo_e_verde
  FAILED — commits B (muda) e C (reverte) depois do hash da evidencia: o gate antigo ve-os no
  `git log` e da stale com o conteudo IGUAL. Simetrico: conteudo realmente mudado sem evidencia nova
  -> stale (test_r41_vigiado_alterado_sem_evidencia_nova_e_stale). Evidencia, sec. 2.
review_by: 2026-10-12
```

---

## TD-029 — O bloqueio de WiFi no Linux falha aberto em tres pontos, e nenhum teste distingue o resultado

```yaml
td_id:     TD-029
title:     "A guarda do ADR-0005 em Linux (`probe_linux`, led-hal/src/network_guard.rs:229) e o pre-voo do daemon deixam o show arrancar sem verificar o WiFi: operstate ilegivel ou nao-`up`, e qualquer `ProbeUnavailable`, contam como rede OK"
severity:  High
status:    open
origin:    "ROADMAP 0.8 (candidato) e plano consolidado 0.H; recon e falsificacao (agente separado) a 2026-10-04, por leitura do codigo — nao executado em Linux real. Severidade High decidida pelo operador: guarda de seguranca fail-open."
source_files: crates/led-hal/src/network_guard.rs, crates/led-daemon-bin/src/preflight.rs
context: |
  Tres pontos, todos medidos no codigo:
  (1) `probe_linux` (:260) — `if let Ok(state) = fs::read_to_string(&operstate_path)`: uma interface sem fio cujo
      operstate nao se le e IGNORADA; o resultado final e `Ok(())`, "sem WiFi".
  (2) `probe_linux` (:261) — so bloqueia `state == "up"`. [INFERIDO, doc do kernel citada de memoria, por confirmar]
      `dormant` (L1 activo a espera de 802.1X/WPA) e `unknown` (que o kernel manda tratar como capaz de dados)
      tambem passam como "sem WiFi".
  (3) `preflight.rs:151-156` — `Err(ProbeUnavailable)` → `network_ok: true` ("prosseguindo com aviso"). Por isso o
      ramo que ja existe para `/sys/class/net` ausente ou `read_dir` falhado (network_guard.rs:234-244) tambem deixa
      o show arrancar; e uma correccao que so devolva `ProbeUnavailable` em (1) NAO bloquearia nada.
  E o journal escreve `network_checked: sem WiFi ativo` (preflight.rs:141) no caso (1) — afirma que verificou.
  Cobertura: `rg probe_linux crates/` devolve so a definicao (:229) e a chamada (:128). O ramo Linux e EXECUTADO no
  job ubuntu por `led-hal/src/hal.rs:234` (`hal_with_guard_wifi_block_does_not_panic`), que aceita qualquer resultado
  — executado sem oraculo. A decisao de rede do pre-voo e testada so com guarda FALSA (`GuardaFalsa`, alvo
  192.168.2.156); o `WifiBlockGuard` real nunca e chamado pelo pre-voo em teste (os E2E usam loopback e saem em
  `todos_loopback()`, preflight.rs:120). E o teste `sonda_indisponivel_prossegue_mas_nunca_afirma_ter_verificado`
  (preflight.rs:462-471) FIXA o defeito (3): afirma `ProbeUnavailable` → `network_ok true`.
  Drift doc/codigo: network_guard.rs:107 diz `/sys/class/net/wl*/operstate`; o codigo deteta por `wireless/` ou
  `phy80211/` (:251-252).
impact: |
  Um show em Linux pode arrancar sobre WiFi activo — o que o ADR-0005 proibe (jitter de 31 ms medido na bancada de
  2026-07-20) — com o journal a dizer que a rede foi verificada. Nao observado em Linux real.
mitigation_now: |
  Nenhuma automatica. Em macOS o pre-voo usa `probe_macos`. [2026-10-07] Desde o R4.T o ponto (3) so se aplica a
  plataformas NAO suportadas (D1); os fail-open por interface (Linux operstate/flatten, macOS ifconfig) continuam.
required_fix: |
  Decisao do operador (toca a politica do ADR-0005 e area protegida): fail-closed no PRE-VOO, nao so na sonda —
  `ProbeUnavailable` com alvo de rede → `network_ok: false` com mensagem explicita; override so por flag explicita,
  registada no journal. Na sonda: operstate ilegivel → `ProbeFailed` [2026-10-07: D1 — `ProbeUnavailable` passou a
  significar SO nao suportado e NAO bloqueia; usa-lo aqui reabria o fail-open]; estados nao-`down` de uma interface sem
  fio (pelo menos `up`, `dormant`, `unknown`) → tratados como activos, a confirmar na doc do kernel. A decisao da sonda
  extraida para uma funcao pura sobre uma raiz injectavel (`probe_linux_em(raiz: &Path)`). O teste
  preflight.rs:462 tem de ser INVERTIDO (nao apagado) na mesma fatia, com o porque no commit. Accept aprovado antes.
falsification_required: |
  Sonda, com sysfs falso em tempdir: wlan0 `up`, `dormant`, `unknown` → activo; `down` → inactivo; sem `wireless/` →
  ignorada; operstate ilegivel → ProbeFailed [D1, 2026-10-07] (reprova com o codigo actual). Pre-voo, com alvo NAO-loopback e
  guarda injectada: ProbeUnavailable → network_ok false (reprova com o codigo actual, preflight.rs:156); com a flag de
  override → true e a linha no journal. Os dois a correr no job ubuntu, com N_executado == N_esperado.
progress: |
  2026-10-07 (fix/td-029-sonda-falhada-bloqueia, accept R5 aprovado — opcao B). FEITO: o ponto (3) para
  plataformas SUPORTADAS (numa nao suportada continua nao-fatal, de proposito: D1) e o ramo
  `/sys/class/net` ausente/`read_dir` falhado. Variante nova `ProbeFailed { probe, error }` (Display CRITICAL);
  Linux :252/:260 e macOS :164/:170 (linhas em 2926a3a) devolvem-na; `ProbeUnavailable` fica SO para SO nao suportado (D1, zero
  docs/adr/). Pre-voo: ProbeFailed → network_ok false + `network_probe_failed`; com `--assume-no-wifi` (so CLI,
  por execucao) → true + `network_assumed_by_operator` em CADA pre-voo; flag com sonda OK → `network_override_unused`;
  WifiActive bloqueia sempre. O teste preflight.rs:462 foi PRESERVADO para ProbeUnavailable e INVERTIDO para
  ProbeFailed (`sonda_falhada_numa_plataforma_suportada_bloqueia`).
  ACHADO D4 (observabilidade): o caminho IPC do `load` (run.rs, `apply_ipc`) PERDIA TODAS as notices do pre-voo —
  nenhuma chegava ao journal, nao so as de rede. Passa a escreve-las com o mesmo `notice_to_json` do caminho CLI,
  zero campos novos (e2e `dois_loads_ipc_com_override_deixam_dois_eventos_no_journal`).
  POR FAZER (mantem o TD aberto): pontos (1) operstate ilegivel ignorado e (2) `dormant`/`unknown` como inactivos,
  e a extraccao `probe_linux_em(raiz)` — fora do accept R5. Nao executado em Linux real.
  RESIDUAIS medidos pelo falsificador (R4.T, ~/lumyx-evidence/2026-10-07/r4.T/falsifier.md):
  (a) os 4 sitios D3 (macOS :164/:170, Linux :252/:260) devolvem ProbeFailed mas NENHUM teste o exercita — repor
      `ProbeUnavailable` ou `Ok(())` ali deixa a suite verde (MA2/MA3). Fecha so com sondas injectaveis
      (`probe_macos_com`/`probe_linux_em`) — alteracao do led-hal fora do accept.
  (b) O equivalente macOS do ponto (1): se o `ifconfig <if>` falhar, `is_interface_active_macos` devolve `false` e a
      interface Wi-Fi conta como INACTIVA; o pre-voo regista `network_checked`. Medido sem mutacao (FI_E). Fail-open.
  (c) Observabilidade: no arranque com IPC (`run_with_control_com`) o Arm/Play do show inicial descarta os eventos
      (`let _ = rt.apply(..)`): o journal nao mostra `transitioned` para ready/playing nesse caminho.
  (d) [por leitura, nao executado] Linux: `entries.flatten()` ignora em silencio entradas de `/sys/class/net` com erro
      de I/O. macOS: `networksetup` com exit 0 mas sem bloco Wi-Fi reconhecivel conta como «sem WiFi». Mesma classe do
      ponto (1): fail-open dentro da sonda. O mesmo efeito por outra via (falsificador ronda 4, MR7): um embrulho no
      led-daemon-bin que converta `ProbeFailed` em `ProbeUnavailable` antes do pre-voo — nenhum teste o ve, porque a
      sonda real nunca falha nos testes. Escopo: apesar do titulo «Linux», (b) e (d) sao macOS.
  (e) Limites dos testes (falsificador ronda 2): remover um #[test] so e apanhado pela contagem N (o cargo da exit 0);
      o teste de ambiente procura padroes textuais so no src/ do led-daemon-bin (um meio que nao use `std::env`/`var(`,
      ou uma leitura do ambiente noutro crate que alimente `Config`, escapa-lhe — MB1d, ronda 3); os oraculos temporais
      observam uma janela finita (40 ticks) depois do arranque. O teste da guarda REAL no binario reconhece a
      permissiva pelo NOME e, num runner sem WiFi, envia frames por software para 192.0.2.10 (TEST-NET); a sua
      asserção «sem a flag nao ha override» (MR4/MR5) so discrimina onde a guarda real nao reprova por WiFi (CI sem
      WiFi, ou com falha injectada) — numa maquina com WiFi activo e cega a esse mutante. No caminho IPC o fio e
      medido (status `frames == 0` depois de um load recusado); no modo CLI nao (MCLI4, NOT_MEASURED: UDP para o IP
      do proprio en0 e descartado nesta maquina, sem recetor que sirva de oraculo). A ORDEM das notices face aos
      eventos no journal nao e afirmada (MC8b, ronda 8: o D4 fala de formato, nao de ordem). O `esc()` do journal nao
      escapa `\n`: um erro de sonda com quebra de linha parte o JSONL nos dois caminhos (anterior ao TD-029).
  (f) Leitura do D2(a) por decidir pelo operador (verifier O1): o aviso em stderr sai UMA vez, no arranque; em cada
      pre-voo sai a notice JSONL. Se o D2(a) pede um aviso em stderr POR pre-voo, isso nao esta implementado.
review_by: 2026-10-31
```
