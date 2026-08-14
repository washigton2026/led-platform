# ADR-0028 — Topologia da Web Platform e a **fronteira de estado** que ela não pode atravessar

- **Status:** 🟢 aceito · **Data:** 2026-08-10
- **Depende de:** [ADR-0013](0013-engine-daemon-separado.md) (engine headless, UI é processo cliente) · [ADR-0016](0016-stack-console-provisorio.md) (React + TypeScript) · [ADR-0026](0026-console-daemon-boundary.md) (a fronteira console↔daemon) · [ADR-0027](0027-contrato-tipos-rust-typescript.md) (o contrato TS é gerado)
- **Escrito antes do código.** Nenhuma linha da Web Platform existe quando este ADR é aceite.

## Contexto e problema

A Web Platform **ainda não existe**. Não há `.tsx`, não há `package.json` de aplicação, não há
bundler. O que existe é a fronteira do lado do servidor, e um vocabulário de verdade que ainda
não tem quem o preencha.

Estado verificado no repositório (COMMAND 00), não presumido:

- **O console já serve HTTP e SSE.** `crates/led-console-bin/src/http.rs` tem `serve(addr, Config)`,
  onze rotas encaminhadas a partir da tabela `ROTAS` de `surface.rs`, SSE com fan-out e
  reconexão upstream — tudo exercitado por testes de integração contra o daemon real.
- **Mas `led-console-bin` é apenas uma *library*.** Não há `src/main.rs` nem `[[bin]]`; só os
  testes invocam `serve()`. **Nada lança o processo**, e por isso a garantia de isolamento do
  ADR-0013 ainda não é exercida — não está violada, simplesmente não é exercida.
- **O contrato TypeScript existe e é gerado** (ADR-0027): `contract/lumyx-contract.generated.ts`,
  com `EstadoUi`, `Elo`, `DaemonState`, `CodigoErro`, `ROTAS`, `Instantaneo<T>`, `Resposta`,
  `Evento`. É verificado por dois caminhos independentes e por `tsc --noEmit`.
- **`/api/state` devolve dado real, mas não descrito.** O handler repassa a linha do daemon
  verbatim (`http.rs`), cujos campos nascem do `Snapshot` publicado pelo laço
  (`crates/led-daemon-bin/src/server.rs`, `Cmd::Status`): `state`, `position_ms`, `duration_ms`,
  `ticks`, `show_id`. O contrato TS **não tem tipo para esse corpo** — só para o envelope.
- **O vocabulário de verdade não tem produtor.** `crates/led-console-bin/src/truth.rs` define
  `EstadoUi` (9), `Elo` (5), `Instantaneo<T>` e `Evidencia`, com construtores completos. Fora de
  testes, o **único** consumidor no repositório é `contract.rs` — e apenas para os *enumerar* ao
  gerar o `.ts`. **Nenhum código de produção os constrói**, e nenhuma rota os emite.
- **Node 22 é a única versão normativa declarada.** Está em `.github/workflows/ci.yml` (job
  `contract`, bloqueante). Não existe `.nvmrc` nem `engines`.

O problema desta fatia é, portanto, **decidir onde a Web Platform vive e o que lhe é permitido
afirmar** — antes de existir a primeira linha, e não depois.

## Decisão

### D1 · Topologia — `console-web/`, fora de `crates/`

A aplicação TypeScript vive em `console-web/`, no topo do repositório, com `package.json`
próprio e **sem npm workspaces** neste momento.

**Fora de `crates/`** porque `crates/` é o workspace Rust: o `Cargo.toml` da raiz lista os
membros explicitamente, e meter `node_modules/` e artefactos de bundler dentro de um crate
poluiria o que o `cargo` empacota. A fronteira que o ADR-0013 e o ADR-0026 estabelecem é de
**processo**, não de directório — colar a UI dentro do crate criaria acoplamento sem
contrapartida.

**Sem npm workspaces** porque não há topologia npm a preservar: o único `package.json` do
repositório é o do gate do contrato, que se declara explicitamente *"não é a UI, não é o
frontend"*. Introduzir workspaces agora seria resolver um problema que ainda não existe.

### D2 · Gate de TypeScript — dois projectos, um comando

`console-web/` tem o **seu** `tsconfig.json`, com `jsx` ligado. O projecto do contrato
(`crates/led-console-bin/contract/tsconfig.json`) **permanece isolado**, com o seu `include` de
dois ficheiros.

`scripts/tsc_gate.sh` passará a correr `tsc --noEmit` sobre **ambos**, e a CI continua
**bloqueante** — o job `contract` não é `continue-on-error`.

São dois projectos porque provam **propriedades diferentes**: o do contrato prova que os tipos
gerados compilam e que as suas asserções invertidas (`@ts-expect-error`) continuam a valer; o da
app prova que a UI os **usa** correctamente. Fundi-los obrigaria o projecto mais estrito a
aceitar `jsx`, e afrouxar um gate para acomodar o outro é o oposto do que se quer.

**Nenhum código da UI escapa ao typecheck:** o `include` da app cobre a sua árvore de fontes, e
o gate falha se **qualquer** dos projectos falhar.

### D3 · Fronteira de verdade — o vocabulário fica fora da Phase 1

`EstadoUi`, `Elo`, `Instantaneo<T>` e `Evidencia` **não serão usados como estado operacional da
UI** enquanto não existirem produtores reais.

Não é um juízo sobre a qualidade destes tipos — eles estão certos, e existem precisamente para
impedir que a interface minta. É um facto sobre o repositório: **ninguém os produz**. Usá-los na
Phase 1 obrigaria a inventar os valores, e uma interface que inventa `PASS` é exactamente o que
o ADR-0026 §9 (*"OBSERVABILITY ≠ PHYSICAL EVIDENCE"*) e §8 (*"a cadeia de evidência não
colapsa"*) existem para impedir.

**Nenhum endpoint será criado apenas para satisfazer a UI.** Primeiro prova-se que existe uma
fonte de verdade operacional; só depois se expõe.

### D4 · `/api/state` — o dado é válido; falta a descrição

O payload actual é **real e adequado**: os campos nascem do `Snapshot` que o laço publica, o
`state` é tipado na origem desde a F5, e `show_id` é `null` quando não há show — nunca `0`.
Daemon em baixo é **503** com `console.daemon_offline`, nunca um `200` com estado fabricado.

O que falta é **descrição tipada** no contrato TS. A correcção futura segue o caminho do
ADR-0027, e só esse:

```
Rust (fonte de verdade) → contrato TS gerado → frontend
```

**Nunca** `Rust → tipo TS escrito à mão`. Um tipo manual seria a segunda fonte de verdade que o
ADR-0026 §15 proíbe e que a obrigação inseparável do ADR-0016 exclui.

A alteração é **aditiva**: toca no gerador e no artefacto gerado. **Não** toca no IPC v1, no
`led-daemon` nem no `led-core` — o formato do fio não muda; passa apenas a estar descrito.

### D5 · Node — 22 é normativo

**Node 22 (LTS)** é a versão normativa para desenvolvimento e CI. É a única declarada em
qualquer sítio do repositório, e é a que bloqueia merges.

`.nvmrc` e `engines` são a forma de o tornar explícito, e são **implementação posterior** — não
desta fatia. Fica registado que existe hoje uma divergência conhecida entre a máquina de
desenvolvimento e a CI, e que ela passa a ser visível em vez de silenciosa.

### D6 · Processo do console — `led-console-bin/src/main.rs`

`crates/led-console-bin/src/main.rs` será o processo separado que lança `serve()`.

**O console não é embutido no daemon.** O ADR-0013 é explícito: *"o output em tempo real não
compartilha processo de falha com a UI"*, e o ADR-0026 §2 concretiza-o: *"um pânico no parser
HTTP mata o console, não o show"*. Essa garantia exige que o console **seja** um processo, e
hoje ele não é nenhum.

A `Config` que `serve()` recebe já é **dado injectado** — caminho do socket e endereço do
exporter; nada é descoberto. O binário é o invólucro que faltava, não desenho novo.

### D7 · Fronteira de segurança — mantém-se como está

Mantém-se: **loopback-only** (verificado antes do bind, em `limits.rs`), **sem LAN**, **sem
autenticação nova**, **sem CORS permissivo**, **sem `shutdown` por HTTP**.

O `ClientRegistry` continua vazio, e é isso que mantém o console loopback-only (ADR-0026 §10).
Isto é a **condição actual**, não uma promessa de autenticação futura. Qualquer decisão adicional
pertence à fase apropriada.

### D8 · `load` — duas acções nomeadas, nunca uma caixa

**A regra.** A afirmação de integridade é uma **acção com nome próprio**, e a acção que a faz
tem de **nomear a consequência operacional**. Nunca uma caixa de verificação.

- **"Carregar sem armar"** → `assume_integrity: false`
- **"Assumir integridade e armar"** → `assume_integrity: true`, **com confirmação explícita
  antes do envio**

**Porque uma caixa está errada aqui.** Lido no aplicador (`run.rs:360-399`), `assume_integrity`
faz **duas** coisas, não uma: afirma a integridade (`Integrity::AssumedByOperator`) **e**
dispara o pré-voo e o `Arm`. E o daemon **nunca verifica** — o GS2 decidiu-o por
impossibilidade técnica (*"`pixel_hash` exige o show inteiro em RAM; hash em fluxo não
existe"*), e `Integrity` é um `enum` e não um `bool` precisamente para que *"assumido"* e
*"verificado"* não fiquem indistinguíveis.

Uma caixa **pré-marcada** faria o operador afirmar integridade sem saber que a afirmou —
o colapso exacto que o `enum` existe para impedir, reintroduzido na última camada. Uma caixa
**desmarcada** produz um `load` que parece funcionar e um `play` seguinte que recusa com
`not_armed` sem nada no ecrã a explicar porquê. As duas falham, por razões opostas.

Duas acções distintas removem a ambiguidade **por construção**: não há estado intermédio onde o
operador possa ter afirmado algo sem reparar. É a mesma disciplina do `shutdown` em duas fases
do GS3 — *"não é segredo criptográfico; existe contra o engano"*.

### D9 · A matriz de estados **não** é replicada no browser

`load` só é aceite em `idle`; `unload` em tudo menos `idle` e `playing` (tabela dos 80 pares,
ADR-0023). A UI **não antecipa** nenhuma destas regras: envia o comando e mostra a **recusa
real** — `show_already_loaded`, `no_show_loaded`, `not_applicable`, `in_error_state` — com o
código do daemon verbatim.

Desactivar botões consoante o estado seria reimplementar 80 pares no frontend, e eles
divergiriam no dia em que a matriz mudasse: a segunda fonte de verdade que o ADR-0026 §15
proíbe. É a decisão que a superfície de transporte já tomou, estendida a `load`/`unload` sem
excepção — e o custo aceite é o mesmo: um clique que não se aplica devolve uma recusa em vez
de um botão cinzento.

**Não há catálogo de shows, e não se inventa um.** Zero rotas os listam. A entrada é o
**caminho**, e o daemon recusa o que não existir com `load_failed` — código que já está no
contrato gerado, com o `detail` a trazer o erro real do loader. Uma lista fabricada no console
seria a D3 outra vez, noutro campo.

## Operational Truth Boundary — regra normativa

A Web Platform **MUST NOT** inferir, derivar ou apresentar como facto:

- saúde de hardware;
- saúde de controlador;
- saúde de rede;
- certificação;
- evidência física;
- frescura de dados;
- sucesso de playback;

**quando esses estados não forem produzidos por uma fonte operacional real.**

`NOT_MEASURED` **não é** `PASS`.
Ausência de erro **não é** evidência de sucesso operacional.
Um `sendto` bem-sucedido **não é** prova de que um pixel acendeu.

As transformações no frontend são **só de apresentação** — formatar, ordenar, agrupar. **Nunca
derivar estado.**

## Alternativas consideradas

**D1 — app dentro de `crates/led-console-bin/`.** Rejeitada: poria `node_modules/` e artefactos
de bundler dentro de um crate Rust, e criaria dois `package.json` com propósitos diferentes sob
o mesmo tecto. Nenhum ADR a exige, e o acoplamento de directório sugeriria uma unidade que a
arquitectura não tem.

**D1 — outra topologia já prevista.** Procurada e **não encontrada**: nenhum ADR ou documento de
arquitectura fixa a localização da app.

**D2 — alargar o `include` do contrato para apanhar a app.** Rejeitada: acopla o gate do contrato
à UI e faz o `verifica.ts` deixar de ser um projecto isolado.

**D2 — project references.** Adiada: acrescenta complexidade de build incremental sem
necessidade demonstrada.

**D2 — um `tsconfig` único.** Rejeitada: obrigaria o projecto mais estrito a aceitar `jsx`.

**D3 — expor `EstadoUi`/`Elo` já, com valores derivados no console.** Rejeitada: derivar estado
no tradutor é precisamente a segunda fonte de verdade que o ADR-0026 §15 proíbe.

**D4 — escrever o tipo do `status` à mão no frontend.** Rejeitada pelo ADR-0016 e pelo ADR-0027.

**D5 — promover o Node local (26) a normativo.** Rejeitada: exigiria mudar a CI para uma versão
não-LTS sem evidência que o justifique.

**D6 — lançar o console a partir do `led-daemon-bin`.** Rejeitada: junta os domínios de falha e
contradiz o ADR-0013.

## Consequências

### Positivas

- O frontend **não polui** o workspace Rust.
- O contrato continua **fonte única de verdade**, e o caminho Rust → TS gerado → UI é o único.
- A UI **não pode fabricar estado**: o que não tem produtor não entra.
- O console mantém **isolamento de processo** (ADR-0013).
- O TypeScript da UI entra no **gate bloqueante**.
- A versão de Node deixa de ser implícita.

### Negativas

- **Dois projectos TypeScript** para manter.
- O **caminho de import** do contrato a partir de `console-web/` é mais longo, e terá de ser
  decidido (caminho relativo ou dependência de ficheiro).
- O **gerador de contrato terá de ser alterado** para descrever o corpo do `status`.
- O frontend **não poderá mostrar** estados que ainda não são produzidos — o que é honesto, mas
  torna a primeira interface deliberadamente magra.

## Não-goals

Este ADR **não** decide, e nada nele deve ser lido como decidindo:

design system · componentes · layout · dashboard · UI de hardware · UI de rede · UI de
certificação · shows · playback · timeline · studio · monitorização avançada · polimento
comercial · autenticação · exposição em LAN · F7.2 · IPC v1 · **a implementação do frontend**.

## Fronteira da F7.2

Este ADR **não altera, não corrige e não reinterpreta** nenhuma dívida da F7.2, e **não** trata
nenhuma delas como `PASS`.

Registado como estava:

- **macOS** — `absolute_pacing_on_schedule_reports_no_lateness`: fenómeno de escalonamento/pacing
  **sustentado experimentalmente** fora do repositório. **A reprodução exacta de 3/6 NÃO foi
  obtida**, e não deve ser afirmada.
- **Ubuntu** — `ddp_backend_send_path_is_alloc_free`: 4 alocações em 10 000 envios,
  não-determinístico, **causa por demonstrar**. Permanece **independente** do macOS.
- **Nenhuma correcção da F7.2 faz parte deste ADR.**

## Impacto de implementação (FUTURO — nada executado aqui)

Ordem de dependência, a executar **só após** aprovação de cada fatia:

1. `crates/led-console-bin/src/main.rs` + `[[bin]]`;
2. formalizar o tipo do corpo de `/api/state` no gerador;
3. regenerar `lumyx-contract.generated.ts`;
4. criar `console-web/`;
5. criar `console-web/tsconfig.json` com `jsx`;
6. ampliar `scripts/tsc_gate.sh` para os dois projectos, **falsificando** que reprova em cada um;
7. formalizar Node 22 (`.nvmrc`, `engines`);
8. integrar `console-web` no job `contract` da CI.

Nenhum destes passos é executado nesta fatia.

## Critério de reversão

Se alguma vez existir um produtor operacional real para `Evidencia`/`Instantaneo` — por exemplo,
quando o GS4.5 desbloquear e houver medição de hardware —, a **D3** deve ser revisitada: o
vocabulário passa a ter dono, e a fronteira move-se. Até lá, mantém-se.

Se o repositório adoptar npm workspaces por outra razão de peso, a **D1** deve ser reavaliada.
