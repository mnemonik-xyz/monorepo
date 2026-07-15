---
status: ready
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

