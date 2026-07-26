# ADR-0015 — Caminho de dados do preview (cópia lossy fora do hot-path)

- **Status:** aceito (pré-implementação)
- **Data original:** 2026-07-26
- **Fonte:** Decisão de arquitetura UI/Preview + invariante do triple buffer (ADR-0008)

## Contexto e problema
O preview ao vivo precisa mostrar os pixels que o engine está renderizando. O caminho
render→send é um **triple buffer lock-free** cuja invariante de segurança é "render e send
**nunca** compartilham buffer" (ADR-0008). Se a UI (ou um publicador de preview) **ler o
triple buffer** ou impuser backpressure, quebra o isolamento e ameaça o jitter do output.

## Decisão
O preview é alimentado por uma **cópia separada, downsampled, rate-limited e lossy**,
publicada pelo daemon **fora do hot-path**. A UI **nunca** lê o triple buffer e **nunca** faz
backpressure no engine. Se o consumidor de preview está lento, frames de preview são
**descartados** (best-effort), sem afetar render/send.

## Escopo / Não-escopo
- **Escopo:** contrato de que o preview é uma via read-only, lossy, downsampled, desacoplada
  do hot-path.
- **Não-escopo:** o algoritmo exato de downsample/LOD; renderização 2D/3D no cliente
  (WebGPU, ADR-0016); a taxa numérica final (a medir).

## Alternativas descartadas
- **Tap direto no triple buffer** — proibido; quebra a invariante do ADR-0008.
- **Preview síncrono/backpressured** — ameaça o jitter do output.

## Limites de segurança
O preview carrega só cor de pixel downsampled — nenhum dado sensível de rede/controle.
Trafega pelo mesmo canal restrito do ADR-0014.

## Isolamento do hot-path
A publicação de preview é uma **cópia** tomada fora do caminho crítico (não em `send_frame`,
não lendo os slots do triple buffer). Sem alocação no hot-path; sem lock compartilhado com
render/send. Esta é uma **regra de isolamento, não uma otimização**.

## Compatibilidade de OS
Agnóstico: é um stream de dados. O rendering do preview no cliente depende de GPU
(ADR-0016), com fallback (abaixo).

## Degradação segura
Preview lento → descarta frames (lossy). GPU do cliente indisponível → preview cai para
2D/CPU ou desliga com aviso; o engine **já** é `gpu`-gated com fallback CPU
(`AutoGpuPlasma`), então o **output nunca depende da GPU da UI**. Preview ausente ≠ show
ausente.

## Consequências
**Boas:** preview rico sem risco ao output; escala a rigs grandes via downsample/LOD.
**Ruins/custos:** preview não é pixel-perfeito nem frame-exato (é aproximação lossy) —
aceito conscientemente; custo de CPU/banda da cópia (mitigado por rate-limit/downsample).

## Métricas / gates
Gate: teste/prova de que ativar o preview **não altera** o p99 de output nem introduz
alocação no hot-path (reusa o `no_alloc` + `MetricsEmitter`). Preview alvo ~30 fps,
degradável.

## Critério de reversão
Nenhum para a **regra de isolamento** (tap direto no triple buffer permanece proibido). O
*mecanismo* de publicação (canal, downsample) é substituível se não bater o budget, desde
que preserve lossy + fora-do-hot-path.
