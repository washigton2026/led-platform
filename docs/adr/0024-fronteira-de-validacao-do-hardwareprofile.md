# ADR-0024 — A fronteira de validação do `HardwareProfile`

- **Estado:** aceite
- **Data:** 2026-08-07
- **Contexto:** achado A2 da auditoria de 2026-08-07 ([hardware-profile-audit.md](../architecture/hardware-profile-audit.md))
- **Relacionados:** ADR-0018 (o profile e o validador), ADR-0005 (WiFi proibido), ADR-0023 (contrato do transporte, **congelado** na GS1.6)

## Contexto e problema

O ADR-0018 entregou `validate(profile, &Available{interfaces, protocols})` com 9 achados —
incluindo `WifiNotPermittedLive` e `RgbwOverDdpDataType` (data type **não validado em
hardware**, registado em 2026-07-30).

A auditoria de 2026-08-07 encontrou que **o daemon nunca o chama**:
`grep validate crates/led-daemon-bin/src/` devolve zero. Um preset com RGBW sobre DDP, com
schema desconhecida ou com pixels que não cabem no universo arranca sem uma palavra.

O catálogo é curado hoje e nenhum preset viola. Mas o ADR-0018 decidiu que **acrescentar
hardware é acrescentar uma linha** — e a linha seguinte pode violar. Um validador sem
consumidor é uma verificação que não verifica.

## As duas perguntas que este ADR responde

### 1. Onde é que a validação acontece, sem tocar no contrato congelado?

O `PreflightReport` do ADR-0023 tem **três campos** — `integrity_verified`, `network_ok`,
`devices_present` — e está **congelado desde a GS1.6**. Nenhum deles significa "o profile é
inválido", e acrescentar um quarto seria alterar o contrato que atravessou GS2, GS3 e GS4
sem mudar.

**Decisão: a validação estática acontece na construção da saída** (`OutputConfig::resolve`),
que no laço já corre **antes** do pré-voo e do `Arm`:

```
load  →  abrir_palco (resolve → VALIDATE)  →  preflight  →  Arm  →  Ready  →  Play
             │
             └── Error ⇒ output_failed ⇒ NeverStarted    (nunca chega a Ready)
```

Um profile com erro **não abre saída**, e o daemon devolve `NeverStarted` — exatamente o
mesmo caminho que um endereço inválido já usava desde a GS4.2. **Zero alterações ao
`led-daemon`, zero alterações ao `led-core`.**

Corolário deliberado: **um profile inválido nunca é `Loaded` com saída**. O show pode ser
carregado (o `.lumyx` não tem culpa do preset), mas o palco não abre.

### 2. Quem fornece o `Available{}`?

O `Available` existe porque detectar *"não há driver para isto"* exigiria o
`led-hardware-profile` conhecer os drivers — o que o faria deixar de ser leaf (ADR-0018,
slice 2). A lista chega como **dado**.

**Decisão: o `OutputManager` fornece-a**, porque é o único sítio do projeto que sabe quais
protocolos consegue construir — é lá que vive o `match cfg.protocol`.

Para que a lista **não possa divergir** do `match`, ela é derivada de
`OutputProtocol::ALL`, e há um teste que obriga `ALL` a cobrir todas as variantes por
`match` exaustivo — o compilador reprova quem acrescentar uma variante e esquecer a lista.

Interfaces com driver: **`Ethernet` e `WiFi`**. As duas são UDP sobre IP e o daemon fala com
ambas; `Spi` e `Pwm` não têm driver (ADR-0018 di-lo explicitamente). **WiFi continua
declarável e continua a gerar `Warning`** — o bloqueio ao vivo é do `WifiBlockGuard`, no
pré-voo, contra a interface **do host**. Recusar WiFi aqui seria mover o enforcement do
ADR-0005 para o sítio errado e duplicá-lo.

## Decisão

1. **`Severity::Error` recusa a saída.** Nenhum profile com erro produz `OutputManager`.
2. **`Severity::Warning` é registado e prossegue.** É a mesma hierarquia que o pré-voo já usa
   para `network_unverified`/`devices_unverified`: avisar não é aprovar, e bloquear um aviso
   impediria o `esp32-poe-wled-rgbw-ddp` (que avisa por desenho) de existir.
3. **A validação é estática.** Não consulta a rede, não consulta o nó, não depende de
   hardware — corre igual num CI sem rig.
4. **A validação dependente de hardware continua onde está**: WiFi ativo e presença de
   controladores são do `preflight`; firmware e pixels reais são do GS4.5.
5. **O erro chega ao cliente IPC** pelo caminho que já existe: `load` com `--output`
   configurado devolve `load_error` com o texto do achado. Nenhum código de protocolo novo.

## Alternativas rejeitadas

| Alternativa | Porque não |
|---|---|
| Quarto campo em `PreflightReport` | Altera o contrato congelado na GS1.6 e mistura validação **estática** com **runtime** — as duas categorias que este ADR separa |
| Validar no `main.rs` ao ler `--profile` | Deixaria o caminho IPC (`load`) sem validação: dois caminhos, um validado |
| Validar dentro do `led-hardware-profile` sem `Available` | Faria o crate conhecer os drivers e deixar de ser leaf (contradiz o ADR-0018) |
| Recusar `Warning` também | Bloquearia presets legítimos que avisam por desenho; e mudaria o significado de `Warning` fixado no ADR-0018 |

## Consequências

**Positivas.** O validador passa a ter consumidor. Um preset novo inválido é apanhado na
primeira execução, não no palco. A separação estático/runtime fica explícita e testável sem
rig.

**Negativas.** A validação corre a cada abertura de palco (incluindo a cada `load` por IPC).
É `O(campos)` sobre uma struct pequena, no startup, **nunca no hot path** — a mesma disciplina
do ADR-0019.

**Não coberto.** Que os valores declarados correspondam ao hardware real. `firmware_version`
e `serial` continuam `HARDWARE-DEPENDENT`; a evidência é do GS4.5.

## Critério de reversão

Se um preset legítimo passar a ser recusado por um `Error` que se revele demasiado estrito, a
correção é na **regra do validador** (ADR-0018), não em desligar a chamada. Desligar a
chamada devolve o sistema ao estado que esta ADR corrigiu.
