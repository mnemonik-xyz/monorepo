---
created: 2026-08-22
status: draft
type: scope
size: L
priority: P1
related:
  - "https://github.com/x402-foundation/x402/blob/main/specs/x402-specification-v2.md"
  - "https://github.com/x402-foundation/x402/blob/main/specs/transports-v2/http.md"
  - "https://github.com/x402-foundation/x402/blob/main/specs/schemes/exact/scheme_exact_svm.md"
  - "work/universal-paywall-integration/ — the shipped bespoke rail this replaces"
---

# Scope: real x402 v2 conformance (server side)

## Why

The server advertises x402 but does not implement it. Three separate wire
contracts exist in the codebase and none matches the spec:

- `X402Response` — `x402Version: 1`, v1 field names, JSON body
- `UniversalPaywallPaymentRequired` — bespoke `{status:"awaiting_payment", payment:{…}}`
- the unmerged July client — `{status:"payment_required", accepts:[{scheme:"stake"}]}`

Worse, the *closest-to-conformant* rail is wired as a hard error:
`api.rs:386` returns `500 "unexpected payment rail"` for `PaymentGate::NeedPayment`.

Goal: one conformant x402 v2 rail, settled through a real facilitator, with no
mock signer in the path.

## Conformance gaps

Evidence is `mcp/src/` at `944ccf7`.

| Concern | x402 v2 | Current | Where |
| --- | --- | --- | --- |
| Version | `x402Version: 2` | `1` | `payment.rs:52,616` |
| 402 transport | base64 `PAYMENT-REQUIRED` **header** | JSON body | `mcp.rs:1494`, `api.rs:376` |
| Client header | `PAYMENT-SIGNATURE` | `X-Payment` | `payment.rs:107` |
| Amount field | `amount` | `maxAmountRequired` | `payment.rs:63` |
| Network id | CAIP-2 (`solana:5eykt4Us…`) | `"solana-mainnet"` | `payment.rs:599` |
| Required fields | `resource{}`, `maxTimeoutSeconds` | absent | `payment.rs:51-72` |
| **Client proof** | `payload.transaction` — partially-signed tx | `{tx_sig, network}` | `payment.rs:40` |
| **Settlement** | facilitator `/verify` + `/settle` | direct on-chain lookup | `payment.rs:629` |
| Settlement reply | base64 `PAYMENT-RESPONSE` header | none | — |
| Replay defence | scheme + facilitator | `x402_nonces` keyed by `tx_sig` | `payment.rs:560-590` |

The bolded two are the redesign. The rest is renaming and relocation.

### The one that matters

`X402PaymentProof { tx_sig, network }` encodes **pay-first-then-prove**: the
client submits its own transaction and presents the signature as a receipt.

The spec's `exact` SVM scheme is the inverse. The client builds a transaction
paying the merchant, signs it **partially** — leaving the sponsor's `feePayer`
signature missing — and base64-serializes it into `payload.transaction`. The
sponsor verifies, adds the final signature, and submits.

Consequences of the current model that the spec's model removes:

- the client pays fees and needs SOL, not just USDC
- payment and request are separate events, so "paid but the request then failed"
  is a real state the server must reconcile
- the server can only observe payment after the fact, so replay defence has to be
  bolted on (`x402_nonces`)

Keeping `tx_sig` means not implementing x402, whatever the headers say.

## Gating decision 1 — chain: stay on Solana

`exact` has a normative SVM binding (`scheme_exact_svm.md`), so conformance does
**not** require an EVM pivot. Existing `treasury_pubkey`, `usdc_mint` and
`solana_rpc_url` config carries over; the network identifier becomes CAIP-2.

EVM (`evm_usdc_token`, `evm_treasury`, EIP-3009) can remain as a second entry in
`accepts[]` later. It is not on the critical path.

## Gating decision 2 — facilitator: unresolved, blocks everything

In SVM `exact` the facilitator is also the **sponsor**: it holds a keypair, signs
as `feePayer`, funds fees, and submits. That is an operational commitment, not an
integration detail. Three options:

1. **Conform Universal Paywall** — it already proxies settlement
   (`universal_paywall.rs:224` → `POST {url}/v1/payments/settle`). Needs
   `/verify`, `/settle`, `/supported` at the spec's contracts. Cheapest if the
   provider is ours to change.
2. **Public facilitator** — no sponsor keypair to run; adds a third-party
   dependency on the paid path and constrains supported networks.
3. **Self-host** — the spec explicitly permits the resource server to host these
   endpoints. Most control, most operational burden (hot wallet, fee funding,
   sponsor acceptance policy).

**This decision must be made before T3.** Everything downstream inherits it.

## Tasks

| # | Task | Size | Depends |
| --- | --- | --- | --- |
| 1 | v2 wire types | S | — |
| 2 | HTTP transport binding | S | 1 |
| 3 | Facilitator client | M | 1, decision 2 |
| 4 | `exact` SVM payment path | L | 3 |
| 5 | Make conformant rail primary | M | 4 |
| 6 | Remove mocks, stage on real facilitator | M | 5 |

**T1 — v2 wire types.** `PaymentRequired`, `PaymentRequirements`, `ResourceInfo`,
`PaymentPayload`, `SettlementResponse`. `x402Version: 2`, `amount` not
`maxAmountRequired`, CAIP-2 networks, `maxTimeoutSeconds`. Replaces
`X402Response` / `PaymentOption`. Pure types, no behaviour.

**T2 — HTTP transport binding.** Emit base64 `PAYMENT-REQUIRED` on 402; read
`PAYMENT-SIGNATURE`; emit base64 `PAYMENT-RESPONSE` on success. Retire
`extract_x402_proof`'s `X-Payment` path. Decide whether the JSON body is retained
transitionally for existing callers — the MCP JSON-RPC envelope at `mcp.rs:1494`
still needs *some* body.

**T3 — Facilitator client.** `POST /verify`, `POST /settle`, `GET /supported`,
with the spec's error codes (`insufficient_funds`, `unsupported_scheme`, …).
Reuse the HTTP plumbing in `universal_paywall.rs`; replace the bespoke contract.
`/supported` should gate which `accepts[]` entries are advertised.

**T4 — `exact` SVM payment path.** The redesign. Accept
`payload.transaction`, forward to `/verify`, run the resource, `/settle`.
Delete `verify_usdc_transfer` from the x402 path. Rework replay: `x402_nonces`
keyed by `tx_sig` is meaningless before settlement — the transaction is not
submitted by the client and has no signature yet. Reconcile with
`paid_operation`'s state machine, which assumes the approval-URL flow.

**T5 — Make conformant rail primary.** Replace the `500 "unexpected payment
rail"` at `api.rs:386` with the real path. Retire
`UniversalPaywallPaymentRequired`. Reconcile the two 402 emitters
(`mcp.rs:1494` JSON-RPC and `api.rs:376` sign-callback) onto one shape.

**T6 — Remove mocks, stage on a real facilitator.** Drop `/api/mock-sign` and
`approval_mock_signer` (`approval.rs:504-579`) and the
`MNEMONIC_DEFERRED_SYNTHETIC_ANCHOR` bypass. Note the ordering trap: removing
mocks before a facilitator exists leaves the paid path untestable. T6 lands after
T3, never before.

## Open question — is the browser approval page still needed?

`approval.rs` (581 lines) plus the embedded approval UI implement a
human-in-the-browser flow: 402 carries `approval_url`, the user opens a page,
connects a wallet, approves.

x402 is agent-to-server. The client signs and retries a header; no browser, no
redirect. For agent callers the approval page is not part of the flow.

If the product still needs a human path, it survives as a *separate* surface, not
as the x402 rail. If it does not, T5 gets substantially larger — and mostly
deletions. Worth answering before T5 is planned in detail.

## Non-goals

- **Sessions / spending caps.** The July client's `stake` scheme with
  `recommendedCap` / `maxPerAnchor` / `validForSeconds` is not in the spec. v2
  supports this via `extensions` and scheme extensibility, but it needs a written
  scheme definition on top of conformant `exact`. Upstream's "exact-only phase
  one" call was right; keep it.
- **EVM.** Second `accepts[]` entry later.
- **Client work.** SDK, webapp and CLI come after the server contract is real —
  writing them first means writing them twice. Prefer an off-the-shelf x402
  client; this repo currently has zero x402 dependencies and hand-rolls all of it.

## Risks

- **Sponsor operations.** A hot keypair that funds fees and submits transactions
  needs a risk policy, monitoring, and funding alerts. Currently nothing in the
  repo does this.
- **`paid_operation` assumptions.** The state machine was built around
  approval-URL semantics; `exact` may not need most of those states.
- **Protocol surface owned in-house.** Every hand-rolled wire type is drift risk
  against a moving spec. v2 already superseded v1 here.
