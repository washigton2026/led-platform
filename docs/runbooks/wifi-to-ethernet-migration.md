# Runbook — Migração do rig WiFi → Ethernet cabeado

Migrar os 5 controladores ESP32/WLED (robô led 1–5, hoje em WiFi
192.168.2.156–160) para Ethernet cabeada. Pré-requisito de qualquer show ao
vivo — ver [ADR-0005](../adr/0005-wifi-proibido-producao.md).

> Documento apenas. Nenhum comando altera o código da plataforma. A validação
> por controlador usa o `led-player --discover` que já existe.

---

## 1. Por que a regra proíbe WiFi ao vivo

Saída de LED em tempo real depende de timing estável. WiFi introduz três falhas
que são fatais para show sincronizado com música:

| Problema | Efeito no palco |
|---|---|
| **Jitter 5–50 ms** | frames chegam atrasados/agrupados → movimento tremido, batida fora de sincronia |
| **Perda de pacote** | pixels congelam; após ~2,5 s de silêncio o WLED entra em *safe mode* (apaga) |
| **Interferência / contenção** | outros dispositivos 2,4 GHz (público, celulares, luzes) roubam banda no pior momento — o show |

Ethernet cabeada elimina os três: latência determinística, sem perda por rádio,
sem disputa de espectro. É por isso que o `NetworkGuard` da plataforma **recusa
iniciar** um show com interface WiFi ativa.

---

## 2. Opções de hardware

Um ESP32 só faz Ethernet cabeada se tiver um **PHY Ethernet** no board. Não
existe "adaptador" que converta um board WiFi-only — a migração é uma **troca de
controlador** por um com Ethernet nativo. Três caminhos:

### Opção A — Olimex ESP32-POE / ESP32-POE-ISO (recomendado)
- ESP32 + PHY LAN8710 + **PoE 802.3af** embutido: energia e dados num só cabo.
- `-ISO` tem isolamento galvânico (recomendado para rigs grandes / distâncias).
- Suportado nativamente pelo WLED (builds com Ethernet).
- **Melhor custo/simplicidade** para 5 nós: 1 cabo por robô, sem fonte separada.

### Opção B — QuinLED com Ethernet nativo (QuinLED-Dig-Octa Brainboard)
- A linha QuinLED-**Dig-Uno/Quad** é primariamente WiFi; quem tem Ethernet
  nativo é o **Dig-Octa Brainboard** (PHY W5500/LAN8720).
- Vale se você quiser mais saídas por controlador (Octa = 8 saídas) e já usa o
  ecossistema QuinLED. PoE conforme a variante do board.

### Opção C — Board Ethernet + switch PoE + injetores (manter fontes atuais)
- Qualquer board ESP32 com PHY Ethernet (ex.: WT32-ETH01) + um **switch PoE**
  no rack + injetores/splitters onde o board não aceita PoE direto.
- Mais peças, mas reaproveita fontes de energia existentes.

> Em todos os casos o board precisa de **PHY Ethernet**. "PoE + adaptador" serve
> para **energia e cabeamento** dos boards já Ethernet-capazes — não converte um
> board WiFi-only.

---

## 3. Topologia de rede recomendada (LAN isolada de palco)

```
   [ Máquina de controle ]  192.168.2.10 (IP estático)
            │
     [ Switch (PoE) ]  — dedicado ao show, SEM uplink para internet/casa
       │   │   │   │   │
     .156 .157 .158 .159 .160   ← 5 controladores, IPs estáticos
     robô1 robô2 robô3 robô4 robô5
```

Regras da LAN de palco:
- **Subnet dedicada e isolada**: `192.168.2.0/24`. Sem uplink para a rede de
  casa/internet — nada de tráfego concorrente no momento do show.
- **IPs estáticos, SEM DHCP no palco**: cada controlador com IP fixo
  (.156–.160), a máquina de controle em .10. DHCP no palco = risco de um nó
  pegar IP errado e sumir. Se quiser DHCP no laboratório, use reservas por MAC —
  mas no palco, estático.
- **Switch dedicado ao show** (PoE se usar Opção A): não compartilhar com rede
  de escritório. Cabos Cat5e/Cat6, comprimento ≤ 90 m por lance.
- Manter os **mesmos IPs .156–.160** que a plataforma já conhece
  (`xlights_networks.xml`) → nenhuma reconfiguração de layout necessária.

---

## 4. Configuração no WLED (por controlador)

Para cada board (repetir 5×):

1. **Firmware com Ethernet**: instalar/atualizar para um build WLED que inclua
   suporte a Ethernet (os binários "_ETH" ou build próprio com
   `-D WLED_USE_ETHERNET`).
2. Na UI do WLED → **Config → WiFi Setup**:
   - **Ethernet Type**: selecionar o board (ex.: "Olimex ESP32-POE"). Este
     dropdown só aparece em builds com Ethernet.
   - **Static IP**: definir o IP do controlador (ex.: `192.168.2.156`), máscara
     `255.255.255.0`, gateway `192.168.2.10` (ou o do switch), sem depender de
     DHCP.
   - **WiFi**: pode deixar as credenciais em branco ou desabilitar — no palco a
     saída é Ethernet. (WiFi só para config de bancada, nunca para output ao
     vivo.)
3. **Protocolo de sync** → manter o que o rig usa (ArtNet hoje; DDP é o caminho
   de capacidade recomendado — ver [ADR-0003](../adr/0003-ddp-protocolo-preferencial.md)).
4. Reiniciar o board com o cabo Ethernet conectado e confirmar que ele aparece
   no IP estático.

---

## 5. Checklist de validação (por controlador, com a plataforma)

Com a máquina de controle e o(s) controlador(es) na LAN de palco, use o
`led-player --discover` (ArtPoll broadcast) para confirmar presença **antes** de
qualquer show:

```
# Um controlador de cada vez, à medida que migra:
cargo run -p led-player -- robot_sequence.lumyx --artnet 192.168.2.156 --discover --info
```

Saída esperada quando o controlador responde:
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ✅ respondeu
```

Quando não responde (ainda em WiFi, cabo solto, IP errado):
```
discovery: probing 192.168.2.156 (ArtPoll, 1.5s)… ⚠ SEM resposta
```

Checklist por controlador:

- [ ] Cabo Ethernet conectado; LED de link do switch aceso
- [ ] Board no IP estático correto (.156–.160), pingável de .10
- [ ] WLED reporta "Ethernet" como interface ativa (não WiFi)
- [ ] `led-player --discover` → **✅ respondeu**
- [ ] Repetir para os 5 → só então rodar o show completo com `--require-all`

Validação final dos 5 juntos (aborta se algum não responder):
```
cargo run -p led-player -- robot_sequence.lumyx --artnet 192.168.2.156 --require-all --info
```
> Nota: o `--discover` atual sonda **um** alvo (o de `--artnet`/`--ddp`). Para os
> 5 numa passada única, um discovery multi-IP é evolução futura; hoje, valide um
> por um pela lista acima.

---

## 6. Critério de conclusão da migração

- Os 5 controladores em Ethernet, IPs estáticos, LAN de palco isolada.
- `--discover` responde ✅ para todos os 5.
- Nenhuma interface WiFi ativa nas máquinas de output (o `NetworkGuard` deixa o
  show iniciar).
- Só então: smoke test de luz real (ver [show-startup.md](./show-startup.md)).
