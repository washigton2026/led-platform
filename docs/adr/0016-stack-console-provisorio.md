# ADR-0016 — Stack do console de operador (PROVISÓRIO — pendente spike)

- **Status:** 🟡 **proposto (provisório)** — **não** é decisão final; fecha só após o spike abaixo
- **Data original:** 2026-07-26
- **Fonte:** Decisão de arquitetura UI/Preview (tabela de trade-offs)

## Contexto e problema
O console precisa de **acessibilidade WCAG 2.2 AA** (teclado-completo, leitor de tela) e
**preview 2D/3D**. Toolkits Rust nativos imediatos (egui/Iced) têm acessibilidade imatura; o
**DOM** tem a melhor história de acessibilidade; o preview exige **GPU**.

## Decisão provisória
**Console web acessível: controles em DOM + preview em WebGPU.** Candidato preferido:
**Leptos (Rust→WASM)** — honra "Rust puro" e mantém a11y via DOM. **Contingência:
TypeScript/React ou Svelte** (a11y provada, mais velocidade, ao custo de abrir mão de Rust no
front). Empacotamento provável: app desktop (webview do SO). **Iced/egui rejeitados** para o
console (a11y); egui admissível apenas para um HUD de debug interno.

**Esta subdecisão (Leptos vs TS) NÃO está fechada** — depende do spike. **Leptos+Tauri não é
decisão final.**

## Escopo / Não-escopo
- **Escopo:** direção provisória (web/DOM/WebGPU) + critérios objetivos para fechar a stack.
- **Não-escopo:** decisão final da stack (pós-spike); empacotamento definitivo.

## Alternativas descartadas (para o console)
Iced e egui (a11y insuficiente hoje); Electron (footprint); UI nativa por-OS (custo de
manutenção × 3 + a11y fragmentada).

## Limites de segurança / hot-path / OS / degradação
Herdados dos ADRs 0013–0015: a stack da UI é irrelevante para o isolamento do output (o
daemon garante). GPU indisponível → fallback de preview (ADR-0015). Cross-platform de
autoria; Windows não orienta a arquitetura.

## Consequências / critério de reversão
Provisório por natureza: o spike pode inverter Leptos↔contingência. **Reversão = resultado
do spike.**

## Plano do spike (time-boxed, 5 dias úteis)
Constrói o **mesmo protótipo mínimo** em **Leptos** e em **TypeScript/React (ou Svelte)**,
ambos consumindo um read-model fake e renderizando um preview WebGPU de ~10k pixels.

| Eixo | Critério mensurável (passa/reprova) |
|---|---|
| **Acessibilidade** | axe/Lighthouse **0 violações críticas**; navegação teclado-completa; leitor de tela (VoiceOver/NVDA) anuncia `HealthStatus` via live-region |
| **Build/dist** | build limpo nos 3 OS de autoria; bundle e tempo de build medidos; instalável |
| **Preview** | 10k px @ ≥ 30 fps em WebGPU; fallback 2D quando GPU ausente |
| **Produtividade** | tempo p/ implementar 1 painel read-only + 1 comando; linhas/deps; DX anotada |
| **Isolamento** | prova de que a UI só fala com um IPC fake — zero acesso a estado de engine |

**Saída do spike:** tabela comparativa preenchida + veredito → promove este ADR de
"provisório" para "aceito" com a stack vencedora.
