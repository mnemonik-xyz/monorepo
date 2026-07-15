---
status: superseded
priority: P1
depends_on:
  - tasks/06-add-payment-recovery-and-security-matrix.md
---

# Validate staging Irys/Solana delivery and operational readiness

> Superseded by Tasks 07a–07c. This file remains as the parent scope and
> acceptance record; execute the smaller tasks in order.

## Goal

Move from local `arlocal`/validator evidence to staging validation with the
real delivery dependencies and operator safeguards.

## Scope

- Add a separately gated staging test using real Irys and Solana testnet or
  approved staging infrastructure.
- Verify receipt-to-delivery reconciliation and restart recovery.
- Document service credentials, receipt key publication, health/readiness,
  settlement lag, failed-operation, and gas/balance metrics.
- Define the user-visible refund/service-credit policy for a settled but
  permanently undeliverable operation.

## Acceptance criteria

- Staging proves exact payment through verified external delivery and recall.
- Monitoring detects stuck payments and reconciliation mismatches without
  leaking secrets.
- Production-switch gates in `../tech-spec.md` have named evidence.
