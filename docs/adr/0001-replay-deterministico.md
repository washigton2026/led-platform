# ADR-0001 — Replay determinístico via hash de pixels

- **Status:** aceito
- **Data original:** 2026-06-26 (formato `.lumyx` + `pixel_hash`); estendido em
  2026-06-27 (validação inline no `led-demo`) e 2026-06-28 (`compute_pixel_hash`
  FNV-1a em `Provenance`)
- **Fonte:** CLAUDE.md changelog 2026-06-26/27/28

## Contexto e problema
Um show profissional precisa ser **reproduzível**: o que foi aprovado no estúdio
tem de sair pixel-por-pixel idêntico no palco, em qualquer máquina, e uma
regressão de render (efeito, gamma, ordem RGB) precisa ser detectável
automaticamente. Sem uma âncora determinística, "o show mudou" vira uma
discussão subjetiva olhando GIFs.

## Decisão
Adotar hashing determinístico de pixels como contrato de reprodutibilidade:
- Formato binário `.lumyx` (`led-show-recorder`) grava o stream de
  `LogicalFrame` cru (RGB, sem compressão).
- `pixel_hash` / `compute_pixel_hash` usam **FNV-1a 64-bit** — um hash simples,
  sem dependências, byte-idêntico em qualquer plataforma de inteiros.
- `ReplayManifest` guarda hash por-frame + agregado; `verify_replay` compara o
  replay contra o hash gravado; `cross_node_verify` compara dois nós.
- O `led-demo` re-lê o `.lumyx` que acabou de gravar e afirma
  `pixel_hash(replayed) == pixel_hash(rerendered)` inline.

## Consequências
**Boas:** regressão de pixel vira teste falsificável (gate P10/P15); dois nós
provam sincronia por igualdade de hash; a base de todo o trabalho posterior de
Provenance e assinatura (ADR-0004). Determinismo cross-platform mensurável
(`determinism_vector.rs`, goldens pinados).
**Ruins/custos:** `.lumyx` guarda RGB cru (não comprimido) — arquivos grandes;
aceito conscientemente ("simplicidade sobre tamanho; compressão em v2 via bump de
versão do formato"). O hash de render usa `f32` trig no Plasma, que **não** é
garantido idêntico entre implementações de libm — por isso o hash de intent
(inteiro) é gate duro e o de render é instrumento de medição, não gate universal.

## Alternativas rejeitadas
- **Comparar imagens/GIF renderizados** — subjetivo, caro, não falsificável.
- **Hash criptográfico (SHA-256) dos pixels** — desnecessário para detecção de
  regressão (não é superfície de ataque aqui) e mais lento; FNV-1a basta. A
  camada criptográfica entra só na fronteira de confiança (ADR-0004).
- **Gravar comprimido desde v1** — adiado; compressão acopla o formato a um codec
  e complica o parser std-only.
