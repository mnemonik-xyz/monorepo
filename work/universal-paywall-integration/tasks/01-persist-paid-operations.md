---
status: in_progress
priority: P1
depends_on:
  - tasks/00-restore-mcp-test-fixtures.md
---

# Persist minimal paid-operation state and recovery

## Goal

Replace process-local Universal Paywall quote and authorization maps with a
SQLite-backed `PaidOperation` record keyed by `operation_id`.

## Scope

- Add an idempotent SQLite migration and typed storage API.
- Persist correlation ID, authenticated subject reference, wallet reference,
  binding digest, quote expiry, provider payment state, provider receipt, and
  delivery receipt references.
- Do not persist private artifact plaintext in the payment record.
- Define typed transitions: `awaiting_signature`, `awaiting_payment`,
  `payment_authorizing`, `payment_ready`, `anchoring`,
  `verifying_delivery`, `anchored`, plus documented recovery/terminal states.
- On restart, recover payment state using `GET /v1/payments/{operation_id}` and
  resume the same operation; never create a replacement quote or charge.

## Acceptance criteria

- Reopening the approval URL and restarting Mnemonic return the same operation
  and receipt.
- Provider and MCP restart scenarios preserve idempotency.
- A delivery retry reuses the original exact receipt.
- Existing legacy artifacts remain recallable without reading paid-operation
  records.

## Implementation progress (2026-07-15)

Implemented in `mcp/src/paid_operation.rs` and the Universal Paywall payment
gate:

- an idempotent `paid_operations` SQLite migration and typed operation states;
- minimal persistent records that exclude artifact plaintext;
- persisted quote metadata and provider receipts; and
- recovery of a settled exact payment through provider status before creating a
  new quote.

The existing local mocked E2E passes with this path and now restarts MCP after
browser settlement before delivering the signed artifact. This proves receipt
and delivery-context recovery from SQLite. This is deliberately not marked
complete yet: browser reload and delivery-retry scenarios need direct
integration coverage, and the remaining in-process maps must cease to be a
required source of truth.
