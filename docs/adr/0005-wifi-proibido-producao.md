# ADR-0005 — WiFi proibido para shows ao vivo

- **Status:** aceito
- **Data original:** 2026-06-03 (regra no `LUMYX_GOSL.md` desde a fundação);
  enforcement em código 2026-06-25 (`NetworkGuard`)
- **Fonte:** LUMYX_GOSL.md Hardware Rules; CLAUDE.md changelog 2026-06-25

## Contexto e problema
Saída de LED em tempo real precisa de timing estável — controladores WLED/FPP
entram em modo de segurança após ~2,5 s de silêncio, e frames atrasados causam
flicker visível. WiFi introduz **jitter de 5–50 ms** e perdas esporádicas que
são fatais para saída ao vivo. O rig atual do usuário está, ironicamente, todo
em WiFi — o que torna a regra concreta, não teórica.

## Decisão
**WiFi é proibido para output ao vivo.** A saída roda em Ethernet cabeada (ou
DMX/SPI com fio). WiFi pode existir só para config/monitoramento, e a UI deve
tornar a limitação explícita — nunca tratá-la como bug.
- Enforcement: `NetworkGuard` (`led-hal`) — `WifiBlockGuard` sonda a plataforma
  (macOS `networksetup`+`ifconfig`; Linux `/sys/class/net/wl*/operstate`) e
  recusa iniciar o show se uma interface WiFi estiver ativa. `PermissiveGuard`
  para simulador/testes.
- A checagem é no **start do show** (uma chamada antes do 1º frame), nunca no
  hot-path — zero overhead por frame (provado por `CountingGuard`).

## Consequências
**Boas:** a plataforma se recusa a iniciar num setup que vai falhar ao vivo —
o operador descobre no ensaio, não no palco. Erro tipado com prefixo
CRITICAL/WARNING conforme as Hardware Rules. Reforça o ADR-0003 (DDP + fio como
caminho de capacidade).
**Ruins/custos:** o rig atual precisa migrar de WiFi para Ethernet
(ESP32-POE/QuinLED) antes de qualquer show ao vivo — custo de hardware real para
o usuário. `WifiBlockGuard` retorna `ProbeUnavailable` (não-fatal) em
plataformas não suportadas, permitindo ambientes não-hardware prosseguirem com
aviso.

## Alternativas rejeitadas
- **Permitir WiFi com buffer maior** — buffer esconde jitter às custas de
  latência; para show sincronizado com música, latência variável é tão ruim
  quanto perda.
- **Só documentar, sem enforcement** — o rig do próprio usuário prova que a
  documentação sozinha não impede o erro; o guard torna a regra executável.
- **Detecção de jitter em runtime** — reativo demais; a falha já aconteceu no
  palco. O guard é preventivo, no start.
