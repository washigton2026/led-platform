# ADR-0031 — Negociação de versão no handshake: o `hello` é o chão, e a ligação tem um dialecto

- **Estado:** 🟢 **aceito** — a **derivação** do `accepts` está implementada (decisão 2, metade
  emissora): no `hello` (`crates/led-daemon-bin/src/server.rs:306`, fatia 1-A, `3da3b8a`) e na
  recusa por versão (`crates/led-daemon-bin/src/proto.rs:176`, fatia 1-B, `b1dbac3`), as duas
  derivadas de `proto::SUPORTADAS`. 🟡 **A decisão 2 está incompleta:** o daemon **não lê** o
  `accepts` do pedido — a metade bilateral fecha com a decisão 3. ⬜ **Decisões 1, 3, 4, 5, 6 e
  7 por implementar** — exigem estado de versão **por ligação** e uma segunda versão suportada;
  `SUPORTADAS` tem um só elemento e `PROTOCOL_V` é 1.
- **Data:** 2026-08-31
- **Decide sobre:** o handshake do IPC (`led-daemon-bin`: `proto.rs`, `server.rs`) e os dois
  clientes (`ledctl`, `led-console-bin`). **Não** toca `led-daemon` (ADR-0023, congelado),
  `led-core`, o transporte, a autenticação, nem o vocabulário de comandos.
- **Fecha:** as quatro decisões pendentes deixadas pela [Emenda 3 do ADR-0027](0027-contrato-tipos-rust-typescript.md).

## Contexto e problema

A [Emenda 3 do ADR-0027](0027-contrato-tipos-rust-typescript.md) decidiu que **um comando novo
é mudança incompatível de vocabulário** e exige `PROTOCOL_V` novo. Ao preparar a
implementação, o gate de precondição encontrou que **a negociação está documentada e nunca foi
implementada**, e que o contrato não determina o mecanismo.

**O que os documentos afirmam:**

- [ADR-0014](0014-ipc-seguranca-ui-engine.md) — *«Comandos tipados e versionados (schema com
  **versão negociada no handshake**)»*, e *«a **política não é reversível**; o transporte é
  substituível»*.
- [`control-protocol.md`](../architecture/control-protocol.md) — *«`v` é a versão do protocolo,
  **negociada no `hello`**»*.
- [`ipc-protocol-v1.md`](../architecture/ipc-protocol-v1.md) — *«O handshake é **por
  ligação**»*.

**O que o código faz.** `Request::from_line` (`proto.rs:130-140`) recusa `v != PROTOCOL_V`
**dentro do parser, em cada linha**, antes de saber sequer se a linha é um `hello`.
`handle_connection` (`server.rs:250`) guarda apenas `hello_done: bool` — **não há estado de
versão por ligação**. Os três emissores (`ok_line`, `err_line`, `event_line`) interpolam a
constante global. E o `accepts:[1]` de `server.rs:305` é um **literal de string sem um único
leitor** em todo o repositório — verificado por `grep`.

Não existe negociação. Existe uma comparação de igualdade contra uma constante, repetida por
linha.

**A lacuna concreta, e é um ovo-e-galinha.** Um cliente anuncia a sua versão pelo campo `v`
da própria linha — mas o `v` do `hello` é validado **antes** de o `hello` ser despachado. Não
há campo para uma *lista* de versões no pedido: o `accepts` só existe no sentido
daemon→cliente. Portanto *«como é que o cliente anuncia mais do que uma versão»* não tinha
resposta no contrato.

## Decisão

### 1 · O `hello` viaja **sempre** em `v:1`, e isso é permanente

O `v` da linha de `hello` é **sempre 1**, em qualquer versão futura do protocolo. A v1 é o
**chão**: a língua franca do aperto de mão, não a versão da conversa que se segue.

Isto resolve o ovo-e-galinha sem tocar no parser: a linha que abre a ligação é sempre uma
linha que **qualquer** daemon deste projecto sabe analisar.

```jsonc
→ {"v":1,"id":1,"cmd":"hello","client":"ledctl/0.1","accepts":[1,2]}
← {"v":1,"id":1,"ok":true,"engine":"lumyx-daemon/0.1.0","accepts":[1,2],"client":"ledctl/0.1"}
```

**Consequência que se aceita de propósito:** a v1 nunca pode ser retirada do daemon. Não é um
custo escondido — é o preço de haver um chão, e um chão que muda não é um chão.

### 2 · `accepts` é **bilateral** e **derivado**, nunca literal

O campo `accepts` passa a existir nos dois sentidos, com o mesmo nome e o mesmo significado:
*«as versões que eu falo»*. Reusar o nome é deliberado — não se inventa vocabulário para o
sentido novo, e a simetria diz o que o campo é.

**Em ambos os lados a lista é derivada do conjunto de versões suportadas**, nunca escrita à
mão. É o precedente do `OutputProtocol::ALL` (ADR-0024): a lista deriva do `enum` que o `match`
já usa, e por isso **não pode divergir dele**. O `accepts:[1]` literal de hoje é exactamente o
defeito que este parágrafo proíbe, e a Emenda 3 já o tinha nomeado.

**`accepts` ausente no pedido significa `[1]`.** Um cliente v1, que não conhece o campo, é
lido como falando só a v1 — que é a verdade. Nenhum cliente existente precisa de mudar.

### 3 · A versão da ligação é a **maior da intersecção**, e vincula-se ao handshake

```
versão_da_ligação = max(accepts_cliente ∩ accepts_daemon)
```

Tudo o que se segue ao `hello` — pedidos, respostas e eventos — viaja nessa versão. O
handshake é por ligação (`ipc-protocol-v1.md`), portanto **duas ligações ao mesmo daemon podem
falar dialectos diferentes**, e isso é correcto.

`PROTOCOL_V` deixa de significar *«a versão do daemon»* e passa a significar **o dialecto
daquela ligação**. É a resposta à questão crítica que o gate de precondição levantou.

**A versão negociada é obrigatória nos emissores**, não opcional. Um emissor que aceite
responder sem saber em que versão está seria a porta pela qual um daemon bilingue responderia
`v:1` a um cliente v2 — o defeito que a Emenda 3 nomeia. Pela disciplina de *tornar
irrepresentável* (ADR-0026 §5: SSE em vez de WebSocket; GS3: sem TCP, `0.0.0.0` não é
representável), o estado errado deve deixar de ser construível, não ser proibido por
convenção.

Pelo mesmo motivo, **«ainda não negociado» e «versão 1» não podem colapsar** num só valor. É a
regra do `Integrity` do GS2, que é um `enum` e não um `bool` precisamente para que *assumido* e
*verificado* não fiquem indistinguíveis.

### 4 · Sem intersecção: `unsupported_version`, explícito e **não fatal**

```jsonc
← {"v":1,"id":1,"ok":false,"error":{"code":"unsupported_version",
     "detail":"cliente aceita [3,4]; este daemon aceita [1,2]"}}
```

O código de erro **já existe** (`proto.rs:25`). O detalhe nomeia **o que veio e o que se
esperava**, na forma do `UnknownSchemaVersion { found, expected }` do ADR-0018
(`validate.rs:44`) — porque uma recusa que não diz o que o outro lado fala obriga a adivinhar.

**A ligação não fecha.** Em `server.rs:288` toda a recusa faz `continue`; o único caso que
fecha é o enquadramento partido (linha acima de 64 KiB), e aqui o enquadramento está intacto.
Fechar seria criar uma terceira categoria de desfecho sem necessidade. O cliente é que decide
se desliga.

**Nunca se degrada em silêncio.** Sem intersecção não há conversa possível, e inventar uma
seria o *best-effort* que os dois documentos normativos proíbem.

### 5 · Descer de versão **não é degradação** — e a distinção é o ponto mais subtil deste ADR

`control-protocol.md:77` e `ipc-protocol-v1.md:45` dizem que *«versão desconhecida → recusa
explícita, nunca best-effort/degradada»*. Um leitor apressado pode achar que a decisão 3 a
viola, porque um cliente v2 contra um daemon v1 acaba a falar v1.

**Não viola, e a diferença é quem decide.** A regra proíbe **o daemon adivinhar**: receber um
`v` que não conhece e prosseguir na esperança de que dê certo. Aqui o daemon nunca adivinha —
publica a lista do que fala e mais nada. Quem escolhe é o **cliente**, de forma explícita, de
entre versões que o daemon **declarou** suportar. Não há suposição em lado nenhum.

E a regra continua a morder onde sempre mordeu: um `v` fora do conjunto suportado é recusado
com `unsupported_version`, sem degradação — decisão 4.

### 6 · Antes do vínculo, responde-se em `v:1`

Qualquer linha emitida antes de a ligação estar vinculada sai em **`v:1`**: `bad_request`,
`unsupported_version`, `unauthenticated` e a recusa por linha demasiado longa.

Não é uma decisão independente — é consequência aritmética da decisão 1. Se o `hello` é sempre
v1, então todo o cliente sabe ler v1, e v1 é a única versão em que uma resposta pré-handshake
pode ser garantidamente compreendida. O corolário importa: um erro **não pode** sair na versão
que o cliente pediu, porque essa é precisamente a versão que o daemon pode não falar.

### 7 · `Cmd::Version.protocol` é a versão **da ligação**

O `protocol` devolvido por `version` (`server.rs:311`) é o dialecto **daquela ligação**, não o
tecto do daemon.

A capacidade total do daemon já viajou no `accepts` do `hello`. Devolvê-la outra vez aqui
criaria uma **segunda fonte de verdade** para o mesmo facto — que é exactamente o que a
Emenda 1 do ADR-0018 existe para impedir, e a razão pela qual `ports` não entrou em `Limits`.
Nenhuma informação se perde: quem quer saber o que o daemon fala lê o `accepts`; quem quer
saber em que língua está a falar lê o `protocol`.

## O que este ADR **não** faz — e é a maior parte dele

**Não cria a v2.** Depois deste ADR:

- `PROTOCOL_V` continua **1** no código (`proto.rs:17`);
- o `enum Cmd` continua com **exactamente 12 comandos**, e o gate que o congela
  (`proto.rs:275`, `assert_eq!(nomes.len(), 12)`) fica intacto;
- **a v2 não é operacional** — fica *negociável por contrato* e **vazia**.

**Não implementa nada.** Nenhuma linha de Rust foi escrita. O `accepts` continua o literal
`"[1]"`, os emissores continuam com a constante global, e os dois clientes continuam a escrever
`"v":1` à mão.

**O mecanismo só será implementado quando houver um comando que exija a v2.** É o gatilho, e é
o mesmo padrão que o ADR-0012 (fan-out paralelo adiado até ao 2.º nó físico) e o TD-011 (medido
e adiado) já usaram: decidir agora, construir quando o caso existir. Implementar a negociação
para uma v2 sem comandos seria construir um mecanismo cujo único utilizador é ele próprio.

## Alternativas rejeitadas

**A — Um flag de versão no arranque do daemon, sem negociação.** Mais barato, e **proibido**:
o ADR-0014 fixa a negociação no handshake como política, e declara a política irreversível.

**B — O `hello` viaja na versão pretendida pelo cliente.** É o que o ovo-e-galinha torna
impossível sem reescrever o parser: `from_line` teria de aceitar qualquer `v` para depois
descobrir se a linha é um `hello`, e um `v` arbitrário deixaria de ser recusado no sítio onde
hoje é. Fixar o chão em v1 obtém o mesmo resultado sem enfraquecer a validação.

**C — Um comando `negotiate` próprio, antes do `hello`.** Acrescenta vocabulário — e um comando
novo é, por esta mesma linha de decisões, mudança incompatível. Negociar a versão com um
comando que exige versão nova é circular.

**D — `accepts` como intervalo (`{"min":1,"max":2}`) em vez de lista.** Assume que o suporte é
contíguo. Um daemon que fale 1 e 3 mas não 2 — durante uma migração, por exemplo — não é
exprimível. A lista não custa mais e não fecha essa porta.

## Invariantes que precisam de teste novo na implementação

**Estado por invariante (2026-09-11).** O **3** está implementado e com gate; o **6** já era
verdade e continua a valer; o **2** tem teste, mas a semântica fica vacuosa enquanto nada ler o
`accepts` do pedido. Os **1**, **4** e **5** continuam **sem código**:

1. **`hello` a `v:1` é aceite por qualquer versão do daemon** — incluindo um daemon que já não
   tenha a v1 no `accepts`. O chão não é negociável.
2. **`accepts` ausente no pedido ⇒ `[1]`** — um cliente v1 literal, sem o campo, negoceia v1.
3. **A lista deriva do conjunto suportado** — um teste que reprove se alguém voltar a escrever
   o `accepts` à mão, na forma do gate do `OutputProtocol::ALL`.
4. **A resposta sai na versão da ligação, não na do daemon** — com um daemon que suporte duas
   versões e duas ligações simultâneas em dialectos diferentes. Com uma só ligação, ou uma só
   versão, as duas hipóteses são indistinguíveis e o teste não prova nada (é a lição do
   ADR-0029 §8: com um alvo, «por nó» e «agregado» são iguais).
5. **Sem intersecção: recusa que nomeia os dois conjuntos, e a ligação continua viva** — um
   pedido seguinte na mesma ligação tem de ser lido.
6. **O `id` sobrevive à recusa de versão** — já provado por
   `versao_desconhecida_e_recusada_nao_degradada`; tem de continuar a valer.

## Invariantes que **não** mudam

UDS `0o600` · `hello` obrigatório (`unauthenticated` antes dele) · `id` extraído antes de
validar o resto · versão fora do conjunto = recusa explícita · `unknown_command` **não** fecha
a ligação · linha acima de 64 KiB **é** fatal e fecha · `MAX_BODY == MAX_LINE` · JSON de uma
linha · `id:null` é resposta e não evento · o transporte não apaga o palco · zero comandos
novos.

## Ficheiros potencialmente afectados na implementação futura

| Crate | Sítios |
|---|---|
| `led-daemon-bin` | `proto.rs` (`PROTOCOL_V`, `from_line`, os três emissores), `server.rs` (estado por ligação, `accepts`, `Cmd::Version`) |
| `led-daemon-bin` (bin) | `ledctl.rs` — `"v":1` escrito à mão em 6 sítios |
| `led-console-bin` | `ipc.rs:89` — `"v":1` escrito à mão |
| Contrato TS | **nenhum** — `lumyx-contract.generated.ts:119` já declara `readonly v: number`, não o literal `1` |
| Docs | `ipc-protocol-v1.md` e `control-protocol.md` ganham a secção do handshake bilateral |

## Consequências

O `accepts` deixa de ser um literal decorativo e passa a ter significado. Um cliente v2 contra
um daemon v1 passa a **descer limpamente** em vez de levar `unsupported_version` — melhor
comportamento do que hoje, com menos código no lado v1: **zero**. E `PROTOCOL_V` passa a ter um
significado definido — dialecto da ligação — em vez de ser uma constante cujo alcance nunca foi
escrito.

O custo é o chão permanente na v1 e a obrigação de os emissores conhecerem a versão.

## Critério de reversão

Se a negociação por ligação se mostrar desproporcionada — o transporte é owner-only e
same-host, e os dois únicos clientes compilam do mesmo commit que o daemon — a alternativa
**não** é voltar ao aditivo em silêncio. É uma decisão nova, com a mesma formalidade, que tem
de dizer **onde** a incompatibilidade passa a aparecer ao operador. É o critério que a Emenda 3
do ADR-0027 já escreveu, e este ADR herda-o sem o enfraquecer.

Se alguma vez existir um daemon que não fale v1, a decisão 1 cai e este ADR tem de ser
revisto por inteiro — o chão é o que sustenta as decisões 3 e 6.
