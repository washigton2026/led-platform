# ADR-0016 — Stack do console de operador

- **Status:** 🟢 **aceito** (2026-08-09) — promovido de "provisório" pelo spike + medições de F2
- **Data original:** 2026-07-26 · **Decidido:** 2026-08-09
- **Anexo de evidência:** [0016-anexo-evidencia-e-matriz.md](0016-anexo-evidencia-e-matriz.md)
- **Spike:** [`spike/`](../../spike/) — **continua descartável e não entra no baseline**

> **Nota sobre o nome do ficheiro.** Continua `…-provisorio.md` de propósito: nove documentos
> ligam para ele, e renomear partiria essas ligações sem ganhar nada. O nome é histórico; o
> estado é o do cabeçalho.

---

## CONTEXT

O console precisa de **acessibilidade WCAG 2.2 AA** (teclado completo, leitor de tela) e de um
**preview de pixels** com milhares de pontos. Toolkits Rust nativos (egui/Iced) têm a11y
imatura; o **DOM** tem a melhor história de acessibilidade; o preview exige **GPU**.

A direção *web DOM + WebGPU* foi fixada em 2026-07-26 e **não é o que este ADR decide**. O que
ficou em aberto — e é o que se fecha aqui — foi a **subdecisão Leptos vs TypeScript/React**.

O ADR ficou bloqueado nove meses por uma razão registada em §3.2 do anexo: *"decidir agora
seria decidir contra o lado que não teve chance de ser medido"*. O Leptos não fora medido em
axe, bundle nem build **porque o ambiente do agente não tinha `trunk`, `wasm-pack` nem o
target `wasm32-unknown-unknown`**.

**Em 2026-08-09 essa condição deixou de valer:** `trunk 0.21.14`, `wasm32-unknown-unknown`,
node v26.3.0 e npm 11.16.0 estão presentes e foram verificados. A assimetria que bloqueava o
ADR foi **eliminada por medição**, não por argumento.

## OPTIONS

- **A — React + TypeScript** (Vite, DOM, WebGPU)
- **B — Leptos** (Rust→WASM, DOM, WebGPU)

Descartados anteriormente e **não reabertos**: Iced e egui (a11y insuficiente), Electron
(footprint), UI nativa por-OS (custo × 3 + a11y fragmentada).

## EVALUATION CRITERIA

Os do plano do spike: acessibilidade · build/dist · preview · produtividade · isolamento.
Mais os eixos de manutenção: maturidade do ecossistema, componentes existentes, risco de
manutenção a longo prazo, e **ergonomia do contrato Rust↔frontend**.

Classificação herdada do anexo: **[M]** medido neste repositório · **[V]** facto público
verificável · **[J]** juízo · **[H]** pendente de medição humana. **Só [M] e [V] sustentam
esta decisão**; nenhum [J] foi usado como prova.

## MEASURED EVIDENCE

Medido em 2026-08-09, nesta máquina, com os dois protótipos a compilar e a servir.

| Eixo | React + TS | Leptos | Veredito |
|---|---|---|---|
| **axe-core 4.12.1 — violações** | **0** (regime estável, 4 corridas) | **0** | **EMPATE** |
| axe — regras aprovadas | 37 | 35 | **não comparável** (ver 1) |
| axe — incomplete | `color-contrast`:1 | — | — |
| **Bundle da app (gzip)** | **47,82 kB** (147,32 kB raw) | **84,72 kB** (277,3 kB raw) | React **1,77×** menor |
| Bundle como está no spike (gzip) | 211,89 kB (axe embutido) | 84,72 kB + axe em ficheiro à parte (153 kB) | ver 2 |
| **Build morno, release** | **4,27 s** (vite) · 8,49 s de parede | **17,37 s** (trunk) | React **~4×** |
| `led-console-bin` → wasm32 | n/a | **FALHA** — `mio`←`tokio`←`led-protocols` | ver 3 |
| `truth.rs` isolado → wasm32 | n/a | **COMPILA**, 2,46 s, zero deps | ver 3 |
| WebGPU disponível | `navigator.gpu` = true | mesmo browser | **não diferencia** |
| Deps transitivas | 115 | 179 | Leptos traz mais |
| Canvas2D, 10k pontos | 3 fps | mesma abordagem | **reprova nos dois** |

### 1. As regras aprovadas (37 vs 35) **não são comparáveis** — e o spike diz o contrário

O `spike/README.md` afirma que *"os dois protótipos implementam a mesma tela mínima"*.
**Não implementam.** Verificado no código: o React tem quatro secções (Controladores,
**Métricas**, Preview, Acessibilidade); o Leptos tem duas em Rust (Controladores, Preview)
mais o relatório de a11y injetado pelo `index.html`. O React tem **mais elementos**, logo mais
regras aplicáveis. É isso que explica 37 vs 35 — não qualidade de a11y.

**As contagens de violações (0 vs 0) continuam comparáveis** e são o que importa.

### 2. O "0 violações" reproduz-se — mas só em **regime estável**

A corrida de axe que a **própria página React** faz na montagem reporta **1 violação séria,
`color-contrast` × 16 nós**. Quatro corridas minhas, depois de o render assentar, reportam
**0 violações, 37 aprovadas, 1 incomplete**. As duas medições são verdadeiras em **momentos
diferentes**.

**Consequência operacional:** uma verificação contínua de a11y que corra na montagem reporta
violações que o regime estável não tem. E a regra frágil é precisamente `color-contrast` — que
é propriedade dos **tokens de cor**, não do framework. Está tratado nos tokens (ver
CONSEQUENCES).

### 3. A partilha de tipos é **real, mas condicional** — e as duas metades foram medidas

- `cargo build -p led-console-bin --target wasm32-unknown-unknown` **falha**, em `mio`, que
  entra por `tokio` ← `led-protocols` ← `led-daemon-bin` ← `led-console-bin`.
- O `truth.rs` — o ficheiro que contém os **nove estados** e os **cinco elos** — **compila
  para wasm32 sozinho, com zero dependências**, em 2,46 s.

Ou seja: a vantagem única do Leptos exigiria extrair um crate leaf (`led-console-model`), e
essa extração está **provada barata**, não suposta. O facto não foi inflacionado nem
descartado: entra na decisão pelo seu valor real.

## NOT_MEASURED

Nenhum número foi inventado para nenhuma destas linhas.

| # | Eixo | Porquê |
|---|---|---|
| H1 | VoiceOver (macOS) anuncia a mudança de estado? | **exige humano + leitor de tela** |
| H2 | NVDA (Windows) idem | **exige humano + Windows** |
| H4 | O foco é percetível em todos os controlos? | **exige juízo visual humano** |
| H5 | A live-region anuncia sem interromper nem repetir? | **exige leitor de tela** |
| H6 | DX / que ecossistema manter durante anos | **juízo do responsável** — reservado por §4.1 |
| — | **fps real em WebGPU a 10k pontos** | **nenhum dos protótipos implementa o caminho WebGPU**; só se verificou que `navigator.gpu` existe |
| — | Build a frio, nos dois | só houve comparação morno-vs-morno; o `target/` do Leptos tem 1,2 GB pré-aquecidos |
| — | Lighthouse (score de a11y) | não corrido |

**H1–H5 deixaram de diferenciar as stacks.** Os dois protótipos dão **0 violações** sobre HTML
equivalente, o que confirma pela medição o que o §3.3 do anexo já argumentava: **a11y é
propriedade do HTML emitido e do trabalho de implementação, não do framework**. Continuam
**obrigatórios como critério de aceitação da F3**, sobre a stack que for escolhida — mas não
são input desta decisão. A lista de bloqueadores humanos do ADR caiu de **seis para um**.

E o um que restava, **H6**, foi respondido pelo responsável em 2026-08-09.

## DECISION

**React + TypeScript.**

Com uma obrigação inseparável, sem a qual esta decisão não é válida: **o frontend não pode
conter nenhum enum escrito à mão que espelhe `EstadoUi` ou `Elo`.** Os tipos são **gerados** a
partir do Rust, e um teste reprova a CI se divergirem (ver CONSEQUENCES).

## RATIONALE

1. **Todos os eixos medidos empatam ou favorecem o React.** A a11y empata. O WebGPU empata e
   **sai da matriz**. Bundle e build favorecem o React — com a ressalva honesta de que o
   **bundle pesa pouco aqui**: o console é *loopback-only*, de um só operador, numa só
   máquina; não há CDN, rede móvel nem SEO. 4 s contra 17 s é real, mas os dois são aceitáveis.
2. **O que falta construir é exatamente onde o ecossistema paga.** Tabelas densas e
   virtualizadas, uma timeline, um log de eventos e grelhas de dados. O React tem componentes
   maduros para todos; em Leptos seriam escritos à mão. **[V]**
3. **Leptos 0.6 é major-zero, com quebras de API documentadas entre versões**, para um console
   que deve sobreviver a várias temporadas de show. **[V]**
4. **A vantagem única do Leptos é alcançável no React com máquina que este repo já constrói
   bem.** O precedente está em árvore: `os_limites_sao_os_do_gs3_e_nao_copias` fixa
   `MAX_BODY == led_daemon_bin::server::MAX_LINE` através de uma fronteira de crate. O mesmo
   formato de gate fixa a união TS contra `EstadoUi::as_str()`. **[M]** para o precedente.

## TRADE-OFFS

**O que se perde, dito sem atenuação:**

- Um **segundo toolchain** entra num repo e numa CI que eram 100 % Rust — Node, um segundo
  lockfile, um segundo cache, e uma segunda superfície de auditoria de dependências. É
  permanente e não é grátis.
- A partilha de tipos deixa de ser **por construção** e passa a ser **por gate**. Um gate pode
  falhar; a compilação não falha em silêncio. Este repo já foi mordido duas vezes por gates
  que passaram sem correr (KB-012, e o harness de injeção da sessão F1-B). **Por isso o gate
  de contrato nasce com falsificação obrigatória.**
- O argumento "Rust puro" do ADR original é **abandonado explicitamente** para o console. A
  fronteira UI↔engine já é **processo**, não linguagem (ADR-0013/0014), o que neutraliza a
  maior parte do custo — mas não todo.

## NON-GOALS

- **Não decide o empacotamento** (webview do SO vs browser). Fica em aberto.
- **Não decide a biblioteca de componentes**, nem CSS framework, nem gestor de estado.
- **Não reabre** a direção web DOM + WebGPU, nem a rejeição de Iced/egui/Electron.
- **Não autoriza** UI construída: a F2 não implementa interface (ver `CLAUDE.md`, F2).
- **Não altera** o `led-daemon`, o `led-core` nem o IPC v1.

## CONSEQUENCES

1. **Gate de contrato, obrigatório e falsificável.** A união TypeScript de estados é gerada de
   `EstadoUi::as_str()` e de `Elo::as_str()`. Um teste compara os membros gerados com os do
   Rust e **reprova a CI** se divergirem. **Tem de ser falsificado ao nascer** — plantar um
   membro a mais e confirmar que reprova — pela regra do KB-012.
2. **Sem enum paralelo.** Nenhum `.ts` declara `"PASS" | "FAIL" | …` à mão. As transformações
   no frontend são **só de apresentação**: formatar, ordenar, agrupar. Nunca derivar estado.
3. **CI ganha um passo Node**, em `ubuntu-latest` e `macos-latest` (os dois bloqueantes).
4. **`color-contrast` é obrigação dos tokens.** Os rácios são **calculados**, não afirmados; o
   token `border` decorativo (1,46) **nunca** pode ser o único portador de significado, e há um
   `border-strong` (3,77 sobre o fundo, 3,49 sobre a superfície) para as fronteiras que
   significam. Verificação contínua de a11y corre **depois do render assentar**, nunca na
   montagem.
5. **O preview será WebGPU/instanced.** Não é preferência: 3 fps contra um critério de 30 é
   reprovação por ordem de grandeza, medida — e **independente da stack**.
6. **`led-console-model` continua útil**, embora deixe de ser obrigatório: dá à geração de
   tipos uma fonte única e isolada. Fica como recomendação, não como requisito.

## FUTURE EXIT CONDITIONS

Esta decisão deve ser **revisitada** se qualquer uma se verificar:

1. **O gate de contrato falhar em produção** — isto é, chegar a divergir `EstadoUi` do TS sem
   a CI reprovar. Seria a prova de que "por gate" não substitui "por construção", e o
   argumento do Leptos passaria a ganhar.
2. **O segundo toolchain custar mais do que o ecossistema poupa** — medível: tempo de CI,
   incidentes de supply-chain, tempo gasto em manutenção de build Node.
3. **Leptos atingir 1.0** com ecossistema de tabelas/timeline comparável. O eixo [V] que mais
   pesou aqui deixaria de valer.
4. **O empacotamento exigir binário único sem runtime web** — mudaria o problema, não a
   resposta a este problema.

Reverter **não é grátis**: implicaria reescrever a UI. O momento barato para reverter é
**antes de a F3 começar**.
