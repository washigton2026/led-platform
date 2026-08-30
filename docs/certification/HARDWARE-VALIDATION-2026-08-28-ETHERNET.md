# Hardware Validation Report — primeiro nó em Ethernet (Olimex ESP32-POE)

- **Data:** 2026-08-28
- **Executado por:** agente, sem operador presente (o pedido foi *"continua todos os passos que
  podes fazer sem mim"*). Por isso **nenhuma etapa visual foi marcada** — nada aqui foi
  confirmado a olho.
- **Marco:** o elo 2 do Golden Slice — *"enviar por Ethernet"* — deixa de estar bloqueado por
  recurso físico. Ver [ADR-0005](../adr/0005-wifi-proibido-producao.md).

## Topologia sob teste

```
ROUTER ──── Cabo 1 ──── Mac   (en7, USB-Ethernet, 192.168.2.163)
       └─── Cabo 2 ──── ESP32-POE (192.168.2.162)
```

O Mac está nas **duas** redes ao mesmo tempo — o WiFi (`en0`, 192.168.2.32) continua ligado à
mesma sub-rede. Isto **não** invalida as medições, mas obrigou a fixar a origem em todas elas:
`route get 192.168.2.162` → `interface: en7`, e o `ping`/sonda usam `-S 192.168.2.163`. A
evidência mais forte de que o tráfego foi mesmo pelo cabo não é a rota — é o WLED a reportar
`lip: 192.168.2.163`, o IP **Ethernet** do Mac, durante os envios.

## Nó sob teste

| Item | Valor | Como foi obtido |
|---|---|---|
| Controlador | WLED **16.0.1**, release **`ESP32_Ethernet`** | `/json/info` |
| IP | **192.168.2.162** | ARP em `en7` |
| MAC Ethernet | `20:e7:c8:72:bc:17` | ARP — é o MAC base `…bc:14` **+3**, que é o offset do ETH no ESP32 |
| Link | Ethernet, **não** WiFi | `wifi.rssi: 0`, `bssid: ""`, `ap: false` |
| `eth.type` | 2 | `/json/cfg` |
| LEDs configurados | **30 px**, pin 4, GRB | `hw.led.ins` |
| Sync Interfaces | `live.en: true`, **`port: 5568`**, `mc: false`, `dmx.uni: 1`, `mode: 4` | `/json/cfg` |

> **O nó tem 30 px configurados, não 720.** Os testes enviaram 720 px de payload; o WLED
> aceita e usa o que cabe. Isto mede **transporte e aceitação de protocolo**, nunca cobertura
> de fita. Não há prova nenhuma, neste relatório, sobre pixels acesos.

## ETAPA 1 — latência: Ethernet contra WiFi, mesmo nó, mesmo instante

100 pacotes, intervalo 100 ms, a única variável é a interface de saída do Mac.

| Caminho | perda | min | **média** | máx | **jitter (stddev)** |
|---|---|---|---|---|---|
| **Cabo** (`-S 192.168.2.163`, en7) | 0 % | 0.906 ms | **1.511 ms** | 2.384 ms | **0.301 ms** |
| WiFi (`-S 192.168.2.32`, en0) | 0 % | 2.173 ms | **4.183 ms** | **91.209 ms** | **8.766 ms** |

Contexto histórico: a bancada de 2026-07-20, com o nó **inteiro** em WiFi, mediu
**99 ms médios / 146 ms de pico / 31 ms de jitter**. O cabo dá **~65× menos latência média** e
**~100× menos jitter**.

E a linha do WiFi acima é ainda **generosa com o WiFi**: aqui só a perna Mac→router é sem fios
(o nó já está no cabo), e mesmo assim injeta uma cauda de 91 ms. É a confirmação empírica mais
limpa que o ADR-0005 tem até hoje.

**Resultado: PASS (medido).**

## ETAPA 2 — que portas o WLED tem mesmo com bind

Um socket UDP `connect`ado entrega o ICMP port-unreachable como `ECONNREFUSED`. Isso distingue
*porta com bind* de *porta sem bind*, sem privilégios.

**Uma primeira tentativa com `nc -u -z` foi descartada, e a razão importa:** os controlos
negativos (portas 1, 5569, 9999) reportaram **OPEN**. Uma sonda que diz "aberto" para tudo não
mede nada, e o resultado teria sido lido como confirmação. Só depois de os controlos negativos
darem *SEM BIND* é que os positivos passaram a valer.

| Porta | ESP32-POE (Ethernet) | ESP32 DevKit (WiFi, 2026-07-23) |
|---|---|---|
| 4048 — DDP | **com bind** | com bind |
| 6454 — Art-Net | **SEM bind** | com bind |
| 5568 — sACN/E1.31 | **com bind** | SEM bind *(era o bloqueio registado)* |
| 21324 — UDP sync WLED | com bind | — |
| 1 / 5569 / 9999 — controlo negativo | SEM bind | — |

**Resultado: PASS (medido, com controlo negativo).**

## ETAPA 3 — protocolos, com aceitação lida do próprio WLED

Critério de aceitação: `live: true` + `lm` (live mode) igual ao protocolo + `lip` igual ao IP
de origem, tudo lido do `/json/info` **durante** o envio. O `played` do emissor só prova que o
`sendto` correu — não prova recepção.

| Protocolo | Emissor | `played`/`failed` | WLED durante o envio | Veredito |
|---|---|---|---|---|
| **DDP** | `led-player striptest.lumyx --ddp 192.168.2.162` | **94 / 0** | `live:true` · `lm:"DDP"` · `lip:192.168.2.163` — **15/16 amostras** | ✅ **aceite** |
| **sACN** | `examples/sacn_send striptest.lumyx 192.168.2.162 1` | **94 / 0** | `live:true` · `lm:"E1.31"` · `lip:192.168.2.163` — **13/15 amostras** | ✅ **aceite** |
| **Art-Net** | `led-player … --artnet 192.168.2.162 --first-universe 1` | **94 / 0** | `live:false` · `lm:""` — **0/16 amostras** | ⛔ **não aceite** |

Hash da gravação idêntico nos três (`0x23b8ee876a18e5a5`) — o replay determinístico aguenta.

**sACN passa a estar validado em hardware pela primeira vez.** Fecha o item que estava aberto
desde 2026-07-23 como *"BLOQUEADO (firmware)"*, e confirma retroactivamente o diagnóstico dessa
sessão: o `packet.rs` estava correcto, o bloqueio era do lado do WLED.

**Art-Net regrediu neste nó** — mas por configuração, não por código: a sonda de porta previu a
rejeição *antes* do teste, e o teste confirmou-a. O mesmo emissor está validado em hardware
desde 2026-07-23 na outra placa.

### A explicação que unifica as duas placas

`if.live.port` do WLED vale **5568** nesta placa e valia **6454** na anterior. Uma única
hipótese — *o WLED faz bind de **uma** porta de entrada, e é essa que decide se fala E1.31 ou
Art-Net; o DDP (4048) é independente* — explica **as seis observações** das duas placas, sem
excepções.

> Registo de honestidade: isto é **inferência a partir do campo de configuração**, não leitura
> do código do firmware. O que está medido são as seis portas e os seis vereditos de aceitação.
> A consequência prática, essa, é firme: **neste WLED, sACN e Art-Net são mutuamente
> exclusivos.** Um rig que precise dos dois precisa de nós diferentes ou de outro firmware.

**Resultado: DDP PASS · sACN PASS · Art-Net BLOQUEADO por config do nó (não medido como falha
do LUMYX).**

## ETAPA 4 — `lumyx-hwcheck`

`lumyx-hwcheck 192.168.2.162 --profile esp32-poe-wled-ddp --amostras 30`

| Etapa | Resultado | Evidência |
|---|---|---|
| alcance+latencia | **NÃO MEDIDO** | `sem resposta: os error 35` |
| controlador | PASS | ver 16.0.1 · freeheap 119272 · uptime 629 s |
| protocolo:ddp | **PASS** | 40 frames, 0 erros · `live:true` · `lm:"DDP"` |
| protocolo:artnet / sacn | NÃO MEDIDO | o preset declara Ddp |
| heartbeat | PASS | período 800 ms · maior intervalo real **805 ms** · 0 erros |
| queda+recovery | NÃO MEDIDO | etapa interactiva (`--cabo`), exige operador |

**O `NÃO MEDIDO` da latência é um limite do instrumento, não do nó.** O `hwcheck` mede latência
por **ArtPoll**, e este nó não tem listener Art-Net (ETAPA 2). Com um preset DDP, essa etapa é
estruturalmente incapaz de medir — e o `ping` da ETAPA 1 mostra que o nó responde em 1.5 ms.
Duas observações independentes concordam sobre a causa. Registado abaixo como achado.

## ETAPA 5 — burn-in por Ethernet: **ABORTOU, e pior que em WiFi. REPRODUZIDO.**

`led-player striptest.lumyx --ddp 192.168.2.162 --loop 150`, duas corridas:

| Corrida | Passes limpos | Aborto | Frames falhados |
|---|---|---|---|
| 1 | 10 | pass 11 | 22 |
| 2 | **15** | pass 16 | **25** |

Comparação directa com o burn-in WiFi de 2026-07-23: **45 passes limpos e depois 1 falha**.
Em Ethernet: aborto ao 11º e ao 16º pass, com **22 e 25** falhas. **Isto é uma regressão face
ao WiFi, é reprodutível, e não deve ser arredondada** — a expectativa era o contrário.

O que está estabelecido sobre esta falha, nas duas corridas:

- **O nó não é o culpado.** Uptime monotónico ao longo de tudo (3445 → 4946 → 7941 → 9328 s,
  **sem um único reset**); freeheap entre 117.6 k e 119.3 k (**sem leak**). O ESP32-POE
  comportou-se exactamente como em WiFi: impecável.
- **O fio não é o culpado.** `netstat -i` em `en7`, **medido depois das falhas**:
  **`Ierrs 0 · Oerrs 0 · Coll 0`** sobre 10.102 pacotes enviados, link **1000baseT
  full-duplex**. Zero erros ao nível do NIC significa que o pacote **nunca chegou ao driver**.
- Logo a falha é do **`sendto` em espaço de utilizador**, acima do driver e abaixo do nó.
- **Assinatura consistente:** um surto de ~2 s (22–25 frames a 11.6 fps) em que *todo* envio
  falha, e depois recupera — o `--loop` é que aborta. **Não é temporizado**: ~89 s numa
  corrida, ~130 s na outra, o que exclui um temporizador fixo como o de expiração de ARP a
  1200 s.

Quatro corridas, todas a abortar:

| Corrida | Aborto | Frames falhados |
|---|---|---|
| 1 | pass 11 | 22 |
| 2 | pass 16 | 25 |
| 3 | pass 18 | 49 |
| 4 | pass 9 | 76 |

**Resultado: FALHOU (medido, reproduzido 4/4).**

### O que foi excluído por medição

- **ARP** — hipótese minha, **refutada pelos dados**. Amostragem contínua durante uma corrida:
  1912 amostras, **0 `incomplete`**, e 57 amostras na janela exacta do aborto todas com o MAC
  resolvido. Além disso os abortos deram-se aos ~89 s, ~130 s e ~146 s, o que exclui o
  temporizador de 1200 s (`net.link.ether.inet.max_age`).
- **Buffers e rotas do IP** — `netstat -s -p ip` antes e depois de uma corrida com 76 falhas:
  `output packet dropped due to no bufs` **0 → 0**; `output packets discarded due to no route`
  **20 → 20**. Nenhum contador do IP se moveu.
- **EAGAIN** — o socket do `DdpDevice` é **bloqueante**: não há `set_nonblocking` em nenhum
  ponto de `ddp.rs`, `router.rs` ou `led-player/src/lib.rs`.

### A experiência que nomeia a variável

Uma sonda externa em Python, a imitar o padrão de envio (mesmo destino, mesmo tamanho de
datagrama, mesma cadência, `connect()` + `send()` como o `DdpDevice`), foi corrida em dois
braços que diferem **só no endereço de bind**:

| Braço | Bind | Corridas | Resultado |
|---|---|---|---|
| A | **`0.0.0.0`** — o que `DdpDevice` fazia | 2 | **ENOBUFS** (`errno 55`) em surto, aos ~124 s e aos ~165 s |
| B | **`192.168.2.163`** — fixa a saída no cabo | 2 | **0 falhas**, 300 s cada |

**O endereço de bind muda o comportamento de falha.** E o errno — ENOBUFS — é o mesmo que a
sessão de 2026-07-23 tinha suposto para a falha em **WiFi**.

Contexto que importa: este Mac está **dual-homed na mesma sub-rede** — `en0` (WiFi,
192.168.2.32) e `en7` (cabo, 192.168.2.163), ambas com rota para 192.168.2.0/24 e ambas com
rota por omissão. Com bind em `0.0.0.0` a interface de origem fica ao critério da tabela de
rotas; com bind explícito, não.

**Hipótese testada e REFUTADA:** que o tráfego com bind `0.0.0.0` saísse por WiFi. Medido por
delta de `Opkts` por interface durante 60 s: **en7 +1708, en0 +24** — vai pelo cabo. *(Ressalva:
essa janela não teve falhas; mostra o caminho normal, não o caminho durante o surto.)*

**O mecanismo continua por determinar.** O que está estabelecido é a variável de controlo e o
errno, não a cadeia causal. Determiná-la exige o errno do lado do produto — que hoje é
impossível, porque o `led-player` o descarta (`Err(_)`, `crates/led-player/src/lib.rs:138`).
É esse o achado mais accionável desta sessão.

## ETAPA 6 — origem explícita: a hipótese foi testada em hardware e **REFUTADA**

O `led-player` ganhou `--bind <ip[:port]>`, que fixa o endereço local de saída no caminho DDP
(ver *Alterações* abaixo). Isso tornou a hipótese testável pelo próprio burn-in.

| Braço | Bind | Corridas | Aborto (pass) | Tempo aprox. |
|---|---|---|---|---|
| Antigo | wildcard `0.0.0.0` | **5** | 9 · 11 · 15 · 16 · 18 | 73–146 s |
| Novo | `192.168.2.163` (cabo) | **2** | **22 · 35** | **~178–284 s** |

**Fixar a origem NÃO elimina a falha.** As duas corridas com origem fixada abortaram na mesma
(61 e 37 frames falhados). O critério de aceitação *"o burn-in não reproduz as falhas
anteriores"* **não está cumprido**.

**E isto obriga a rebaixar uma evidência anterior desta mesma sessão.** O braço B da sonda
externa (bind no cabo) correu **300 s sem falhas, duas vezes**, e eu li isso como imunidade. O
player com bind falhou aos **~284 s** — ou seja, os 300 s da sonda estavam **na fronteira**, não
acima dela. Aquelas duas corridas nunca distinguiram "imune" de "ainda não chegou lá". Registado
como correcção ao registo, não apagado.

O que **resta** de sinal, e é preciso ser exacto sobre a sua força: as duas corridas com origem
fixada abortam aos passes **22 e 35**, e as cinco com wildcard entre **9 e 18**. Os dois
intervalos **não se sobrepõem** — o pior caso com bind é melhor que o melhor caso sem. É
compatível com *"atrasa mas não evita"*.

**Não é uma conclusão**, e as razões estão escritas para não serem arredondadas depois: `n=2`
contra `n=5`, sem mecanismo conhecido, e com uma variável de confusão real — as corridas não
foram feitas sob carga controlada (a corrida 3 do wildcard, a única com um amostrador ARP em
paralelo, foi também a que mais falhas produziu). Separar *"o bind atrasa"* de *"a máquina
estava mais livre"* exige repetição com carga fixada, que não foi feita.

O bind foi verificado como aplicado, **não assumido**: o binário imprime
`origem: 192.168.2.163:0 (interface fixada pelo operador)`, e há um teste que reprova se o
parâmetro deixar de chegar ao socket.

### O que isto elimina da lista de hipóteses

A explicação *"o wildcard entrega a interface à tabela de rotas, e é isso que causa o ENOBUFS"*
está **morta**: com a interface fixada, o ENOBUFS continua. Combinado com o que já estava
excluído (nó, fio, ARP, rotas e buffers do IP, EAGAIN), o que sobra é uma saturação a jusante
do socket que **nenhum contador do host regista** — e para a nomear é preciso o errno do lado
do produto, no instante da falha, que hoje se perde.

## Conclusão

**O meio Ethernet está provado; a fiabilidade contínua sobre ele NÃO está.**

Validado (medido, sem operador):
- Latência e jitter do cabo, com o WiFi como controlo no mesmo instante.
- DDP e **sACN** aceites em hardware, com evidência lida do WLED.
- Heartbeat dentro do orçamento (805 ms contra o tecto de 2400 ms do `LUMYX_GOSL`).
- Nó estável sob carga: sem reset, sem leak.

**NÃO validado — continua em aberto:**
- **Burn-in** — falhou, e pior que em WiFi. É o bloqueador desta fase.
- **Qualquer coisa visual** — nenhum pixel foi observado. O nó tem 30 px configurados.
- **Art-Net neste nó** — bloqueado pela config do WLED.
- **Rig completo** — 1 nó de 5; 6.200 px continuam por tocar.
- **Show musical real** — foi `striptest.lumyx` sintético.
- **`--cabo` (queda e recuperação)** — exige operador.
- **Burn-in 72 h**, chaos físico.

## Achados para o código (não corrigidos nesta sessão)

1. **`led-player` engole o errno do envio.** `Err(_)` em
   `crates/led-player/src/lib.rs:138` — um burn-in aborta e o operador não sabe porquê.
   É a mesma classe que o repositório já nomeou noutro sítio (*"erro engolido é indistinguível
   de sucesso"*), e foi exactamente o que travou o diagnóstico da ETAPA 5.
2. **`lumyx-hwcheck` não consegue medir latência num nó sem Art-Net.** A etapa
   `alcance+latencia` usa ArtPoll; com um preset DDP contra um nó sem listener 6454, o
   resultado é `NÃO MEDIDO` garantido. O `NÃO MEDIDO` está correcto (o instrumento não mente),
   mas a etapa é inalcançável por construção nessa combinação.
3. **Não há preset Art-Net para ESP32-POE** no catálogo (só `esp32-devkit-wled-artnet`), o que
   impediu medir Art-Net pelo `hwcheck` neste nó.
4. **`DdpDevice` não consegue honrar `output_interface`.** `ddp.rs:285` faz
   `UdpSocket::bind("0.0.0.0:0")` — a interface de saída fica ao critério da tabela de rotas, e
   **não há API para a escolher**. Isto torna concreto o *TEST GAP* que a auditoria de
   2026-08-07h já tinha nomeado: *"a protecção do ADR-0005 vem do `WifiBlockGuard` a sondar o
   **host**, não da declaração do profile, e as duas podem divergir"*. Num host dual-homed —
   como este — o `output_interface` declarado no `HardwareProfile` não tem como ser respeitado
   pelo caminho DDP. **Não medi frames a saírem por WiFi** (o teste de `Opkts` mostra o
   contrário), portanto isto é uma lacuna de garantia, não um incidente observado.

## Artefactos

- Sonda de porta com controlos negativos: `udp_probe.py` (scratchpad da sessão)
- Amostragem de aceitação durante envio: `aceitacao.sh` (scratchpad da sessão)
- Log do burn-in: `burnin-eth.jsonl` · saúde do nó: `saude-burnin.txt`
- Show de teste: `striptest.lumyx` (720 px, 94 frames, `0x23b8ee876a18e5a5`)
