---
status: ready
priority: P1
depends_on:
  - tasks/06-add-payment-recovery-and-security-matrix.md
---

# Build a separately gated external-delivery E2E

## Goal

Make the existing automated exact-payment E2E runnable against an approved
staging EVM network, Solana testnet/staging RPC, and Irys endpoint without
changing the fast local CI path.

## Scope

- Introduce an explicit staging E2E configuration contract: RPC endpoints,
  chain ID, USDC asset, payee, facilitator, Solana endpoint, Irys endpoint,
  and secret references.
- Keep secrets out of source, logs, URLs, fixtures, and task evidence.
- Make the external test opt-in and fail closed when required configuration is
  absent or internally inconsistent.
- Assert the same security invariants as the local test: canonical signed
  binding, one exact payment, durable receipt, restart recovery, one delivery,
  and recall from external delivery evidence.
- Preserve the local Anvil/validator/Arlocal test as the fast default gate.

## Acceptance criteria

- A non-secret command validates configuration before opening a wallet.
- The staging command cannot silently use localhost or a mock signer.
- A dry-run reports required configuration keys but never their values.
- The automated test emits a minimal redacted evidence record on success.

