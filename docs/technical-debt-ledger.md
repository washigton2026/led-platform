# LUMYX Technical Debt Ledger

Canonical source of truth for all tracked debt items. One entry per TD-ID.
Updates: edit this file + commit. Session ledger (in-chat) must not diverge.

Last updated: 2026-06-17 (LOW-1 close)

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

## Closed items — summary table

| TD-ID  | Title (short)                         | Closed     | Commit   |
|--------|---------------------------------------|------------|----------|
| TD-007 | cargo-audit not installed             | 2026-06-17 | LOW-1    |
| TD-008 | flash_buf alloc in render loop        | 2026-06-17 | e858fa8  |
| TD-009 | cargo fix → slice panic + zip timing  | 2026-06-17 | 73376ed  |

## Closed items — summary table

| TD-ID  | Title (short)                         | Closed     | Commit   |
|--------|---------------------------------------|------------|----------|
| TD-003 | 8 thread::sleep in tests              | 2026-06-18 | 845e010  |
| TD-007 | cargo-audit not installed             | 2026-06-17 | LOW-1    |
| TD-008 | flash_buf alloc in render loop        | 2026-06-17 | e858fa8  |
| TD-009 | cargo fix → slice panic + zip timing  | 2026-06-17 | 73376ed  |

## Open items — priority order

| TD-ID  | Severity | Title (short)                          | Milestone |
|--------|----------|----------------------------------------|-----------|
| TD-004 | High     | wgpu→Metal block on startup            | MEDIUM-1  |
