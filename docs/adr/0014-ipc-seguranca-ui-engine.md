# ADR-0014 — IPC e segurança do canal UI ↔ engine

- **Status:** aceito (pré-implementação)
- **Data original:** 2026-07-26
- **Fonte:** Decisão de arquitetura UI/Preview + gate `/security` do `LUMYX_GOSL.md`

## Contexto e problema
O ADR-0013 introduz um daemon. A UI precisa **ler estado** e **emitir comandos**. Hoje a
única escuta é o HTTP de métricas, cujo `bind` é um parâmetro **sem auth e sem restrição
forçada** (`crates/led-hal/src/prometheus.rs`). O gate `/security` proíbe endpoints de
controle sem auth e sockets em `0.0.0.0`. Um canal de controle mal projetado é uma via para
negar/adulterar um show ao vivo.

## Decisão
- **Mesmo host (autoria):** IPC por **Unix domain socket com permissão de arquivo
  owner-only**; autorização = credencial do SO do dono do socket.
- **LAN (laptop → appliance):** **TCP com token e/ou mTLS**, **bind em interface
  específica** (nunca `0.0.0.0` por padrão).
- **Comandos tipados e versionados** (schema com versão negociada no handshake); read-model
  **somente leitura**.
- **Ações irreversíveis** (ex.: reboot de device via `IDevice`; futuramente blackout)
  exigem **confirmação explícita + log auditável**.

## Escopo / Não-escopo
- **Escopo:** transporte, autenticação/autorização, forma tipada/versionada de read-model e
  comandos, política de bind.
- **Não-escopo:** os nomes concretos de comandos e a serialização exata (definidos na
  implementação, sem inventar aqui); **blackout (ADR-0017)**; descoberta de daemon na rede.

## Alternativas descartadas
- **HTTP sem auth** — reprova `/security`.
- **gRPC** — deps pesadas vs. o ethos std-only/embedded-friendly; reavaliável se um
  transporte tipado maduro for necessário.

## Limites de segurança
Nenhum bind em `0.0.0.0` por padrão. Read-model e comandos são canais distintos; comando
sempre autenticado. UDS protegido por permissão de arquivo; LAN por token/mTLS. Auditoria de
comandos alinhada à trilha Ed25519/Provenance já existente.

## Isolamento do hot-path
Comandos são aplicados pelo daemon **no limite de frame** (o play loop já é não-hot-path,
`crates/led-player/src/lib.rs`), nunca dentro de `send_frame`/render. O IPC roda em thread de
controle própria, separada de render/send.

## Compatibilidade de OS
UDS nativo em macOS/Linux; no Windows, named pipe/UDS-equivalente **ou** loopback
autenticado — decisão de implementação, sem orientar a arquitetura. TLS/token é agnóstico.

## Degradação segura
Canal caído → daemon segue o show; a UI trata desconexão como read-only-perdido e reconecta.
Comando malformado/não-autenticado → rejeitado e logado, nunca aplicado.

## Consequências
**Boas:** superfície de controle fechada por construção; reuso do precedente HTTP para
read-model; autoria remota segura. **Ruins/custos:** gestão de token/certificado; handshake
de versão; complexidade de dois transportes (UDS + TCP/TLS).

## Métricas / gates
Gate `/security`: sem `0.0.0.0` padrão, sem endpoint de controle sem auth. Teste que prova
rejeição de comando não-autenticado. Round-trip de comando ≤ 50 ms.

## Critério de reversão
Trocar de transporte (ex.: UDS → HTTP local autenticado) se UDS não fechar o gate
`/security` num OS-alvo, **preservando** as invariantes (auth obrigatória, bind restrito,
comandos tipados/versionados). A *política* não é reversível; o *transporte* é substituível.
