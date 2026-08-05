# ADR-0017 — Anexo: análise do blackout e proposta (B1)

**Data:** 2026-08-05 · **Nenhum código escrito.** Este anexo **não** aceita o ADR-0017 — ele
reúne a evidência e apresenta uma proposta para o responsável decidir.

---

## 1. O que o repositório já fixa

### 1.1 O invariante, no código

`crates/led-hal/src/heartbeat.rs`:

```rust
last_valid: Mutex<Option<LogicalFrame>>,
pub fn record(&self, frame: &LogicalFrame) { *self.last_valid.lock().unwrap() = Some(frame.clone()); }
// "sends nothing (Ok(false)) — it NEVER fabricates a zeroed/blackout frame."
```

Travado por teste: `crates/led-hal/tests/contract.rs:76` —
`assert_eq!(sim1.frames_sent(), 0, "must not blast a blackout frame")`.

E pelas Hardware Rules do `LUMYX_GOSL.md`: *"The heartbeat always sends the last **valid**
frame — never a zeroed frame. A zero frame blacks the rig; silence trips safe mode."* Gap
máximo a qualquer device: **2,4 s** (Warning 2,0 s, Critical 2,4 s).

### 1.2 A tensão que o ADR-0017 registou

> *"Se o heartbeat gravar o frame preto como «último frame válido», ele reenvia preto
> (blackout persistente — correto). Se não gravar, reenvia o frame pré-blackout (o rig
> «acende de volta» no próximo heartbeat — errado)."*

### 1.3 O que já está decidido em ADRs vizinhos

| Origem | Restrição que incide sobre o blackout |
|---|---|
| ADR-0014 + `docs/architecture/control-protocol.md:90` | Ações irreversíveis (`device.reboot`, firmware e **futuramente blackout**) exigem **duas fases** |
| ADR-0019 + `led-hal/src/hal.rs` | Já existe ponto de interceção **por device, entre o `apply` e o fan-out**, com buffer separado (`cal_scratch`) |
| ADR-0022 Q1 | O traje autónomo que falha **apaga** — mas o estado de falha é **parâmetro declarado**, porque num número de trajes as roupas são a **única fonte de luz** e um traje apagado torna o bailarino **invisível** |
| ROADMAP B1 | Implementação já esboçada: máscara no HAL, **memset** no scratch, heartbeat **empurra** (`send_frame`) |

---

## 2. Como o setor trata — evidência pesquisada

> **Nível de confiança declarado por linha.** Parte do material público mistura consoles
> diferentes; onde não consegui confirmar na documentação do próprio fabricante, está dito.

| Plataforma | Comportamento | Confiança |
|---|---|---|
| **ETC Eos** | Blackout key e Grand Master são **intensity masters**: escalam intensidade; **todos os parâmetros não-intensidade ficam inalterados**. A reprodução de cues **continua**. Canais *parked* também vão a zero com blackout/GM em zero | **Alta** — comunidade oficial ETC |
| **ChamSys MagicQ** | **DBO** fica acima do Grand Master e põe **todos os valores HTP a zero**. Os playbacks **continuam a correr**; a saída é que é suprimida. Ao soltar, **os níveis HTP são restaurados**. Existe opção **"Ignore Masters"** por cue stack, para canais que não podem apagar (luz de casa, máquina de fumo) | **Alta** — manual MagicQ |
| **MA / grandMA3** | O Grand Master **limita a intensidade** de todos os fixtures do show | **Média** — confirmado; **o DBO do grandMA3 não foi confirmado na doc da MA**: a página de Grand Masters que consultei **não discute blackout**, e o detalhe de DBO que apareceu na busca vinha da documentação **HOG (ETC)**, não da MA. Não tratar como facto sobre grandMA3 |
| **Falcon Player (FPP)** | Separa **"Stop Now"** (pára imediatamente) de **"Stop Gracefully"** (pára no fim da sequência). O apagar é **configuração à parte** — "blackout outputs" garante que as luzes apagam no fim da sequência ou num Stop Now | **Alta** — manual FPP |
| **xLights** | **Não tem master de blackout.** Tem o toggle **"Output To Lights"** (porta de saída) e, para forçar apagado, usa-se um **efeito "Off"** explícito contra um grupo. O Stop **repõe a posição no início** | **Média-alta** — manual xLights |

### 2.1 O padrão que emerge — e é consistente

Nas consoles profissionais (Eos, MagicQ, MA), **blackout é máscara de saída, não comando de
transporte**:

1. **A reprodução continua a correr por baixo.** O tempo não pára, os cues avançam.
2. **A supressão é na saída**, aplicada sobre intensidade/HTP.
3. **Soltar restaura** o que estiver a tocar naquele instante — não o que estava quando se
   apagou.
4. **Existe escape declarado** para canais que não podem apagar (o "Ignore Masters").

Nas ferramentas de pixel (FPP, xLights), que são o domínio direto do LUMYX, **STOP e apagar
são coisas separadas e explicitamente configuráveis** — o FPP tem duas variantes de stop e um
ajuste independente para apagar saídas.

**Conclusão factual:** em nenhuma das plataformas pesquisadas o blackout é implementado como
"gravar preto como último estado". Ele é sempre uma **camada de supressão a jusante** do
motor de reprodução.

---

## 3. As seis perguntas

| # | Pergunta | Resposta baseada na evidência |
|---|---|---|
| 1 | Como cada plataforma trata blackout? | Como **máscara de saída** sobre intensidade/HTP (Eos, MagicQ, MA). Em ferramentas de pixel, como **opção de apagar ligada ao stop** (FPP) ou **inexistente como master** (xLights) |
| 2 | Blackout interrompe efeitos? | **Não.** Os efeitos continuam a ser calculados; só a saída é suprimida |
| 3 | Blackout interrompe timeline? | **Não.** O transporte continua; o tempo não pára |
| 4 | Blackout preserva estado? | **Sim** — é essa a razão de ser máscara. O estado vivo continua por baixo, intacto |
| 5 | Blackout retorna automaticamente? | **Não automaticamente por tempo.** Retorna quando **solto/desarmado**, e restaura **o estado corrente** naquele momento, não o congelado |
| 6 | Diferença entre STOP e BLACKOUT? | **Sim, e é a distinção central.** STOP é **transporte** (pára o tempo, muda a posição — no xLights repõe ao início); BLACKOUT é **saída** (suprime o que sai do fio, sem tocar no transporte) |

---

## 4. Proposta para o ADR-0017

### 4.1 A observação que dissolve o dilema do §1.2

O ADR-0017 formulou a questão como *"o blackout deve ou não virar o «último frame válido»
reenviado?"* — e as duas respostas eram más. **Essa pergunta assume que a máscara e o
armazenamento vivem na mesma camada. No LUMYX eles não precisam viver.**

Se a máscara for aplicada **a jusante** de `heartbeat.record()`:

```
frame vivo ──► record()  [guarda o frame REAL, nunca preto]
                  │
                  └──► apply ──► [calibração ADR-0019] ──► ★ máscara de blackout ★ ──► fan-out
                                                             (memset no scratch do device)
heartbeat ──► reenvia o último frame REAL ────────────────────────┘  (passa pela MESMA máscara)
```

Então, simultaneamente:

- **O preto persiste** — porque a máscara também se aplica ao reenvio do heartbeat. Opção
  (a) do ROADMAP, sem gravar preto em lado nenhum.
- **O restore volta ao conteúdo vivo** — porque o que está guardado nunca foi preto. Resolve
  a questão 4 do ADR ("`restore` volta ao último frame não-preto? Como é rastreado?"): não é
  preciso rastrear nada.
- **O invariante continua literalmente verdadeiro** — o heartbeat **nunca fabrica** um frame
  zerado; ele continua a reenviar o último frame válido. Quem zera é a máscara de saída, que
  é **comandada**. Resolve a questão 1 do ADR.
- **O gap de 2,4 s nunca é violado** — continua a sair frame a ≥1 Hz. O rig não entra em
  safe-mode; ele recebe preto **comandado**, que é diferente de silêncio.

Isto **coincide com o padrão do setor** (§2.1) e com a implementação já esboçada no ROADMAP
(máscara no HAL, `memset`, heartbeat empurra).

### 4.2 Proposta concreta

| # | Proposta |
|---|---|
| **P1** | **Blackout é máscara de saída, no HAL, por device**, aplicada depois da calibração e antes do fan-out. `memset` no scratch — não multiplicação por-byte |
| **P2** | **`heartbeat.record()` nunca recebe frame mascarado.** O invariante fica intacto e o restore é trivial |
| **P3** | **STOP e BLACKOUT são comandos distintos** e não se implicam, seguindo FPP/Eos/MagicQ. STOP é transporte; BLACKOUT é saída |
| **P4** | **Blackout é latching** (armado/desarmado explicitamente), com estado **visível e persistente** na UI enquanto ativo |
| **P5** | **Duas fases + log auditável**, como já exige o ADR-0014 e o `control-protocol.md` para ações irreversíveis |
| **P6** | **Escape declarado por device** — o equivalente ao "Ignore Masters" do MagicQ. **Não é conveniência: é segurança.** Ver §5 |
| **P7** | **Sem atalho de teclado nesta fatia.** A questão 5 do ADR (conflito com foco de texto, atalhos de SO e acessibilidade) não foi resolvida, e um `B` mal colocado apaga um palco por acidente |

### 4.3 Vantagens

- Resolve as questões 1, 2 e 4 do ADR-0017 **sem trade-off** — não é escolher o menos mau.
- Alinha com o que operadores de palco já esperam de Eos/MagicQ: **curva de aprendizado zero**.
- Zero custo quando inativo (uma verificação por device por frame, fora do caminho por-pixel).
- Não toca em nenhum contrato `Frozen` do `led-core`; a máscara vive no HAL.

### 4.4 Riscos

| Risco | Gravidade | Nota |
|---|---|---|
| **Traje de dança apagado torna o bailarino invisível** | 🔴 **Alta — física, não estética** | O ADR-0022 Q1 já registou isto: num número de trajes o palco é preto e as roupas são a **única fonte de luz**. Um blackout global sobre trajes é **risco de integridade física**. É por isto que **P6 não é opcional** |
| Operador esquece o blackout armado e culpa o sistema | Média | Mitigado por P4 (estado visível e persistente) |
| Blackout confundido com STOP | Média | Mitigado por P3 + rótulos distintos |
| Máscara no caminho de saída introduz custo | Baixa | Ramificação por **device**, não por pixel — mesmo padrão já medido no ADR-0019 (+2,7 % do orçamento **com** calibração ativa) |
| Blackout não alcança o traje autónomo | **Informativo** | O traje **não tem rede nem heartbeat** (ADR-0022). Um blackout de consola **não o apaga**. Isto tem de estar escrito na doc do operador, ou cria falsa sensação de segurança |

### 4.5 O que esta proposta **não** decide

- **Não decide o comportamento de fade** (blackout instantâneo × com rampa). Não pesquisei
  evidência suficiente para propor, e um fade tem implicações de timing próprias.
- **Não decide atalho de teclado** (P7 adia deliberadamente).
- **Não decide a UI** — é escopo do ADR-0016/FASE D.
- **Não decide o comportamento em cluster** (`SyncedCluster`): se um nó perde o comando de
  blackout, o palco fica meio apagado. **Isto precisa de resposta antes de D6.**

---

## 5. A recomendação que eu destacaria

De tudo acima, o ponto que mais me preocupa não é o heartbeat — é o **P6**.

O ADR-0022 já estabeleceu, com razão física, que num número de trajes as roupas são a única
fonte de luz. Se o console ganhar um blackout global sem escape por device, existe um caminho
em que um operador aciona blackout e **deixa bailarinos no escuro total num palco escuro**.
O MagicQ tem "Ignore Masters" exatamente por esta classe de razão (luz de casa, fumo).

**Sugiro que o ADR-0017, ao ser aceito, torne o escape por device um requisito de aceitação —
não um item de trabalho futuro.** Um blackout sem escape é mais perigoso que a ausência de
blackout, porque parece seguro.

---

## 6. Estado do ADR-0017 após este anexo

**Continua `proposto (adiado)`.** Este anexo não o promove e **nenhum blackout pode ser
implementado**. Para aceitar, o responsável precisa de: confirmar P1–P7 (ou corrigi-los),
decidir fade (§4.5), e responder ao comportamento em cluster.

---

## Fontes

- [Parked channels and blackout key/master fader — ETC Community](https://community.etcconnect.com/control_consoles/eos-family-consoles/f/eos-family/55078/parked-channels-and-blackout-key-master-fader)
- ["The Master thing" — ETC Community](https://community.etcconnect.com/control_consoles/eos-family-consoles/f/eos-family/27488/the-master-thing)
- [Dead Black Out (DBO) — ChamSys MagicQ User Manual](https://www.manualslib.com/manual/855941/Chamsys-Magicq.html?page=120)
- [Playback Priority / Playbacks Ignore Masters — ChamSys MagicQ User Manual](https://www.manualslib.com/manual/855941/Chamsys-Magicq.html?page=128)
- [Playback — ChamSys Documentation](https://secure.chamsys.co.uk/docs/magicq/manual/playback.html)
- [Grand Masters — grandMA3 Help](https://help.malighting.com/grandMA3/2.1/HTML/masters_grand.html)
- [Grand Master — ETC HOG (origem do detalhe de DBO que a busca devolveu)](https://www.etcconnect.com/webdocs/Controls/HOG/HTML/en/sect-grand_master.htm)
- [Status Page — Falcon Player Manual](https://falcon-player.gitbooks.io/falcon-player-manual/content/chapter_seven_status__control/status_page.html)
- [FPP 5.0 Manual](https://falconchristmas.github.io/FPP_Manual(5.0).pdf)
- [Off effect — xLights Manual](https://manual.xlights.org/xlights/effects/off/off)
- [Timeline and Waveform — xLights Manual](https://manual.xlights.org/xlights/chapters/chapter-four-sequencer/timeline-and-waveform)
