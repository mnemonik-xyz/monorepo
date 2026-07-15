---
status: ready
priority: P1
depends_on:
  - tasks/01-persist-paid-operations.md
  - tasks/03-link-wallet-to-mnemonic-subject.md
---

# Secure approval handoff, receipt, and resume

## Goal

Remove the public raw-authorization retrieval flow and make the browser, CLI,
and IDE resume a durable operation through authenticated status and signed
provider receipts.

## Scope

- Remove or compile-gate `/api/authorization` for test-only use; it must not
  exist in the hosted production router.
- Validate that the approval request operation ID matches the binding before
  settlement.
- Persist the provider receipt and expose authenticated operation status.
- Make MCP use provider status/receipt to continue an operation without a
  client copying an `X-Payment` proof.
- Return a user-facing receipt and recovery state from the approval surface.

## Acceptance criteria

- Raw EIP-3009 authorization payloads are not exposed, logged, or required by
  normal clients.
- Reloading the page or reconnecting an IDE resumes the same operation.
- Duplicate browser callbacks return the same receipt and cannot charge twice.

