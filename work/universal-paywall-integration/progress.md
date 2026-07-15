---
updated: 2026-07-15
status: remediation-in-progress
phase: 1-exact-payment-foundation
related:
  - user-spec.md
  - tech-spec.md
  - "GitHub issue #203 — end-to-end x402 paid anchoring journey"
  - "GitHub issue #216 — bind paid receipts to canonical client-signed artifacts"
---

# Universal Paywall integration — progress and remediation plan

## Current progress

The local exact-payment smoke test is working end to end:

```text
canonical artifact -> client COSE_Sign1 -> quote -> browser approval
-> MetaMask EIP-3009 settlement -> local Solana + Arweave anchoring -> recall
```

The real MetaMask mode now selects the local Anvil chain automatically and
closes MetaMask's **Select network** dialog. The Dappwright compatibility work
is in the local clone at commit `96aa291`; E2E consumes it through a temporary
`file:../dappwright` dependency at E2E commit `09073e0`.

This is a local Phase 1 smoke-test milestone. It is not a production-readiness
claim and it does not validate real Irys or production Solana delivery.

### Durable-operation progress

Mnemonic now has a minimal SQLite-backed `PaidOperation` record. It persists
the operation binding reference, quote metadata, state, and provider receipt
without storing artifact plaintext. On a resumed payment gate it asks the
provider for the existing operation status before issuing a quote, so a settled
exact payment can be recovered after the MCP's in-memory state is gone.

The unit suite (`cargo test -p mnemonic-mcp --lib`, 235 tests) and the local
mocked E2E pass with this foundation. The E2E now restarts MCP after browser
settlement and before the delivery callback, proving that the staged COSE,
delivery context, and provider receipt resume the same operation. Browser
reload, delivery-retry, and failure-path tests remain required.

The quote is now derived from a verified, staged COSE_Sign1 envelope using the
versioned signed-artifact hash; raw request content is no longer used by the
Universal Paywall route. The mocked E2E exercises this ordering and no longer
reads or resends the raw EIP-3009 authorization. The public legacy
authorization endpoint still exists and is scheduled for removal in Task 04.

## Review findings

The following items block Phase 2, staging-hosted paid anchoring, and mainnet
work:

1. **Canonical artifact binding.** The current quote binds a hash of raw input
   content before the canonical client-signed artifact exists. A quote and
   receipt must bind the versioned canonical signed artifact hash. This is
   tracked in [#216](https://github.com/mnemonik-xyz/monorepo/issues/216).
2. **Durable operation recovery.** Mnemonic currently retains Universal
   Paywall quote/authorization state in memory. A browser reload, client
   disconnect, or MCP restart must resume the same operation and must never
   request a second charge.
3. **Wallet/subject binding.** Production must link the authenticated Mnemonic
   subject to the payer wallet using a fresh wallet-signed challenge. A
   configured test wallet is not an identity-binding mechanism.
4. **No raw authorization handoff.** The public E2E authorization-retrieval
   route must not be part of the production flow. Browser, CLI, and IDE
   clients should use an authenticated operation-status/receipt contract;
   raw wallet authorizations are never exposed or logged.
5. **Test health.** The Mnemonic MCP test initializers need updating for the
   new payment fields, and coverage must include restart, replay, duplicate
   callback, quote-expiry, wallet-rejection, delivery-failure, and exact-mode
   concurrency scenarios.

## Design decision: durable state and chain evidence

The chain is the source of truth for EIP-3009 settlement and for completed
Solana/Arweave delivery evidence. It cannot be the only operation store:

- private artifact bytes and local signed-artifact staging must not be put on
  chain merely to support recovery;
- an unbroadcast authorization, quote expiry, browser handoff, and retry state
  have no complete on-chain representation; and
- operation recovery needs to present typed user-visible states before a
  settlement or anchor transaction exists.

Mnemonic will therefore persist a minimal `PaidOperation` record keyed by
`operation_id`. The record contains only correlation data, binding digest,
state, and provider/delivery receipt references—not private artifact contents.
Provider receipts and chain transactions remain independently verifiable
evidence. Legacy anchored artifacts continue through their existing recall and
verification paths unchanged.

## Remediation plan

1. Restore MCP test compilation and add regression coverage for current
   payment-state initialization.
2. Define and persist the minimal `PaidOperation` state machine in Mnemonic;
   recover provider status by `operation_id` after MCP restart.
3. Canonicalize and client-sign the artifact before quote creation, introduce a
   versioned artifact-binding format, and retain legacy recall compatibility.
4. Add fresh wallet-to-subject challenge binding before producing a quote.
5. Replace raw authorization polling with authenticated operation status and
   signed provider receipts for web, CLI, and IDE handoff/resume.
6. Keep `exact` as the default Phase 1 method. Stake/allowance support remains
   opt-in and gated until the reservation, payee-binding, and security-review
   requirements in `tech-spec.md` are met.
7. Expand tests: exact concurrency, provider/MCP restart, proof replay,
   wallet rejection, quote expiry, duplicate callback, uncertain settlement,
   delivery retry, and real Irys/Solana staging delivery.

## Exit criteria for Phase 1

- A one-time payment is bound to exactly one canonical client-signed artifact.
- Reloads, disconnects, and MCP/provider restarts recover the same operation.
- No retry can create a duplicate charge.
- Wallet identity is bound to the authenticated Mnemonic subject.
- Raw wallet authorizations and secrets are never exposed to other callers.
- Legacy anchored artifacts remain recallable and verifiable.
- The local and staging test matrices pass, including the failure/recovery
  cases listed above.
