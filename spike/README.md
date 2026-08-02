# Spike de stack de UI — Leptos vs React (ADR-0016)

**Objetivo:** fechar a subdecisão da ADR-0016 (stack do console do operador) com
**evidência medida por você, na sua máquina**. Este diretório é *descartável* e está
`exclude`ído do workspace Rust — nada aqui entra no baseline de produção.

> **O que a IA NÃO carimbou (fica com você):** acessibilidade (axe/Lighthouse + leitor de
> tela), fps do preview WebGPU, tempo de build/preview a frio/quente, e DX subjetiva. Estes
> exigem browser, GPU e julgamento humano — **não são mensuráveis no ambiente do agente** e
> nenhum número foi inventado. O agente só preparou os dois protótipos e este checklist.

Os dois protótipos implementam **a mesma tela mínima**: um painel de saúde acessível que
consome um read-model **falso** (mesmo shape do `led-readmodel::ReadModel` real) + um preview
WebGPU de ~10k pontos com fallback 2D.

---

## Resultados MEDIDOS pelo agente (2026-07-30)

Parte do spike **foi executada de verdade** — o que segue é evidência, não estimativa. O que
não pôde ser medido está marcado como tal e continua com você.

| Eixo | React/Vite | Leptos/WASM |
|---|---|---|
| **Compila de primeira** | ✅ sim | ❌ **2 correções** necessárias¹ |
| **Build** | `vite build` **1,64 s** | `cargo build --target wasm32` **44,98 s** (debug)² |
| **Bundle** | **47 kB gzip** | ⏳ precisa de `trunk` para empacotar |
| **axe-core** | ✅ **0 violações · 37 regras aprovadas** (axe 4.12.1) | ⏳ não medido (precisa servir a página) |
| **Árvore de a11y** | ✅ `main` · `status`+`aria-live=polite` · `region`+headings · `table`+caption · `img`+label | ⏳ não medido — o HTML é o mesmo, mas *não presumir* |
| **WebGPU disponível** | ✅ `navigator.gpu = true` | mesmo browser |
| **fps (Canvas2D, 10k pts)** | **3 fps** | ⏳ não medido (mesma abordagem, mesmo resultado esperado) |
| **Leitor de tela real** | ❌ **só você** (VoiceOver/NVDA) | ❌ **só você** |
| **Teclado interativo** | ⚠️ estrutura OK (`<button>` nativo); input sintético deu timeout | ❌ só você |
| **DX subjetiva** | ❌ **só você** | ❌ **só você** |

¹ As duas correções foram: `HtmlElement<Canvas>` não converte por `.into()` para
`web_sys::HtmlCanvasElement` (precisa de `Deref`), e `set_fill_style` está depreciado
(`set_fill_style_str`). **Ressalva de justiça:** esse código foi escrito às cegas pelo agente,
sem poder compilar; um dev com a documentação aberta resolveria em minutos. O sinal é fraco.

² Comparação imperfeita: `vite build` é bundle de produção, `cargo build` é wasm em debug.
Serve para ordem de grandeza, não para um número final.

### 🔴 O achado que muda o desenho: **3 fps**

O critério da ADR-0016 é **≥ 30 fps**. O preview com **Canvas2D e 10k `fillRect` individuais
entrega 3 fps** — reprovado por uma ordem de grandeza. Isso é independente de framework (as
duas stacks usariam o mesmo canvas) e **prova empiricamente** o que o ADR-0015 assumia por
teoria: o preview **precisa** ser WebGPU/instanced. Um preview ingênuo não é viável.

> Ressalva: medido no painel de navegador do agente, que pode estar throttled; e o protótipo
> **não** implementa o caminho WebGPU (só verifica que ele existe). O número real do WebGPU
> continua por medir.

### O que isto ainda NÃO decide

Os dois eixos que a ADR-0016 trata como decisivos — **experiência com leitor de tela** e
**DX** — continuam sendo julgamento humano. O agente não os carimbou. A a11y *estrutural* do
desenho está provada (0 violações); a *experiência* não.

## Como rodar

### React/Vite (`spike/react/`) — roda com o Node deste ambiente
```sh
cd spike/react
npm install
npm run dev        # servidor de dev (abre no browser p/ a11y/preview)
npm run build      # build de produção (mede tempo + bundle)
```

### Leptos / Rust→WASM (`spike/leptos/`) — exige toolchain que NÃO está neste ambiente
```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
cd spike/leptos
trunk serve        # dev  (abre no browser)
trunk build --release   # build de produção
```
> ⚠️ **Não builda no ambiente do agente:** faltam `trunk`, `wasm-pack` e o target
> `wasm32-unknown-unknown` (verificado: ausentes). Rode na sua máquina.

---

## Checklist de medição — preencha a tabela abaixo

Rode **o mesmo roteiro** nos dois protótipos e registre.

| Eixo | Como medir | React/Vite | Leptos |
|---|---|---|---|
| **A11y — axe** | DevTools → axe extension (ou `@axe-core/cli`) na tela → nº de violações **críticas/sérias** | | |
| **A11y — Lighthouse** | DevTools → Lighthouse → Accessibility score (0–100) | | |
| **A11y — leitor de tela** | VoiceOver (mac) / NVDA (win): o status muda de Ok→Warning→Critical é **anunciado** via live-region? (sim/não) | | |
| **A11y — teclado** | Navegar a tela inteira **só com teclado**, foco visível, sem trap? (sim/não) | | |
| **Preview — fps** | Cena de ~10k pontos rodando; medir fps (overlay do protótipo) | | |
| **Preview — fallback** | Desabilitar WebGPU (ou GPU ausente) → cai para 2D com aviso? (sim/não) | | |
| **Build a frio** | `rm -rf node_modules/.vite target` e medir `build` do zero | | |
| **Build a quente** | Rodar `build` de novo (cache) e medir | | |
| **Bundle final** | Tamanho do output de produção (KB) | | |
| **DX (subjetivo)** | Tempo p/ adicionar 1 campo ao painel + 1 estado; conforto do ecossistema. **Só você decide.** | | |

### Critério de passa/reprova (da ADR-0016)
- A11y: **0 violações críticas** no axe; Lighthouse ≥ 90; leitor de tela anuncia status.
- Preview: **≥ 30 fps** a 10k pontos; fallback 2D funciona.
- Build/DX: registrados (sem corte duro — entram no julgamento).

## Saída do spike
Preencha a tabela → veredito (Leptos **ou** React/Svelte) → **promova a ADR-0016 de
"provisório" para "aceito"** com a stack vencedora, e então o PR-05 (scaffold da shell) fica
desbloqueado.

## Shape do read-model (fonte de verdade: `crates/led-readmodel/src/lib.rs`)
```json
{ "health": "ok|warning|critical",
  "devices": [ { "id": 0, "connected": true, "frames_sent": 42, "last_send_ms": 0 } ],
  "metrics": { "frames": 100, "drops": 3, "beats": 5, "p50_us": 120, "p99_us": 4100 },
  "discovery": null }
```
Ambos os protótipos usam exatamente este shape (fixture em cada `readmodel`), para o painel
ser comparável.
