# ADR-0017 — Blackout intencional × invariante do heartbeat

- **Status:** 🟢 **aceito** — a **máscara** está implementada
  (`crates/led-daemon-bin/src/output.rs`, commit `1030a7e`): decisões **1–5, 7 e 8**, cada uma
  com teste em `crates/led-daemon-bin/tests/blackout.rs`. ⬜ **Decisão 6 por implementar** —
  exige `PROTOCOL_V = 2` (Emenda 3 do ADR-0027) e `PROTOCOL_V` é 1
  (`crates/led-daemon-bin/src/proto.rs:17`); o ADR-0031, que faz a negociação descer
  limpamente, está aceite e **não** implementado. ⬜ **Decisão 9.C por medir** — exige o rig.
  O **D6** — o botão no console, tal como `docs/ROADMAP.md:328` o define — continua **aberto**:
  o que aterrou é a **pré-condição** dele, não ele.
- **Data original:** 2026-07-26 · **Decidido:** 2026-09-01 · **Revisão da decisão 9:** 2026-09-02
  · **Máscara implementada:** 2026-09-04
- **Fonte:** Revisão do plano de console do operador (regra: blackout requer decisão separada)
- **Análise:** [anexo](0017-anexo-analise-e-proposta.md) (2026-08-05). O **P1** do anexo está
  **corrigido** por este ADR — ver decisão 2.
- **Emenda que invalidou o P1:** [ADR-0019, Emenda 1](0019-calibracao-por-output-no-hal.md)
  (2026-08-07) — a calibração saiu do HAL porque o `DdpOutput` o contorna; o mesmo argumento
  se aplica à máscara de blackout.

## Contexto e problema
O console do operador vai precisar, eventualmente, de um **blackout intencional** (apagar o
rig sob comando). Mas o Baseline 1.0 tem um invariante deliberado e testado: **o heartbeat
NUNCA envia um frame preto/zerado** — porque um frame zero apaga o palco por acidente e o
silêncio total dispara safe-mode nos controladores (`crates/led-hal/src/heartbeat.rs`; teste
`must not blast a blackout frame`). Existe, portanto, uma **tensão real**: "operador manda
apagar" precisa coexistir com "o sistema jamais apaga sozinho".

O problema central não resolvido: **quando o operador aciona blackout, o que o heartbeat
reenvia?** Se o heartbeat gravar o frame preto como "último frame válido", ele reenvia preto
(blackout persistente — correto para blackout). Se não gravar, ele reenvia o frame
pré-blackout (o rig "acende de volta" no próximo heartbeat — errado). Cada opção tem
implicações de segurança de palco.

**O dilema tinha uma premissa falsa, e é isso que o desbloqueia.** Ele assume que a máscara
de blackout e o armazenamento do último frame vivem na **mesma camada** — e por isso obriga a
escolher entre gravar preto e não gravar. No LUMYX não precisam viver na mesma camada.

## Decisão

### 1 · A máscara é aplicada a jusante de `heartbeat.record()` — o preto persiste

```
frame vivo ──► record()  [guarda o frame REAL, nunca preto]
                  │
                  └──► … ──► ★ máscara de blackout ★ ──► fan-out
heartbeat ──► reenvia o último frame REAL ──────────────┘  (passa pela MESMA máscara)
```

O preto persiste **sem gravar preto**; o `restore` é trivial (levantar a máscara); e o
invariante do `LUMYX_GOSL.md` §4-#9 continua **literalmente** verdadeiro — o heartbeat nunca
*fabrica* zeros, quem zera é a máscara, e a máscara é comandada. O gap máximo de 2,4 s nunca
é violado, porque o heartbeat continua a emitir na mesma cadência.

Isto resolve as questões 1, 2 e 4 da versão adiada deste ADR:

- **Q1** — o blackout emite um preto **comandado**, que não é o caminho que o invariante
  proíbe. O invariante proíbe o preto *acidental/por silêncio*.
- **Q2** — o blackout comandado **não** vira o último frame válido.
- **Q4** — `restore` não precisa de rastrear nada: o último frame não-preto é o único que
  alguma vez foi gravado.

### 2 · A máscara vive no `OutputManager`, antes do fan-out

Um só ponto, três protocolos (`crates/led-daemon-bin/src/output.rs:713`, `send` em `:829`).

**Isto corrige o P1 do anexo**, que dizia "no HAL". O P1 é anterior à Emenda 1 do ADR-0019 e
foi invalidado por ela: o `DdpOutput` **contorna o HAL** (decisão de 2026-07-09d), logo o HAL
já não é a fronteira que os três protocolos partilham. Pôr a máscara lá produziria blackout em
Art-Net e sACN e **silenciosamente nenhum no DDP** — que é o protocolo validado em hardware.

Rejeitadas: manter no HAL (*«pior que a ausência uniforme, porque pareceria feito»*, o achado
de 2026-08-07f) e dar ao DDP uma máscara própria (segunda implementação da mesma
transformação).

Custo registado: o argumento do ADR-0019 de que a ramificação é *«por device, não por pixel»*
deixa de valer da mesma forma. A máscara é um `memset` — mais barata que a LUT de calibração
que a Emenda 1 já aceitou nesta mesma fronteira.

### 3 · `record()` nunca recebe frame mascarado

Consequência direta da decisão 1, escrita à parte porque é o ponto que uma implementação
distraída inverteria.

### 4 · STOP ≠ BLACKOUT

Transporte não é saída. `Stop`/`Pause` continuam a não apagar o palco — é o que o ADR-0023
decisão 3 já fixa, e o que `crates/led-daemon/tests/transition_matrix.rs:353` já verifica.

### 5 · Latching, com estado visível

O blackout **mantém-se** até ser levantado, e o estado é observável pelo operador. Um blackout
momentâneo que se desfaz sozinho é indistinguível de uma falha.

### 6 · Confirmação em duas fases + log auditável

Ação irreversível de palco (Q3 da versão adiada). O IPC v1 já tem o padrão em uso no
`shutdown`.

### 7 · Escape por device é REQUISITO NORMATIVO, declarado no `Alvo`

O P6 do anexo passa a requisito, não recomendação: **o D6 não landa sem ele.**

Vive no `Alvo` — a instância concreta — e não no `HardwareProfile`. É a fronteira que o ADR-0029
já traça (*«tudo — e só — o que é da instância»*), e contrasta com a `Calibration`, que **não**
está no `Alvo` precisamente por ser do tipo de hardware e não da instância.

Razão física: um traje de dança autónomo não pode ser apagado por um botão de consola, e um
blackout que o operador julga total mas não é constitui falsa sensação de segurança.

### 8 · O fade é instantâneo

Fecha a lacuna §4.5-1 do anexo. Um mecanismo de segurança não pode ter uma janela em que o
palco ainda ilumina.

### 9 · Cluster — FAIL-SAFE POR NÓ

**Formulação normativa, literal:**

> Se um nó perder o comando/estado necessário para garantir o blackout, esse próprio nó deve
> entrar no seu estado seguro (blackout). A falha de um nó NÃO deve provocar automaticamente
> blackout nos nós saudáveis.

Esta decisão tem **três camadas, e a distinção entre elas é normativa**. Confundi-las é o erro
que este repositório proíbe: transformar «não medido» em «garantido».

#### 9.A · DECISÃO NORMATIVA — o que é exigido

É o **comportamento exigido para a integração LUMYX/controlador**, com a mesma força de
qualquer outro requisito deste ADR. Vale independentemente de estar hoje implementado, medido
ou observado.

#### 9.B · EVIDÊNCIA — o que está provado hoje

**NÃO está provado** que o firmware do controlador actualmente utilizado entra efectivamente
em blackout quando perde o link ou o comando.

| Afirmação | Estado |
|---|---|
| O LUMYX exclui segmentos `Failed` dos envios | **Verificado em código** — `crates/led-hal/src/cluster_sync.rs:176` (`continue`); invariante documentada em `:13` |
| O silêncio faz o controlador entrar em «safe mode» | **Documentado, não medido** — `LUMYX_GOSL.md:85` |
| Esse «safe mode» **é** blackout (saídas a zero) | **NÃO OBSERVADO** — nenhuma medição no repositório |

A terceira linha é o buraco. «Safe mode» é um nome; não diz que cor tem. Nenhum documento em
`docs/certification/` mede o estado das saídas do controlador após perda de link.

> **Proibição explícita:** a ausência de medição **não** autoriza escrever, em documento
> nenhum, que o firmware já garante blackout. Nem como pressuposto, nem como nota de rodapé,
> nem por omissão.

#### 9.C · ACEITAÇÃO — o que fecha o buraco

A propriedade **«estado seguro do controlador = blackout»** deve ser verificada na **validação
física / de integração** antes de a garantia operacional ser declarada comprovada.

Até essa medição existir, o estatuto correcto da segunda metade da decisão 9 é: **requisito de
integração declarado — garantia operacional por comprovar.**

#### Consequência de implementação

Os dois cenários **não têm o mesmo estatuto**:

| Cenário | Meio de garantia | Estatuto |
|---|---|---|
| Comando não chega, **link vivo** | **Garantido por construção** pela decisão 2 — a máscara é aplicada antes do fan-out; o cenário torna-se irrepresentável | Fechado por desenho |
| Nó **perde o link** | **Fora do alcance do software LUMYX** — o segmento `Failed` é excluído dos envios (`cluster_sync.rs:176`); o que acontece a seguir é do firmware | **Aberto** — depende de 9.C |

### 10 · Sem atalho de teclado nesta fatia

Q5 da versão adiada: o atalho exigiria verificar não-conflito com foco de texto, atalhos de
sistema e acessibilidade — trabalho próprio, sem o qual um atalho é um risco, não uma
conveniência. O botão com confirmação em duas fases não depende disso.

## Alternativas rejeitadas

Gravar o preto como último frame válido · deixar o heartbeat reenviar o frame pré-blackout ·
máscara no HAL · máscara própria no `DdpOutput` · fade com rampa · blackout sem escape por
device · a falha de um nó a escalar para blackout do cluster.

**Também rejeitada:** declarar a decisão 9 satisfeita com base no comportamento *documentado*
do «safe mode». Documentação de terceiros não é medição própria.

## Não-escopo
Este ADR **não** decide a UI, o IPC nem o preview (ADRs 0013–0016). É estritamente sobre a
semântica blackout × heartbeat, e sobre onde a máscara vive.

## Consequências

**Desbloqueia o D6.** Cria quatro obrigações:

1. O escape por device é **requisito de aceitação** — o D6 não landa sem ele.
2. A doc de operador tem de dizer que **um blackout de consola não apaga um traje autónomo**.
3. A doc de operador tem de distinguir o que a decisão 9 **garante por construção** (link
   vivo) do que **depende do firmware e ainda não foi medido** (link em baixo).
4. A medição de 9.C entra no plano de validação física — não é adiável para «depois do D6».

**Gates existentes que a implementação preserva:** `crates/led-hal/tests/contract.rs:76`
(*must not blast a blackout frame*) · `crates/led-daemon/tests/transition_matrix.rs:353` ·
`crates/led-console-bin/tests/surface_gate.rs:14` (a lista de palavras proibidas actualiza-se
**quando** o D6 landar, não antes).

## Critério de reversão

**(a) Colisão arquitectural.** Se a máscara na fronteira lógica colidir com o gate de alocação
do hot-path, ou se um caminho futuro voltar a contornar o `OutputManager` como o `DdpOutput`
contornou o HAL — **a alternativa não é duplicar a máscara**, é decisão nova com a mesma
formalidade.

**(b) A medição física contradiz 9.B.** Se a validação de 9.C mostrar que o controlador **não**
entra em blackout ao perder o link, então:

- A decisão de integração 9 **não fica automaticamente implementada**, e **não** pode ser
  reescrita para caber no comportamento observado.
- É uma **falha de requisito / de integração**, e é assim que deve ser registada.
- Exige **decisão formal nova**, com a mesma formalidade deste ADR, **antes de o D6 ser
  considerado aceite**.
- As saídas possíveis incluem: exigir firmware que cumpra, mudar de controlador, aceitar
  formalmente o risco com mitigação declarada, ou alterar a decisão 9. **Nenhuma delas pode
  ser tomada por um agente.**

A decisão 9 não se auto-revoga por a realidade não a cumprir. É a realidade que fica registada
como não-conforme.

## Checklist de aceitação

- [x] Decisões 1–10 registadas · alternativas rejeitadas · critério de reversão escrito
- [x] Decisão 9 separada em normativa (9.A) / evidência (9.B) / aceitação (9.C)
- [x] `docs/adr/README.md` — 0017 de `proposto (adiado)` para `aceito` *(linha 30)*
- [x] `docs/ROADMAP.md` — B1 de 🔴 para ✅ *(`:238` — «FECHADO em 2026-09-01»; contagens em `:103` e `:188` dizem **31 ADRs** e «nenhum ADR por decidir»)*
- [x] `docs/architecture/control-protocol.md` — blackout deixa de ser ⛔ *(`:43` está 🟡; **zero** ocorrências de ⛔ no ficheiro)*
- [x] Doc de operador — trajes autónomos + o que a decisão 9 garante **e o que não** *([blackout-operador.md](../runbooks/blackout-operador.md) §3 e §4, esta última partida em 4.1 ✅ POR CONSTRUÇÃO e 4.2 ⚠ NÃO VALIDADO EM HARDWARE)*
- [ ] **Plano de validação física (9.C)** — medir o estado das saídas do controlador após
      perda de link, antes de declarar a garantia operacional comprovada
- [ ] Implementação (D6) — não faz parte deste ADR
