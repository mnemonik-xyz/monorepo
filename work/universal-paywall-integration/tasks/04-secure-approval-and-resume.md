---
status: complete
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

## Completed implementation

- Removed `/api/authorization` from the production router and deleted the E2E
  raw-authorization polling helper.
- `POST /api/settle` now checks the operation id, wallet address in the
  authorization, and the complete immutable provider quote binding before
  calling settlement. A previously persisted receipt is returned before any
  new facilitator request.
- Provider receipts are immutable durable state in `paid_operations`; the
  approval UI reloads `GET /api/operations/:operation_id` and renders settled
  state instead of prompting for another wallet signature.
- The local end-to-end test asserts the durable status/receipt response and
  that the former raw-authorization endpoint is no longer routed.

The status endpoint uses the high-entropy operation id delivered only through
the client-signed artifact / approval URL as an opaque capability. Before a
public internet rollout, Task 06 should decide whether the browser additionally
needs a first-party session or a separate expiring resume capability.
