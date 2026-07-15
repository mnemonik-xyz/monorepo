---
status: ready
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

