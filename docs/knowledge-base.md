# LUMYX Knowledge Base — Permanent Failure Records

Each entry represents a class of bug that has been permanently closed by a
detector (test or lint) so it can never silently recur.

Format per entry: KB-ID, class, root cause, prevented_by, detector, status.

---

## KB-010 — cargo fix slice-panic hazard

```yaml
kb_id:        KB-010
bug_class:    "cargo-fix converts safe empty range into panicking slice"
root_cause:   |
  cargo fix / clippy::needless_range_loop rewrites:
    for i in start..end.min(total) { s[i] = v; }
  into:
    let end = end.min(total);
    s[start..end].fill(v);

  When `start > total`, the original range is safely empty (no-op).
  The slice form `s[start..end]` where start > end PANICS at runtime.
  Affected: audio-core/src/capture.rs k=7 iteration (start=216000 >
  total=192000), breaking mock_beat_impulses_detected.
prevented_by: |
  Always use an explicit guard when converting range-indexed writes to
  slice operations:
    let end = (start + n).min(total);
    if start < end { s[start..end].fill(v); }
  OR suppress the lint and keep the indexed loop:
    #[allow(clippy::needless_range_loop)]
    for i in start..end.min(total) { s[i] = v; }
detector:     |
  Test: audio-core::capture::mock_adversarial_tests::mock_hop_window_past_buffer_end_no_panic
  Exercises start > total on k=0..4 with start = total + k*interval.
  Proves that the guard prevents panic for all out-of-bounds windows.
first_seen:   2026-06-17
linked_debt:  NEW-TD-009 (see ledger)
status:       permanent
notes:        |
  The fix is the guard `if start < end`. The lesson applies to any
  pattern where a range-indexed loop is converted to slice indexing —
  always verify that start ≤ end before slicing. clippy::needless_range_loop
  is safe to apply ONLY when the loop has a single array accessed by `i`
  AND there is no possibility of `i` being used as a general offset that
  could produce inverted ranges.
```

---

## KB-009 — zip() iterator overhead breaks debug-mode timing budgets

```yaml
kb_id:        KB-009
bug_class:    "Iterator zip() adds overhead that breaks wall-clock timing tests in debug builds"
root_cause:   |
  cargo fix / clippy::needless_range_loop rewrites hot-path indexed loops:
    for i in 0..N { out[i] = buffer[i].process(); }
  into:
    for (o, b) in out.iter_mut().zip(buffer.iter()) { *o = b.process(); }

  The zip() version is semantically correct but adds iterator state machinery
  that, in debug builds (no inlining, no optimizations), can be 3-5x slower.
  Affected: audio-core/src/fft.rs (magnitude_spectrum) and beat.rs (flux loop),
  breaking mock_analyze_all_realtime_speed (<1s) and classifier_10k_frames_fast
  (<2000ms debug budget).
prevented_by: |
  When a hot-path loop accesses two arrays simultaneously, suppress the lint:
    #[allow(clippy::needless_range_loop)]
    for i in 0..N { ... }
  Document WHY: "zip() adds debug-mode overhead that breaks timing budgets".
detector:     |
  Tests: mock_analyze_all_realtime_speed, classifier_10k_frames_fast
  Both assert wall-clock budgets and will regress if zip() is introduced
  in the FFT/beat-detection hot path.
first_seen:   2026-06-17
linked_debt:  TD-003 (TEST-WALLCLOCK-001 — timing tests regress under system load)
status:       permanent
notes:        |
  The #[allow] is placed WITH a comment explaining the performance reason.
  Future reviewers must not remove the allow without re-running the timing
  tests in release mode to verify correctness.
```

---

## KB-011 — AudioFeatures cross-thread: snapshot coerente obrigatório

```yaml
kb_id:        KB-011
bug_class:    "Cross-thread AudioFeatures lida campo-a-campo viola coerência semântica"
root_cause:   |
  AudioScalars contém campos semanticamente acoplados:
    beat: bool  +  timestamp_ms: u64
  BeatFlash verifica: `beat && timestamp_ms != last_beat_ts`
  Se beat e timestamp_ms chegam de publishes DIFERENTES (tearing), o detector
  de "novo beat" misfires: pode disparar flash extra ou perder beat.

  Instâncias que falharam neste projeto:
  1. Per-field atomics (60afc4a): 7 loads separados = incoerente.
     beat@publish_N+1, timestamp_ms@publish_N → falsa identidade de beat.
  2. RwLock<AudioScalars> (57f7722): coerente, mas adquiria lock no render
     hot-path por frame (RT-LOCK-RENDER-001 ainda disparava).

prevented_by: |
  Publicar AudioScalars SEMPRE como struct inteira via transporte atômico:
    - ArcSwap<AudioScalars>: store() = um swap atômico de ponteiro;
      load() = um load atômico, sem lock, sem tearing. (solução adotada)
    - tokio::sync::watch<AudioScalars>: borrow() retorna guard coerente
      (mas usa RwLock internamente; adequado se tokio já for dep do crate).
  NUNCA armazenar campos de AudioScalars em atomics separados — mesmo que
  cada campo individualmente seja correto, a combinação pode ser incoerente.

detector:     |
  Test: led-pixel-engine::reactive::adversarial_tests::
        audioshare_scalars_beat_timestamp_coherent_under_concurrency
  Publica 10k frames alternando beat=true/false com timestamp_ms par/ímpar.
  Leitor verifica invariante: beat == (timestamp_ms % 2 == 1) em cada snapshot.
  Qualquer tearing entre os dois campos viola o invariante → teste falha.
  Com per-field atomics: ~5000 violações em 10k frames.
  Com ArcSwap: 0 violações.

grep_detector: |
  grep -n 'load(\|borrow()' crates/led-pixel-engine/src/reactive.rs
  → dentro de scalars(): deve ser 1 chamada, sem read()/write()/lock().
  N loads separados = KB-011 violation.

first_seen:   2026-06-19
linked_debt:  TD-002 (RT-LOCK-RENDER-001)
status:       permanent
notes:        |
  A regra generaliza além de AudioScalars: qualquer struct cujos campos têm
  invariante semântico entre si (ex.: (sequence_num, payload), (beat, ts))
  deve ser publicada como unidade atômica. Campos individuais podem ser
  atômicos apenas se forem semanticamente independentes.
```
