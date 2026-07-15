---
status: complete
priority: P1
depends_on:
  - tasks/01-persist-paid-operations.md
  - tasks/02-bind-canonical-client-signed-artifact.md
  - tasks/03-link-wallet-to-mnemonic-subject.md
  - tasks/04-secure-approval-and-resume.md
  - tasks/05-enforce-exact-first-phase-one-policy.md
---

# Add Phase 1 recovery, replay, and concurrency coverage

## Goal

Turn the current happy-path E2E into a release gate for exact payment safety.

## Required coverage

- fifty concurrent exact attempts for one operation settle once;
- MCP restart and provider restart at every payment state;
- quote expiry and safe refresh before approval;
- wallet rejection and insufficient USDC;
- proof replay with altered artifact, subject, wallet, amount, payee, network,
  nonce, or expiry;
- duplicate approval callback and uncertain RPC settlement;
- delivery failure after payment, retry, abandonment, and receipt visibility;
- legacy artifact recall/verification regression suite.

## Acceptance criteria

- Every scenario asserts no duplicate charge.
- Every successful exact payment produces a verifiable signed receipt.
- Mock E2E remains the fast CI gate; real-wallet E2E runs as a staging or
  pre-release gate.

## Agreed recovery invariants

- A durable exact provider receipt is the sole authority to begin or resume
  delivery. Delivery retries must never request another EIP-3009 signature or
  call exact settlement again.
- Phase 1 has no vault. When stake is introduced later, delivery retries must
  work from a single durable reservation and may commit it once only; retrying
  delivery must never drain the allowance/vault.
- A settled-but-undelivered operation remains recoverable with bounded
  retries. It becomes `abandoned` / refund-or-credit eligible only after
  durable evidence says delivery cannot be recovered.

## Progress

- Approval/status resume now requires an expiring single-purpose capability
  derived from the facilitator secret and the immutable operation, quote, and
  quote expiry. It survives MCP restart without persisting a browser bearer
  secret. The mock end-to-end test covers this path.
- Delivery attempts now have a durable lease and state. A successful Arweave
  upload is recorded before Solana anchoring; a later callback can reuse that
  stored Arweave id and retry the remaining delivery without returning to
  payment settlement.
- The MCP runs a bounded background recovery worker for settled paid
  deliveries. It polls due attempts in batches of 16, reuses the staged COSE
  envelope, and has no payment proof or settlement authority. Retry delays are
  1, 2, 4, 8, 16, 32, 60, and 60 minutes; the eighth failure is terminal
  `abandoned`, rejects late callbacks, and creates one durable operator review
  case. Production timing is always calculated from `Utc::now()`; fixed
  datetimes exist only in deterministic unit tests.
- The exact provider suite now covers fifty concurrent attempts, provider
  restart in created, settling, failed-retryable, rejected, and settled states
  with a durable exact receipt, quote expiry, insufficient-USDC rejection, and
  replay attempts which alter every quote-bound field. It verifies every exact
  receipt's Ed25519 signature and no-recharge reconciliation after ambiguous
  settlement.
- The mock E2E rejects the first EIP-3009 signature using the standard wallet
  rejection code, verifies the page makes the quote safely retryable, then
  verifies exactly one USDC payment, restart-before-delivery recovery,
  duplicate callback rejection, anchoring, and recall. It now throws rather
  than exits inside assertions so failed iterations reliably clean up their
  validators and browser processes.
- The core rebuild suite now proves that a previously anchored schema-v1
  signed artifact can be rebuilt into SQLite and recalled with its original
  content hash.
- The fully automated real MetaMask/Dappwright pre-release gate was run after
  the final harness changes and passed, as did the mock E2E and full
  facilitator and MCP test suites.

## Verification commands

- `cargo test -p mnemonic-mcp --lib` — 238 passing tests.
- `cargo test -p mnemonic-core --test integration_rebuild` — 8 passing tests.
- `npm test && npm run typecheck` in `packages/facilitator` — 43 passing
  tests and TypeScript check.
- `npm run typecheck && npm test` and `npm run test:real-metamask` in `e2e` —
  mock and fully automated real-wallet pre-release gates passed.
