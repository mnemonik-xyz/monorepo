---
status: complete
priority: P1
depends_on:
  - tasks/01-persist-paid-operations.md
---

# Enforce exact-first capability policy for Phase 1

## Goal

Ship one-time `exact` as the only hosted Phase 1 payment method. Preserve the
stake implementation behind an explicit disabled capability until its
reservation, payee-binding, and security-review gates are complete.

## Scope

- Make quote capabilities advertise `exact` first.
- Do not expose stake in the Phase 1 approval UI or hosted quote response.
- Retain stake code and tests for later hardening; do not remove it.
- Document the later opt-in allowance journey and its required security gates.

## Acceptance criteria

- A first-time user sees one price and one exact-payment approval.
- No vault, deposit, or recurring permission appears in Phase 1.
- Enabling stake requires an explicit feature/capability gate and its own
  reservation/reconciliation test matrix.

## Completed implementation

- The facilitator now has an explicit `enabledSchemes` capability gate.
  Hosted Phase 1 config enables only `exact`; quote responses advertise only
  the EIP-3009 x402 exact method.
- Stake registration and settlement return `stake_payment_disabled` unless
  `stake` is explicitly enabled. The stake implementation and its tests remain
  intact for later use.
- The hosted facilitator fails fast unless `EXACT_PAYMENTS_ENABLED=1`; it can
  no longer silently start in a stake-only or no-payment configuration.
- Unit coverage proves the exact-only quote surface and stake rejection.

## Later stake enablement gate

Stake is an opt-in, capped, expiring allowance—not a fallback for an exact
quote. Enabling it later requires all of the following before adding `stake`
to `enabledSchemes`: payee-bound on-chain policy, durable reservations and
release/commit semantics, balance and settlement reconciliation, concurrent
spend/retry tests, expiry/revocation handling, receipt/recovery behavior, and
a dedicated security review. It must then be introduced as a separate product
choice; Phase 1 never presents a vault, deposit, or recurring permission.
