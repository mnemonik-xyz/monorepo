---
status: blocked_on_staging_configuration
priority: P1
depends_on:
  - tasks/07a-build-staging-external-delivery-gate.md
  - tasks/07b-document-operational-readiness-and-remedy.md
---

# Run and record the external staging delivery gate

## Goal

Execute the approved staging E2E with a real wallet and real external Solana
and Irys delivery, then record only redacted, independently verifiable
evidence.

## Required external authority

- Approved staging EVM/USDC/facilitator deployment and funded test wallet.
- Approved Solana staging/testnet relay identity with fees available.
- Approved Irys endpoint and funded storage identity.
- A named operator responsible for the remedy workflow.

## Acceptance criteria

- The gate passes without manual wallet clicks.
- Evidence links the signed provider receipt, EVM settlement transaction,
  Irys identifier, Solana transaction, and successful recall.
- Restart/reconciliation evidence shows no duplicate charge.
- The evidence record contains no private artifact content, credentials, or
  raw EIP-3009 authorization.
