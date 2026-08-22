---
created: 2026-08-22
updated: 2026-08-22
status: draft
type: scope
size: M
priority: P1
related:
  - "mnemonik-dev/universal-paywall@31761ec — the payment rail"
  - "https://github.com/x402-foundation/x402/blob/main/specs/x402-specification-v1.md"
  - "https://github.com/x402-foundation/x402/blob/main/specs/x402-specification-v2.md"
  - "https://github.com/x402-foundation/x402/blob/main/specs/schemes/batch-settlement/scheme_batch_settlement.md"
  - "work/universal-paywall-integration/ — the wrapper this replaces"
---

# Scope: put mnemonic-mcp on the real x402 rail

## Summary

Universal Paywall already implements x402. mnemonic-mcp bypassed it.

An earlier draft of this document scoped a six-task rebuild of x402 inside the
monorepo. That was wrong, and it was wrong because it was written without
reading the facilitator. The rail exists and works; the defect is that
mnemonic-mcp integrates one layer beneath it and re-implements payment badly on
top.

The work is therefore: **stop wrapping, start speaking the protocol** — plus a
cross-language seam that has to exist because mnemonic-mcp is Rust and UP's
x402 layer is TypeScript.

## Product decision (settled)

**Agents pay unattended. Human approval is an option, not the path.**

An agent hits a 402, pays from its own policy, retries — no browser, no wallet
prompt. The approval UI remains available as a fallback for callers that cannot
pay autonomously. Two surfaces, one rail. UP already has both:
`packages/resource-adapter` (agent) and `packages/approval-ui` (human).

## What UP already provides

Read at `mnemonik-dev/universal-paywall@31761ec`.

UP is a **stake + session-key rail**: the payer locks USDC in a non-custodial
`StakeVault`, grants the facilitator a bounded revocable policy (cap, expiry);
the facilitator meters charges, batches, and settles on-chain. Contracts are
Solidity, client is viem — **the rail is EVM-only**.

Its x402 implementation is real, not decorative:

| Piece | Where | What it does |
| --- | --- | --- |
| 402 edge | `packages/facilitator/src/x402.ts` | `build402Body` → `{x402Version:1, accepts[], grant{}}` |
| Resource gate | `packages/resource-adapter/src/gate.ts` | emits 402, checks on-chain grant headroom, verifies payer proof |
| Payment path | `packages/middleware/src/core.ts` | decodes `X-PAYMENT`, verifies EIP-3009, settles on-chain, returns `X-PAYMENT-RESPONSE` |

That is the x402 `exact` flow as specified. The facilitator logic runs in-process
rather than behind HTTP, which the spec explicitly permits ("resource servers to
delegate blockchain operations to trusted third parties **or host the endpoints
themselves**").

**v1 is not obsolete.** The spec repo carries `x402-specification-v1.md` and
`x402-specification-v2.md` side by side, with `transports-v1` and
`transports-v2`. UP built to a live version of the spec and implemented it
correctly.

Two genuine gaps remain, both narrow:

- **`stake` is unregistered.** UP's `accepts[].scheme = "stake"` plus a
  non-standard top-level `grant{}` means an off-the-shelf x402 client cannot
  participate in the session model. `exact` is conformant; `stake` is a private
  extension.
- **v1, not v2.** A migration (header transport, `amount` over
  `maxAmountRequired`, `resource{}`, `maxTimeoutSeconds`), not a rebuild.

## The actual defect

`universal_paywall.rs:224` calls `POST {url}/v1/payments/settle` — the
facilitator's **internal REST API**. mnemonic-mcp skipped `resource-adapter` and
`middleware` entirely and integrated one layer below UP's x402 surface.

Everything downstream follows from that one choice:

| Symptom | Where |
| --- | --- |
| Bespoke 402 body `{status:"awaiting_payment", approval_url}` | `api.rs:376` |
| The conformant rail wired as a hard error, `500 "unexpected payment rail"` | `api.rs:386` |
| A third payment model, `X402PaymentProof { tx_sig }` — pay-first-then-prove, not any x402 scheme | `payment.rs:40` |
| Legacy `X-Payment` reader for that model | `payment.rs:107` |
| Replay defence bolted on because payment is only observable after the fact | `payment.rs:560-590` |

The unmerged July client (`wip/universal-paywall-july`) was written against UP's
*real* 402 — `accepts:[{scheme:"stake"}]`, `recommendedCap`, `validForSeconds`,
matching `Payment402Body` field for field. It was correct. It failed against
mnemonic-mcp only because the wrapper hides the rail.

## The seam: why HTTP, not a library

mnemonic-mcp is Rust. `resource-adapter` and `middleware` are TypeScript. "Just
use the adapter" is not available to us.

This is exactly what x402's facilitator API is for — it is specified as HTTP so
resource servers in any language can delegate chain work. UP embedded its
facilitator in a TS library, which serves TS resource servers and nothing else.

So responsibility splits cleanly:

- **UP** exposes the facilitator contract over HTTP: `POST /verify`,
  `POST /settle`, `GET /supported`. The logic already exists in `middleware`;
  this is mostly wiring it to routes alongside the existing
  `/v1/quotes`, `/v1/sessions`, `/v1/payments/settle`.
- **mnemonic-mcp** implements only the *resource server* half: emit a conformant
  402, read the payment header, call the facilitator, emit the response header.
  No EIP-3009 verification, no chain writes, no sponsor keypair in this repo.

Porting the gate to Rust is the alternative and is rejected: it duplicates
signature verification and settlement in a second language, against contracts
that will change.

## Chains

UP is EVM-only (viem, `StakeVault.sol`, `eip155:*`). So:

- **EVM `exact` — available now.** UP's middleware does it today; `evm_treasury`
  and `evm_usdc_token` already exist in config.
- **Solana `exact` — later, and not via UP.** `scheme_exact_svm.md` requires a
  sponsor that countersigns as `feePayer` and submits. UP has no Solana rail.
  Options are a Solana-capable public facilitator or new UP work. The monorepo
  already has `treasury_pubkey` / `usdc_mint` / `solana_rpc_url`, so the
  resource-server half is cheap; the facilitator half is not.
- **`stake` on Solana — last.** Needs a Solana program equivalent to
  `StakeVault.sol`.

Because `accepts[]` is an array, adding a chain later is additive — no rework.

## Tasks

### This repo

| # | Task | Size |
| --- | --- | --- |
| M1 | Emit a conformant 402 from the paid path | S |
| M2 | Accept the payment header and delegate to the facilitator | M |
| M3 | Delete the wrapper and the `tx_sig` model | M |

**M1.** Replace `UniversalPaywallPaymentRequired` (`api.rs:376`) with a real
`PaymentRequired` carrying `accepts[]`. Keep the human approval URL as an
`extensions` entry, not a top-level field.

**M2.** Read the payment header, forward `{paymentPayload, paymentRequirements}`
to the facilitator's `/verify` and `/settle`, return the settlement header.
Replaces the bespoke `/v1/payments/settle` call at `universal_paywall.rs:224`.

**M3.** Delete `X402PaymentProof`, `x402_required`, `verify_usdc_transfer` on the
x402 path, and the `x402_nonces` replay table — replay becomes the scheme's and
facilitator's job. Make `PaymentGate::NeedPayment` the success path, not the 500
at `api.rs:386`. Retire the `approval_url` wrapper flow.

### universal-paywall repo

| # | Task | Size |
| --- | --- | --- |
| U1 | Expose `/verify`, `/settle`, `/supported` over HTTP | M |
| U2 | Migrate the wire format to v2 | M |
| U3 | Map `stake` onto `batch-settlement` | L |

**U1.** Route-level exposure of logic that already exists in `middleware`.
Unblocks every non-TypeScript resource server, not just this one.

**U2.** `x402Version: 2`, header transport (`PAYMENT-REQUIRED` /
`PAYMENT-SIGNATURE` / `PAYMENT-RESPONSE`), `amount`, `resource{}`,
`maxTimeoutSeconds`.

**U3.** x402 v2's `batch-settlement` describes UP's model almost verbatim — a
capital-backed commitment where "the trust anchor is the client's own funds",
access granted immediately, "value moves later, through the network binding's
redemption process". `StakeVault` is that capital; `/charge` + `/flush` is that
redemption. Mapping `stake` onto it, with metered pricing onto `upto`, retires
the private scheme and the non-standard `grant{}` — after which agents can use
stock x402 clients for the session model.

Gated on UP's own review: `packages/facilitator/src/session-service.ts:42`
records that hosted Phase 1 is `exact`-only, with `stake` opt-in "until its
separate reservation/reconciliation hardening gate is approved".

### Ordering

`U1 → M1 → M2 → M3` ships agent-unattended `exact` on EVM.
`U2` and `U3` follow and are independent of the monorepo work.

## Mocks

Drop `/api/mock-sign` and `approval_mock_signer` (`approval.rs:504-579`) and the
`MNEMONIC_DEFERRED_SYNTHETIC_ANCHOR` bypass — but only after U1, or the paid path
becomes untestable. Real settlement needs a testnet and a live facilitator, which
UP already runs.

## Non-goals

- **Building x402 in this repo.** The rail exists. This is integration.
- **A sponsor keypair here.** Chain work stays behind the facilitator.
- **Solana**, until there is a facilitator that can sponsor SVM `exact`.
- **Client work**, until the server contract is real — otherwise it gets written
  twice. The July client on `wip/universal-paywall-july` is close to correct
  already and should be revisited after M2, not rewritten now.

## Risks

- **UP's phase gate is upstream of U3.** If stake hardening stalls, the session
  model stalls with it; `exact` is unaffected.
- **v1 vs v2 timing.** Doing U2 before M1/M2 means building the monorepo side
  against a moving target; doing it after means one deliberate migration. Prefer
  after.
- **Two repos, one feature.** M2 cannot be verified end to end until U1 lands.
  Sequence accordingly rather than developing them in parallel.
