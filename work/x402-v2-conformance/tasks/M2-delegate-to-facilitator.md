---
status: pending
priority: P1
size: M
depends_on:
  - tasks/M1-emit-conformant-402.md
  - "U1 (universal-paywall) — POST /verify, POST /settle, GET /supported"
---

# M2 — Accept the payment header and delegate to the facilitator

## Goal

Complete the agent-unattended loop: an agent that receives our 402 pays from its
own policy, retries, and gets the resource. No browser, no human.

## Context

The x402 facilitator API is specified as HTTP precisely so a resource server in
any language can delegate chain work. That is what makes this task small: this
repo verifies nothing on-chain and holds no keys.

Today we call `POST {url}/v1/payments/settle` (`universal_paywall.rs:224`) —
the facilitator's *internal* REST API, one layer beneath its x402 surface.

## Scope

- Read the payment header (`PAYMENT-SIGNATURE`, v2; accept `X-PAYMENT` while UP
  is still on v1) and base64-decode to `PaymentPayload`.
- Call the facilitator's `POST /verify` with `{paymentPayload, paymentRequirements}`.
- On success run the resource — anchor the artifact — then `POST /settle`.
  Order matters: this is the `authorization` payment flow (verify → resource →
  settle). Do not settle before the resource succeeds.
- Return the settlement result as a base64 `PAYMENT-RESPONSE` header.
- Map the spec's error reasons (`insufficient_funds`, `unsupported_scheme`, …)
  onto our existing typed JSON-RPC errors rather than inventing new ones.
- Replace the bespoke settle call at `universal_paywall.rs:224`.

## Non-scope

- Verifying EIP-3009 authorizations here. That is the facilitator's job and must
  not be duplicated in Rust.
- Any keypair, sponsor role, or chain write in this repo.
- Deleting the old rail — M3, once this one is proven.

## Acceptance criteria

- An agent with a funded policy completes anchor-and-pay against a live
  facilitator with no browser interaction at any point.
- `/verify` failure returns 402 with the facilitator's reason surfaced; the
  resource does not run and nothing settles.
- Resource failure after a successful `/verify` settles nothing.
- `/settle` is called at most once per operation; a retry of the same request
  never produces a second charge.
- `PAYMENT-RESPONSE` decodes to a spec `SettlementResponse`.
- Verified end to end against UP's staging facilitator, not a mock.

## Notes

Cannot be verified end to end until U1 lands — the endpoints do not exist yet.
Sequence rather than parallelise; a stub facilitator here would only re-create
the mock problem this work exists to remove.

`paid_operation`'s state machine was built around approval-URL semantics. Expect
several of its states to have no analogue in this flow; leave them alone here
and resolve in M3.
