---
updated: 2026-07-15
status: phase-1-staging-runbook
scope: exact-payment-only
---

# Paid anchoring staging operations runbook

This runbook is for the isolated Base Sepolia staging environment. It is not a
mainnet launch procedure. Its purpose is to detect and remedy a *settled* exact
payment whose Arweave/Irys or Solana delivery has not completed.

## Safety invariants

- A delivery retry resumes the same `operation_id`, signed COSE envelope, and
  receipt. It never creates a quote, resubmits EIP-3009, or charges the wallet.
- The background worker has only staged delivery data. It has no raw wallet
  authorization and no settlement authority.
- Operators identify an operation through `operation_id`, the provider receipt
  id, and transaction identifiers. Do not retrieve artifact plaintext, COSE
  bytes, EIP-3009 typed data, wallet signatures, private keys, or API-key
  values for routine diagnosis.
- The facilitator's file-backed payment store and Mnemonic SQLite database are
  durable state. Never delete their volumes as a retry or rollback action.

## Ownership and secret boundaries

| Component | Operational owner | Secret/configuration location | Routine check |
|---|---|---|---|
| Facilitator receipt signer | Paywall operator | `/opt/universal-paywall-staging/secrets/receipt-private-key.pem`, uid 10001, mode 0600 | Receipt key endpoint and signed receipt verification |
| EVM settlement key | Paywall operator | SOPS-rendered facilitator `.env`; never CI logs | Base Sepolia RPC reachability and facilitator health |
| Mnemonic/approval API credentials | Mnemonic operator | SOPS-rendered MCP/approval configuration | Authenticated operation-status request |
| Solana relay/keypair | Mnemonic operator | SOPS-rendered MCP configuration | relay balance and submitted signature status |
| Irys funding credentials | Mnemonic operator | SOPS-rendered MCP configuration | funded balance and upload availability |
| Base Sepolia RPC / USDC / payee | Release owner | non-secret `.env` values, reviewed at deploy | chain id 84532, USDC address, payee binding |

The facilitator deployment workflow must run through the protected
`paywall-staging` GitHub Environment. CI may list configuration key names and
file modes, but must never emit values.

## Health and readiness

Before an external E2E or any remediation, verify:

1. The isolated `universal-paywall-staging` Compose project is healthy and is
   using an immutable image reference.
2. Facilitator `/health` responds locally, its configured chain is Base Sepolia
   (`84532`), and `EXACT_PAYMENTS_ENABLED=1`.
3. Mnemonic's authenticated operation-status endpoint can read an existing
   operation only with its single-purpose resume capability.
4. The Solana relay has sufficient devnet/test funds for one bounded retry and
   Irys has sufficient testnet/devnet upload credit. Do not top up a production
   account merely to make staging green.
5. Arweave/Irys retrieval and Solana RPC confirmation succeed independently.

The expected redacted monitoring dimensions are:

| Signal | Dimensions | Alert / investigation condition |
|---|---|---|
| Exact settlement | outcome, provider error class, chain id | a settled receipt without a `payment_ready` operation |
| Delivery attempt | state, attempt count, retry age | retry age exceeds the configured service objective |
| Delivery confirmation | stage (`upload`, `solana`, `recall`) | repeated `delivery_retryable` or receipt/recall mismatch |
| Retry worker | batch size, resumed count, errors | worker is silent while due rows exist |
| Relay/Irys capacity | network, remaining balance bucket | balance is below the single-retry threshold |
| Reconciliation | operation state vs provider receipt vs Solana/Arweave ids | any three-way mismatch |

Use opaque ids or one-way hashes in metrics and logs. `operation_id` may go in
the restricted audit record, not high-cardinality public telemetry.

## Diagnose a delayed paid operation

1. Open a restricted incident record with `operation_id`, time observed,
   authenticated subject reference, and the support request. Do not paste
   content or raw authorizations.
2. Read the authenticated Mnemonic operation status. Record only its state,
   quote id, receipt id, timestamps, and existing Arweave/Solana identifiers.
3. Query Universal Paywall `status(operation_id)` with the service API key.
   Verify the signed receipt binding digest, amount, asset, network, payee,
   payer, and operation id agree with Mnemonic's stored metadata.
4. If a receipt is not settled, the user has not been charged. Handle quote
   expiry or wallet rejection in the UI; do not run delivery recovery.
5. If settled, inspect the staged delivery attempt state and its redacted
   fields: attempt number, lease expiry, next retry, error class, and any
   Arweave/Solana transaction id. The system's state vocabulary is:

   | State | Meaning | Safe next action |
   |---|---|---|
   | `payment_ready` | settled, not yet leased | bounded delivery worker may start |
   | `anchoring` / `verifying_delivery` | a valid lease is active | wait for lease expiry; do not start a parallel upload |
   | `delivery_retryable` | a retry is due or scheduled | let the background worker retry; an operator may trigger the same resume path after diagnosis |
   | `anchored` | receipt and delivery evidence persisted | close as successful |
   | `refund_pending` / `abandoned` | automatic work has stopped | follow the review process below |
6. For an ambiguous Solana RPC outcome, reconcile by the persisted signature
   and the expected memo/anchor before retrying. Only submit again when no
   valid prior submission can be confirmed. An upload with an existing Arweave
   id must be reused, not duplicated.

## Abandoned-operation review and remedy

An operation is `abandoned` only after bounded retries/reconciliation have
failed or a permanent delivery condition is established. It is never an
automatic-refund trigger.

The on-call Mnemonic operator owns the incident until it is handed to the
Paywall release owner. The restricted audit record must include:

- `operation_id`, receipt id, binding digest, timestamps, and redacted error
  class;
- quote/receipt verification result and chain identifiers checked;
- delivery attempts, retry age, lease/reconciliation outcome;
- the chosen outcome, approver, and user communication timestamp.

The release owner chooses exactly one user-visible outcome:

1. **Complete delivery:** resume the existing delivery after the external
   dependency recovers, then confirm recall and persist the delivery receipt.
2. **Refund:** create a separately approved, auditable refund outside the
   retry worker. Link its transaction id to the review case.
3. **Service credit:** issue a documented credit with scope/expiry and link it
   to the review case. It must not silently become a recurring vault balance.

Mark the review case closed only after the user-visible status and the chosen
outcome are both recorded. No routine retry, restart, rollback, or refund may
drain a vault: Phase 1 has no vault rail, and its retries only consume the
already-settled operation's delivery work.

## Evidence required before production switch

- Immutable image digests and protected-environment deployment logs.
- Redacted external E2E record: Base Sepolia receipt, real Solana testnet
  signature, Irys/Arweave id, recall verification, and one-operation/one-charge
  assertion.
- Reconciliation samples for normal delivery, ambiguous Solana timeout,
  delivery retry, MCP restart, duplicate callback, quote expiry, and wallet
  rejection.
- Documented abandoned-operation review showing either a completed delivery or
  a separately approved refund/service-credit outcome.
- Monitoring/alert ownership and tested restoration of the facilitator payment
  store and Mnemonic SQLite state.
