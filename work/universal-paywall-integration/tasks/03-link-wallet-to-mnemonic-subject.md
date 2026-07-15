---
status: in_progress
priority: P1
depends_on:
  - tasks/01-persist-paid-operations.md
  - tasks/02-bind-canonical-client-signed-artifact.md
---

# Bind the payer wallet to the authenticated Mnemonic subject

## Goal

Replace the configured test payer wallet with a fresh wallet-signed challenge
bound to the authenticated Mnemonic subject and the operation context.

## Scope

- Define a typed challenge containing service, subject, operation ID, chain ID,
  nonce, and expiry.
- Verify the wallet signature server-side and persist only the verified link
  metadata needed for recovery.
- Use an opaque subject hash in provider bindings; do not expose raw identity
  material to Universal Paywall.
- Keep deterministic test-wallet setup behind explicit test-only configuration.

## Acceptance criteria

- A wallet proof cannot be replayed for another subject, operation, chain, or
  expiry.
- A quote binds the verified wallet, not an environment default.
- Test and production paths use the same verification logic.

## Implementation progress (2026-07-15)

`mcp/src/wallet_link.rs` defines a five-minute EIP-191 `personal_sign`
challenge bound to the opaque subject hash, operation id, EVM chain id, and a
random nonce. Its persistent record is single-use and the verifier recovers
the signer address server-side. The next slice wires this challenge into the
approval UI and makes quote creation reject operations without its verified
wallet link.
