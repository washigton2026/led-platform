# ADR-0013 — Engine headless em daemon separado; UI é cliente

- **Status:** aceito (pré-implementação — precede a primeira linha de código de UI)
- **Data original:** 2026-07-26
- **Fonte:** Decisão de arquitetura UI/Preview + auditoria de fronteiras do Baseline 1.0

## Contexto e problema
O Baseline 1.0 é um motor Rust maduro (663 testes) **sem UI, editor ou preview**. Ao
introduzir um console de operador, a restrição inegociável é: **a UI nunca pode bloquear,
alocar no hot-path, aumentar jitter/flicker ou derrubar o output do show**. Auditoria do
repo confirma: o output roda em threads próprias (render+send desacoplados por triple
buffer, ADR-0008; heartbeat em thread própria), o runtime toca o hardware **só** por
`Arc<dyn ProtocolOutput>` (`crates/led-hal/src/engine.rs`), e **não existe plano de
controle** — a única superfície de escuta é o HTTP de métricas (`prometheus.rs`).

## Decisão
O **engine roda como daemon headless** (processo próprio, dono das threads de
render/send/heartbeat e do output). A **UI é um processo cliente separado** que fala com o
daemon por IPC (ADR-0014). O output em tempo real **não compartilha processo de falha** com
a UI.

## Escopo / Não-escopo
- **Escopo:** separação de processo; o daemon como autoridade do runtime/output; a UI como
  consumidor de read-model + emissor de comandos management-plane.
- **Não-escopo:** stack da UI (ADR-0016, provisório); protocolo/segurança do IPC (ADR-0014);
  caminho de preview (ADR-0015); **blackout intencional (adiado — ADR-0017)**.

## Alternativas descartadas
- **In-process / Tauri-embed-engine** — coloca o output no mesmo espaço de falha da UI (um
  panic de UI ou crash de driver GPU do webview pode derrubar o processo que detém as
  threads de output). Rejeitado pela restrição nº 1.

## Limites de segurança
O daemon é o único processo com acesso ao output e à rede de show. A UI não recebe handle do
`ProtocolOutput`, do triple buffer, nem dos sockets de saída. Toda influência da UI passa por
comandos autenticados (ADR-0014).

## Isolamento do hot-path
O hot-path (`eng → triple buffer → HAL → DeviceDriver → UDP`) vive **inteiro dentro do
daemon** e é intocado por esta decisão. A fronteira de processo reforça o isolamento já
existente, não abre nova via para ele.

## Compatibilidade de OS
- **Linux** = alvo de output ao vivo: o daemon roda headless numa appliance **sem GUI**.
- **macOS/Windows** = autoria: rodam a UI cliente; podem também rodar o daemon localmente.
- Windows **não orienta** a arquitetura; a separação de processo é agnóstica de OS.

## Degradação segura
UI travada/crashada/desconectada → **daemon intacto, output continua** (é a razão de existir
desta decisão). Reconexão retoma o read-model; ausência de comando = nenhuma mudança de
estado.

## Consequências
**Boas:** garantia forte da restrição nº 1; a appliance de show não precisa de GPU/webview/
browser; autoria remota (laptop→appliance) fica natural. **Ruins/custos:** introduz IPC
(latência + superfície a proteger — ADR-0014); dois artefatos para empacotar/versionar;
exige contrato de versão entre daemon e UI.

## Métricas / gates
Gate de aceite: **p99 de latência de output COM a UI conectada == SEM** (medido pelo
`MetricsEmitter` já existente). Round-trip de comando ≤ 50 ms (alvo). Qualquer regressão de
jitter atribuível à UI reprova.

## Critério de reversão
Reverter para in-process só se: (a) o overhead de IPC violar o budget de comando de forma
irremediável **e** (b) o isolamento real-time puder ser garantido por outro mecanismo
comprovado (um panic de UI não derruba o output). Sem ambos, a decisão se mantém.
