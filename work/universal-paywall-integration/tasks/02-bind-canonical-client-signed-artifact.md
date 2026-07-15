---
status: in_progress
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

## Implementation design (2026-07-15)

The existing route is ordered `quote -> payment -> unsigned canonical CBOR ->
COSE callback`. It cannot meet this task by substituting a different pre-sign
hash: the signed envelope does not exist yet.

The replacement route is `unsigned canonical CBOR -> local COSE_Sign1 ->
versioned signed-artifact binding -> quote/payment -> anchor`. The binding is
`blake3("mnemonic:paid-artifact:v1\\0" || exact_cose_sign1_bytes)`. It commits
to the artifact payload, signature, protected headers, and signer key id.

`mcp/src/paid_artifact.rs` now owns this derivation and its regression tests.
The next implementation slice must persist the verified COSE envelope and its
pending delivery context between signature verification and payment completion;
only then may it create the quote. Existing stored artifacts retain their
current recall/verification format and are not reinterpreted.
