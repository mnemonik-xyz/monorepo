---
status: in_progress
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
  payment settlement. Automatic background retry/backoff and Solana
  submission-reconciliation remain follow-up work; retries currently occur
  only through the normal explicit resume path.
