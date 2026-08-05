# Golden Slice — plano executável

**Data:** 2026-08-05 · **Planeamento. Nenhum código.**
**Definição canónica:** [`docs/ROADMAP.md` §Golden Slice](ROADMAP.md).

---

## 0. Duas coisas a resolver antes de começar

### 0.1 🔴 Colisão de nomenclatura — `D1`…`D5` já significam outra coisa

A FASE D do ROADMAP já usa `D1`–`D8` com um significado **diferente** do proposto na
conversa de planeamento:

| Sigla | ROADMAP (existente) | Sprints propostos na conversa |
|---|---|---|
| D1 | **Daemon** (engine headless) | Estrutura do console |
| D2 | **IPC** | Timeline |
| D3 | **Shell do console** | Preview |
| D4 | **Preview WebGPU** | Controller Manager |
| D5 | **Timeline visual** | Hardware Ethernet |

Manter os dois esquemas garante confusão em commits, ADRs e conversas futuras.
**Este documento usa `GS1`…`GS7`** para as entregas do Golden Slice e **preserva `D1`–`D8`**
com o significado do ROADMAP. Cada entrega `GS` diz de que PRs `D` depende.

### 0.2 🔴 O sequenciamento proposto salta duas peças, e a segunda é estrutural

A ordem proposta começa em "estrutura do console" (janela, menu, toolbar). Mas o ROADMAP
regista, na FASE D:

> *"hoje **não há o que comandar** — o engine não tem um estado de show controlável em
> runtime. D1 é onde isso nasce."*

Ou seja: **o control-plane está vazio.** Um console construído antes do daemon (D1) e do IPC
(D2) é uma casca sem interlocutor — abre, redimensiona, e não tem com o que falar. O próprio
plano de PRs do ROADMAP põe `D3` (shell) a depender de `D2`.

**Recomendação:** o daemon e o IPC entram **antes** da casca visual. Não é preciosismo de
ordem — é a diferença entre uma UI que integra e uma UI que depois precisa ser reescrita
quando o estado de runtime aparecer.

---

## 1. Legenda

**Esforço** é **relativo**, nunca em dias — não conheço a sua velocidade e um número em dias
seria estimativa apresentada como facto: **S** (pequeno) · **M** · **L** · **XL**.

**Evidência exigida** segue a regra do repo: sem artefato citável, não fecha.

---

## 2. As entregas

### GS0 — Desbloqueio (não é código)

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | ADR-0016 aceito (stack escolhida) · ADR-0017 aceito (semântica do blackout) |
| **Dependências** | Medições humanas H1–H6 do [anexo do ADR-0016](adr/0016-anexo-evidencia-e-matriz.md) |
| **Critérios de aceite** | Os dois ADRs com status `aceito` e veredito registado |
| **Riscos** | Decidir sem eliminar a assimetria de medição (o Leptos não foi medido em a11y) |
| **Evidência** | Tabela do `spike/README.md` preenchida nas duas colunas |
| **Esforço** | **S** (mas é tempo humano, não de agente) |

> **Tudo abaixo de GS3 está atrás disto.** GS1 e GS2 **não** estão.

---

### GS1 — Control-plane: daemon + IPC *(elo: base de tudo)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | Engine headless como processo próprio; estado de show controlável em runtime (load/play/pause/seek/stop); IPC UDS owner-only, comandos **tipados e versionados**; read-model servido |
| **Dependências** | ADR-0013, ADR-0014 (ambos **já aceitos**). PRs `D1`+`D2` |
| **Critérios de aceite** | Daemon sobe sem UI · comandos tipados round-trip · **recusa bind não-loopback** · UI matar não derruba output |
| **Riscos** | 🔥 **É a maior peça sem precedente no repo.** Hoje não existe estado de show em runtime — é desenho novo, não refactor |
| **Evidência** | Testes de IPC (comando → mudança de estado observável no read-model) · teste negativo de bind não-loopback · teste de isolamento de falha |
| **Esforço** | **XL** |

**Não bloqueado por B2** — é Rust puro, independe da stack de UI.

---

### GS2 — Portas físicas múltiplas *(elo: configurar controladores)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | `Port { index, pixel_count, color, calibration }` no profile; `compile_layout` distribui por porta; presets ganham portas (Falcon F16V3 tem **16**, hoje declara 1); 9.º check do guardião |
| **Dependências** | Nenhuma — **pode começar hoje** |
| **Critérios de aceite** | Preset multi-porta compila para `CompiledLayout` correto · `pixels_per_universe` declarado é honrado · porta **não vaza** para o runtime |
| **Riscos** | C2 é o coração do mapeamento — erro aqui corrompe endereçamento silenciosamente |
| **Evidência** | Teste de bytes E2E por porta · controle negativo (porta a mais/a menos recusa) · guardião verde |
| **Esforço** | **M** |

---

### GS3 — Shell do console *(elo: editar — casca)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | Janela, menu, toolbar, área de timeline, painel lateral, barra de estado, layout responsivo. **Consome o read-model do GS1** — não é mock |
| **Dependências** | **GS0** (stack) + **GS1** (há com o que falar) |
| **Critérios de aceite** | Abre · redimensiona sem quebrar · **navegação por teclado completa, foco visível** · axe **0 violações críticas** · mostra saúde real vinda do daemon |
| **Riscos** | Construir sobre mock e depois reescrever ao ligar no daemon |
| **Evidência** | axe/Lighthouse · **captura de leitor de tela anunciando mudança de estado** · vídeo de navegação só-teclado |
| **Esforço** | **L** |

> A11y entra **aqui**, não depois. Retrofit de acessibilidade é a forma cara de a fazer.

---

### GS4 — Timeline *(elo: editar)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | Playhead, zoom, scroll horizontal, tracks, seleção, snap, cursor temporal. **Lê o modelo do `led-sequencer`, que já existe** |
| **Dependências** | GS3 |
| **Critérios de aceite** | 60 fps em zoom/scroll · playhead preciso · sem flicker · **operável por teclado** |
| **Riscos** | 60 fps com muitos clips é problema de renderização, não de modelo — pode empurrar para canvas/WebGPU antes do previsto |
| **Evidência** | **Benchmark de renderização com número e condição** (nº de clips, resolução, máquina) · testes automatizados |
| **Esforço** | **L** |

---

### GS5 — Preview WebGPU *(elo: pré-visualizar)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | Render WebGPU dos LEDs virtuais; play/pause/seek; fallback 2D com aviso |
| **Dependências** | GS0, GS1 (cópia de preview vem do daemon), GS4 |
| **Critérios de aceite** | **≥30 fps a 10k pontos** (critério do ADR-0016) · mesmo timestamp do motor · **fallback funciona** |
| **Riscos** | 🔴 **Canvas2D já foi medido em 3 fps** — reprovado por ordem de grandeza. O caminho WebGPU **ainda não foi medido em lado nenhum** |
| **Evidência** | fps medido **com condição** (GPU, nº de pontos) · prova de que a UI **nunca** lê o triple buffer (ADR-0015) |
| **Esforço** | **L** |

> ⚠️ **Sobre "mesmo hash do estado":** o preview é **lossy por contrato** (ADR-0015) —
> downsampled e rate-limited. **Ele não pode ter o mesmo hash do frame do motor**, e exigir
> isso contradiz o ADR. O critério correto é **mesmo timestamp e mesma origem**, com o hash
> a ser verificado no **replay** (elo 7), onde já existe e é exato.

---

### GS6 — Controller Manager *(elo: configurar controladores)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | Adicionar/editar/remover controlador; DDP, Art-Net, sACN; IP e porta; teste de ligação; persistir configuração |
| **Dependências** | GS2 (portas), GS3 (UI) |
| **Critérios de aceite** | Persistência · validação · **sem duplicidade de universo/canal** · teste de ligação usa o **discovery ArtPoll que já existe** |
| **Riscos** | Duplicar a validação que o `led-hardware-profile` já faz — deve **reusar**, não reescrever |
| **Evidência** | Testes de persistência e validação · **controle negativo: config conflituosa é recusada** |
| **Esforço** | **M** |

---

### GS7 — Ethernet + hardware real *(elos: enviar / executar / validar)*

| Campo | Conteúdo |
|---|---|
| **Funcionalidade mínima** | Show real, do console ao LED físico, **por Ethernet cabeada**, sem WiFi |
| **Dependências** | GS1, GS6 + **hardware ESP32-POE + switch + cabo** (recurso externo) |
| **Critérios de aceite** | **Zero WiFi** · burn-in sem abortos · **jitter e latência medidos** · sem perda de frames · hash de replay estável |
| **Riscos** | 🔴 Depende de compra/montagem de hardware · o rig está **offline desde 2026-07-11** · sACN está **bloqueado por firmware do WLED** (porta 5568 sem listener) — usar DDP ou Art-Net |
| **Evidência** | Relatório de validação de hardware datado, no formato de `docs/certification/HARDWARE-VALIDATION-2026-07-20.md` |
| **Esforço** | **M** de código, **XL** de logística |

---

## 3. Versões

| Versão | Contém | O que já dá para fazer | O que ainda **não** |
|---|---|---|---|
| **MVP** | GS1 + GS2 + GS7 (sem UI) | **O Golden Slice fecha por CLI, em Ethernet real.** Prova a cadeia inteira ponta a ponta | Não é usável por não-programador |
| **Alpha** | + GS3 + GS4 | Operador **vê** e edita a timeline; saúde real na tela | Sem preview; sem gestão de controladores na UI |
| **Beta** | + GS5 + GS6 | **Golden Slice completo pela UI** — os sete elos | Sem blackout (D6), sem editor de layout (D7), sem empacotamento (D8) |
| **Produção** | + D6 + D7 + D8 + burn-in 72 h | App instalável, blackout auditável, rig de 5 nós | Paridade de efeitos (E1, ~25 restantes) é contínua |

> **A escolha que mais muda o risco:** pôr **GS7 no MVP**, antes da UI. Ethernet é o elo com
> risco **externo** (hardware, logística, firmware) e nenhum risco de UI o resolve. Descobrir
> um problema de Ethernet depois de construir o console inteiro seria a pior ordem possível.
> O CLI já existe e basta para provar a cadeia.

---

## 4. Caminho crítico

```
GS2 (portas) ──────────────┐         [pode começar HOJE]
                           ├──► GS6 (controller mgr) ──┐
GS1 (daemon+IPC) ──────────┤                           ├──► GS7 ──► MVP físico
   [pode começar HOJE]     └──► GS3 ──► GS4 ──► GS5 ───┘
                                 ▲
GS0 (ADR-0016 + 0017) ───────────┘   [espera medição humana]
```

**Duas frentes podem começar hoje, sem qualquer decisão pendente: GS1 e GS2.**
GS1 é a maior peça do projeto e **não depende da stack de UI** — o que significa que a
espera por B2 **não precisa parar o trabalho**.

---

## 5. Riscos transversais

| Risco | Gravidade | Mitigação |
|---|---|---|
| **Control-plane não existe** — GS1 é desenho novo, sem precedente no repo | 🔴 alta | Fatiar GS1 (estado → comandos → IPC); cada fatia com gate |
| **Ethernet depende de compra e montagem** | 🔴 alta | Antecipar GS7 para o MVP; comprar o ESP32-POE já |
| **sACN bloqueado por firmware do WLED** | média | Usar DDP/Art-Net (**ambos validados em hardware**) |
| **fps do preview nunca foi medido em WebGPU** | média | Medir num spike antes de comprometer GS5 |
| **A11y tratada como retrofit** | média | Critério de aceite do GS3, não fase posterior |
| **Blackout entra antes do ADR-0017** | 🔴 alta | Proibido por ADR; e ver o risco do traje no [anexo do 0017](adr/0017-anexo-analise-e-proposta.md) |
| **Rig offline há semanas** | média | GS7 é o único elo com dependência física — tratar a logística como tarefa, não como pressuposto |

---

## 6. O que este plano deliberadamente **não** faz

- **Não estima em dias.** Esforço é relativo; qualquer número absoluto seria inventado.
- **Não escolhe stack** — GS0 depende de medição humana.
- **Não inclui blackout** no Golden Slice: é D6, atrás do ADR-0017, e **não** é elo do fluxo.
- **Não assume que a paridade de efeitos (E1) bloqueia** — não bloqueia. 13 efeitos bastam
  para fechar o Golden Slice; os ~25 restantes são trabalho contínuo e paralelo.
