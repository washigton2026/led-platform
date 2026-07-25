# ADR-0002 — ArcSwap substitui Mutex no AudioShare

- **Status:** aceito
- **Data original:** 2026-06-19 (TD-002, RT-LOCK-RENDER-001)
- **Fonte:** CLAUDE.md changelog 2026-06-19; KB-011

## Contexto e problema
`AudioShare.scalars()` era lido no **render hot-path a cada frame** (60 fps) pela
thread de render, enquanto a thread de áudio escrevia a ~200 Hz. A primeira
implementação usava `Mutex`/`RwLock` — contenção de lock no caminho crítico de
tempo real. Uma segunda tentativa, com 7 campos atômicos independentes, era
lock-free mas **incoerente**: havia *tearing* entre `beat` e `timestamp_ms`
(o leitor via um `beat` novo com um `timestamp` velho), o que quebrava o
`BeatFlash`. O invariante violado: um snapshot de `AudioFeatures` cruzando
threads tem de ser coerente **inteiro**, não campo-a-campo (KB-011).

## Decisão
Usar `ArcSwap<AudioScalars>` — publicação atômica da struct inteira:
```
publish(): self.scalars.store(Arc::new(AudioScalars{..}))  // 1 swap atômico
scalars(): *self.scalars.load().as_ref()                   // 1 load atômico, sem lock
```
Dependência `arc-swap = "1"` adicionada a `led-pixel-engine` com justificativa
explícita (RT-LOCK-RENDER-001). Zero `unsafe` em `reactive.rs` — o arc-swap
encapsula o próprio unsafe.

## Consequências
**Boas:** leitura lock-free e **coerente** no hot-path; `BeatFlash` correto sob
concorrência. Detector semântico
(`audioshare_scalars_beat_timestamp_coherent_under_concurrency`, 10k frames):
0 violações com ArcSwap vs ~5000 violações que ocorreriam com atômicos
campo-a-campo. Superfície `unsafe` do crate não cresceu (só `triple.rs`).
**Ruins/custos:** cada `publish` aloca um `Arc` novo (o leitor solta o antigo) —
alocação na thread de áudio, não na de render; aceito por não estar no hot-path
crítico de render. Uma dependência externa a mais no crate.

## Alternativas rejeitadas
- **Mutex/RwLock** — contenção de lock no render hot-path; a origem do problema.
- **7 atômicos campo-a-campo** — lock-free porém incoerente (tearing semântico);
  descartado **permanentemente** e registrado em KB-011.
- **`tokio::sync::watch`** — usa `RwLock` internamente e traria `tokio` para o
  `led-pixel-engine`, que é std-only por design.
