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
> - ⚠ **NÃO VALIDADO EM HARDWARE** — depende do firmware do controlador e **nunca foi medido**.
> - ⬜ **NÃO IMPLEMENTADO** — decidido, mas ainda não existe código.

---

## 0. O facto operacional mais importante de hoje  ⬜ NÃO IMPLEMENTADO

**Hoje não existe controlo de blackout.** O ADR-0017 está **aceito**, o que significa que a
decisão está tomada e o desenho está fechado — **não** que o botão exista. A implementação é o
**D6** do roadmap e ainda não foi escrita.

> **decidido ≠ construído.** Se hoje precisa do palco preto, use os meios da instalação
> (dimmer, corte de alimentação, cena preta na timeline). Não procure um botão de blackout na
> consola: ele não está lá.

O resto deste documento define a semântica que o blackout **terá**, para que ninguém a
descubra pela primeira vez em palco — e, sobretudo, para fixar já o que **nunca** poderá ser
assumido (secções 3 e 4).

---

## 1. `stop` **não** é blackout  ✅ POR CONSTRUÇÃO

Esta é a confusão que mais custa caro em palco, e é intencional que sejam coisas diferentes
(ADR-0017, decisão 4).

| Comando | O que faz à fita |
|---|---|
| `pause` | a fita fica **acesa** no último frame |
| `stop` | a fita fica **acesa** no último frame |
| *blackout* (⬜ futuro) | a fita **apaga**, e **fica** apagada até ser desarmado |

Isto já é o critério de aceite exercido no rig
(`docs/runbooks/gs4-hardware-ethernet.md:179-180`): *«`pause` e `stop` deixam a fita acesa no
último frame»*. Parar o transporte **não** é um mecanismo de segurança. Se quer preto, peça
preto.

---

## 2. O blackout é *latching*, e desarma-se em duas fases  ⬜ NÃO IMPLEMENTADO

Semântica decidida (ADR-0017, decisões 5, 6, 8 e 10):

- **Fica armado** até alguém o desarmar. Não expira, não se desarma sozinho ao dar `play`.
- **Estado visível** — o operador vê que o blackout está armado, não o adivinha pelo palco.
- **Fade instantâneo.** Um mecanismo de segurança não pode ter uma janela em que o palco ainda
  ilumina.
- **Duas fases + log auditável** para desarmar, como o `shutdown` já faz hoje
  (`crates/led-daemon-bin/tests/ipc.rs:251`).
- **Sem atalho de teclado** nesta fatia. Não há como armá-lo por engano com a mão no teclado.

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

## 5. Escape por device  ⬜ NÃO IMPLEMENTADO

Nem tudo pode apagar. Uma luz de segurança, uma baliza, um device que não é cenografia — o
blackout tem de os poupar.

O escape é declarado **por instância concreta** (no `Alvo`), não no `HardwareProfile`: é a
mesma fronteira que já separa `Alvo` de `Calibration`. Um perfil descreve um *modelo* de
hardware; se um device concreto está isento, isso é propriedade **daquele** device na
instalação, não do modelo.

É **requisito de aceitação** do D6: o blackout não aterra sem ele (ADR-0017, decisão 7).

---

## 6. O que falta para este runbook deixar de ser condicional

| Pendência | Tipo | Bloqueia |
|---|---|---|
| **D6** — implementar a máscara no `OutputManager` + escape por device | código | as secções 0, 2 e 5 |
| **9.C** — medir o estado das saídas do controlador após perda de link | hardware | a secção 4.2 |

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
