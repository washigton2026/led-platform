# ADR-0008 — Triple buffer entre render e send

- **Status:** aceito
- **Data original:** 2026-06-03 (Phase 1 foundation)
- **Fonte:** CLAUDE.md changelog 2026-06-03; "Invariants that bite"

## Contexto e problema
A thread de render produz `LogicalFrame`s e a thread de send os transmite ao
dispositivo. Se as duas compartilharem um buffer mutável, ou há contenção de
lock no hot-path, ou risco de **frame rasgado** (o send lê enquanto o render
escreve). O invariante do projeto: render e send **nunca** compartilham um
buffer mutável, e não pode haver alocação no hot-path.

## Decisão
Um triple buffer lock-free: **3 slots `UnsafeCell` + 1 `AtomicUsize`**
(index | fresh-bit). Cada thread sempre tem seu próprio slot; a troca é um swap
atômico de índice. A **invariante de permutação dos 3 slots é o argumento de
segurança inteiro** — nunca dois donos do mesmo slot.

## Consequências
**Boas:** handoff render→send sem lock e sem alocação; nunca há frame rasgado.
Verificado sob **Miri em 24 seeds de scheduler** + stress de 200k/1M ciclos —
a corretude do `unsafe` é provada, não assumida. É a base do orçamento de
latência (ADR relacionado ao budget ≤5 ms cabeado).
**Ruins/custos:** um bloco `unsafe` real (com o requisito de que todo `unsafe`
venha com teste que o exercite, e Miri se concorrente). Três buffers de pixel
alocados no setup (não no hot-path) — custo de memória fixo, aceito.

## Alternativas rejeitadas
- **`Mutex`/`RwLock` sobre um buffer** — contenção no hot-path de render; mesma
  classe de problema que o ADR-0002 resolveu no AudioShare.
- **Double buffer** — insuficiente: com dois slots, produtor e consumidor podem
  colidir quando o produtor quer publicar um novo frame antes do consumidor
  soltar o anterior; o terceiro slot quebra o empate.
- **Canal SPSC copiando frames** — cópia por frame no hot-path (alocação);
  aceitável para o ring de áudio (dados pequenos), não para pixels.
