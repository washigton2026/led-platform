# LUMYX — Supply chain

## Política de dependências

- **std-only por padrão.** Uma dependência externa entra apenas com justificativa
  escrita no `Cargo.toml` do crate que a usa (convenção do CLAUDE.md).
- Dependências atuais com justificativa: `cpal`/`rustfft`/`tokio-sync` (audio-core,
  leaf), `arc-swap` (RT-LOCK-RENDER-001), `wgpu` (feature `gpu`), `gif` (led-demo),
  `ed25519-dalek` (assinatura de replay/snapshot — pure Rust, sem feature de RNG;
  seeds vêm do SO em `signing.rs`).
- `Cargo.lock` é commitado — builds reprodutíveis.

## Gates automatizados (rodam no `~/lumyx-e2e.sh`)

| Gate | Fase | O que prova |
|---|---|---|
| `cargo audit` | Phase 5 | 0 vulnerabilidades HIGH/CRITICAL nas 144 deps externas |
| Debt ledger (`audit_gate.py`) | Phase 5b | todo TD fechado tem evidência + controle negativo |
| SBOM | manual/CI | inventário CycloneDX 1.5 completo |

## SBOM

```sh
python3 scripts/generate_sbom.py          # → docs/sbom/sbom.cdx.json
```

CycloneDX 1.5, gerado de `cargo metadata --locked` (sem rede além do registry já
resolvido). 158 componentes: 14 do workspace, 144 externos, cada um com purl
(`pkg:cargo/...`) e licença — o formato que `cosign attest --type cyclonedx` espera.

## Assinatura (implementado)

- **Ed25519** (`led-show-recorder/src/signing.rs`): manifests de replay e
  snapshots `.lumyx` assinados/verificados offline; sidecar `*.sig`.
- **cosign v3** (`scripts/release_sign.sh`): cada binário de release recebe
  `*.cosign.bundle` (assinatura) + `*.sbom.bundle` (attestation CycloneDX do
  SBOM), verificados no próprio pipeline (`verify-blob` faz parte do script —
  assinatura não-verificável é bug). Chave local em `release/` (gitignored,
  passphrase via `COSIGN_PASSWORD`); migrar para OIDC keyless quando houver CI
  com identidade federada.

## Ameaças consideradas

1. **Dependência maliciosa/comprometida** → superfície mínima (std-only), audit
   no CI, lockfile, SBOM para resposta a incidente.
2. **Show adulterado entre estúdio e palco** → manifest Ed25519; verificação no
   player antes do primeiro frame.
3. **Firmware/controlador** → fora do escopo do workspace; Hardware Rule: nunca
   atualizar firmware durante show; kill switch testado antes de evento público.
