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
  - "mnemonik-dev/universal-paywall — the facilitator; packages/facilitator/src/x402.ts"
  - "https://github.com/x402-foundation/x402/blob/main/specs/schemes/batch-settlement/scheme_batch_settlement.md"
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

## Universal Paywall: what it actually is

Read at `mnemonik-dev/universal-paywall@31761ec`. This section corrects an
earlier draft of this document that scoped the work without it.

UP is a **stake + session-key rail**: the payer locks USDC in a non-custodial
`StakeVault` and grants the facilitator a bounded, revocable policy (cap,
expiry). The facilitator meters charges, batches them, and settles on-chain.
Contracts are Solidity (`contracts/src/rail/StakeVault.sol`); the client library
is viem. **The rail is EVM-only.**

### Does it correspond to x402?

Partially — x402-shaped at the edge, bespoke underneath.

| | UP | x402 v2 |
| --- | --- | --- |
| 402 body | `accepts[]`, CAIP-2 networks | same idea | 
| Version | `x402Version: 1` everywhere | `2` |
| Scheme | `"stake"` — unregistered | `exact` / `upto` / `batch-settlement` / `auth-capture` |
| Extra fields | top-level `grant{}` | not in schema |
| Transport | JSON body | `PAYMENT-REQUIRED` / `PAYMENT-SIGNATURE` / `PAYMENT-RESPONSE` headers |
| Facilitator API | `/charge`, `/flush`, `/v1/quotes`, `/v1/payments/settle`, `/v1/sessions` | `/verify`, `/settle`, `/supported` |

### The `stake` scheme is already standardised

x402 v2's `batch-settlement` describes UP's model almost verbatim — a
capital-backed commitment where "the trust anchor is the client's own funds",
access is granted immediately, and "value moves later, through the network
binding's redemption process". UP's `/charge` + `/flush` is that redemption
process; `StakeVault` is that capital.

So conformance does **not** mean inventing a scheme or abandoning the rail. It
means mapping `stake` onto `batch-settlement` (and metered/variable pricing onto
`upto`), then exposing the standard facilitator contract over the existing
implementation.

### UP's own phase gate

`packages/facilitator/src/session-service.ts:42` — hosted Phase 1 supports
one-time `exact` only; `stake` is opt-in "until its separate
reservation/reconciliation hardening gate is approved". Stake is not hosted-ready
regardless of what we do here.

## Gating decision 1 — chains: both, in two phases

x402 is multi-chain by construction: `accepts[]` is an array of scheme+network
pairs, the client picks one, and `/supported` advertises
`signers: {"eip155:*": […], "solana:*": […]}`. Every scheme we need has both
bindings — `scheme_{exact,upto,batch_settlement}_{evm,svm}.md`.

So both work, but they are not equally cheap:

- **`exact` on both — cheap.** Solana already has `treasury_pubkey`, `usdc_mint`,
  `solana_rpc_url`; EVM already has `evm_treasury`, `evm_usdc_token` and an
  EIP-3009 path. Both become `accepts[]` entries with CAIP-2 ids.
- **`batch-settlement` (= UP stake) on Solana — expensive.** The rail is
  `StakeVault.sol`. An SVM binding needs an equivalent Solana program, not a
  config change.

**Decision: ship conformant `exact` on eip155 + solana first. Add
`batch-settlement` on EVM when UP's hardening gate opens. Solana stake last, and
only if the product needs it.**

## Gating decision 2 — facilitator

UP is ours (`mnemonik-dev/universal-paywall`), so this is a sequencing question,
not a build-or-buy one.

1. **Conform UP to x402 v2, mnemonic passes through.** Add `/verify`, `/settle`,
   `/supported`; move to v2 types and header transport; map `stake` →
   `batch-settlement`. The rail and contracts stay as they are. Clients then talk
   real x402 and can use off-the-shelf libraries. **Most of this work lands in the
   universal-paywall repo, not this one.**
2. **Keep UP bespoke, mnemonic keeps wrapping it.** What upstream ships today
   (`api.rs:376` → `awaiting_payment` + `approval_url`). Works now; never
   conformant from the client's point of view.
3. **Public facilitator for `exact` now, UP for `batch-settlement` later.**
   Fastest route to genuine x402, at the cost of running two rails during the
   transition.

**Recommended: 1, with 3 as a bridge if conformance is needed before UP can
move.** Note that in SVM `exact` the facilitator is also the *sponsor* — it holds
a keypair, signs as `feePayer`, funds fees and submits. That is an operational
commitment (hot wallet, funding alerts, sponsor acceptance policy) that nothing
in either repo does today.

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
