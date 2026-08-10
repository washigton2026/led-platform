# Protocolo de controlo v1 — implementação (GS3)

> Concretiza [`control-protocol.md`](control-protocol.md) e o [ADR-0014](../adr/0014-ipc-seguranca-ui-engine.md).
> O que ali era especificação, aqui é código: `crates/led-daemon-bin/src/{json,proto,server}.rs`.
> Cliente de referência: `ledctl`.

## Transporte

**Unix Domain Socket, `0o600` (owner-only)**, criado pelo daemon com `--socket <CAMINHO>`.
Não há TCP: `0.0.0.0` **não é sequer representável** aqui — é a forma mais forte de cumprir a
regra do ADR-0014, mais forte que uma verificação em runtime.

Um socket órfão de uma execução anterior é removido no arranque. Sem isso, qualquer paragem
abrupta (e, sem tratamento de sinais, é o caso comum) impediria o próximo arranque.

## Enquadramento

Uma mensagem **por linha** (`\n`), JSON. `id` correlaciona pedido e resposta; respostas fora
de ordem são permitidas. Linha acima de **64 KiB** é recusada — sem esse limite, um cliente
que nunca envie `\n` faz o daemon crescer sem limite.

O teto é imposto **durante** a leitura, não depois dela: o daemon lê no máximo 64 KiB + 1
byte de cada vez, portanto um cliente que abra o socket e nunca envie `\n` **não** faz a
memória crescer com o que escreve. Verificar o comprimento na linha já lida seria tarde
demais — para a verificação correr, a linha teria de estar inteiramente em memória, que é
exatamente o cenário contra o qual o limite existe.

**Ao exceder o teto, o daemon responde uma vez e fecha a ligação.** Não drena o resto da
linha: drenar até ao próximo `\n` seria ler uma quantidade que o **cliente** escolhe — a
mesma negação de serviço noutro sítio. E prosseguir sem drenar seria pior, porque a leitura
seguinte retomaria a meio da linha gigante e o resto seria analisado como um pedido novo.
Depois de um corte a meio de uma linha, o enquadramento desta ligação não é recuperável; o
cliente deve reconectar e repetir o pedido dentro do limite.

## Handshake — obrigatório

```jsonc
→ {"v":1,"id":1,"cmd":"hello","client":"ledctl/0.1"}
← {"v":1,"id":1,"ok":true,"engine":"lumyx-daemon/0.1.0","accepts":[1],"client":"ledctl/0.1"}
```

**Nada é aceite antes do `hello`** — qualquer outro comando devolve `unauthenticated`. O
handshake é **por ligação**: autenticar um cliente não autentica outro.

`v` desconhecida é **recusada explicitamente**, nunca degradada (mesma regra do
`schema_version` do ADR-0018).

## Comandos

| Comando | Args | Passa pela fila? | Notas |
|---|---|---|---|
| `hello` | `client` | não | obrigatório, primeiro |
| `ping` | — | não | vivo? |
| `version` | — | não | `protocol` + `engine` |
| `status` | — | **não** | lê um **snapshot** publicado pelo laço |
| `load` | `path`, `assume_integrity` | sim | ver abaixo |
| `unload` `play` `pause` `stop` | — | sim | mapeiam 1:1 no ADR-0023 |
| `seek` | `to_ms` (inteiro ≥ 0) | sim | |
| `subscribe` | — | não | esta ligação passa a receber eventos |
| `shutdown` | `confirm` | não | **duas fases** |

### `load` e o gate de pré-voo **visível no fio**

Com `assume_integrity: true` o daemon carrega **e arma** (estado `ready`). Sem ela, fica em
`loaded`, e o `play` seguinte devolve **`not_armed`**. O gate do ADR-0023 aparece na resposta
em vez de ser escondido por um arm implícito:

```jsonc
← {"v":1,"id":2,"ok":true,"state":"loaded","position_ms":0,"events":2}
← {"v":1,"id":2,"ok":false,"error":{"code":"not_armed","detail":"NotArmed"}}
```

### `shutdown` — duas fases

O `control-protocol.md` exige duas fases para ações irreversíveis. Hoje o daemon não tem
saída; no GS4 terá, e **acrescentar confirmação depois de existirem clientes custa versão de
protocolo**. Por isso já é assim:

```jsonc
→ {"v":1,"id":2,"cmd":"shutdown"}
← {"v":1,"id":2,"ok":false,"error":{"code":"confirmation_required","detail":"repita com \"confirm\":\"cfm-0-18a3\""}}
→ {"v":1,"id":3,"cmd":"shutdown","confirm":"cfm-0-18a3"}
← {"v":1,"id":3,"ok":true,"shutting_down":true}
```

O token é de **uso único**. Não é segredo criptográfico: o socket já é owner-only, e a
confirmação existe contra o **engano**, não contra quem já tem a credencial do dono.

## Eventos assíncronos

Só para quem fez `subscribe`. **Não têm o campo `id`** — não respondem a nada, e é por isso
que um cliente os distingue de uma resposta:

```jsonc
{"v":1,"async":true,"payload":{"t_ms":1200,"event":"position_changed","ms":1180,"cause":"advanced"}}
```

Uma ligação que morre é **podada** da lista de subscritores; sem isso o `Sender` acumularia
para sempre.

## Modelo de erro

Códigos **enumerados**, nunca string livre:

`unauthenticated` · `unsupported_version` · `unknown_command` · `invalid_args` ·
`confirmation_required` · `refused_by_policy` · `engine_busy` · `load_failed` · `bad_request`

Mais os códigos de recusa do runtime, que vão **inalterados** para o fio: `no_show_loaded`,
`not_armed`, `not_applicable`, `show_already_loaded`, `preflight_failed`,
`seek_out_of_range`, `in_error_state`. **Foi para isto que o contrato foi congelado na
GS1.6** — `no_show_loaded` significa o mesmo dos dois lados.

O `id` sobrevive a **qualquer** erro analisável: é extraído antes de validar o resto, para
que o cliente nunca fique à espera de uma resposta que não vem.

### `id: null` — o erro que não se consegue atribuir

Quando o pedido **não chega a ser analisável**, não há `id` de onde o extrair. Nesse caso a
resposta leva `"id": null` — hoje, apenas a recusa por **linha demasiado longa**.

A distinção que importa, e que os clientes já implementam: um evento **não tem a chave**
`id`; uma resposta não-atribuível **tem a chave com o valor `null`**. O critério é a
*presença da chave*, não a verdade do valor — `{"id":null,…}` é uma resposta, e é lida como
tal pelo `ledctl` e pelo `led-console-bin`. Um cliente que testasse "o `id` é um número"
em vez de "a chave existe" trataria esta recusa como evento e ficaria à espera para sempre.

**Este `id` não é recuperável, e não se tenta recuperá-lo.** O `id` pode estar para lá do
byte 65 536, e adivinhá-lo a partir de um prefixo truncado exigiria um analisador de JSON
incompleto — mais superfície, para um caso em que a ligação vai fechar de qualquer maneira.
Como só pode haver **um** destes por ligação (o que se segue é o fecho), o cliente pode
atribuí-lo com segurança ao pedido que tinha em curso.

## Isolamento: **um só aplicador**

As threads de ligação **nunca tocam** o `ShowRuntime`. Analisam, validam e **enfileiram**; o
laço principal aplica **no limite do tick** e responde. Duas consequências que valem mais que
a conformidade com o `control-protocol.md`:

- o runtime continua com **um único dono**, então o determinismo do ADR-0023 sobrevive à
  chegada da concorrência;
- `status` não passa pela fila — **consultar nunca compete com comandar**.

## Códigos de saída do `ledctl`

`0` ok · `1` o daemon recusou · `2` erro de uso · `3` não consigo falar com o socket.

## O que este protocolo **não** faz

Sem TCP, sem token/mTLS (é same-host por agora — a LAN é do ADR-0014 e fica para quando
houver appliance). Sem `blackout` (ADR-0017). Sem grand master. Sem calibração ao vivo.
E o daemon continua **sem saída**: nenhum frame o deixa até ao GS4.
