---
status: ready
priority: P1
issue: https://github.com/mnemonik-xyz/monorepo/issues/216
depends_on:
  - tasks/01-persist-paid-operations.md
---

# Bind quotes and receipts to the canonical client-signed artifact

## Goal

Create a versioned paid-artifact binding after canonicalization and local client
signing, replacing the current hash of raw request content.

## Scope

- Specify the exact bytes hashed and the binding-version field.
- Canonicalize the artifact, have the client produce COSE_Sign1 locally, then
  derive `artifact_hash` from the versioned signed artifact representation.
- Create the quote only after this binding is immutable.
- Add compatibility fixtures for legacy artifact formats; do not migrate or
  reinterpret existing anchors.

## Acceptance criteria

- Any change to artifact, signer, subject, wallet, amount, asset, network,
  payee, expiry, or nonce invalidates the quote/proof.
- A settled receipt cannot be reused for another artifact.
- Legacy recall and verification fixtures pass unchanged.

