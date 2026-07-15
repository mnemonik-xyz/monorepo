---
status: complete
priority: P1
depends_on:
  - tasks/07a-build-staging-external-delivery-gate.md
---

# Define staging operations, observability, and paid-delivery remedy

## Goal

Provide the operational controls needed to detect, reconcile, and remedy a
settled exact payment whose external delivery is delayed or permanently fails.

## Scope

- Document ownership and secret references for facilitator receipt signing,
  EVM settlement, Solana relay, Irys funding, and staging endpoints.
- Define health/readiness checks and redacted metrics for payment settlement,
  delivery attempts, retry age, abandonment, relay balances, and
  receipt/delivery mismatches.
- Specify the audited operator workflow for `abandoned` operations: evidence
  collection, user-visible status, refund or service-credit decision, and
  closure. No automatic refund transaction is introduced by this task.
- Identify the named evidence required by the Phase 1 production-switch gates.

## Acceptance criteria

- Operators can identify a stuck operation by `operation_id` without accessing
  artifact plaintext or raw wallet authorization.
- The document makes clear that delivery retries never resettle or recharge.
- An abandoned operation has a defined owner, audit record, and user-visible
  outcome.

## Delivered

`../staging-operations.md` defines the restricted-data diagnostic path,
health/readiness checks, metric dimensions, reconciliation rules, explicit
abandoned-operation review, and the production-switch evidence set. It states
that retries resume delivery only and cannot resettle or recharge a wallet.
