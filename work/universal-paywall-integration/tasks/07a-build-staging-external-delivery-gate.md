---
status: in_progress
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

## Progress

- Completed: `e2e` has a fail-closed `staging:validate` configuration command
  and a secret-free `.env.staging.example`; it refuses localhost, Anvil, and
  malformed public endpoints.
- Completed: Universal Paywall publishes an immutable facilitator image, while
  Coding Fabric provides a separate, approval-gated Base Sepolia compose stack
  and manual deployment workflow. The stack is loopback-only, exact-only, and
  does not share the production MCP directory or volumes.
- Completed: the staging stack now contains its own Mnemonic MCP and a
  same-origin approval UI asset volume. Caddy exposes only the configured
  staging hostname on the tailnet; the facilitator remains private behind MCP.
- Completed: Coding Fabric renders the staging `.env`, facilitator receipt
  key, and Mnemonic delivery keypair from its existing age/SOPS secret source.
  `UNIVERSAL_PAYWALL_URL` is the explicit MCP-to-facilitator location boundary,
  so the reference co-located VPS topology can be split across servers later.
- Completed: the merged Universal Paywall release workflow successfully
  published pullable facilitator and approval-UI images to GHCR. Coding
  Fabric's deployment gate now accepts only `@sha256:` digest references (not
  moveable tags) and requires both the facilitator and MCP health checks before
  it reloads the public approval route. The current MCP image is likewise
  available under an immutable GHCR digest.
- Completed: `e2e` now has `npm run test:staging`, an opt-in headless runner
  for the deployed same-origin approval UI. It preflights the Base Sepolia
  configuration, uses a dedicated staging OAuth token and test wallet, and
  keeps the wallet key outside the page by accepting only the provider-issued
  exact EIP-3009 typed data. It verifies facilitator health, durable receipt,
  one USDC settlement, external Solana confirmation, duplicate-callback
  rejection, and recall; its evidence output excludes the raw authorization
  and all credentials.
- Still required: provision the protected `paywall-staging` GitHub Environment
  and SOPS-rendered VM secrets, deploy a reviewed staging image, wire a staging
  MCP/approval UI to the facilitator, then run the external E2E to produce
  redacted evidence.
