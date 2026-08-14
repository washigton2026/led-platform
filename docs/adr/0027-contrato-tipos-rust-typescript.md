# ADR-0027 — O contrato TypeScript é **gerado** do Rust, e um gate prova que não divergiu

- **Status:** 🟢 aceito · **Data:** 2026-08-09
- **Depende de:** [ADR-0016](0016-stack-console-provisorio.md) (React/TypeScript) · [ADR-0026](0026-console-daemon-boundary.md) (a fronteira do console)
- **Escrito antes do código**, como a F3 exigiu.

## Contexto e problema

O ADR-0016 escolheu **React + TypeScript** e registou o custo dessa escolha sem o atenuar: em
Leptos os tipos seriam **partilhados por construção**; em TypeScript passam a ser mantidos
**por gate**. O próprio ADR-0016 tornou o gate condição de validade da decisão — *"nenhum enum
escrito à mão que espelhe `EstadoUi` ou `Elo`"*.

O que está em jogo não é conforto de tipagem. Os nove estados de [`truth.rs`](../../crates/led-console-bin/src/truth.rs)
e os cinco elos da cadeia de evidência **são** a semântica de segurança do produto: o ADR-0026
existe, em boa medida, para que a UI **não possa** afirmar `hardware_ok`. Uma cópia
TypeScript que ganhe vida própria tem um modo de falha muito concreto e muito caro:

> o backend diz `NOT_MEASURED` e o ecrã do operador diz `PASS`.

Sem contrato gerado, esse defeito **não é detetável por nenhum teste existente**: os testes do
Rust continuam verdes, os do frontend também, e o erro vive exatamente na costura entre os
dois — que é onde ninguém está a olhar.

## Estado inspecionado antes de decidir

Verificado, não presumido:

- **Não existe** nenhum mecanismo de geração ou schema no repositório: zero `serde`, zero
  `schemars`, zero `ts-rs`, zero `typeshare`. O JSON é escrito à mão (`to_json()`), por
  decisão do ADR-0026 e da convenção std-only.
- **Não existe** nenhum `.ts` fora de `spike/`.
- `Elo::ALL` e `State::ALL` existem e são enumeráveis.
- **`EstadoUi` não tinha `ALL`.**
- **`Rejected::code()` e `proto::code::*` não são enumeráveis programaticamente** — o primeiro
  é um `match` exaustivo, o segundo um conjunto de `const` soltas — e ambos vivem a montante
  (`led-daemon` está **congelado**; não é opção acrescentar-lhes nada).

## Decisão

### 1 · A fonte de verdade é o **Rust**, e o TypeScript é um **artefacto**

O ficheiro `crates/led-console-bin/contract/lumyx-contract.generated.ts` é **gerado** e está
**versionado**. É versionado de propósito: o frontend importa-o sem precisar de correr
`cargo`, e a revisão de um PR mostra a mudança de contrato como **diff legível**, em vez de a
esconder atrás de um passo de build.

O ficheiro declara-se gerado no cabeçalho e **não pode ser editado à mão** — o gate reprova.

### 2 · O contrato cobre só o que atravessa o fio

`EstadoUi` (9) · `Elo` (5, **ordenados** do mais fraco ao mais forte) · `State` do daemon (8) ·
os códigos de erro do protocolo (9) · os códigos de recusa do runtime (7) · a tabela `ROTAS`.

Não entram tipos internos. O contrato é a **fronteira**, não o modelo de domínio — pela mesma
razão que o `led-console-bin` transporta em vez de reimplementar (ADR-0026 §15).

### 3 · **Dois caminhos independentes**, porque um só seria falso-verde

Esta é a decisão que mais importa, e nasce do KB-012.

- **Caminho A — o gerador** usa os **valores Rust compilados**: `EstadoUi::ALL`, `Elo::ALL`,
  `State::ALL`, as `const` de `code`, e `ROTAS`.
- **Caminho B — o gate** lê o **texto-fonte** dos ficheiros Rust e extrai as variantes dos
  `enum` e os literais dos `match`/`const`.

O gate exige que **A cubra tudo o que B encontrou**, e que A seja **byte a byte** igual ao
ficheiro versionado.

**Porquê dois.** Se só existisse o gerador, uma variante acrescentada a `enum EstadoUi` mas
esquecida em `EstadoUi::ALL` produziria um TypeScript **sem** essa variante — e o ficheiro
versionado, regenerado pelo mesmo gerador, **concordaria**. Verde, e errado. O caminho B é o
**controlo negativo**: não partilha a lista com o gerador, portanto não partilha o erro.

É a mesma técnica que o `surface_gate.rs` já usa (`include_str!` sobre a fonte) e o mesmo
raciocínio do `audit_gate.py`: *um gate tem de ter uma execução descrita que o faça REPROVAR.*

### 4 · Opcional é **`| null`**, nunca campo ausente

`Option<T>` do Rust atravessa como `T | null` **explícito**. Um campo que desaparece do JSON e
um campo a `null` são indistinguíveis para quem lê com `?.`, e a distinção aqui é semântica:
`Instantaneo::stale_ms()` devolve `Option<u64>` **precisamente** para que "nunca houve
instantâneo" não se confunda com "idade zero" — o zero artificial que o ADR-0026 §7 proíbe.

Caso já vivo e não hipotético: a recusa por linha demasiado longa emite `"id": null`, e os
clientes distinguem resposta de evento pela **presença da chave**. O contrato tem de preservar
essa diferença, senão reintroduz o defeito que a correção do `MAX_LINE` fechou.

### 5 · O TypeScript **nunca** se torna fonte independente

Proibido no frontend: declarar à mão qualquer união que espelhe estes enums; alargar o
contrato com valores que o Rust não emite; "corrigir" o contrato editando o `.ts`.

O caminho legítimo de mudança é **um só**: mudar o Rust → correr o gerador → commitar o `.ts`
regenerado. O gate reprova qualquer outra ordem.

### 6 · Política para mudanças incompatíveis

O contrato atravessa o IPC v1, que tem `v` negociado. Portanto:

- **Aditivo** (variante nova, campo novo opcional): regenerar e commitar. O gate obriga o
  frontend a tratar o valor novo — uma união TypeScript que ganha um membro **quebra** o
  `switch` exaustivo do lado do consumidor, que é exatamente o efeito desejado.
- **Incompatível** (remover ou renomear uma variante, mudar um tipo): **não** é uma edição.
  Exige a mesma disciplina que o `PROTOCOL_V` já tem — versão nova do protocolo e migração de
  cliente — porque um valor removido é um valor que um daemon mais antigo ainda emite.
- **Nunca** se remove um código de erro para "limpar" o contrato: `no_show_loaded` significa o
  mesmo dos dois lados desde a GS1.6, e foi para isso que o contrato foi congelado.

## Alternativas rejeitadas

- **`serde` + `schemars` + geração por JSON Schema.** Traria três dependências e um segundo
  formato canónico para um repo que escreve JSON à mão por decisão. O contrato aqui são
  **uniões de strings**, não estruturas ricas; o peso não se justifica.
- **`ts-rs` / `typeshare` (macros derive).** Exigiriam anotar tipos em `led-daemon`, que está
  **congelado**. Inaceitável: a fronteira não pode obrigar o núcleo a mudar.
- **Escrever o `.ts` à mão com revisão humana.** É precisamente a "cópia manual" que o
  critério de aceite da F3 proíbe, e a revisão humana é o controlo mais fraco disponível.
- **Gerar em tempo de build, sem versionar.** Esconde a mudança de contrato do diff do PR e
  obriga o frontend a ter `cargo` para compilar.

## Consequências

- `EstadoUi` ganha `ALL`, e um gate garante que `ALL` não perde variantes.
- `led-daemon` e `led-core` **não mudam** — a extração por texto existe precisamente para os
  não obrigar a mudar.
- O frontend passa a ter **uma** origem de tipos, e um PR que mude o contrato mostra-o.
- Custo assumido: o gerador é código que tem de ser mantido, e a extração por texto é
  sensível ao **formato** da fonte. Mitigado por o gate reprovar quando a extração devolve
  menos do que o esperado — extrair zero variantes é tratado como **falha**, nunca como
  "nada a verificar", que seria o KB-012 na sua forma mais barata.

## Emenda 1 (2026-08-10, F7/B) — **correspondência não é compilabilidade**

O ADR original tinha uma lacuna, e vale nomeá-la em vez de a corrigir em silêncio: o gate de
dois caminhos prova que o `.ts` **corresponde** ao Rust, e nada mais. Um ficheiro pode
corresponder byte a byte ao que o gerador produz **e não compilar** — o gerador emitiria o
mesmo erro nas duas pontas, e os dois caminhos concordariam com ele.

Ou seja: *"bate com o gerador"* **não** significa *"é TypeScript válido"*.

O gate passa a ter **duas propriedades independentes**, e nenhuma substitui a outra:

| # | Propriedade | Mecanismo | O que falha se faltar |
|---|---|---|---|
| 1 | O `.ts` **corresponde** ao Rust | `contract_gate.rs` (caminhos A e B) | o frontend fica com uma união desatualizada |
| 2 | O `.ts` **compila** | `scripts/tsc_gate.sh` → `tsc --noEmit` | o frontend não constrói, e o contrato é inútil |

### O `verifica.ts` é o que torna o `tsc` significativo

`tsc` sobre um ficheiro **só de tipos** prova pouco — prova que o texto é sintaxe válida.
Nenhum tipo é *usado*, portanto nada é *verificado*. O `verifica.ts` (escrito à mão,
**não** gerado) usa-os, e cada `@ts-expect-error` é uma **asserção invertida**: se o erro não
acontecer, `tsc` reprova.

É assim que se prova, em tempo de compilação, aquilo que a comparação de bytes nunca poderia:

- a união é **fechada** — `EstadoUi = "HEALTHY"` não compila, e `HEALTHY` é precisamente o
  nome que alguém inventaria para dizer "está tudo bem";
- `Elo = "hardware_ok"` não compila — a cadeia de evidência não colapsa num booleano;
- `DaemonState = ""` não compila — o defeito que a F5 corrigiu na origem não é sequer
  representável no cliente;
- `| null` **não** é opcional — omitir `staleMs` é erro, e `undefined` não passa por `null`
  (`exactOptionalPropertyTypes`);
- um `Evento` com `id` não compila, e uma `Resposta` sem `id` também não — é a distinção que
  a F1-B documentou;
- o `switch` exaustivo **sem `default`** obriga o frontend a *tratar* um valor novo em vez de
  o ignorar: um membro acrescentado à união deixa a função sem retorno e `tsc` reprova.

### O toolchain, e uma armadilha verificada

`typescript` **5.9.3**, pinada, como única dependência de um `package.json` mínimo em
`crates/led-console-bin/contract/`. Não é o frontend — é o menor comando reprodutível que
compila o contrato. `node_modules` é artefacto e está gitignorado; `package.json`,
`tsconfig.json` e `verifica.ts` são fonte e vão versionados.

**`npx tsc` não serve, e isto foi verificado, não presumido:** no registo npm o pacote `tsc`
é um stub antigo (2.0.4) que **não é** o TypeScript. O script invoca o binário instalado
diretamente, para não haver ambiguidade.

O `tsconfig` é **estrito de propósito** (`strict`, `exactOptionalPropertyTypes`,
`noUncheckedIndexedAccess`, `noFallthroughCasesInSwitch`): um gate em modo permissivo prova
muito menos do que parece.

### O gate **não salta**

Se `node` faltar, `scripts/tsc_gate.sh` **falha** — não se declara "nada a verificar". Um gate
que passa por não ter corrido é a forma mais barata do KB-012, e este repo já foi mordido por
ela (Miri N=0).

### Ainda **não** está na CI

A CI é hoje 100 % Rust (verificado: nenhum `setup-node`, nenhum passo npm em
`.github/workflows/ci.yml`). O `tsc` corre como comando reprodutível, documentado, e **não**
está dentro do `cargo test` — de propósito: pôr Node no caminho do `cargo test` faria a suíte
Rust falhar em máquinas sem Node, o que é um preço maior do que o problema. Integrá-lo na CI
é a fatia seguinte, e fica registado como pendente em vez de dado como feito.

## Emenda 2 (2026-08-14) — o contrato passa a cobrir o que se **envia**, não só o que se recebe

A decisão 2 dizia *"o contrato cobre só o que atravessa o fio"* — e cobria, mas só numa
direcção. `EstadoDoDaemon`, `EstadoUpstream`, `EventoPayload`, `Resposta`, `Evento`, `ROTAS`:
tudo **resposta**. **Nada descrevia os argumentos de um comando.**

**Até agora isso não custou nada, e é por isso que passou despercebido.** O único comando com
argumentos era o `seek`, que manda `{to_ms}` — dois caracteres difíceis de errar, e um erro
ali é imediatamente visível. A assimetria era invisível porque a superfície era trivial.

**Com o `load` deixa de ser trivial.** O `Cmd::Load { path, assume_integrity }` tem dois campos
de tipos diferentes, e o segundo **não é uma opção**: dispara o pré-voo e o `Arm`. Escrever
essa forma à mão no frontend seria exactamente a segunda fonte de verdade que a Phase 1.1
eliminou do lado da resposta — *"sem ele, a UI escreveria a forma à mão"* — só que na direcção
que ninguém tinha olhado.

**Decisão: os argumentos entram no contrato, pelos mesmos dois caminhos.** O **caminho A**
emite-os dos valores Rust compilados. O **caminho B** extrai-os do **texto-fonte do `enum Cmd`**
em `proto.rs` — o produtor real, o mesmo princípio que já se aplica ao arm `Cmd::Status`. Um
campo novo num comando que não chegue ao TypeScript reprova.

**O que NÃO entra**, para o contrato continuar a ser a fronteira e não o modelo de domínio: os
comandos que a superfície HTTP nunca expõe (`hello`, `subscribe`, `shutdown` — a tabela
`NUNCA_EXPOSTOS` do `surface.rs`). Descrever argumentos de comandos que o browser não pode
enviar seria alargar o contrato para lá do que atravessa **esta** fronteira.

**Consequência aceite:** o gerador passa a ler duas secções de `proto.rs` em vez de uma, e o
gate ganha uma extracção nova. É o preço de a assimetria deixar de existir — e o momento certo
para o pagar é agora, quando o primeiro comando que a torna perigosa está a ser construído.

## Critério de reversão

Se o repo alguma vez adotar `serde` por outra razão de peso, geração por schema passa a ser
mais barata que esta e este ADR deve ser revisitado. Enquanto o JSON for escrito à mão, esta
é a solução proporcional.
