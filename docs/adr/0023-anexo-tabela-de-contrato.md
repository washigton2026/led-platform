# ADR-0023 — Anexo: auditoria de contrato e tabela completa (GS1.5)

**Data:** 2026-08-05 · **Nenhuma alteração à máquina de estados.**
O único código novo é `crates/led-daemon/examples/contract_table.rs`, o **gerador** desta
tabela — ferramenta de documentação, não gate e não produção.

> **Veredito: o contrato ainda NÃO deveria ser congelado.** A máquina está correta e os 80
> pares comportam-se como declarado, mas a auditoria achou **quatro divergências de
> contrato** (§3). Nenhuma é um bug de estado — são inconsistências na *superfície observável*
> (eventos e códigos de recusa). Congelar agora **serializa-as no fio** no GS3, e mudá-las
> depois custa versão de protocolo. Detalhe e recomendação em §3 e §5.

---

## 1. Auditoria das cinco propriedades

### (a) Todos os comandos possuem semântica única — ✅ **com uma ressalva**

`Play` é aceite a partir de `Ready`, `Paused` e `Stopped`. Parece sobrecarga ("começar" vs
"retomar"), mas **não é**: a semântica é uma só — *"passar a avançar o tempo a partir da
posição corrente"*. O que muda entre os três é a posição, que já era o que era. É por isso
que não existe `Resume` (ADR-0023, alternativas descartadas).

`Tick` **não** tem semântica única — ver **F1** em §3.

### (b) Todos os eventos possuem origem única — ❌ **falha em `PositionChanged`**

| Evento | Origens | Inequívoco? |
|---|---|---|
| `Transitioned{from,to}` | 9 sítios | ✅ **sim** — ver abaixo |
| `ShowLoaded` | `load` | ✅ |
| `ShowUnloaded` | `unload` | ✅ |
| `ReachedEnd` | `tick` | ✅ |
| `Faulted` | `fault` | ✅ |
| `FaultCleared` | `clear_fault` | ✅ |
| **`PositionChanged{ms}`** | **`seek`, `pause`, `stop`, `tick`** | ❌ **não** — ver **F2** |

**Por que `Transitioned` passa apesar de 9 origens.** O gerador verifica a injetividade: cada
par `(from→to)` é produzido por **exatamente um** comando. O consumidor deduz a causa do par,
sem precisar de campo extra. Verificado por execução, não por leitura.

### (c) Nenhuma transição depende de tempo implícito — ✅

Verificação mecânica: `grep -n "Instant\|SystemTime\|now()\|elapsed()" src/lib.rs` devolve
**apenas um comentário**. A máquina não tem acesso a relógio; o único tempo é o `now_ms`
injetado em `apply`.

**Precisão que importa:** `apply` **não** é função pura de `(estado, comando, now_ms)` — a
guarda de monotonicidade (`now_ms.max(last_now_ms)`) faz o resultado depender também do
histórico de tempos vistos. O ADR-0023 §2 já está redigido corretamente ("a mesma
**sequência** de `(comando, now_ms)`"), mas quem implementar o IPC precisa de saber: replay de
um comando isolado, fora da sequência, pode divergir.

### (d) Nenhum estado é inalcançável — ✅

| Estado | Como se chega |
|---|---|
| `Idle` | inicial · `unload` de 6 estados |
| `Loaded` | `load` (de `Idle`) · `clear_fault` (de `Error`) |
| `Ready` | `arm` (de `Loaded`/`Stopped`/`Finished`/`Ready`) |
| `Playing` | `play` (de `Ready`/`Paused`/`Stopped`) |
| `Paused` | `pause` (de `Playing`) |
| `Stopped` | `stop` (de `Playing`/`Paused`/`Finished`) |
| `Finished` | `tick` (de `Playing`, ao atingir a duração) |
| `Error` | `fault` (de 6 estados) |

Provado **por construção**: `runtime_in()` constrói os oito e o gate afirma o estado obtido.

### (e) Nenhum estado é terminal por acidente — ✅

Todos os oito têm saída. `Error` é **absorvente por decisão** (ADR-0023), não por acidente, e
tem duas saídas explícitas: `clear_fault → Loaded` e `unload → Idle`. `Finished` tem três
(`arm`, `stop`, `unload`) — não é terminal, apesar do nome.

---

## 2. Tabela completa — 80 pares

**Gerada executando a máquina de produção**, não escrita à mão:

```sh
cargo run -p led-daemon --example contract_table
```

<!-- GERADO por `cargo run -p led-daemon --example contract_table`. Não editar à mão. -->

| # | Estado atual | Comando | Resultado | Evento(s) emitido(s) | Próximo estado |
|--:|---|---|---|---|---|
| 1 | `idle` | `load` | ✅ aceite | Transitioned(idle→loaded) · ShowLoaded(42) | `loaded` |
| 2 | `idle` | `unload` | ❌ `no_show_loaded` | *(nenhum)* | `idle *(inalterado)*` |
| 3 | `idle` | `arm` | ❌ `no_show_loaded` | *(nenhum)* | `idle *(inalterado)*` |
| 4 | `idle` | `play` | ❌ `no_show_loaded` | *(nenhum)* | `idle *(inalterado)*` |
| 5 | `idle` | `pause` | ❌ `not_applicable` | *(nenhum)* | `idle *(inalterado)*` |
| 6 | `idle` | `stop` | ❌ `no_show_loaded` | *(nenhum)* | `idle *(inalterado)*` |
| 7 | `idle` | `seek` | ❌ `no_show_loaded` | *(nenhum)* | `idle *(inalterado)*` |
| 8 | `idle` | `tick` | ✅ aceite | *(nenhum)* | `idle` |
| 9 | `idle` | `fault` | ❌ `no_show_loaded` | *(nenhum)* | `idle *(inalterado)*` |
| 10 | `idle` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `idle *(inalterado)*` |
| 11 | `loaded` | `load` | ❌ `show_already_loaded` | *(nenhum)* | `loaded *(inalterado)*` |
| 12 | `loaded` | `unload` | ✅ aceite | Transitioned(loaded→idle) · ShowUnloaded(42) | `idle` |
| 13 | `loaded` | `arm` | ✅ aceite | Transitioned(loaded→ready) | `ready` |
| 14 | `loaded` | `play` | ❌ `not_armed` | *(nenhum)* | `loaded *(inalterado)*` |
| 15 | `loaded` | `pause` | ❌ `not_applicable` | *(nenhum)* | `loaded *(inalterado)*` |
| 16 | `loaded` | `stop` | ❌ `not_applicable` | *(nenhum)* | `loaded *(inalterado)*` |
| 17 | `loaded` | `seek` | ✅ aceite | PositionChanged(1000) | `loaded` |
| 18 | `loaded` | `tick` | ✅ aceite | *(nenhum)* | `loaded` |
| 19 | `loaded` | `fault` | ✅ aceite | Transitioned(loaded→error) · Faulted(device_lost) | `error` |
| 20 | `loaded` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `loaded *(inalterado)*` |
| 21 | `ready` | `load` | ❌ `show_already_loaded` | *(nenhum)* | `ready *(inalterado)*` |
| 22 | `ready` | `unload` | ✅ aceite | Transitioned(ready→idle) · ShowUnloaded(42) | `idle` |
| 23 | `ready` | `arm` | ✅ aceite | Transitioned(ready→ready) | `ready` |
| 24 | `ready` | `play` | ✅ aceite | Transitioned(ready→playing) | `playing` |
| 25 | `ready` | `pause` | ❌ `not_applicable` | *(nenhum)* | `ready *(inalterado)*` |
| 26 | `ready` | `stop` | ❌ `not_applicable` | *(nenhum)* | `ready *(inalterado)*` |
| 27 | `ready` | `seek` | ✅ aceite | PositionChanged(1000) | `ready` |
| 28 | `ready` | `tick` | ✅ aceite | *(nenhum)* | `ready` |
| 29 | `ready` | `fault` | ✅ aceite | Transitioned(ready→error) · Faulted(device_lost) | `error` |
| 30 | `ready` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `ready *(inalterado)*` |
| 31 | `playing` | `load` | ❌ `show_already_loaded` | *(nenhum)* | `playing *(inalterado)*` |
| 32 | `playing` | `unload` | ❌ `not_applicable` | *(nenhum)* | `playing *(inalterado)*` |
| 33 | `playing` | `arm` | ❌ `not_applicable` | *(nenhum)* | `playing *(inalterado)*` |
| 34 | `playing` | `play` | ❌ `not_applicable` | *(nenhum)* | `playing *(inalterado)*` |
| 35 | `playing` | `pause` | ✅ aceite | Transitioned(playing→paused) · PositionChanged(0) | `paused` |
| 36 | `playing` | `stop` | ✅ aceite | Transitioned(playing→stopped) · PositionChanged(0) | `stopped` |
| 37 | `playing` | `seek` | ✅ aceite | PositionChanged(1000) | `playing` |
| 38 | `playing` | `tick` | ✅ aceite | PositionChanged(0) | `playing` |
| 39 | `playing` | `fault` | ✅ aceite | Transitioned(playing→error) · Faulted(device_lost) | `error` |
| 40 | `playing` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `playing *(inalterado)*` |
| 41 | `paused` | `load` | ❌ `show_already_loaded` | *(nenhum)* | `paused *(inalterado)*` |
| 42 | `paused` | `unload` | ✅ aceite | Transitioned(paused→idle) · ShowUnloaded(42) | `idle` |
| 43 | `paused` | `arm` | ❌ `not_applicable` | *(nenhum)* | `paused *(inalterado)*` |
| 44 | `paused` | `play` | ✅ aceite | Transitioned(paused→playing) | `playing` |
| 45 | `paused` | `pause` | ❌ `not_applicable` | *(nenhum)* | `paused *(inalterado)*` |
| 46 | `paused` | `stop` | ✅ aceite | Transitioned(paused→stopped) · PositionChanged(0) | `stopped` |
| 47 | `paused` | `seek` | ✅ aceite | PositionChanged(1000) | `paused` |
| 48 | `paused` | `tick` | ✅ aceite | *(nenhum)* | `paused` |
| 49 | `paused` | `fault` | ✅ aceite | Transitioned(paused→error) · Faulted(device_lost) | `error` |
| 50 | `paused` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `paused *(inalterado)*` |
| 51 | `stopped` | `load` | ❌ `show_already_loaded` | *(nenhum)* | `stopped *(inalterado)*` |
| 52 | `stopped` | `unload` | ✅ aceite | Transitioned(stopped→idle) · ShowUnloaded(42) | `idle` |
| 53 | `stopped` | `arm` | ✅ aceite | Transitioned(stopped→ready) | `ready` |
| 54 | `stopped` | `play` | ✅ aceite | Transitioned(stopped→playing) | `playing` |
| 55 | `stopped` | `pause` | ❌ `not_applicable` | *(nenhum)* | `stopped *(inalterado)*` |
| 56 | `stopped` | `stop` | ❌ `not_applicable` | *(nenhum)* | `stopped *(inalterado)*` |
| 57 | `stopped` | `seek` | ✅ aceite | PositionChanged(1000) | `stopped` |
| 58 | `stopped` | `tick` | ✅ aceite | *(nenhum)* | `stopped` |
| 59 | `stopped` | `fault` | ✅ aceite | Transitioned(stopped→error) · Faulted(device_lost) | `error` |
| 60 | `stopped` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `stopped *(inalterado)*` |
| 61 | `finished` | `load` | ❌ `show_already_loaded` | *(nenhum)* | `finished *(inalterado)*` |
| 62 | `finished` | `unload` | ✅ aceite | Transitioned(finished→idle) · ShowUnloaded(42) | `idle` |
| 63 | `finished` | `arm` | ✅ aceite | Transitioned(finished→ready) | `ready` |
| 64 | `finished` | `play` | ❌ `not_applicable` | *(nenhum)* | `finished *(inalterado)*` |
| 65 | `finished` | `pause` | ❌ `not_applicable` | *(nenhum)* | `finished *(inalterado)*` |
| 66 | `finished` | `stop` | ✅ aceite | Transitioned(finished→stopped) · PositionChanged(0) | `stopped` |
| 67 | `finished` | `seek` | ✅ aceite | PositionChanged(1000) | `finished` |
| 68 | `finished` | `tick` | ✅ aceite | *(nenhum)* | `finished` |
| 69 | `finished` | `fault` | ✅ aceite | Transitioned(finished→error) · Faulted(device_lost) | `error` |
| 70 | `finished` | `clear_fault` | ❌ `not_applicable` | *(nenhum)* | `finished *(inalterado)*` |
| 71 | `error` | `load` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 72 | `error` | `unload` | ✅ aceite | Transitioned(error→idle) · ShowUnloaded(42) | `idle` |
| 73 | `error` | `arm` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 74 | `error` | `play` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 75 | `error` | `pause` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 76 | `error` | `stop` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 77 | `error` | `seek` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 78 | `error` | `tick` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 79 | `error` | `fault` | ❌ `in_error_state` | *(nenhum)* | `error *(inalterado)*` |
| 80 | `error` | `clear_fault` | ✅ aceite | Transitioned(error→loaded) · FaultCleared | `loaded` |

**80 pares** — 8 estados × 10 comandos.

## Sinais extraídos da execução

- **`PositionChanged` tem 4 origens distintas:** `seek`, `pause`, `stop`, `tick`. Um consumidor que receba só o evento **não distingue** um avanço contínuo de um salto do operador.
- **Auto-transições que emitem `Transitioned` com `from == to`:** `ready+arm`. O consumidor recebe um evento de mudança onde nada mudou.
- **`Transitioned` é inequívoco:** cada par `(from→to)` é produzido por **exatamente um** comando — o consumidor deduz a causa sem campo extra.

---

## 3. Divergências encontradas

### F1 — `Tick` é recusado em `Error`, contra a razão declarada 🔴

**A doc do comando diz** (`src/lib.rs:232`):

> *"Só tem efeito em `Playing`; noutros estados é **aceite e inócuo**, porque o daemon tica em
> cadência fixa e **não deve ter de saber o estado** para o fazer."*

**O que a máquina faz:** aceite e inócuo em 7 estados (linha 8 da tabela: `idle + tick` →
aceite), mas **recusado em `Error`** (linha 78: `in_error_state`), porque a guarda absorvente
do `apply` só deixa passar `clear_fault` e `unload`.

**Consequência:** um daemon que tique a cadência fixa — exatamente o que a doc descreve —
recebe um **fluxo de recusas** enquanto estiver em `Error`, e passa a ter de conhecer o
estado para evitar isso. A razão declarada e o comportamento contradizem-se.

**Opções:** (a) juntar `Tick` à lista de comandos permitidos em `Error` (inócuo, coerente com
a doc); (b) manter a recusa e **corrigir a doc**, aceitando que o daemon filtre. Não decido —
mas as duas não podem coexistir.

### F2 — `PositionChanged` não distingue avanço de salto 🔴

Quatro origens: `seek`, `pause`, `stop`, `tick`. Um consumidor que receba
`PositionChanged{ms: 5000}` **não sabe** se o playhead avançou continuamente ou se o operador
saltou.

**Por que importa no GS3/GS4:** uma timeline de console precisa exatamente dessa distinção —
avanço contínuo anima o playhead, salto reposiciona. Sem ela, ou a UI infere por heurística
(diferença de tempo), ou o protocolo ganha um campo depois, já com clientes no ar.

**Opção:** `PositionChanged { ms, cause }` com `cause ∈ {Advanced, Sought, Reset}`. É aditivo
e barato **agora**; depois do GS3 é mudança de protocolo.

### F3 — Códigos de recusa inconsistentes em `Idle` 🟡

| Comando em `Idle` | Código | Causa raiz |
|---|---|---|
| `unload`, `arm`, `play`, `stop`, `seek`, `fault` | `no_show_loaded` | não há show |
| **`pause`** | **`not_applicable`** | **não há show** — mesma causa, código diferente |
| `clear_fault` | `not_applicable` | correto: depende de estar em `Error`, não de haver show |

`cmd_pause` verifica `state != Playing` antes de olhar para o show, e cai no ramo genérico.
Num protocolo cujo modelo de erro é **enumerado e consumido por máquina**, dois códigos para a
mesma causa é uma armadilha para quem escrever o cliente.

### F4 — `ready + arm` emite `Transitioned{from == to}` 🟡

Re-armar em `Ready` é legítimo (re-correr o pré-voo), mas emite um evento de **mudança onde
nada mudou** (linha 23 da tabela). É a única auto-transição que emite — verificado pelo
gerador.

**Opção:** não emitir `Transitioned` quando `from == to`, ou emitir um `Rearmed` próprio.

---

## 4. Transições redundantes ou impossíveis

- **Redundante:** apenas **F4** (`ready + arm`). Nenhuma outra auto-transição emite evento.
- **Impossível declarada no código:** `cmd_play` tem `State::Error => unreachable!()`. É de
  facto inalcançável — a guarda de `apply` filtra `Error` antes —, mas cria **acoplamento**:
  a ausência de pânico ali depende de uma guarda noutra função. Não é bug; é fragilidade a
  registar, porque um refactor que mova a guarda transforma-a em pânico em produção.
- **Nenhum par impossível na tabela:** os 80 estão definidos e verificados.

---

## 5. Recomendação

**Não congelar ainda.** Corrigir **F1** e **F2** antes do GS3, e decidir F3/F4.

O raciocínio é de custo, não de perfeição: **F1** e **F2** são a superfície que o IPC vai
serializar. Enquanto o contrato só existe em Rust, mudá-los é uma edição; depois de existir no
fio com um `v` negociado, mudá-los é versão de protocolo e migração de cliente. As quatro
correções somadas são pequenas — e é por isso que o momento é agora.

**F3** e **F4** são cosméticos hoje e ásperos depois; a decisão é sua.

Se preferir congelar como está, isso é legítimo — mas então **F1 e F2 devem entrar no ledger
de dívida** com gatilho no GS3, para não serem descobertos por quem escrever o primeiro
cliente.

---

## 6. Consistência ADR ↔ implementação

| Afirmação do ADR-0023 | Implementação | Estado |
|---|---|---|
| §1 — 8 estados, enum fechado | `State`, 8 variantes, `State::ALL` | ✅ |
| §1 — `Loaded` ≠ `Ready` (pré-voo) | `cmd_arm` gate por `PreflightReport` | ✅ |
| §2 — tempo injetado, sem relógio | verificado por `grep` | ✅ |
| §2 — relógio retrógrado clampado | `now_ms.max(last_now_ms)` | ✅ |
| §3 — `Stop`/`Pause` não apagam | nenhum comando de saída; teste dedicado | ✅ |
| §4 — `Play` de `Finished` recusado | linha 64 da tabela | ✅ |
| §5 — `Stop` zera, `Pause` preserva | linhas 36/35 | ✅ |
| §6 — recusa não muda estado | verificado nos 80 pares | ✅ |
| §7 — pré-voo é dado injetado | `Command::Arm(PreflightReport)` | ✅ |
| **doc de `Command::Tick`** | **contradiz o comportamento em `Error`** | ❌ **F1** |

**Nove de dez consistentes.** A única divergência é **F1**, e é entre a doc do comando e o
código — o corpo do ADR-0023 não a afirma, portanto o ADR **não** precisa de correção; a doc
inline precisa, ou o código.
