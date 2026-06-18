# LUMYX Technical Debt Ledger

Canonical source of truth for all tracked debt items. One entry per TD-ID.
Updates: edit this file + commit. Session ledger (in-chat) must not diverge.

Last updated: 2026-06-18 (HIGH-3 close)

---

## Status legend
- `open`       — unfixed, work required
- `diagnosed`  — root cause known, not yet fixed
- `closed`     — permanently fixed + detector added (test/lint/CI)
- `wontfix`    — acknowledged, intentionally deferred

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
status:    diagnosed
source:    LOW-1 investigation
type:      runtime / startup
root_cause: |
  wgpu::Instance::enumerate_adapters() spawns a Metal command queue on the
  main thread. On macOS 14+ without an active CAMetalLayer, the
  MTLCreateSystemDefaultDevice() call blocks indefinitely waiting for the
  WindowServer connection. Reproduces 100% in headless CI, intermittently
  under load on dev machines.
chain:     wgpu 0.19 → metal-rs 0.28 → objc2-metal → block_on(request_device)
fix: |
  Option A: spawn wgpu init on a dedicated thread, timeout 2s, fall back
            to software (wgpu::Backends::GL or CPU path).
  Option B: gate GPU crate behind `feature = "gpu"`, always off in CI.
  Option C: upgrade wgpu ≥0.20 (fixes Metal headless init).
milestone: MEDIUM-1 (22→29 Jun, dedicated session — DO NOT TOUCH outside that)
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
```

---

## TD-002 / TD-010 — RT-LOCK-RENDER-001: lock no render hot-path (AudioShare)

```yaml
td_id:     TD-002
alias:     TD-010 (registrado em 2026-06-18 antes de reconciliação)
title:     "AudioShare scalars() adquiria lock no render hot-path por frame; atomics violavam coerência"
severity:  High
status:    closed
closed_on: 2026-06-18
history: |
  Commit f6c496c: Mutex → RwLock (melhoria parcial — ainda bloqueante, coerente).
  Commit 60afc4a: RwLock → 7 AtomicU32/U64/Bool (lock-free mas snapshot incoerente —
    beat e timestamp_ms podiam vir de publishes diferentes, quebrando BeatFlash).
  Commit final: atomics removidos; AudioShare reestruturado com snapshot coerente.
fix: |
  led-pixel-engine/src/reactive.rs: AudioShare reescrito com dois RwLocks separados:
    scalars:  RwLock<AudioScalars>  — snapshot coerente de struct completa
    spectrum: RwLock<Vec<f32>>      — separado, nunca tocado por render()

  publish(): write() substitui a struct inteira atomicamente.
  scalars(): um único read() → copia a struct (~40 bytes) → drop do guard.
    Todos os campos vêm do mesmo publish() — coerência garantida.
  with_spectrum(): read() separado — render() nunca chama este método.

  Não usa tokio::sync::watch (led-pixel-engine é std-only; watch internamente
  usa RwLock de qualquer forma — mesma semântica, mais dependência).
reproduce: |
  grep -n 'load(\|borrow()\|read()' crates/led-pixel-engine/src/reactive.rs
  → read() só em scalars() (1 call) e with_spectrum() (1 call). Nunca em render().
verified: |
  49 led-pixel-engine tests pass incluindo:
    - audioshare_concurrent_publish_read_no_deadlock (8 threads)
    - audioshare_scalars_beat_timestamp_coherent_under_concurrency (NOVO — 10k frames,
      leitor verifica beat == (timestamp_ms % 2 == 1) em cada snapshot, 0 violações)
  Clippy -D warnings: 0. Workspace: 312 passed, 0 failed.
```

---

## TD-006 — TEST-BUDGET-001: wall-clock budget em teste é paliativo

```yaml
td_id:     TD-006
title:     "mock_analyze_all_realtime_speed: budget 2.0s alargado é paliativo, não fix"
severity:  Medium
status:    open
source:    AEGS audit / KB-009 reconciliation (2026-06-18)
type:      test-reliability / wall-clock
location:  crates/audio-core/src/capture.rs:559
root_cause: |
  O teste asserta elapsed < 2.0s mas o real invariante é "processa 1s de áudio
  muito mais rápido que real-time". O budget foi expandido de 1.0s → 2.0s para
  evitar flap sob workspace parallelism. Isso mascara a causa raiz: o teste
  usa Instant::now() wall-clock que é afetado por carga do sistema.
fix: |
  Opção A: marcar o teste com #[ignore] e invocar separadamente no e2e script
           com cargo test -p audio-core -- mock_analyze_all_realtime_speed
  Opção B: medir em release (--release) onde o budget é muito mais folgado
  Opção C: usar um critério baseado em contagem de hops, não wall-clock
milestone: MEDIUM-3 (junto com outros melhorias de test-reliability)
note:      "KB-009 documenta a classe. O budget 2.0s é provisório."
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
```

---

## Closed items — summary table

| TD-ID   | Title (short)                         | Closed     | Commit   |
|---------|---------------------------------------|------------|----------|
| TD-003  | 8 thread::sleep em tests (tests/)     | 2026-06-18 | 845e010  |
| TD-003b | 9º sleep cluster.rs #[cfg(test)]      | 2026-06-18 | pending  |
| TD-007  | cargo-audit not installed             | 2026-06-17 | LOW-1    |
| TD-008  | flash_buf alloc em render loop        | 2026-06-17 | e858fa8  |
| TD-009  | cargo fix → slice panic + zip timing  | 2026-06-17 | 73376ed  |
| TD-002  | RT-LOCK-RENDER-001: lock em scalars() | 2026-06-18 | pending  |
| TD-005  | adapt() aloca per-call               | closed     | adapt_into em produção |

## Open items — priority order

| TD-ID  | Severity | Title (short)                          | Milestone |
|--------|----------|----------------------------------------|-----------|
| TD-004 | High     | wgpu→Metal block on startup            | MEDIUM-1  |
| TD-006 | Medium   | wall-clock budget em teste (paliativo) | MEDIUM-3  |

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
