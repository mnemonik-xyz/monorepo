---
created: 2026-07-13
status: draft
size: L
related:
  - work/universal-paywall-integration/user-spec.md
  - "GitHub issue #203 — end-to-end x402 paid anchoring journey"
  - "https://github.com/mnemonik-dev/universal-paywall"
---

# Tech Spec: Universal Paywall integration for paid anchoring

## Summary

Universal Paywall will expose two interoperable payment methods behind one
versioned quote and receipt contract:

- `stake` — the primary capped, expiring seamless-session authorization;
- `exact` — an optional one-time x402 fallback for one operation.

Mnemonic will integrate through a provider-neutral HTTP boundary rather than
importing the TypeScript resource adapter into the Rust MCP server. Both methods
bind payment to the same durable Mnemonic operation and produce the same receipt
shape. V1 settles each session checkpoint synchronously before anchoring. It
deliberately defers reservation and batched settlement until real volume proves
that their gas savings justify the added complexity.

## Current-state gaps

The current Mnemonic x402 gate verifies a previously broadcast USDC transfer
from `X-Payment: {tx_sig, network}`. It binds the transfer to amount and
treasury, but not to an authenticated identity, content hash, correlation ID,
or expiring quote.

The current Universal Paywall stake rail:

- gates only on on-chain policy headroom;
- serves before reporting the charge;
- stores pending charges in memory;
- signs an access proof containing only payer and timestamp;
- allows the facilitator to settle to arbitrary payees within the cap; and
- publishes the rail packages as `0.0.0` source packages rather than a stable
  integration release.

Those properties are useful for an MVP streaming meter but insufficient for a
service that must receive an idempotent payment before incurring Irys and Solana
costs.

## Architecture

```text
Mnemonic client / IDE
        │ client signs artifact
        ▼
Mnemonic approval webapp ───── wallet approval ───── EVM wallet
        │ operation + payment proof
        ▼
Mnemonic MCP (Rust)
        │ quote / settle from session / exact fallback / status
        ▼
Universal Paywall HTTP API
        │
        ├── stake facilitator ─────── settle one session checkpoint
        └── exact x402 facilitator ── settle optional one-time payment

Mnemonic MCP ── operator-funded relay ── Irys + Solana Memo
```

The payment provider never receives the artifact plaintext. It receives an
opaque operation binding containing hashes and identifiers.

## Shared operation binding

Every quote, authorization, payment, and receipt is bound to:

```json
{
  "version": 1,
  "operation_id": "<stable UUID/correlation ID>",
  "payer_subject": "<opaque hash of authenticated Mnemonic subject>",
  "payer_wallet": "0x...",
  "artifact_hash": "<blake3 of canonical client-signed artifact>",
  "amount": "<micro-USDC decimal string>",
  "asset": "<USDC contract address>",
  "network": "eip155:<chain-id>",
  "pay_to": "<Mnemonic treasury/service address>",
  "expires_at": "<RFC3339 timestamp>",
  "nonce": "<Mnemonic-generated random UUID>",
  "scope": {
    "workspace_hash": "<optional blake3 workspace hash>",
    "visibility": "private",
    "action": "manual"
  }
}
```

V1 serializes the struct in the field order shown above as compact UTF-8 JSON
and computes a lowercase hex BLAKE3 digest. Shared fixtures must lock this wire
contract before production. Changing any field invalidates the quote or proof.

The first HTTP 402 is intentionally provisional because no payer wallet is
known yet. After wallet connection, the hosted page calls:

```text
POST /api/paid-operations/{operation_id}/prepare
{"payer_wallet":"0x..."}
```

Mnemonic binds the first valid wallet and returns `binding_status: "final"`,
the final binding, and its digest. The wallet/payment authorization is created
only after this response. The initial provisional digest is never placed in a
hosted-page URL and must never be signed.

`operation_id` is the idempotency key across Mnemonic, the webapp, and Universal
Paywall. Provider records must enforce uniqueness for `(service,
operation_id)`.

## Quote and hosted-handoff contract

Mnemonic creates a quote only after it has verified and durably stored a
canonical client-signed artifact. The HTTP 402 response advertises both
supported methods when available:

```json
{
  "operation_id": "<stable UUID>",
  "binding_status": "provisional",
  "binding_digest": "<provisional digest>",
  "binding": { "...": "shared operation binding" },
  "quote": {
    "amount": "1000",
    "asset": "0x...",
    "network": "eip155:5042002",
    "pay_to": "0x...",
    "expires_at": "2026-07-13T12:05:00Z",
    "scope": { "...": "same operation scope" },
    "accepts": [
      {
        "scheme": "stake",
        "recommended": true,
        "payment_url": "https://mnemonik-dev.github.io/universal-paywall-site/",
        "recommended_cap": "5000000",
        "max_per_anchor": "50000",
        "valid_for_seconds": 604800
      },
      { "scheme": "exact", "protocol": "x402", "recommended": false }
    ]
  }
}
```

Requirements:

- `stake` appears first and is the recommended client path. `exact` remains an
  optional alternative.
- The quote returns one total price; no fee may be appended after approval.
- Quotes expire quickly and cannot be modified in place.
- Network, asset, recipient, and protocol version are explicit.
- The response is safe to render in an untrusted client: no secrets and no
  executable callback URL supplied by the payer.
- Hosted links contain only `operation_id`, selected `scheme`, MCP base URL,
  and an optional return URL. The paywall reads status, connects the wallet,
  prepares the final binding, obtains authorization, then posts it to
  `/api/paid-operations/{operation_id}/authorize`.

## Optional method: one-time x402 (`exact`)

Universal Paywall must retain and productize its one-time x402 implementation as
a first-class rail rather than treating it only as legacy middleware.

### Requirements

- Accept the selected standard x402 wire version and `exact` scheme.
- Prefer an off-chain EIP-3009 USDC authorization where supported, so a
  connected user performs one wallet signature and the facilitator submits the
  transaction.
- Do not require a StakeVault, deposit, recurring policy, or token approval for
  this method.
- Verify the complete operation binding before settlement.
- Settle at most once for an operation, even under concurrent retries.
- Return the existing receipt for an idempotent retry instead of rejecting it as
  a new payment.
- Return a signed provider receipt containing binding digest, payer, amount,
  network, recipient, settlement transaction, and settlement time.
- Expose `GET /v1/payments/{operation_id}` for restart recovery and
  reconciliation.
- Maintain interoperability fixtures against at least one independent x402
  client and document any extension fields used for operation binding.

For one-time payment, settlement completes before Mnemonic begins paid anchoring.
If anchoring then fails, the settled receipt remains attached to the same
resumable operation. It cannot be moved to another artifact and is never paid a
second time.

## Primary method: seamless anchoring session (`stake`)

The StakeVault method is the primary real-usage journey. The user approves a
bounded session once; each conforming manual or automatic checkpoint settles
from that allowance without another wallet prompt.

### Contract requirements

- A policy must restrict settlement to the intended service/payee, not only a
  facilitator. V1 may support one fixed `pay_to`; a later version may use an
  allowlist commitment.
- Policy enforcement includes facilitator, allowed payee, cap, spent amount,
  expiry, and epoch.
- The gate verifies both policy headroom and funded vault balance.
- Revocation behavior and the settlement cooldown are displayed accurately to
  clients.
- Contract version and deployed bytecode are discoverable from the quote.
- Production contracts receive an independent security review and verified
  source publication.

### Session authorization

The session binding extends the on-chain policy with an off-chain, wallet-signed
typed authorization containing:

- session ID and payer wallet;
- opaque Mnemonic subject binding;
- fixed Mnemonic payee;
- total cap and per-anchor price ceiling;
- expiry and nonce;
- workspace hash and visibility;
- allowed actions such as `manual`, `pre_compaction`, and `session_end`; and
- optional maximum checkpoint count/rate.

The session never renews automatically. A new or enlarged cap, expiry, scope, or
price ceiling requires a new wallet approval.

### Durable synchronous settlement API

The existing serve-then-charge adapter is not used for Mnemonic. V1 settles
before anchoring:

```text
POST   /v1/sessions                     register/return the wallet-approved session
GET    /v1/sessions/{session_id}        cap, spent, expiry, scope, status
POST   /v1/payments/settle              synchronously settle one checkpoint
GET    /v1/payments/{operation_id}      recover the idempotent receipt/status
```

A per-operation payment has these states:

```text
created -> settling -> settled
       \-> failed_retryable
       \-> rejected
```

Requirements:

- Settlement validates session scope, policy headroom, funded vault balance,
  price ceiling, expiry, operation binding, and fixed payee.
- Settlement completes before MCP begins paid anchoring.
- Concurrent requests for one operation produce at most one settlement.
- The operation ledger and receipt store are durable across restart.
- An idempotent retry returns the same settled receipt.
- Failed or uncertain settlement is reconciled on-chain before retrying.
- On-chain reconciliation repairs uncertain transaction outcomes.
- A failed session constraint leaves the checkpoint local and unpaid.
- Each checkpoint is settled independently in V1; batching is deferred.

### Provider HTTP contract used by Mnemonic V1

Mnemonic authenticates server-to-server calls using `x-api-key`. Redirects are
disabled and all calls have bounded connect/request timeouts.

```text
POST /v1/payments/settle
GET  /v1/payments/{operation_id}
```

`POST /v1/payments/settle` receives:

```json
{
  "binding": { "...": "final shared operation binding" },
  "payment": {
    "scheme": "stake",
    "session_id": "s_...",
    "payer_wallet": "0x...",
    "authorization": { "...": "opaque provider payload" }
  }
}
```

For one-time x402, `payment.scheme` is `exact`, `session_id` is absent, and
`authorization` contains the exact-payment proof. The provider returns the
same receipt for every idempotent retry:

```json
{
  "operation_id": "<stable UUID>",
  "scheme": "stake",
  "status": "settled",
  "binding_digest": "<final binding digest>",
  "payer_wallet": "0x...",
  "amount": "1000",
  "asset": "0x...",
  "network": "eip155:5042002",
  "pay_to": "0x...",
  "settlement_tx": "0x...",
  "settled_at": "<RFC3339>",
  "receipt": { "...": "non-null signed provider receipt" }
}
```

Mnemonic rejects the response unless every receipt field matches the final
binding and authorization method. `GET /v1/payments/{operation_id}` returns
`operation_id`, `status` (`created`, `settling`, `settled`, `rejected`, or a
documented retryable failure), and the receipt when settled.

## Authentication and identity binding

- Mnemonic binds its OAuth/Ed25519 subject to the payer's EVM wallet using a
  fresh challenge signed by that wallet.
- Universal Paywall access proofs use typed structured data and bind service,
  operation ID, quote digest, chain ID, nonce, and expiry. The current
  `payer + timestamp` personal-sign proof is insufficient.
- Service-to-facilitator calls use scoped, rotatable credentials. Credentials
  authorize only the configured service/payee.
- Provider receipts are signed and verifiable without access to the provider's
  database.
- Raw wallet signatures, authorization payloads, API credentials, and private
  artifact content are never logged.

## Mnemonic paid-operation state

Mnemonic persists the paid operation independently of its recall index:

```text
awaiting_signature
  -> awaiting_payment
  -> payment_authorizing
  -> payment_ready
  -> anchoring
  -> verifying_delivery
  -> anchored
```

V1 recovery states are `payment_failed` and `delivery_retryable`. An expired
unpaid quote is replaced in place with a new nonce and expiry. V1 does not add
reserve/commit/release, batching, or an automatic refund state machine.

Payment persistence must not become a dependency for recalling already anchored
artifacts. Chain recovery remains able to restore anchored memories without the
payment database.

### Exact mapping

- `payment_ready` means the exact payment is settled and has a signed receipt.
- A delivery retry reuses that receipt.

### Stake mapping

- `payment_ready` means the checkpoint charge has settled from the session and
  has a signed receipt.
- A delivery retry observes and reuses that receipt without another settlement.

## Provider-neutral Mnemonic client

Add a Rust payment-provider boundary with operations equivalent to:

```text
settle(final_binding, session_or_exact_authorization) -> PaymentReceipt
status(operation_id) -> ProviderPaymentState
```

Mnemonic owns quote creation and the durable signed-operation state. The
interface is mockable; MCP tool logic does not depend on Universal Paywall
TypeScript types or internal database schemas.

## UX integration requirements

- The webapp is the canonical wallet surface for browser, CLI, and IDE handoff.
- A bounded seamless anchoring session is the recommended first-use path.
- The session is explained as a visible spending limit, not staking terminology.
- One-time payment remains an optional secondary action.
- Technical fields are available under **Payment details**.
- The approval URL identifies only one durable operation and cannot redirect to
  an arbitrary payer-provided origin.
- Polling is read-only and never creates a session, quote, authorization, or
  payment.
- Wallet rejection and quote expiry return the user to the same locally saved
  artifact.

## Post-settlement delivery policy

V1 deliberately avoids an automatic refund/service-credit state machine. For an
exact or session payment that settled but has not reached verified delivery:

- the operation remains resumable with the same receipt;
- status is visible to the user;
- the same receipt cannot purchase another operation; and
- permanent failure is reconciled manually by support using the operation ID
  and provider receipt until observed failure data justifies a productized
  refund policy.

## Packaging and operational requirements for Universal Paywall

- Publish a stable versioned HTTP API and OpenAPI/schema fixtures.
- Publish packages needed by Universal Paywall clients with non-zero semantic
  versions, or explicitly support HTTP-only integration for non-TypeScript
  services.
- Add a repository license consistent with the advertised open-core model.
- Document supported production networks, deployed contract addresses, USDC
  contracts, confirmation policy, and facilitator fee/gas model.
- Replace the in-memory ledger in production and document backup, restore, and
  reconciliation procedures.
- Expose health, readiness, settlement lag, failed operations, and balance/gas
  metrics without leaking payment secrets.

## Rollout

### Phase 1 — synchronous paid-session foundation

1. Freeze the versioned operation binding, quote, and receipt schemas.
2. Add the payee-bound StakeVault policy and typed session authorization.
3. Implement durable, idempotent synchronous settlement and signed receipts.
4. Integrate the paid-session path into Mnemonic behind a staging feature flag.
5. Retain/productize standard one-time x402 as an optional fallback.
6. Complete real-wallet tests and the full Irys/Solana delivery loop.

### Phase 2 — frictionless client surfaces and production readiness

1. Ship the canonical web approval page.
2. Add CLI/SDK typed session, handoff, status, and resume.
3. Add IDE/extension paid-session controls and one hook-capable automatic
   checkpoint integration.
4. Confirm that unsupported clients receive an actionable upgrade response.
5. Complete contract/facilitator security review and stage setup, revoke,
   insufficient-cap, price-ceiling, concurrency, and restart journeys.
6. Enable the paid hosted path only after compatibility, monitoring,
   reconciliation, and recovery gates pass. `local` remains free.

### Future optimization — reserve and batch only when justified

Add `reserve -> commit/release -> batch settle` only when measured transaction
volume shows that facilitator gas is material. The future design must preserve
the same operation binding, receipt, retry, cap, payee, and session UX.

## Acceptance criteria

### User journey

- [ ] A bounded paid session is the recommended path and states cap, per-anchor
      ceiling, expiry, payee, workspace, visibility, allowed actions, and revoke
      behavior in plain language.
- [ ] One wallet-approved session supports repeated conforming checkpoints
      without repeated wallet prompts.
- [ ] Automatic paid checkpoints occur only inside the explicitly approved
      session scope; everything else stays local and free.
- [ ] One-time x402 remains available as an optional fallback and requires no
      recurring grant.
- [ ] No surface requires copying payment proofs, transaction hashes, or
      correlation IDs.
- [ ] Browser/IDE restart resumes the same operation.

### Correctness and security

- [ ] Exact and session payments bind the full operation digest.
- [ ] Fifty concurrent attempts for one operation yield one settlement receipt.
- [ ] Session settlement enforces funded policy headroom and per-anchor ceiling.
- [ ] Provider/MCP restart at every state preserves idempotency.
- [ ] Proof replay against different content, user, amount, recipient, network,
      or expiry is rejected.
- [ ] A compromised service credential cannot redirect allowance settlement to
      an arbitrary payee.
- [ ] Delivery failure never requests a duplicate payment.
- [ ] One settled session checkpoint cannot be settled again on delivery retry.
- [ ] Already anchored memories remain recallable without the payment store.

### Interoperability and operations

- [ ] Exact mode passes fixtures with an independent standard x402 client.
- [ ] Universal Paywall publishes versioned schemas and production deployment
      metadata.
- [ ] Staging tests cover wallet rejection, network switch, insufficient USDC,
      quote expiry, duplicate callbacks, concurrent requests, facilitator crash,
      RPC uncertainty, Irys failure, Solana failure, verification failure, resume,
      revoke, and session expiry.
- [ ] Production dashboards alert on stuck payments, unsettled exact
      payments, settlement lag, reconciliation mismatch, and facilitator gas.

## Explicit non-goals

- Universal Paywall does not sign Mnemonic artifacts.
- Mnemonic does not custody user USDC or private wallet keys.
- The integration does not make Irys or Solana writes client-submitted.
- The recurring rail does not permit unlimited or non-expiring policies.
- V1 does not implement reservation, commit/release, or batch settlement.
- Payment state is not used as the source of truth for anchored-memory recall.
