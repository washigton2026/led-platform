# ADR-0016 — Anexo: evidência levantada e matriz comparativa (B2)

> **Leia isto primeiro.** O corpo deste anexo (§1–§5) foi escrito a **2026-08-05**, *antes*
> da decisão, e está preservado tal como estava — incluindo as frases que dizem que nada foi
> escolhido. **A decisão foi tomada a 2026-08-09: React + TypeScript**
> ([ADR-0016](0016-stack-console-provisorio.md)). A **§6** acrescenta medições posteriores que
> fecham lacunas do levantamento e **não reabrem** a decisão.

**Data do levantamento:** 2026-08-05 · HEAD `a336f03` · **Nenhum código alterado, nenhuma
stack escolhida** *(verdade nessa data)*. · **Adenda:** §6, 2026-08-15, HEAD `1bac5d3`.

Este anexo existe para que a decisão do [ADR-0016](0016-stack-console-provisorio.md) seja
tomada sobre **evidência separada de opinião**. Ele **não decide** — a decisão é do
responsável, e §4 lista o que faltava para tomá-la.

## Como ler as classificações

Cada afirmação abaixo carrega a sua origem. Isto não é formalidade: metade das linhas de uma
comparação de stacks costuma ser folclore, e misturar folclore com medição é o que produz
decisão mal fundamentada.

| Classe | Significado |
|---|---|
| **[M]** | **Medido neste repositório**, com artefato citável |
| **[V]** | **Verificável e não controverso** — fato público de baixa disputa (ex.: "React tem mais componentes prontos") |
| **[J]** | **Juízo** — depende de contexto, preferência ou experiência. **Não é evidência** |
| **[H]** | **Pendente de medição humana** — não mensurável pelo agente |

---

## 1. Evidência medida no repositório

Fonte primária: [`spike/README.md`](../../spike/README.md), executado em 2026-07-30, mais
medições novas de 2026-08-05 registadas aqui.

### 1.1 O que o spike executou de facto

| Eixo | React/Vite | Leptos/WASM | Classe |
|---|---|---|---|
| Compila de primeira | ✅ sim | ❌ 2 correções necessárias¹ | [M] |
| Build | `vite build` **1,64 s** | `cargo build --target wasm32` **44,98 s** (debug)² | [M] |
| Bundle | **47 kB** gzip | ⏳ não medido (exige `trunk`) | [M] / ⏳ |
| **axe-core 4.12.1** | ✅ **0 violações · 37 regras aprovadas** | ⏳ não medido | [M] / ⏳ |
| Árvore de a11y | ✅ `main`, `status`+`aria-live=polite`, `region`+headings, `table`+caption | ⏳ não medido — o HTML é o mesmo, **mas não presumir** | [M] / ⏳ |
| WebGPU disponível | ✅ `navigator.gpu = true` | mesmo browser | [M] |
| **fps (Canvas2D, 10k pts)** | **3 fps** | não medido (mesma abordagem) | [M] |

¹ `HtmlElement<Canvas>` não converte por `.into()` (precisa de `Deref`) e `set_fill_style`
está depreciado. **Ressalva de justiça registada no próprio spike:** o código foi escrito às
cegas, sem poder compilar; um humano com a documentação aberta resolveria em minutos.
**O sinal é fraco e não deve pesar na decisão.**

² Comparação imperfeita e assim declarada: `vite build` é bundle de produção, `cargo build` é
wasm em **debug**. Serve para ordem de grandeza, não para número final.

### 1.2 Medições novas (2026-08-05)

| Métrica | React/Vite | Leptos | Classe |
|---|---|---|---|
| Pacotes de topo | **40** | — | [M] |
| **Dependências transitivas** | **115** (`package-lock.json`) | **179** (`Cargo.lock`) | [M] |
| Peso em disco | 74 MB (`node_modules`) | 1,2 GB (`target/`) | [M] |

> ⚠️ **Caveat obrigatório:** as duas contagens **não são estritamente comparáveis**.
> `Cargo.lock` inclui build-deps e o grafo completo; o `package-lock` cobre prod+dev deste
> projeto. E `node_modules` (fontes) contra `target/` (artefatos de build) é comparação
> injusta em disco. **O que o número suporta:** a intuição de que "Rust puro ⇒ menos
> dependências" **não se confirma aqui** — Leptos traz *mais* dependências transitivas que
> React+Vite+TS+axe. **O que não suporta:** nenhuma conclusão sobre risco de supply-chain,
> que depende de quais deps são, não de quantas.

### 1.3 Estado do repositório que a decisão afeta

| Fato | Valor | Classe |
|---|---|---|
| Produto hoje | **100 % Rust** — zero `.ts`/`.tsx`/`.js` fora de `spike/` | [M] |
| CI hoje | **100 % Rust** — nenhum `setup-node`, nenhum passo npm | [M] |
| OS bloqueantes na CI | `ubuntu-latest` + `macos-latest`; Windows não-bloqueante | [M] |
| Convenção do repo | *"Std-only where possible; add a dependency only with a reason"* (`CLAUDE.md`) | [M] |

**Consequência factual, não opinião:** escolher React/TS adiciona um **segundo toolchain** ao
projeto e à CI (Node, npm, um lockfile próprio, um cache próprio, e uma segunda superfície de
`cargo audit`-equivalente). Escolher Leptos adiciona `trunk` + target `wasm32` — também um
passo novo de CI, mas dentro do ecossistema já existente. **Nenhuma das duas é grátis.**

### 1.4 🔴 O achado que já mudou o desenho — e que **não** depende da stack

O critério do ADR-0016 é **≥ 30 fps** a 10k pontos. O preview com **Canvas2D e 10k
`fillRect` individuais entrega 3 fps** — reprovado por **uma ordem de grandeza**.

Isto é **independente de framework** (as duas stacks usariam o mesmo canvas) e prova
empiricamente o que o ADR-0015 assumia por teoria: **o preview precisa ser WebGPU/instanced;
um preview ingénuo não é viável.**

> Ressalva do próprio spike: medido no painel de navegador do agente, possivelmente
> *throttled*; e o protótipo **não** implementa o caminho WebGPU (só verifica que existe).
> **O número real do WebGPU continua por medir.**

**Este achado remove um eixo da comparação:** WebGPU não diferencia as stacks — é requisito
das duas.

---

## 2. Matriz comparativa

Os treze eixos pedidos. **A coluna "Classe" é a parte que importa:** só as linhas **[M]**
sustentam decisão sozinhas.

| # | Eixo | React + TypeScript | Leptos (Rust→WASM) | Classe |
|---|---|---|---|---|
| 1 | **Produtividade** | build 1,64 s; compilou de primeira; ciclo de edição rápido | 44,98 s (debug/wasm); 2 correções no 1.º build | **[M]** parcial — mas o tempo p/ "1 painel + 1 comando" do plano do spike **não foi cronometrado nos dois** |
| 2 | **Curva de aprendizado** | exige TS/JSX/ecossistema npm — **novo para este projeto** | exige Rust (já dominado) + WASM/`web-sys` (novo) | **[J]** — depende de quem mantém |
| 3 | **Manutenção a longo prazo** | 2.º toolchain, 2.º lockfile, churn conhecido do ecossistema | 1 toolchain; Leptos é jovem e teve quebras entre versões | **[J]** com base **[M]** (§1.3) |
| 4 | **Integração com Rust** | via IPC apenas (ADR-0014) — **a fronteira já é processo, não linguagem** | nativa; tipos partilháveis com o daemon | **[M]** para a fronteira; **[J]** para o benefício |
| 5 | **WebGPU** | `navigator.gpu` disponível; **3 fps em Canvas2D reprova o caminho ingénuo** | idêntico — mesmo browser, mesma API | **[M]** — **não diferencia** |
| 6 | **Acessibilidade** | **0 violações axe, 37 regras aprovadas**, árvore verificada | ⏳ **não medido** — HTML equivalente, mas não presumir | **[M]** vs ⏳ |
| 7 | **Componentes existentes** | ecossistema muito maior (grades, timelines, date pickers) | escasso; a maior parte teria de ser escrita | **[V]** |
| 8 | **Maturidade** | React 18 amplamente usado em produção há anos | Leptos 0.6 — **major zero**, API ainda em evolução | **[V]** |
| 9 | **Comunidade** | ordens de grandeza maior | pequena, ativa, focada | **[V]** |
| 10 | **Tooling** | Vite/TS/axe/Lighthouse maduros e usados no spike | `trunk`+`wasm-pack` **não estavam no ambiente**; exigem instalação | **[M]** (ausência verificada) + **[V]** |
| 11 | **Hot reload** | Vite HMR — padrão do ecossistema | `trunk serve` recarrega; **não exercitado no spike** | **[V]** / ⏳ |
| 12 | **Testes** | ecossistema grande, mas **fora** do `cargo test` do repo | `wasm-bindgen-test` integra com `cargo`; menos maduro para UI | **[J]** com base **[M]** (a suite hoje é 100 % `cargo test`) |
| 13 | **Documentação** | vasta | boa para o núcleo; escassa nas bordas (`web-sys`, WebGPU) | **[V]** |

### 2.1 O que a matriz mostra quando se olha só para **[M]**

Restam **quatro** linhas com evidência medida, e elas dizem o seguinte:

1. **A11y estrutural:** React **provado** (0 violações); Leptos **não medido**. Não é
   "React ganha" — é "um foi medido e o outro não". A assimetria é de *evidência*, não
   necessariamente de *mérito*.
2. **Build:** React ~27× mais rápido no ciclo medido, com a ressalva de que a comparação é
   release-vs-debug.
3. **WebGPU:** **não diferencia**. Sai da decisão.
4. **Dependências e toolchain:** Leptos traz *mais* deps transitivas; React traz um *segundo
   ecossistema*. Trade-off real, sem vencedor factual.

---

## 3. Recomendação baseada apenas em fatos

**Não é uma escolha de stack** — o passo pediu explicitamente que eu não escolhesse, e a
evidência também não autoriza escolher. O que os fatos sustentam:

### 3.1 O que a evidência **já decide** (independente da stack)

- **O preview será WebGPU/instanced.** Não é preferência: 3 fps contra um critério de 30 fps
  é reprovação por ordem de grandeza, medida. Um preview em Canvas2D ingénuo **está fora**,
  em qualquer stack.
- **WebGPU sai da matriz de decisão.** É requisito comum.
- **A fronteira UI↔engine é processo, não linguagem** (ADR-0013/0014). Portanto "integração
  com Rust" pesa **menos** do que a intuição sugere: a UI fala IPC tipado com o daemon nos
  dois casos. Este é o ponto que mais frequentemente é sobrevalorizado numa decisão destas, e
  a arquitetura já o neutralizou.

### 3.2 O que a evidência **não** decide, e por quê

A comparação está **assimétrica por omissão**: o Leptos **não foi medido** em a11y, bundle,
fps nem hot reload — não porque falhou, mas porque **o ambiente do agente não tinha `trunk`,
`wasm-pack` nem o target `wasm32`**. Decidir agora seria decidir contra o lado que não teve
chance de ser medido.

**Recomendação de processo, não de produto:** antes de escolher, **completar a coluna do
Leptos** nos eixos que já foram medidos no React (axe, bundle, build a frio/quente). São
horas de trabalho, não dias, e eliminam a assimetria. Sem isso, qualquer escolha carrega um
viés conhecido e evitável.

### 3.3 Um risco que a matriz não captura

O critério do ADR-0016 é **WCAG 2.2 AA**, e o repo trata isso como requisito, não aspiração.
**A11y não é propriedade do framework — é propriedade do HTML gerado e do trabalho de
implementação.** Os dois protótipos emitem HTML equivalente. Portanto:

- O 0-violações do React **não prova** que React entrega a11y — prova que **este desenho de
  tela** entrega a11y.
- O ⏳ do Leptos **não indica** problema de a11y — indica ausência de medição.

O fator real de risco é **quem mantém a a11y ao longo do tempo** e com que ferramentas de
verificação contínua. Isso é [J], e é do responsável.

---

## 4. Medições humanas pendentes — o que falta para fechar o ADR-0016

Estas **não são mensuráveis pelo agente**: exigem browser real, GPU real, leitor de tela
real e julgamento. Nenhum número foi inventado para nenhuma delas.

### 4.1 Bloqueantes (o ADR não fecha sem)

| # | Medição | Como | React | Leptos |
|---|---|---|---|---|
| H1 | **VoiceOver** (macOS) — a mudança `Ok→Warning→Critical` é **anunciada** pela live-region? | ativar VoiceOver, alterar o status, ouvir | ⏳ | ⏳ |
| H2 | **NVDA** (Windows) — idem | idem em Windows | ⏳ | ⏳ |
| H3 | **Navegação por teclado** — percorrer a tela inteira só com teclado | Tab/Shift-Tab por toda a UI | ⚠️ estrutura OK (`<button>` nativo); **input sintético deu timeout** — não conta como medido | ⏳ |
| H4 | **Foco visível** — o indicador de foco é percetível em todos os controlos? | inspeção visual sob teclado | ⏳ | ⏳ |
| H5 | **Live regions** — `aria-live=polite` anuncia sem interromper, e sem repetir em excesso? | com leitor de tela ativo | árvore verificada, **anúncio não** | ⏳ |
| H6 | **DX subjetiva** — qual ecossistema você quer manter pelos próximos anos? | julgamento | ⏳ | ⏳ |

> **H1–H5 são a mesma pergunta vista de cinco ângulos:** a a11y *estrutural* está provada no
> React (axe, 37 regras); a a11y *experienciada* não está provada em nenhum dos dois. axe
> verifica regras estáticas — **não** verifica se um operador cego consegue conduzir um show.

### 4.2 Assimetrias a eliminar antes de decidir (recomendado, não bloqueante)

| # | Medição | Porquê |
|---|---|---|
| A1 | axe-core no Leptos | o React tem 0 violações; o Leptos **não tem número nenhum** |
| A2 | Bundle do Leptos (`trunk build --release`) | comparar com os 47 kB do React |
| A3 | Build a frio e a quente, nos dois | o 1,64 s × 44,98 s compara release com debug |
| A4 | **fps real em WebGPU**, nos dois | o 3 fps é do caminho Canvas2D, que já está descartado. **O número que interessa não existe ainda** |
| A5 | Hot reload do `trunk serve` | não exercitado |

### 4.3 O que **não** precisa de mais medição

- **WebGPU vs Canvas2D** — decidido pelos 3 fps. Não repetir.
- **Iced/egui para o console** — já descartados pelo ADR-0016 por a11y.
- **Electron** — descartado por footprint.

---

## 5. Estado do ADR-0016 após este anexo *(registo de 2026-08-05 — ver §6)*

> **Histórico.** O que esta secção diz era verdade na data em que foi escrita. O ADR-0016 foi
> **aceito a 2026-08-09** (React + TypeScript). Mantém-se aqui por inteiro porque um anexo de
> evidência que reescreve o seu próprio passado deixa de servir para auditar a decisão.

**Continua `proposto (provisório)`.** Este anexo não o promove.

Para promovê-lo a `aceito` é preciso: **H1–H6 respondidas** (e, idealmente, A1–A5 medidas),
preencher a tabela do `spike/README.md`, e registar o veredito no corpo do ADR-0016 com a
stack vencedora. Só então o **PR-05 (scaffold da shell)** e a **FASE D** ficam desbloqueados.

**Nada aqui autoriza começar o console.**

---

## 6. Medições posteriores à decisão *(2026-08-15)*

> **Estas medições NÃO reabrem o ADR-0016.** A decisão — **React + TypeScript** — está aceita
> desde 2026-08-09 e o seu fundamento principal não é nenhum número abaixo: é que o frontend
> **não pode conter enums escritos à mão** que espelhem `EstadoUi`/`Elo`, e os tipos são
> gerados do Rust ([ADR-0027](0027-contrato-tipos-rust-typescript.md)). O que segue fecha
> `⏳ não medido` que ficaram em aberto em §1 e §4, para que o anexo deixe de ter buracos.

### 6.1 Condições de medição

| Condição | Valor |
|---|---|
| Data · HEAD | 2026-08-15 · `1bac5d3` |
| Máquina | Darwin 23.6.0 · x86_64 · Intel Core i5-8210Y @ 1,60 GHz · 4 núcleos · 8 GB |
| Toolchain | node v26.3.0 · npm 11.16.0 · trunk 0.21.14 · rustc 1.96.0 |
| Versões | react 18.3.1 · vite 5.4.21 · leptos 0.6.15 · axe-core 4.12.1 |
| Alimentação / estado térmico | **não registada** |
| Carga concorrente na máquina | **não registada** |
| Browser dos testes de a11y | painel de preview do agente; **versão não registada** |

**O que mudou desde julho, e é a razão de isto ser mensurável agora:** `trunk` e o target
`wasm32-unknown-unknown` **passaram a existir** nesta máquina. Em 2026-07-30 não existiam, e
foi por isso que a coluna do Leptos ficou com `⏳`.

### 6.2 Build e bundle

| Eixo | React/Vite | Leptos/WASM | Classe |
|---|---|---|---|
| Build a frio | **24,6 s** (`npm install` 12,5 + `vite build` 12,1) | **363,6 s** (`trunk build --release`, `target/` apagado) | [M] |
| Rebuild após 1 edição de fonte | **8,9 s** (`vite build`) | **10,6 s** (`trunk build` debug, cache quente) | [M] |
| Rebuild sem alteração nenhuma | não registada | **1,6 s** | [M] |
| Bundle da aplicação (gzip) | **45,7 kB** (142,5 kB bruto) | **84,0 kB** (275,0 kB bruto: wasm 243,9 + glue JS 31,1) | [M] |

**Três ressalvas sem as quais estes números enganam.**

1. **Os builds não são equivalentes.** `vite build` é bundle de produção; o rebuild do Leptos
   foi medido em **debug**, e o de frio em **release**. Serve para ordem de grandeza, não para
   um rácio final.
2. **O bundle React aparecia como 212 kB gzip.** Eram os **572 kB do `axe-core`**, que é
   harness de medição e não embarcaria. O valor da tabela é um baseline isolado
   (`react` + `react-dom` + app trivial). O do Leptos exclui o mesmo harness (`axe.min.js`).
3. **Para um console *loopback-only*, o tamanho do bundle quase não decide** — não há rede
   entre o servidor e o browser. Está registado por completude, não por peso.

### 6.3 Acessibilidade — o `⏳` do Leptos fecha

| Eixo | React/Vite | Leptos/WASM | Classe |
|---|---|---|---|
| axe-core, violações | **0** | **0** | [M] |
| axe-core, regras aprovadas | 37 | 35 | [M] |
| Viewport da medição | **não registada** (página curta, ~393 caracteres de texto) | **1280 × 2200** | — |

A diferença 37 vs 35 é do **conteúdo das páginas** (número de regras aplicáveis), não de
qualidade — os dois protótipos não renderizam exactamente o mesmo DOM.

**Assimetria declarada:** a medição do React correu numa viewport que não registei, sobre uma
página curta que cabia nela; a do Leptos correu a 1280×2200. **Não são condições idênticas**,
e a §6.4 explica porque é que isso importa mais do que parece.

### 6.4 ⚠️ O falso-positivo de contraste, e a condição que o eliminou

A **primeira** execução do axe sobre o protótipo Leptos deu **1 violação séria**:
`color-contrast` em **10 nós**, com o axe a reportar `fgColor #eeeeee` sobre
`bgColor #ffffff` — rácio 1,16:1.

**Era falso.** Os estilos computados no browser mostravam `<main>` com
`background-color: rgb(17,17,17)`, `color: rgb(238,238,238)`, e os nós acusados **dentro** de
`<main>` (cadeia verificada elemento a elemento). O contraste real é o mesmo do protótipo
React, que declara `color:#eee; background:#111` no mesmo sítio.

**Causa:** o `color-contrast` do axe resolve o fundo por ponto e **cai para branco** quando
não o consegue determinar — o que acontece com elementos **abaixo da dobra**. A página do
Leptos é mais alta que a do React (tem tabela + canvas), por isso só ela foi afectada.

**Condição que o elimina:** viewport alta o bastante para conter a página inteira. A
1280×2200 (altura da página = 2200 px), as violações passam a **0** e sobra 1 `incomplete`.

**Porque isto fica escrito:** sem esta nota, a próxima pessoa a medir a11y neste repositório
repete o erro e atribui à stack um defeito da janela. Um falso-**vermelho** contra uma stack é
tão caro quanto o falso-verde do KB-012 — e mais fácil de acreditar, porque parece rigor.

**Regra para futuras medições de a11y:** registar a viewport e garantir que ela contém a
página; ou percorrer a página em segmentos. Um número de contraste sem viewport declarada
**não é evidência**.

### 6.5 Os 3 fps continuam a ser do Canvas2D — não do React

O **3 fps** de §1.4 é resultado de **um desenho de preview** (Canvas2D com 10k `fillRect`
individuais), medido no protótipo React **porque era o único que corria na altura**. Ele
**não** é um resultado da stack React, e continua a **não diferenciar** as duas: as duas
usariam o mesmo canvas e a mesma API. A conclusão que ele suporta é uma só, e é a de §1.4 —
**o preview será WebGPU/instanced**.

**Continua por medir:** fps real em WebGPU (a assimetria **A4** de §4.2). Nenhum dos dois
protótipos implementa o caminho WebGPU; ambos apenas verificam que `navigator.gpu` existe.

### 6.6 O que estas medições mudam no ADR-0016

**Nada na decisão.** Fecham `⏳`:

- §1.1 — *"Bundle Leptos: precisa de `trunk`"* → **medido** (84,0 kB gzip).
- §1.1 — *"axe-core Leptos: não medido"* → **medido** (0 violações).
- §1.1 — *"Árvore de a11y Leptos: não medido — não presumir"* → **medido**, e o *"não
  presumir"* provou-se sábio: a primeira leitura foi um falso-positivo.
- §4.2 **A5** (hot reload do `trunk serve`) e **A1–A4** continuam **não medidos**.

**Continua a exigir julgamento humano e não foi medido:** leitor de ecrã real
(VoiceOver/NVDA), teclado interactivo com foco visível, e DX subjectiva. Ver §4.1.
