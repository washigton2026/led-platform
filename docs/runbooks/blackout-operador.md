# Runbook — Blackout: o que apaga, o que não apaga

O que o operador pode e **não pode** assumir quando quer o palco preto. Deriva do
[ADR-0017](../adr/0017-blackout-intencional-vs-heartbeat.md) (aceito 2026-09-01, revisão da
decisão 9 em 2026-09-02).

> Documento apenas. Nenhum comando altera o código da plataforma.
>
> **Escopo.** Isto trata do **blackout intencional** — o operador quer o palco preto. **Não**
> é um runbook de falha de rede (ver [controller-offline.md](./controller-offline.md)) nem de
> *cluster-failover*.
>
> **Legenda de validação**
> - ✅ **POR CONSTRUÇÃO** — a propriedade é garantida pela forma do código; não depende de
>   medição nem de firmware de terceiros.
> - ✅ **IMPLEMENTADO** — existe código e teste discriminante que reprova sem ele.
> - 🟡 **PARCIAL** — parte da alínea tem código, parte não; a linha diz qual é qual.
> - ⬜ **SEM COMANDO DE OPERADOR** — o mecanismo existe, mas não há superfície pela qual o
>   operador o acione. Distinto de «não implementado»: aqui o código está lá.
> - ⚠ **NÃO VALIDADO EM HARDWARE** — depende do firmware do controlador e **nunca foi medido**.
> - ⬜ **NÃO IMPLEMENTADO** — decidido, mas ainda não existe código.

---

## 0. O facto operacional mais importante de hoje  ⬜ SEM COMANDO DE OPERADOR

**Hoje não existe controlo de blackout na consola.** E a razão mudou — o que muda onde
procurar, não o que fazer em palco.

O **mecanismo** existe desde 2026-09-04 (`1030a7e`): a máscara vive no `OutputManager`, com
escape por device, e as decisões 1–5, 7 e 8 do ADR-0017 estão implementadas e testadas. O que
**não** existe é a superfície pela qual um operador o aciona — a decisão 6 (duas fases + log
auditável) exige `PROTOCOL_V = 2`, e `PROTOCOL_V` é 1
(`crates/led-daemon-bin/src/proto.rs:17`). O **D6** do roadmap é o botão, e continua aberto.

> **decidido ≠ construído, e construído ≠ acionável.** Se hoje precisa do palco preto, use os
> meios da instalação (dimmer, corte de alimentação, cena preta na timeline). Não procure um
> botão de blackout na consola: ele não está lá — e o gate
> `crates/led-console-bin/tests/surface_gate.rs:14` reprova se alguém o puser lá antes de o
> protocolo o permitir.

O resto deste documento distingue, secção a secção, o que já **é** do que ainda **será** — e,
sobretudo, fixa o que **nunca** poderá ser assumido (secções 3 e 4).

---

## 1. `stop` **não** é blackout  ✅ POR CONSTRUÇÃO

Esta é a confusão que mais custa caro em palco, e é intencional que sejam coisas diferentes
(ADR-0017, decisão 4).

| Comando | O que faz à fita |
|---|---|
| `pause` | a fita fica **acesa** no último frame |
| `stop` | a fita fica **acesa** no último frame |
| *blackout* (⬜ sem comando de operador) | a fita **apaga**, e **fica** apagada até ser desarmado — o mecanismo existe e está testado; o gesto que o aciona, não |

Isto já é o critério de aceite exercido no rig
(`docs/runbooks/gs4-hardware-ethernet.md:179-180`): *«`pause` e `stop` deixam a fita acesa no
último frame»*. Parar o transporte **não** é um mecanismo de segurança. Se quer preto, peça
preto.

---

## 2. O blackout é *latching*, e desarma-se em duas fases  🟡 PARCIAL

Semântica decidida (ADR-0017, decisões 5, 6, 8 e 10). O latching **existe**; o gesto que o
desarma, não:

- ✅ **Fica armado** até alguém o desarmar. Não expira, não se desarma sozinho ao dar `play`.
  Provado por `o_latching_mantem_se_e_o_estado_e_visivel` (5 frames sem se desfazer).
- ✅ **Estado visível** — o operador *poderá* ver que está armado, não adivinhá-lo pelo palco.
  O estado existe (`blackout_activo`); o ecrã que o mostra é o D6.
- ✅ **Fade instantâneo.** Um mecanismo de segurança não pode ter uma janela em que o palco
  ainda ilumina. Provado por `o_fade_e_instantaneo_o_primeiro_frame_ja_sai_preto`.
- ⬜ **Duas fases + log auditável** para desarmar, como o `shutdown` já faz hoje
  (`crates/led-daemon-bin/tests/ipc.rs:251`). **Não implementado** — é a decisão 6, e exige
  `PROTOCOL_V = 2`. Hoje o desarme existe como chamada interna (`blackout_levantar`), sem
  confirmação nem log, porque ainda não há operador que o possa invocar.
- ⬜ **Sem atalho de teclado** nesta fatia. Não há como armá-lo por engano com a mão no
  teclado — hoje **vacuosamente**, porque não há superfície nenhuma. Passa a ser um requisito
  a verificar quando o D6 landar.

---

## 3. Um blackout de consola **não apaga um traje autónomo**  ✅ POR CONSTRUÇÃO

**Esta é a secção que não pode ser esquecida numa emergência.**

Um traje de LED é um **player**, não um device: toca o show sozinho a partir de um artefato
assado localmente, e **durante o número não há rede no caminho do traje**
(`docs/adr/0022-wearable-playback-autonomo-sync-deterministico.md:39-45,108`). O ADR-0022
di-lo de forma explícita: *não depende do ADR-0017 — um traje autónomo não tem heartbeat*
(`:11`).

Consequência operacional, sem rodeios:

> **O blackout da consola atinge o que está no fan-out de saída do daemon. Um traje autónomo
> não está nesse fan-out. Carregar no blackout não o apaga.**

Se o procedimento de emergência do espectáculo precisa que os **trajes** apaguem, esse
procedimento é **outro mecanismo** — e não existe. Não o assuma coberto por este.

---

## 4. Perda de link: o que é garantido e o que **não** foi medido

A decisão 9 do ADR-0017 é **fail-safe por nó**, na formulação literal:

> Se um nó perder o comando/estado necessário para garantir o blackout, esse próprio nó deve
> entrar no seu estado seguro (blackout). A falha de um nó NÃO deve provocar automaticamente
> blackout nos nós saudáveis.

Isto reparte-se em dois cenários **muito** diferentes, e a diferença é a razão de ser desta
secção:

### 4.1 Link vivo, o nó está a receber  ✅ POR CONSTRUÇÃO

O cenário «o comando de blackout não chegou a um nó que está vivo» é **irrepresentável**. A
máscara vive no `OutputManager`, **a montante** do fan-out de protocolos (decisão 2): há um só
ponto de mascaramento para os três protocolos. Não há caminho por onde um nó vivo receba
frames não mascarados enquanto o blackout está armado.

O heartbeat também não fura a máscara: o `record()` guarda sempre o frame **real** (decisão 3)
e o reenvio passa **pela mesma máscara** (decisão 1) — por isso o preto persiste sem que
alguma vez se grave preto, e o invariante do `LUMYX_GOSL.md:84-85` («o heartbeat envia sempre
o último frame *válido*, nunca um frame zerado») continua **literalmente** verdadeiro.

### 4.2 Link em baixo, o nó deixou de receber  ⚠ NÃO VALIDADO EM HARDWARE

Aqui a garantia **sai das mãos do LUMYX**. O que se sabe, e só isso:

| Facto | Estado |
|---|---|
| O LUMYX exclui segmentos `Failed` dos envios | ✅ verificado — `crates/led-hal/src/cluster_sync.rs:176` (`continue`) |
| O silêncio prolongado põe o controlador em «safe mode» | 📄 documentado — `LUMYX_GOSL.md:85-87`, gap máximo 2,4 s |
| Esse «safe mode» **é** blackout (saídas a zero) | ⚠ **NÃO OBSERVADO** — nenhuma medição no repositório |

> **Proibido, em qualquer documento, procedimento ou briefing:** escrever ou dizer que *«o
> firmware do controlador já garante blackout ao perder o link»*. Não está medido. Tratar a
> ausência de medição como garantia é exactamente o erro que este projecto proíbe.

Enquanto a medição não existir, o operador deve assumir o pior caso: **um controlador que
perde o link pode ficar aceso no último frame que recebeu.**

---

## 5. Escape por device  ✅ IMPLEMENTADO

Nem tudo pode apagar. Uma luz de segurança, uma baliza, um device que não é cenografia — o
blackout tem de os poupar.

O escape é declarado **por instância concreta** (no `Alvo`), não no `HardwareProfile`: é a
mesma fronteira que já separa `Alvo` de `Calibration`. Um perfil descreve um *modelo* de
hardware; se um device concreto está isento, isso é propriedade **daquele** device na
instalação, não do modelo.

Era **requisito de aceitação** do D6, e **está cumprido**: `Alvo.escapa_blackout` existe desde
`1030a7e`. Provado com dois nós, um declarado como escape — um fica preto, o outro continua
aceso (`o_escape_por_device_poupa_o_no_declarado_e_apaga_os_outros`), com o controlo negativo
ao lado (`sem_escape_declarado_todos_os_nos_apagam`). Com um só alvo o teste não provaria nada,
pela mesma razão do ADR-0029 §8.

O que falta é **declará-lo pela consola**: hoje o escape é propriedade da instância na
configuração de saída, e não há superfície de operador para o editar ao vivo — o que é o D6,
não este requisito.

---

## 6. O que falta para este runbook deixar de ser condicional

| Pendência | Tipo | Bloqueia |
|---|---|---|
| ~~máscara no `OutputManager` + escape por device~~ | ✅ **feito** em `1030a7e` (2026-09-04) | — |
| **ADR-0031 → `PROTOCOL_V = 2`** — a negociação de versão do IPC | governança + código | a decisão 6, e por consequência o D6 |
| **D6** — botão + confirmação em duas fases + log auditável na consola | código | as secções 0 e 2 |
| **9.C** — medir o estado das saídas do controlador após perda de link | hardware | a secção 4.2 |

A ordem importa e não é negociável: `PROTOCOL_V` é 1, e a Emenda 3 do ADR-0027 exige 2 para um
comando novo. Subir para 2 hoje parte `ledctl` e o console no parser. **Implementar o ADR-0031
é o passo anterior a qualquer superfície de operador para o blackout.**

Sobre a 9.C, um aviso que é decisão registada e não interpretação: se a medição mostrar que o
controlador **não** apaga, a decisão 9 **não fica automaticamente implementada**. É falha de
requisito de integração e exige **decisão formal nova** antes de o D6 ser aceite.

---

## Ver também

- [ADR-0017](../adr/0017-blackout-intencional-vs-heartbeat.md) — a decisão e as suas dez alíneas.
- [ADR-0022](../adr/0022-wearable-playback-autonomo-sync-deterministico.md) — porque o traje é
  um *player* e não está no fan-out.
- [controller-offline.md](./controller-offline.md) — controlador mudo **na partida**.
- [gs4-hardware-ethernet.md](./gs4-hardware-ethernet.md) — o critério de aceite de
  `pause`/`stop` no rig.
