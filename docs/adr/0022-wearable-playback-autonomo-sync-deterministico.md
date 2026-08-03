# ADR-0022 — Traje de LED: playback autônomo com sincronização determinística

- **Status:** 🟡 **proposto** — decisão estrutural, aguarda aceite. Nenhuma linha de firmware,
  rádio ou `bake` deve ser escrita antes do aceite.
- **Data:** 2026-08-03
- **Fonte:** FASE F1 do roadmap (`docs/ROADMAP.md`) — trajes de LED para performance de dança
- **Relaciona-se com:** ADR-0001 (replay determinístico), ADR-0004 (Ed25519 com chave fixada),
  **ADR-0005 (WiFi proibido ao vivo — permanece íntegro)**, ADR-0010 (failover parcial),
  ADR-0013 (isolamento de falha), ADR-0017 (blackout — **ainda pendente**)

---

## Contexto e problema

O LUMYX hoje é uma plataforma de **streaming**: `engine → HAL → DeviceDriver → fio →
controlador → pixel`, quadro a quadro, em tempo real.

Um traje de dança **não tem fio**. E o ADR-0005 proíbe WiFi ao vivo — com medição própria
que confirma o porquê: **jitter de 31 ms** com RSSI −44 (sinal forte), e uma falha de
`sendto` a cada ~6 min no burn-in (`CLAUDE.md`, 2026-07-20 e 2026-07-23). Pela nossa
própria evidência, streaming sem fio para palco é inviável.

Isso não é um impasse: é um sinal de que **a pergunta está errada**. "Como transmitir para
um traje?" pressupõe que o traje precisa receber quadros durante o número. Não precisa.

## Decisão

**O traje toca o show sozinho, a partir de um artefato assado localmente. O que o mantém
junto com os outros não é um fluxo de dados — é um relógio comum.**

Sete decisões derivadas:

### D1 — O traje é um *player*, não um *device*

Esta é a inversão que faz o resto encaixar.

No modelo atual, um alvo de saída é um `DeviceDriver` no fim do fan-out do HAL. Um traje
**não é isso**: ele é uma instância **par** do `led-player`, com o seu próprio show em
memória e a sua própria fita local.

Consequência forte e desejável: **nenhum seam Frozen é envolvido**. `ProtocolOutput`,
`DeviceDriver`, `IDevice`, `CompiledLayout` e `UniverseData` não aparecem no caminho do
traje, porque não há rede no caminho do traje. Um traje é ao rig o que um segundo computador
tocando o mesmo filme é ao primeiro — não um monitor do primeiro.

### D2 — Sincronismo é relógio, não fluxo

Trajes não trocam quadros. Compartilham uma **base de tempo** e cada um decide, sozinho,
qual quadro corresponde a `t`. O contrato de sincronismo é, portanto, um contrato **de
tempo**, não de transporte:

```
mesmo show assado  +  mesma base de tempo  ⇒  mesmo quadro no mesmo instante
```

O LUMYX já tem as duas metades e elas nunca foram ligadas uma na outra:
`SharedClock` (monotônico, offset assinado, `now_ms() = max(anterior, atual)` —
`crates/led-hal/src/shared_clock.rs:37-95`) e `net_time`
(`TimeServer`/`measure_offset`/`best_of`/`sync_to` — `crates/led-hal/src/net_time.rs:45-146`,
offset medido a ±10 ms).

### D3 — Pacing é por tempo **absoluto**; o pacing atual é incremental e **não serve**

> **Este é o achado que justifica o ADR existir agora, e não é hipótese.**

`led_player::play_instrumented` (`crates/led-player/src/lib.rs:112-120`) espaça quadros
assim:

```rust
let gap = r.timestamp_ms.saturating_sub(prev);
std::thread::sleep(Duration::from_micros(gap as f64 * 1000.0 / f as f64));
…
let t0 = std::time::Instant::now();
output.send_frame(&frame)?;          // t0.elapsed() é MEDIDO…
m.record_frame(t0.elapsed()…);       // …e registrado
```

Duas propriedades estruturais disso:

1. **É livre-corrente (free-running).** Cada `sleep` é do *intervalo*, não até um *instante
   alvo*. O erro de granularidade do escalonador não é corrigido no quadro seguinte —
   **acumula**.
2. **O tempo de envio não é descontado.** O período real é `gap + tempo_de_envio`, e
   `t0.elapsed()` é medido para métrica mas nunca subtraído do próximo `sleep`. Isso não é
   jitter: é um **viés sistemático de lentidão**, monotônico ao longo do número.

Para o rig cabeado isso nunca importou — um só player, e a percepção é relativa. Para N
trajes independentes, é exatamente o mecanismo que os separa.

**Decisão:** o playback sincronizado espaça por **instante alvo absoluto** contra um
`SharedClock` — `alvo = epoch_do_show + timestamp_ms` — dormindo até o alvo e **descartando
ou repetindo** quadro quando atrasado, em vez de empurrar o erro para a frente. Assim o
desvio fica limitado pela **exatidão do relógio**, não pelo acúmulo do escalonador.

O `Speed::Factor` atual **não é removido**: continua correto para bancada, teste e
re-verificação de integridade. O modo absoluto é **aditivo**.

### D4 — A janela de sincronismo é **antes** do número, nunca durante

Trajes sincronizam relógio e recebem o show por **caminho cabeado** (Ethernet/USB, na
montagem ou na passagem de som) e depois ficam **autônomos**. Durante o número não há
rádio, não há rede, não há recepção.

**Isto não abre exceção ao ADR-0005 — reforça-o.** O ADR-0005 governa *saída ao vivo por
rede* ("a saída roda em Ethernet cabeada (ou DMX/SPI com fio)"). Aqui não existe saída por
rede: o único caminho de dados durante o show é interno ao traje. **O ADR-0005 permanece
literalmente intacto e o `NetworkGuard` não é tocado.**

O que **é** proibido explicitamente por este ADR: usar rádio (WiFi, ESP-NOW, ISM, W-DMX)
para **transportar quadros** durante o número. Um sinal de rádio de **disparo/tempo** é uma
questão em aberto (ver Q2), governado por critério próprio — porque um pacote de tempo
perdido degrada precisão, enquanto um quadro perdido apaga o corpo do bailarino.

### D5 — Deriva é orçamento declarado, medido, e com gatilho

Depois do sincronismo pré-show, cada traje corre com o **seu próprio oscilador**. Deriva é
inevitável; o que não pode é ser **desconhecida**.

Este ADR **não afirma nenhum número de ppm**. Osciladores de MCU derivam na casa de dezenas
de ppm e variam com temperatura — e um traje fica **contra um corpo quente**, que é
justamente o pior caso térmico. Qualquer número aqui seria inventado.

**Decisão:** o orçamento de deriva é um valor **declarado** no perfil do show, e a deriva
real é **medida em bancada antes de qualquer uso em palco** (gate G6). Enquanto não houver
medição, o número correto a escrever em qualquer documento é *não medido*.

### D6 — O invariante do heartbeat **não se aplica**, e isso precisa ser dito

O heartbeat existe porque um controlador **alimentado por streaming** entra em safe-mode
após ~2,4 s de silêncio (`LUMYX_GOSL.md`, Hardware Rules; `crates/led-hal/src/heartbeat.rs`).
Um traje autônomo aciona a própria fita localmente — **não há lacuna de rede para cobrir**.

O risco não desaparece: ele **muda de forma**. No streaming, "silêncio" era a falha; no
autônomo, a falha é o **player parar** — e nesse caso não há ninguém do outro lado para
notar. Por isso D7.

Escopo declarado: o invariante 2,4 s continua valendo **integralmente** para todo alvo
alimentado por rede. Este ADR não o afrouxa; declara que ele não governa um caminho onde
não existe rede.

### D7 — Falha de um traje não pode derrubar o número, e tem de ser **visível antes**

Precedente do ADR-0010: entrega parcial é melhor que nenhuma. Mas há uma inversão cruel —
**sem rede, ninguém fica sabendo que um traje falhou**. Não há `SegmentHealth` para
observar de fora.

Portanto:
- A detecção é **pré-show**, não em runtime: um gate de prontidão (show verificado, relógio
  sincronizado, bateria, integridade da fita) que **reprova o traje antes de ele subir**.
- A falha em cena tem **estado definido e seguro**, decidido por quem monta o show — não um
  comportamento emergente. **Qual é esse estado depende do ADR-0017 (blackout), que
  continua pendente** — um traje que "apaga" e um traje que "congela o último quadro" são
  decisões de palco diferentes, e este ADR não as toma por antecipação.

### D8 — Autenticidade viaja com o show

`verify_manifest_pinned` (`crates/led-show-recorder/src/signing.rs:132`) já resolve o
problema difícil: verificação Ed25519 contra chave **pré-confiada**, não contra a chave
embutida no blob (foi o achado CRITICAL **RT-001**).

Um traje é fisicamente acessível nos bastidores por qualquer pessoa — **mais exposto** que
um controlador em rack. Portanto: o traje **verifica com chave fixada antes do primeiro
quadro** e recusa tocar o que não verificar. Não é recurso novo; é aplicar o que já existe
num lugar onde o risco é maior.

---

## O que **não** está sendo decidido aqui

Explicitamente fora de escopo, e **não** deve ser implementado ou presumido:

| Fora de escopo | Por quê |
|---|---|
| Plataforma embarcada (firmware próprio × preset WLED × outro) | Decisão de engenharia com trade-off real; merece ADR próprio depois de medida |
| Meio físico do sincronismo pré-show | O ADR fixa o **contrato de tempo**, não o fio |
| Rádio de disparo/tempo durante o número | Questão em aberto Q2 — critério próprio |
| Bateria, peso, dissipação, segurança de contato | Engenharia elétrica; entra depois do orçamento medido |
| Formato do artefato assado | `.lumyx` é o candidato natural (`LUMX`/v1, header 16 B — `crates/led-show-recorder/src/lib.rs:56-58`), mas 73 MB para 6.200 px × 3.925 quadros não cabe num traje sem compressão ou redução de escopo. **Dimensionar antes de decidir.** |

---

## Critérios de validação

Um critério só conta se houver uma execução descrita que o faça **reprovar** (KB-012).

| # | Critério | Aceite |
|---|---|---|
| V1 | **Pacing absoluto não acumula erro** | Desvio entre o instante alvo e o real permanece limitado ao longo de um número inteiro, sem tendência crescente |
| V2 | **O pacing atual reprova o mesmo teste** | `Speed::Factor` (incremental) exibe desvio com tendência monotônica — provando que V1 mede algo real |
| V3 | **Assar é determinístico** | Assar o mesmo show duas vezes produz bytes idênticos |
| V4 | **Assar preserva o pixel** | `pixel_hash` do show assado = `pixel_hash` do render do engine (FNV-1a, `lib.rs:311`) |
| V5 | **Show não autenticado é recusado** | Chave errada / manifesto adulterado ⇒ recusa antes do 1º quadro (`verify_manifest_pinned`) |
| V6 | **Deriva real medida**, não presumida | Número com origem, condição e temperatura; enquanto não existir, o valor é *não medido* |
| V7 | **ADR-0005 intacto** | Nenhuma variante nova de `OutputInterface` autoriza rádio ao vivo; `NetworkGuard` inalterado |
| V8 | **Nenhum seam Frozen tocado** | Superfície de seam inalterada no guardião |
| V9 | **Estado de falha é definido** | Um traje que para tem comportamento declarado — **bloqueado pelo ADR-0017** |
| V10 | **Prontidão é pré-show** | Traje não pronto é reprovado antes de subir, nunca descoberto em cena |

---

## Gates

Executáveis, na disciplina do repositório. **Nenhum destes deve ser escrito antes do aceite
deste ADR** — estão aqui para que o aceite seja informado.

| Gate | O que roda | Reprova quando |
|---|---|---|
| **G1** — deriva do pacing | N players simulados sobre `SharedClock`, show de duração realista; mede desvio alvo↔real por quadro | O desvio tem tendência crescente, ou excede o orçamento declarado |
| **G2** — controle negativo de G1 | O mesmo teste com o `Speed::Factor` atual | **G2 tem de FALHAR.** Se passar, G1 não está medindo acúmulo e é um gate falso-verde |
| **G3** — determinismo do bake | Assa 2×, compara bytes | Qualquer diferença |
| **G4** — fidelidade do bake | `pixel_hash(assado)` vs `pixel_hash(render)` | Hashes diferentes |
| **G5** — autenticidade | Show re-assinado por atacante contra chave do estúdio | Se **não** recusar (espelha `pinned_verify_rejects_resigned_tamper`) |
| **G6** — deriva de hardware | Bancada: 2+ nós, sincronizados, livres por ≥ a duração do número, temperatura registrada | **Bloqueia palco enquanto não executado.** Sem este número, o orçamento de D5 é ficção |
| **G7** — não-regressão do ADR-0005 | `NetworkGuard` + presets + `OutputInterface` | Qualquer caminho novo que permita quadros por rádio ao vivo |
| **G8** — seams | `scripts/lumyx_guardian.sh` | Superfície de seam alterada sem bump |

**G6 é o gate que separa este ADR de um plano de slides.** Todos os outros rodam em
software; G6 exige duas placas numa bancada. Enquanto ele não rodar, nada disto está
provado em cima de nada físico.

---

## Questões em aberto (precisam de decisão, não de código)

| # | Questão | Bloqueia |
|---|---|---|
| **Q1** | Um traje que falha em cena **apaga** ou **congela o último quadro válido**? | D7 / V9 — depende do **ADR-0017**, ainda pendente |
| **Q2** | Rádio de **disparo/tempo** (não de quadros) é admissível durante o número, ou o sincronismo é 100 % pré-show? | D4 — muda o desenho de re-sincronismo |
| **Q3** | Qual a duração máxima de número sem re-sincronizar? | Sai de G6; sem a medição, não há resposta honesta |
| **Q4** | O show cabe no traje? 73 MB (6.200 px × 3.925 quadros) é o tamanho atual do artefato do rig | Formato do bake — compressão, subconjunto de pixels, ou redução de taxa |

---

## Alternativas descartadas

**Streaming sem fio com rádio dedicado (ESP-NOW / ISM / W-DMX).** Trocaria WiFi por um rádio
melhor, mas mantém a propriedade que a nossa própria medição condenou: o corpo do bailarino
depende, quadro a quadro, de um enlace sem fio, num ambiente de palco cheio de rádio e de
corpos (água) entre transmissor e receptor. O ADR-0005 nasceu de 31 ms de jitter medidos;
mudar o rádio muda o número, não a categoria de risco.

**Streaming com buffer grande no traje.** É o argumento já rejeitado pelo ADR-0005 em outra
roupagem: buffer troca jitter por latência, e para show sincronizado com música latência
variável é tão ruim quanto perda. Além disso, um buffer grande o bastante para cobrir uma
falha de rádio **já é** o show inteiro — ou seja, já é playback autônomo, só que com um
rádio desnecessário ligado.

**Cabo até o traje.** Resolve tudo tecnicamente e é incompatível com dança. Registrado para
que a rejeição seja explícita, não esquecida.

**Tratar o traje como `DeviceDriver` remoto.** Forçaria os seams Frozen a atravessar um
enlace sem fio e arrastaria `CompiledLayout`/`UniverseData` para dentro do traje sem
necessidade. D1 evita isso inteiramente.
