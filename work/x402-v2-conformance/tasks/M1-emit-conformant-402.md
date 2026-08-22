---
status: pending
priority: P1
size: S
depends_on:
  - "U1 (universal-paywall) — for GET /supported; M1 can land before it with a static accepts[]"
---

# M1 — Emit a conformant x402 `PaymentRequired`

## Goal

Replace the bespoke 402 body with the shape the protocol defines, so any x402
client can read it without possessing our code or Universal Paywall's.

## Context

`api.rs:376` currently answers:

```json
{ "status": "awaiting_payment", "correlation_id": "…",
  "artifact_hash": "…", "payment": { "operation_id", "approval_url", … } }
```

Nothing about that is x402. A client must be written against us specifically.

## Scope

- Add v2 wire types: `PaymentRequired`, `PaymentRequirements`, `ResourceInfo`.
  Replaces `X402Response` / `PaymentOption` (`payment.rs:51-72`).
- Emit from both 402 sites — `api.rs:376` (sign-callback) and `mcp.rs:1494`
  (JSON-RPC envelope). One shape, two transports.
- Field corrections against the current `x402_required()` (`payment.rs:590`):
  `x402Version: 2`; `amount` replaces `maxAmountRequired`; add `resource{}` and
  `maxTimeoutSeconds`; networks become CAIP-2 (`eip155:…`).
- Carry the human approval URL in `extensions`, never as a top-level field.
  The human path stays available without making the body non-conformant.
- HTTP transport: base64 `PAYMENT-REQUIRED` response header. The JSON-RPC path
  still needs a body — keep it there, and treat the header as canonical for the
  REST path.

## Non-scope

- Reading payment proofs — M2.
- Deleting the old model — M3.
- Advertising `stake`. Until U3 maps it onto `batch-settlement` it is not a
  registered scheme and must not appear in a conformant `accepts[]`.

## Acceptance criteria

- A 402 from the paid path decodes as a spec `PaymentRequired` with no
  unrecognised top-level fields.
- `accepts[]` carries one `exact` entry per configured network, CAIP-2 ids.
- The approval URL is reachable via `extensions` and absent from the top level.
- An off-the-shelf x402 client parses the response without custom code.
- No behavioural change to the free `local` path.

## Notes

`accepts[]` is an array, so adding networks or schemes later is additive. Once
U1 lands, populate it from the facilitator's `GET /supported` instead of config,
so we never advertise a rail the facilitator cannot settle.
