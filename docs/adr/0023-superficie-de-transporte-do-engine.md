# ADR-0023 — Superfície de transporte do engine (estado de show em runtime)

- **Status:** 🟢 **aceito** — implementado em `crates/led-daemon` nesta fatia (GS1)
- **Data:** 2026-08-05
- **Exigido por:** `docs/architecture/control-protocol.md` §Fora de escopo — *"a superfície de
  transporte do engine (play/pause/seek — não existe; precisa de **ADR próprio** ... antes de
  qualquer tela de transporte"*

## Contexto e problema

O `led-player` toca um `.lumyx` **linearmente do início ao fim**; os `stop()` do codebase são
desligamento de thread, não transporte. Não existe **noção de posição de reprodução em
runtime**, nem estado de show comandável. A análise de lacuna do `control-protocol.md`
mediu isso: de tudo que um console vai querer comandar, **o engine não tem nada de
transporte**.

Sem essa superfície, o control-plane (ADR-0014) não tem o que comandar e a FASE D não começa.

## Decisão

### 1. O estado de show em runtime é uma **máquina de estados explícita e determinística**

Oito estados, nomeados. Nenhum outro é representável — `State` é um enum fechado, e a única
forma de mudar de estado é `ShowRuntime::apply`.

| Estado | Significado |
|---|---|
| `Idle` | Nenhum show carregado |
| `Loaded` | Artefato carregado; **pré-condições não verificadas** |
| `Ready` | Pré-voo aprovado — **armado**, seguro para tocar |
| `Playing` | Transporte a correr |
| `Paused` | Transporte parado, **posição preservada** |
| `Stopped` | Transporte parado, **posição em zero** |
| `Finished` | Chegou ao fim naturalmente |
| `Error` | Falha de runtime registada, com código |

**Por que `Loaded` e `Ready` são estados distintos.** É onde os gates de pré-voo que já
existem passam a ter lugar no ciclo de vida: verificação de integridade (`--verify <hash>`),
`Hal::check_network()` (regra WiFi do ADR-0005) e discovery pré-show (`--require-all`).
Carregar não é estar pronto, e fundir os dois estados apagaria a distinção — que é
exatamente a que impede um show de começar com um controlador ausente ou sobre WiFi.

### 2. O tempo é **injetado**, nunca lido de dentro

`apply(cmd, now_ms)`. A máquina nunca chama o relógio. Consequências:

- **Determinismo**: a mesma sequência de `(comando, now_ms)` produz sempre o mesmo estado.
- **Testabilidade sem espera**: nenhum teste dorme — a lição do TD-003, onde 8
  `thread::sleep` viraram barreiras causais.
- Um relógio que anda para trás é **clampado**, não pânico (precedente do `SharedClock`).

### 3. `Stop` e `Pause` **não apagam o palco**

O transporte controla **o avanço do tempo**, não a saída. Em `Paused`, `Stopped` e
`Finished` o heartbeat continua a reenviar o último frame válido, e o rig continua aceso.

Isto é a mesma separação que o setor faz — em ETC Eos e ChamSys MagicQ, blackout é **máscara
de saída** e o transporte é coisa separada (ver [anexo do ADR-0017](0017-anexo-analise-e-proposta.md)).
**Apagar é blackout, e blackout está bloqueado pelo ADR-0017.** Esta máquina não tem, e não
pode ganhar, qualquer comando que zere saída.

### 4. `Play` a partir de `Finished` é **recusado**

O caminho para repetir é explícito: `Stop` (ou `Seek`) e depois `Play`.

Rebobinar implicitamente é a classe de surpresa que faz um show **recomeçar no palco** com um
toque acidental. Recusa previsível é melhor que conveniência silenciosa. Segue a disciplina
do ADR-0018 ("o componente declara, a camada com contexto decide").

### 5. `Stop` põe a posição em **zero**; `Pause` **preserva**

Alinhado com o xLights, onde o Stop repõe a posição no início.

### 6. Comando inválido é **recusado com motivo tipado**, nunca ignorado

`apply` devolve `Result<Vec<Event>, Rejected>`. `Rejected` é um enum — códigos enumerados,
nunca string livre, como o modelo de erro do `control-protocol.md` já exige.
**Uma recusa nunca muda o estado.**

### 7. Pré-voo é **dado injetado**, não executado aqui

`Command::Arm(PreflightReport)` recebe o **veredito**. A máquina não sabe verificar hash nem
sondar rede — quem sabe é o daemon. Mantém o crate **leaf e sem dependências**, na disciplina
de injeção de dado do ADR-0018.

## Escopo / Não-escopo

- **Escopo:** modelo de estado, ciclo de vida, transições, comandos, eventos, API pública.
- **Não-escopo:** transporte IPC (GS2/ADR-0014) · UI · blackout (ADR-0017) · grand master ·
  troca de calibração ao vivo · o *processo* daemon em si.

## Alternativas descartadas

- **Estado implícito por flags** (`is_playing: bool`, `is_paused: bool`) — permite
  representar `playing && paused`. Um enum torna-o **irrepresentável**.
- **Ler o relógio dentro da máquina** — mataria o determinismo e obrigaria testes a dormir.
- **`Resume` como comando próprio** — `Play` a partir de `Paused` já é retomar; um comando a
  mais é uma linha a mais na matriz sem semântica nova.

## Consequências

- O control-plane passa a ter **o que comandar** — desbloqueia GS2 (IPC).
- A matriz `estado × comando` é **finita e exaustivamente testável** (8 × 10 = 80 pares).
- `led-core` **intocado**: zero bump, nenhum contrato `Frozen` alterado.

## Critério de reversão

Se o transporte precisar de estados concorrentes (ex.: duas timelines independentes), esta
máquina de instância única deixa de servir e o modelo passa a ser por-*deck*. O gatilho é
**um segundo show tocando ao mesmo tempo** — não existe hoje e não está no roadmap.
