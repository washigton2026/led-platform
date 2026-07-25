# ADR-0004 — Verificação de assinatura com chave fixada

- **Status:** aceito
- **Data original:** 2026-07-12 (achado RT-001 do Red Team + correção)
- **Fonte:** CLAUDE.md changelog 2026-07-12; docs/red-team/findings.md RT-001

## Contexto e problema
O `.lumyx` é assinado com Ed25519 para provar que o show que chega ao palco é o
que o estúdio aprovou (fronteira de confiança estúdio→palco). A primeira
implementação, `verify_manifest`, verificava a assinatura contra a **chave
pública embutida no próprio arquivo**. O Red Team (RT-001) provou o buraco: um
atacante altera o show, re-assina com a **própria** chave, embute a própria
pubkey, e a verificação retorna `Ok`. A assinatura provava *integridade /
consistência interna*, não *autenticidade*.

## Decisão
Separar os dois modos de verificação e usar o correto na fronteira de confiança:
- `verify_manifest` permanece, mas documentado com ⚠️: prova **integridade
  apenas**, para blobs de origem já confiável (arquivo local recém-escrito).
- `verify_manifest_pinned(signed, &trusted_key)` é o caminho **autêntico**:
  rejeita (`UntrustedKey`) se a chave embutida ≠ a chave pré-confiada. A chave
  confiável viaja **out-of-band** (32 bytes), nunca dentro do arquivo.
- Ligado no consumidor real: `led-player --verify-key <hex>` carrega o sidecar
  `.sig`, confere que ele cobre este show, e verifica com a chave fixada.

## Consequências
**Boas:** tamper re-assinado é rejeitado (provado e2e: atacante `34b4…`
rejeitado pela chave do estúdio → `SIG VERIFY FAILED` exit 1). O caminho de
fronteira de confiança agora é autêntico. Guardado contra regressão por
`lumyx_red_team.sh` + teste negativo `pinned_verify_rejects_resigned_tamper`.
**Ruins/custos:** o operador precisa transportar a chave pública por um canal
separado (não é mais "só o arquivo"). `verify_manifest` inseguro continua no
binário — mitigado por doc, mas idealmente `#[deprecated]` no futuro.

## Alternativas rejeitadas
- **Remover `verify_manifest`** — quebraria o uso local legítimo (verificar um
  arquivo que você mesmo acabou de escrever, onde a origem já é confiável).
- **Embutir uma cadeia de certificados** — overengineering para um único
  estúdio; a chave fixada out-of-band resolve o modelo de ameaça real.
- **Confiar na chave embutida com um "registro de chaves conhecidas"** — apenas
  desloca o problema; a fixação explícita por invocação é mais simples e clara.
