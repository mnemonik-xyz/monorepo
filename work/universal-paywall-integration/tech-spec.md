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

- `exact` — standard one-time x402 payment for one operation;
- `stake` — optional capped, expiring authorization for recurring operations.

Mnemonic will integrate through a provider-neutral HTTP boundary rather than
importing the TypeScript resource adapter into the Rust MCP server. Both methods
bind payment to the same durable Mnemonic operation and produce the same receipt
shape. One-time x402 ships first; the stake method remains disabled in
production until its reservation and authorization model is hardened.

## Current-state gaps

The current Mnemonic x402 gate verifies a previously broadcast USDC transfer
from `X-Payment: {tx_sig, network}`. It binds the transfer to amount and
treasury, but not to an authenticated identity, content hash, correlation ID,
or expiring quote.

The current Universal Paywall stake rail:

- gates only on on-chain policy headroom;
- serves before reporting the charge;
- does not reserve individual operations;
- stores pending charges in memory;
- signs an access proof containing only payer and timestamp;
- allows the facilitator to settle to arbitrary payees within the cap; and
- publishes the rail packages as `0.0.0` source packages rather than a stable
  integration release.

Those properties are useful for an MVP streaming meter but insufficient for a
service that incurs Irys and Solana costs for each accepted operation.

## Architecture

```text
Mnemonic client / IDE
        │ client signs artifact
        ▼
Mnemonic approval webapp ───── wallet approval ───── EVM wallet
        │ operation + payment proof
        ▼
Mnemonic MCP (Rust)
        │ quote / authorize / reserve / commit / status
        ▼
Universal Paywall HTTP API
        │
        ├── exact x402 facilitator ── settle one payment
        └── stake facilitator ─────── reserve + batch settlement

Mnemonic MCP ── operator-funded relay ── Irys + Solana Memo
```

The payment provider never receives the artifact plaintext. It receives an
opaque operation binding containing hashes and identifiers.

## Shared operation binding

Every quote, proof, reservation, and receipt is bound to:

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
  "nonce": "<provider-generated random nonce>"
}
```

The canonical serialization and digest algorithm must be specified and shared
as fixtures. Changing any field invalidates the quote or proof.

`operation_id` is the idempotency key across Mnemonic, the webapp, and Universal
Paywall. Provider records must enforce uniqueness for `(service,
operation_id)`.

## Quote contract

Mnemonic requests a quote only after it has a canonical client-signed artifact.
The response advertises both supported methods when available:

```json
{
  "quote_id": "q_...",
  "binding": { "...": "shared operation binding" },
  "accepts": [
    {
      "scheme": "exact",
      "protocol": "x402",
      "authorization": "eip3009",
      "wallet_confirmations": 1
    },
    {
      "scheme": "stake",
      "facilitator": "0x...",
      "factory": "0x...",
      "recommended_cap": "10000000",
      "valid_until": "2026-07-20T00:00:00Z"
    }
  ]
}
```

Requirements:

- `exact` appears first and is the client default.
- The quote returns one total price; no fee may be appended after approval.
- Quotes expire quickly and cannot be modified in place.
- Network, asset, recipient, and protocol version are explicit.
- The response is safe to render in an untrusted client: no secrets and no
  executable callback URL supplied by the payer.

## Method 1: one-time x402 (`exact`)

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

## Method 2: recurring allowance (`stake`)

The StakeVault method is an optimization for repeat users, not a prerequisite
for anchoring.

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

### Durable reservation API

The existing serve-then-charge adapter is not used for Mnemonic. Universal
Paywall adds an atomic reservation lifecycle:

```text
POST   /v1/reservations                 create or return idempotent reservation
GET    /v1/reservations/{operation_id}  recover status
POST   /v1/reservations/{id}/commit     mark delivered and eligible to settle
POST   /v1/reservations/{id}/release    release an unspent reservation
```

A reservation binds the full operation digest and has these states:

```text
reserved -> committed -> settling -> settled
        \-> released
        \-> expired
```

Requirements:

- Creation is atomic and includes pending reservations when calculating
  remaining allowance.
- Concurrent requests cannot reserve the same headroom twice.
- The ledger is durable and survives process restart.
- `commit` and `release` are idempotent.
- A committed reservation is settled only to the quote's `pay_to`.
- Failed batch settlement remains retryable without duplicating a charge.
- On-chain reconciliation repairs uncertain transaction outcomes.
- Reservation expiry cannot silently convert a delivered operation into an
  unpaid success; the MCP sees a typed state and follows the documented recovery
  policy.

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

Additional terminal/recovery states include `payment_rejected`,
`quote_expired`, `payment_failed`, `delivery_retryable`, `refund_pending`, and
`abandoned`.

Payment persistence must not become a dependency for recalling already anchored
artifacts. Chain recovery remains able to restore anchored memories without the
payment database.

### Exact mapping

- `payment_ready` means the exact payment is settled and has a signed receipt.
- A delivery retry reuses that receipt.

### Stake mapping

- `payment_ready` means an allowance reservation exists.
- Mnemonic commits the reservation only after delivery verification.
- A retry observes and resumes the same reservation.

## Provider-neutral Mnemonic client

Add a Rust payment-provider boundary with operations equivalent to:

```text
create_quote(binding) -> Quote
authorize_exact(quote, proof) -> PaymentReceipt
reserve(quote, payer_proof) -> Reservation
commit(operation_id, delivery_receipt) -> PaymentReceipt
release(operation_id, reason) -> Reservation
status(operation_id) -> ProviderPaymentState
```

The interface must be mockable. MCP tool logic must not depend on Universal
Paywall TypeScript types or internal database schemas.

## UX integration requirements

- The webapp is the canonical wallet surface for browser, CLI, and IDE handoff.
- One-time payment is preselected on first use.
- The allowance option is explained as fewer future wallet prompts, not as
  staking terminology.
- Technical fields are available under **Payment details**.
- The approval URL identifies only one durable operation and cannot redirect to
  an arbitrary payer-provided origin.
- Polling is read-only and never creates a quote, reservation, authorization, or
  payment.
- Wallet rejection and quote expiry return the user to the same locally saved
  artifact.

## Refund and abandonment policy

Before production, the operator must publish one policy for an exact payment
that settled but never reached verified delivery. At minimum:

- the operation remains resumable for a stated period;
- status is visible to the user;
- the same receipt cannot purchase another operation; and
- after permanent abandonment, an automatic refund or service credit is
  recorded and traceable from the operation receipt.

Stake reservations that have not been committed are released rather than
charged.

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
- Expose health, readiness, settlement lag, failed reservations, and balance/gas
  metrics without leaking payment secrets.

## Rollout

### Phase 1 — exact payment foundation

1. Freeze the versioned operation binding, quote, and receipt schemas.
2. Productize one-time standard x402 in Universal Paywall.
3. Implement durable, idempotent exact-payment status and receipts.
4. Integrate exact payment into Mnemonic behind a staging feature flag.
5. Complete real-wallet tests and the full Irys/Solana delivery loop.

### Phase 2 — frictionless client surfaces

1. Ship the canonical web approval page.
2. Add CLI/SDK typed handoff and resume.
3. Add IDE and extension explicit anchor actions.
4. Confirm that unsupported clients receive an actionable upgrade response.

### Phase 3 — recurring allowance

1. Add payee-bound StakeVault policy and durable reservations.
2. Complete contract and facilitator security review.
3. Stage allowance setup, revoke, insufficient-cap, and concurrent-reservation
   journeys.
4. Enable the allowance option without changing one-time payment availability.

### Phase 4 — production switch

Enable the paid hosted path only after client compatibility, monitoring,
reconciliation, recovery, and refund gates pass. `local` remains free throughout.

## Acceptance criteria

### User journey

- [ ] One-time x402 is offered first and requires no vault or recurring grant.
- [ ] A connected-wallet user needs no more than one payment approval for an
      exact anchor.
- [ ] Allowance setup is opt-in and states cap, expiry, recipient, and revoke
      behavior in plain language.
- [ ] An active allowance removes the wallet prompt but does not remove explicit
      user confirmation of each anchor.
- [ ] No surface requires copying payment proofs, transaction hashes, or
      correlation IDs.
- [ ] Browser/IDE restart resumes the same operation.

### Correctness and security

- [ ] Exact payment and stake reservation bind the full operation digest.
- [ ] Fifty concurrent attempts for one operation yield one payment or one
      reservation.
- [ ] Pending stake reservations cannot exceed funded policy headroom.
- [ ] Provider/MCP restart at every state preserves idempotency.
- [ ] Proof replay against different content, user, amount, recipient, network,
      or expiry is rejected.
- [ ] A compromised service credential cannot redirect allowance settlement to
      an arbitrary payee.
- [ ] Delivery failure never requests a duplicate payment.
- [ ] Released reservations are not settled; committed reservations settle once.
- [ ] Already anchored memories remain recallable without the payment store.

### Interoperability and operations

- [ ] Exact mode passes fixtures with an independent standard x402 client.
- [ ] Universal Paywall publishes versioned schemas and production deployment
      metadata.
- [ ] Staging tests cover wallet rejection, network switch, insufficient USDC,
      quote expiry, duplicate callbacks, concurrent requests, facilitator crash,
      RPC uncertainty, Irys failure, Solana failure, verification failure, resume,
      revoke, release, and refund/credit.
- [ ] Production dashboards alert on stuck reservations, unsettled exact
      payments, settlement lag, reconciliation mismatch, and facilitator gas.

## Explicit non-goals

- Universal Paywall does not sign Mnemonic artifacts.
- Mnemonic does not custody user USDC or private wallet keys.
- The integration does not make Irys or Solana writes client-submitted.
- The recurring rail does not permit unlimited or non-expiring policies.
- Payment state is not used as the source of truth for anchored-memory recall.

