# ADR-0025 — `refresh_hz` é um limite, e o daemon recusa ultrapassá-lo

- **Estado:** aceite
- **Data:** 2026-08-07
- **Contexto:** achado A3 da auditoria de 2026-08-07 ([hardware-profile-audit.md](../architecture/hardware-profile-audit.md))
- **Relacionados:** ADR-0018 (`Limits`), ADR-0024 (fronteira de validação), ADR-0023 (transporte, **congelado**)

## Contexto e problema

`HardwareProfile.limits.refresh_hz` é declarado por todos os presets e **não tem consumidor
nenhum** em todo o workspace. O `--tick-ms` do daemon é escolhido pelo operador e nunca é
confrontado com ele: um preset a declarar 40 Hz com um daemon a ticar a 100 Hz sobrecarrega o
nó **sem uma palavra**.

Um campo declarado que ninguém honra é a mesma classe de defeito que o `RgbOrder` do GS4.3 e
a `Calibration` do ADR-0019 Emenda 1: *parece* configuração, e não é.

## Inventário (verificado, não presumido)

| Campo | Origem | Consumidor | Efeito | Estado antes deste ADR |
|---|---|---|---|---|
| `refresh_hz` | `HardwareProfile.limits` | **nenhum** | nenhum | **GAP** |
| `heartbeat_ms` | `HardwareProfile.transport` | `OutputConfig::from_profile`, `Stage` | período do keep-alive; fora do teto do GOSL recusa a saída | HONORED |
| `tick_ms` | CLI (`--tick-ms`) | `run` / `run_with_control` | período do laço, prazos absolutos | HONORED |

## As três perguntas

### A — `>` capacidade: rejeitar, avisar ou clampar?

**Decisão: rejeitar.** Duas evidências, nenhuma intuição.

1. **O campo irmão já rejeita.** `max_pixels` vive na **mesma struct `Limits`** e um show maior
   que o nó declarado é recusado na construção da saída desde o GS4.3
   (`um_show_maior_que_o_no_e_recusado`). Tratar `refresh_hz` de outra maneira exigiria
   justificação; tratá-lo igual não exige nenhuma.
2. **Clampar em silêncio está proibido por precedente.** O ADR-0018 fixou que *"o componente
   declara, a camada com contexto decide"*, e o `Strobe` do ADR-0021 recusa-se explicitamente
   a clampar a frequência: *"estroboscópio que muda de frequência sozinho no palco é pior que
   parâmetro documentado"*. Um daemon que baixasse o `--tick-ms` sozinho mentiria sobre a
   cadência que o operador pediu, e o journal registaria uma coisa e o fio faria outra.

### B — capacidade máxima ou frequência nominal?

**Decisão: capacidade máxima.** O ADR-0018 chama à struct que o contém *"`Limits` ← **ÚNICO
lar dos limites**"*. `refresh_hz` está lá dentro, ao lado de `max_pixels` e
`pixels_per_universe`, que são tetos. Lê-lo como "recomendação" seria dar-lhe uma semântica
diferente da dos seus dois vizinhos, na mesma struct, sem nada que o justifique.

**Consequência assumida:** os números dos presets são *"pontos de partida por família, **não
medições**"* (ADR-0018). Um teto errado passa a bloquear em vez de só informar. A correção é
uma linha no `presets.rs` — que é, por desenho, como se ajusta hardware.

### C — bloquear o show mas permitir benchmark?

**Decisão: não existe modo de benchmark no daemon — e não é preciso um.**

A pergunta nasce de uma evidência real: em 2026-07-23 correu-se um sweep de throughput até
**1593 fps** contra um nó que declara 44 Hz. Rejeitar cegamente teria impedido essa medição.

Mas o sweep **não foi feito pelo daemon**. `--speed max` é uma flag do `led-player`
(`main.rs:112`); o daemon não tem conceito de velocidade — tem `--tick-ms`. **A separação que
a pergunta pede já existe, e existe por binário:**

| Binário | Papel | `refresh_hz` |
|---|---|---|
| `led-daemon` | **o caminho do show** | limite duro — recusa |
| `led-player --speed max` | reprodução e sweep de throughput | não se aplica |
| `lumyx-hwcheck` | medição do GS4.5 | não se aplica |

Acrescentar um `--benchmark` ao daemon criaria **superfície nova** para um caso que outro
binário já cobre — e criaria um caminho pelo qual o show poderia arrancar com o limite
desligado. A regra "nunca transformar benchmark em capacidade de show" fica garantida **por
construção**: o binário que faz show não sabe fazer benchmark.

## Decisão

Antes de abrir o palco, com `--output` configurado:

```
requested_hz = 1000 / tick_ms          (ms inteiros ⇒ taxa exata)
requested_hz  >  refresh_hz            ⇒ RECUSA, daemon termina em NeverStarted
requested_hz ==  refresh_hz            ⇒ permitido (o limite é alcançável, não proibido)
requested_hz  <  refresh_hz            ⇒ permitido
```

A comparação corre **uma vez, na abertura do palco** — no mesmo sítio e com o mesmo desfecho
da validação do ADR-0024. **Nunca no laço**: não há relógio novo, não há segundo scheduler, e
o `Pacer` não muda.

**`refresh_hz == 0` já é recusado** pelo validador (`ZeroLimit`) desde o ADR-0018, e desde o
ADR-0024 essa recusa impede a saída de abrir. Zero **não pode** virar capacidade infinita, e
isso não precisa de código novo — precisa de um teste que o prove, que este ADR exige.

**Sem `--output` não há verificação**, porque não há nó: um daemon sem saída pode ticar à
velocidade que quiser. É o mesmo raciocínio da vacuidade do pré-voo no GS2.

## Alternativas rejeitadas

| Alternativa | Porque não |
|---|---|
| Clampar `tick_ms` ao limite | Muda em silêncio o que o operador pediu; contradiz o precedente do `Strobe` (ADR-0021) |
| Só avisar | `max_pixels`, na mesma struct, rejeita. Duas semânticas para dois tetos irmãos é o que a auditoria A3 veio corrigir |
| `--benchmark` no daemon | Superfície nova para um caso que o `led-player` já cobre, e um caminho pelo qual o show arrancaria sem limite |
| Tolerância percentual | Nenhum dado justifica a percentagem. Inventá-la seria comportamento não especificado |

## Invariantes

1. Um `--tick-ms` acima da capacidade declarada **nunca** produz um show a tocar.
2. O daemon **nunca** altera o `tick_ms` pedido.
3. A verificação não introduz relógio, thread ou caminho de envio novo.
4. `refresh_hz = 0` nunca é lido como ilimitado.

## Critérios de aceitação

- Dentro do limite (30 ≤ 40) permitido; **no limite exato** (40 = 40) permitido; acima (50 > 40) recusado.
- A recusa chega ao journal e produz `NeverStarted`.
- Sem `--output`, nenhuma verificação.
- `refresh_hz = 0` recusa a saída (por `ZeroLimit`, ADR-0024).

## Migração

Nenhum preset do catálogo muda. Um operador que hoje corra `--tick-ms 10` contra um nó de
44 Hz **passa a ser recusado** — e é essa a correção. A saída é escolher um `--tick-ms`
compatível (23 ms → 43,5 Hz) ou corrigir o teto do preset, se ele estiver errado.

## Critério de reversão

Se um nó real aceitar sustentadamente mais que o seu `refresh_hz` declarado, a correção é o
**número do preset** — não desligar a verificação. Desligá-la devolve o sistema ao estado que
este ADR corrigiu.
