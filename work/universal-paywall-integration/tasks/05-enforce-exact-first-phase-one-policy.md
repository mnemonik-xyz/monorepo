---
status: ready
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

