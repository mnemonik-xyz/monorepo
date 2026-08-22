---
status: pending
priority: P1
size: M
depends_on:
  - tasks/M2-delegate-to-facilitator.md
---

# M3 — Delete the wrapper and the `tx_sig` model

## Goal

Remove the bespoke rail once the conformant one is proven, so there is exactly
one payment path and no second contract to keep working.

## Context

Three payment models currently coexist here. Two are dead ends and both must go,
or the next reader integrates against the wrong one — which is how we arrived
here.

## Scope

**Delete the `tx_sig` model.** `X402PaymentProof { tx_sig, network }`
(`payment.rs:40`) encodes pay-first-then-prove: the client submits its own
transaction and presents a receipt. That is not any x402 scheme. With it goes:

- `extract_x402_proof`'s `X-Payment` reader (`payment.rs:107`)
- `x402_required()` (`payment.rs:590`) — superseded by M1
- `verify_usdc_transfer` on the x402 path (`payment.rs:629`)
- the `x402_nonces` table and its helpers (`payment.rs:560-590`, `:705`).
  Replay defence belongs to the scheme and the facilitator. Keeping a local
  nonce table implies we still observe payment after the fact.

**Delete the wrapper.** `UniversalPaywallPaymentRequired` (`payment.rs:90`), the
`awaiting_payment` body (`api.rs:376`), and the `approval_url` redirect flow.
The human path survives as the `extensions` entry added in M1.

**Fix the inversion.** `api.rs:386` returns `500 "unexpected payment rail"` for
`PaymentGate::NeedPayment` — the conformant path wired as an error. It becomes
the only path; `PaymentGate` collapses accordingly.

**Reconcile `paid_operation`.** Its states assume approval-URL semantics. Retain
what the `authorization` flow needs (durable operation identity, restart
recovery, at-most-once settlement) and drop what only existed to drive a browser.

## Non-scope

- `approval.rs` and the embedded approval UI. Decide separately whether a human
  surface is still wanted; if it is, it stays as its own surface over the same
  rail. Do not delete it as a side effect of this task.
- Mock removal — a follow-up, and only after a live facilitator exists, or the
  paid path becomes untestable.

## Acceptance criteria

- No code path constructs or parses a `tx_sig` proof.
- `PaymentGate` has no variant that returns 500 for a valid payment.
- Exactly one 402 shape is emitted across `api.rs` and `mcp.rs`.
- Restart mid-operation still resumes without a second charge.
- Legacy attestations anchored under the old rail remain recallable and
  verifiable — this task removes a *payment* path, not a *read* path.
- The full suite passes, including the integration tests that `main` currently
  cannot build (see repo note below).

## Notes

`--features test-support` does not compile on `main`: `test_support.rs` and three
test files miss 10 `McpState` fields. That has to be fixed before this task's
acceptance criteria can be demonstrated at all.
