# ENOBUFS: a causa, medida — 2026-08-30

Continuação de [`HARDWARE-VALIDATION-2026-08-28-ETHERNET.md`](./HARDWARE-VALIDATION-2026-08-28-ETHERNET.md)
e do [`HANDOFF-2026-08-28-GS45.md`](./HANDOFF-2026-08-28-GS45.md).

**Nenhuma linha de código alterada nesta sessão.** O que segue é medição.

## Resultado

O `ENOBUFS` do burn-in por Ethernet **não é do LUMYX, não é do ESP32, não é da pilha IP e não
é do bind.** É o **adaptador USB-Ethernet** (Realtek RTL8153, `0x0bda/8153`), e manifesta-se
por **dois modos de falha distintos** — um em cada dia medido.

O suspeito que o handoff de 28-08 nomeou está confirmado por log do kernel. E **não foi preciso
outra máquina** para o confirmar.

## Modo A — 2026-08-30: a interface deixa de existir

Corrida: `led-player striptest.lumyx --ddp 192.168.2.162 --loop 150`, braço **wildcard**
(sem `--bind`; o binário imprimiu `origem: 0.0.0.0:0 (a tabela de rotas escolhe)` — verificado,
não assumido).

**81 passes limpos**, e depois aborto com **93 frames falhados em 94**.

A correlação é ao milissegundo:

| Instante | Evento |
|---|---|
| 08:00:38.125 | kernel: `destroying 0x1a86/7523 (USB Serial): hardware connection lost` |
| 08:00:41.467 | pass 80 termina **limpo** |
| **08:00:42.283** | kernel: `destroying 0x0bda/8153 (USB 10/100/1000 LAN): hardware connection lost` |
| **08:00:42.290** | kernel: `Interface "en7" terminated` · `ifnet_detach_final` |
| 08:00:42.318 | o amostrador de interfaces emudece (28 ms depois — `en7` saiu do `netstat`) |
| 08:00:49.805 | `BURN-IN ABORT: 93 failed frames on pass 81` |

`en7` **não voltou**: `ifconfig en7` → *does not exist*, e o nó ficou inalcançável.

**Dois dispositivos USB diferentes caíram com 4 s de intervalo** — o adaptador de rede e um
CH340 USB-Serial. Isso não é um adaptador de rede defeituoso: é algo **a montante dos dois**
(hub, cabo ou alimentação USB). Em 28-08 há registo de um hub Genesys Logic
(`0x05e3/0620` + `0x05e3/0610`) na mesma máquina.

Frequência desde o arranque das 07:12 — **4 desanexos em 50 minutos**:

```
07:15:45  LAN perdido      (3 min após o boot)
07:19:38  Serial perdido
08:00:38  Serial perdido
08:00:42  LAN perdido      ← matou o burn-in
```

## Modo B — 2026-08-28: a fila da interface não drena

Em 28-08 **não houve um único desanexo** do adaptador. O log mostra outra coisa:

```
2026-08-28 11:01:28.776  fq_if_add_fcentry: num: 10, scidx: 7, iface: en7, B:517086
2026-08-28 11:01:28.776  fq_detect_dequeue_stall: num: 11, scidx: 7, iface: en7
```

`fq_detect_dequeue_stall` é o AQM do macOS (FQ-CoDel) a detectar que a **fila de saída da
interface não está a drenar**; `fq_if_add_fcentry` é o kernel a activar *flow advisory* sobre a
flow — e um socket UDP sob flow advisory recebe **exactamente `ENOBUFS`** no `send`. Aqui com
**517 086 bytes** em fila.

Estes eventos ocorrem **só em `en7`**, ao longo de todo o dia 28-08. **Nunca em `en0`.**

## Porque é que isto explica tudo o que já estava medido

| Facto de 28-08 | Explicação |
|---|---|
| `Ierrs 0 · Oerrs 0 · Coll 0` no `en7` | Os pacotes são descartados **acima** do driver, no AQM. Nunca chegam ao NIC — foi por isso que os contadores do NIC ficaram a zero. |
| `netstat -s -p ip` sem um contador a mexer | A fila que enche é a **da interface**, não a do IP. O relatório de 28-08 já tinha suspeitado disto («a fila que enche não é a do socket»). |
| Fixar o bind não elimina a falha | A fila é **por interface**. Fixar o endereço de origem não muda a fila em que o datagrama entra — e no Modo A não há sequer interface para onde ligar. |
| Não é temporizado (89 s, 130 s, 146 s, 688 s) | Ambos os modos são estocásticos: um desanexo USB e um *stall* de fila não têm período. |
| O ESP32 impecável (0 reset, 0 leak) | Correcto — o nó nunca esteve envolvido. |

**A comparação bind-vs-wildcard era ruído.** O handoff já avisava para não afirmar «melhora»
sem carga igual. Agora sabe-se mais: o mecanismo é **independente do bind**, portanto a
diferença entre abortar ao pass 9–18 e ao pass 22/35 não media o bind — media a sorte de duas
amostras de um processo estocástico. A hipótese do bind não estava só refutada; estava a medir
a variável errada.

## O que NÃO está estabelecido

- **Porque caem os dispositivos USB.** Hub, cabo, porta ou alimentação — não determinado. Dois
  dispositivos a caírem juntos aponta para montante, mas isso é inferência, não medição.
- **Alinhamento temporal exacto dos abortos de 28-08 com os *stalls*.** Os *stalls* em `en7`
  estão medidos e são exclusivos dessa interface, mas o relatório de 28-08 não registou a hora
  absoluta de cada aborto, só o tempo decorrido. A correspondência é do **mecanismo**, não
  instante-a-instante como no Modo A.
- **Se os dois modos têm a mesma raiz física.** É plausível (o mesmo adaptador, os mesmos
  sintomas de transmissão) mas são assinaturas diferentes e não foram unificadas por medição.

## Consequência para o plano

O passo que o handoff nomeava como decisivo — **correr o burn-in noutra máquina** — deixa de
ser necessário para identificar a causa, e teria sido **enganador**: uma máquina acabada de
arrancar, com outro caminho USB, teria muito provavelmente passado, e isso seria lido como
ilibação do host quando o que mudou era o adaptador.

O que a correcção exige é **físico**, não código:

1. Ligar o adaptador **directamente a uma porta do Mac**, sem hub.
2. Se persistir, trocar cabo/adaptador — ou usar Ethernet que não passe por USB
   (dock Thunderbolt, ou máquina com porta nativa).
3. Só depois repetir o burn-in. **Um burn-in limpo antes disto não prova nada**, porque a falha
   é estocástica e a janela pode simplesmente não a ter apanhado.

## Erro meu nesta rodada, registado

O meu amostrador de interfaces tinha um **caminho mudo**: se uma interface desaparecesse do
`netstat`, o `if all(iface in v ...)` saltava a escrita **em silêncio** e o laço continuava a
girar sem produzir nada nem falhar. Foi assim que ele ficou 57 s vivo e calado.

Por sorte isso acabou por ser o sinal que denunciou o desanexo — mas foi sorte. Um gate que
emudece em vez de reprovar é a classe do KB-012, e a versão correcta teria de **escrever a
ausência da interface como facto**, não deixar de escrever.

## Estado do rig no fim desta sessão

**Em baixo.** `en7` não existe, o nó não responde. Exige intervenção física: voltar a ligar o
adaptador USB-Ethernet (e verificar o que mais está no mesmo hub).
