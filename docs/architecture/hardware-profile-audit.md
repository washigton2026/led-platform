# Auditoria — `HardwareProfile` → bytes no fio (2026-08-07)

Percurso auditado, campo a campo:

```
HardwareProfile → OutputConfig → OutputManager → ProtocolOutput → bytes no fio
```

**A regra desta auditoria:** *"o campo existe"* nunca é evidência de integração. Cada linha
abaixo foi verificada lendo o consumidor real, não a declaração.

## Classes

| Classe | Significado |
|---|---|
| **HONORED** | Chega ao fio (ou ao veredito) e há teste discriminante |
| **METADATA** | Não deve chegar ao fio — e há ADR que o diz |
| **HARDWARE-DEPENDENT** | Só verificável contra um controlador real |
| **IGNORED-UNINTENTIONALLY** | Relevante, sem consumidor, sem decisão que o justifique |
| **TEST GAP** | Consumido, mas sem teste que reprove se deixar de ser |

## Tabela

| Campo | Estado | Evidência (consumidor real) | Teste | Hardware? |
|---|---|---|---|---|
| `capabilities.protocol` | **HONORED** | `OutputProtocol::from_profile` → escolhe DDP/Art-Net/sACN; esquema contraditório é erro | `o_esquema_escrito_tem_de_concordar_com_o_profile` | não |
| `capabilities.color` (formato) | **HONORED** | `ColorFormat::write` no mapper; 4 canais em RGBW | `rgbw_poe_quatro_canais_no_fio_com_o_branco_subtraido` | não |
| `capabilities.color` (RgbOrder) | **HONORED** | `cfg.rgb_order()` → `linear_assignments` | `cada_ordem_de_canais_produz_bytes_proprios` | não |
| `capabilities.supports_discovery` | **CORRIGIDO** (era IGNORED-UNINTENTIONALLY) | `preflight` deixa de sondar quando é `false` | `um_no_que_nao_faz_discovery_nao_e_reprovado_por_nao_responder` + controle negativo | não |
| `capabilities.supports_metrics` | **METADATA** | Só o validador. O `/metrics` do LUMYX é do **daemon**, não do nó | — | não |
| `capabilities.output_interface` | **TEST GAP** | Validador avisa em WiFi; o daemon protege pelo `WifiBlockGuard` a sondar o **host** | ⚠️ nenhum no daemon | parcial |
| `limits.pixels_per_universe` | **HONORED** | `linear_assignments` + `Transport::pixels_per_datagram` | `os_universos_do_artnet_sao_consecutivos_a_partir_do_primeiro` | não |
| `limits.max_pixels` | **HONORED** | `from_profile` recusa show maior que o nó | `um_show_maior_que_o_no_e_recusado` | não |
| `limits.refresh_hz` | **IGNORED-UNINTENTIONALLY** | **Zero consumidores no workspace.** `--tick-ms` é do operador | — | não |
| `transport.mtu_bytes` | **HONORED** | `pixels_per_datagram` → `DdpOutput::with_limits` | `mtus_diferentes_produzem_fragmentacoes_diferentes…` | não |
| `transport.heartbeat_ms` | **HONORED** | `from_profile` recusa heartbeat fora do teto do GOSL | `um_heartbeat_inseguro_impede_a_saida` | não |
| `calibration.gamma` | **HONORED** | `CalibrationLut` no `OutputManager` (ADR-0019 Emenda 1) | `gamma_chega_ao_fio_nos_tres_protocolos` | não |
| `calibration.brightness` | **HONORED** | idem | `brightness_chega_ao_fio_nos_tres_protocolos` | não |
| `power.voltage_v` | **METADATA** | ADR-0018 §109: *"declarativo, **não** é proteção elétrica"*. Validador rejeita `NaN`/≤0 | validador | não |
| `power.max_current_a` | **METADATA** | idem. A proteção real é a fonte e o ABL do controlador | validador | não |
| `identity.vendor` / `model` | **METADATA** | Diagnóstico: journal (`notice:profile`) e mensagens de erro | — | não |
| `identity.firmware` | **METADATA** | Diagnóstico | — | não |
| `identity.firmware_version` | **HARDWARE-DEPENDENT** | Ninguém compara com o `ver` real do nó | — | **sim** |
| `identity.serial` | **HARDWARE-DEPENDENT** | Sem consumidor; só um nó real o pode confirmar | — | **sim** |
| `schema_version` | **HONORED** (no validador) | `validate` rejeita schema desconhecida — **mas o daemon não chama `validate`** | testes do validador | não |

## Achados

### A1 — `supports_discovery` ignorado · **CORRIGIDO**

Dois presets declaram `supports_discovery: false`. O pré-voo sondava sempre por ArtPoll, e
concluiria `devices_missing` → `preflight_failed` → **o daemon recusaria tocar** um nó que se
comporta exatamente como declarou. É o inverso do propósito do RT-003: a descoberta protege
contra **palco escuro**, não contra **shows que não arrancam**.

Corrigido em `preflight.rs`: quando o nó declara que não responde, não é sondado, e o
resultado é `devices_unverified` — o caminho que já existia para *"não foi possível
concluir"*. Falsificado (`if false` → reprova), com controle negativo que garante que o gate
do RT-003 continua a valer para nós que **declaram** responder.

### A2 — O daemon nunca chama `validate()` · **BLOCKED — ADR DECISION**

**Problema.** O validador do ADR-0018 tem 9 achados — incluindo `WifiNotPermittedLive` e
`RgbwOverDdpDataType` (o data type RGBW do DDP **não é validado em hardware**, registado em
2026-07-30). `grep validate crates/led-daemon-bin/src/` devolve **zero**. Um preset com
qualquer um desses problemas arranca sem uma palavra.

**Porque não corrigi.** `validate(profile, &Available{interfaces, protocols})` recebe a lista
de drivers disponíveis por **injeção de dado** — é isso que mantém o `led-hardware-profile`
leaf (ADR-0018, slice 2). Decidir **quem monta essa lista no daemon** é arquitetura:

| Opção | Custo |
|---|---|
| O `OutputManager` declara os três protocolos que sabe construir | Acopla a lista ao sítio que já a conhece; risco de divergir do `match` |
| Derivar de `OutputProtocol::ALL` | Exige um `ALL` que hoje não existe — superfície nova |
| O binário monta e injeta | Mais uma coisa que a CLI tem de saber |

**Impacto.** Médio: hoje o catálogo é curado e nenhum preset viola. O risco cresce quando
alguém acrescentar uma linha (que é, por desenho, como se acrescenta hardware).

**Reversibilidade.** Alta — é aditivo, e recusar vs avisar é uma linha.

**Recomendação.** Primeira opção, com o veredito no journal: **erro** reprova o pré-voo,
**aviso** fica registado e prossegue (a mesma hierarquia que o `preflight` já usa). Mas é
decisão, não edição.

### A3 — `refresh_hz` sem consumidor · **BLOCKED — ADR DECISION**

Declarado nos `Limits` como *"único lar dos limites"* (ADR-0018) e **sem nenhum leitor**. O
`--tick-ms` é escolhido pelo operador: um preset a declarar 40 Hz com um daemon a 200 Hz
sobrecarrega o nó sem aviso.

Cruzar os dois é **comportamento novo** e nenhum ADR o define: recusar? avisar? clampar? Um
show a 200 Hz num nó de 40 Hz pode ser um erro de digitação ou um teste deliberado de
throughput (precedente: o sweep de 1593 fps de 2026-07-23). Fica registado.

### A4 — `output_interface` · **TEST GAP declarado**

A proteção do ADR-0005 existe no daemon, mas vem do `WifiBlockGuard` a sondar o **host** — não
da declaração do profile. As duas podem divergir (profile diz WiFi, host está em Ethernet, ou
o inverso) e hoje ninguém repara. **Não escrevi teste**: fixaria uma semântica que nunca foi
decidida. Depende de A2.

### A5 — `firmware_version` / `serial` · **HARDWARE-DEPENDENT**

O `/json/info` do WLED devolve `ver`, e o `lumyx-hwcheck` já o lê e imprime — mas **ninguém o
compara** com o que o profile declara. Escrever o comparador agora produziria um checker que
nunca correu contra um controlador real.

**Evidência que o GS4.5 deve recolher:** o `ver` e o `arch` reais do nó, para confrontar com
`identity.firmware_version`. Se divergirem, decidir se é erro ou aviso — hoje não há dado
para escolher.

## O que esta auditoria **não** prova

Que os campos HONORED produzem o efeito **físico** correto. Tudo o que está provado descreve
o que sai do daemon e o que o profile decide; nada descreve o que o controlador faz com isso.
