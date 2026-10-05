# Decisions — agentic-swap

Append-only log. Each entry: date, who, what, why, what changes downstream.

---

## 2026-10-05 — Feature folder created

Author: claude, from an analysis of the draft "Agentic Swap Spec v0.1" against
this repository, `mnemonik-xyz/warrant` and `mnemonik-xyz/policy-execution`.

Findings and proposals (not yet owner-approved):

1. Sign, seal, sign is a composition of existing parts. `SEALED_V1` does not
   change.
2. The domain separator goes inside the signed payload (`protocol` field),
   because `sign_cose` has no protected domain header.
3. New swap objects use JCS, not `to_canonical_cbor`.
4. Phase 1 anchors through `mnemonic_attest_a2a`. No new MCP tool.
5. Authorization runs in a separate policy signer owned by Warrant. Only that
   binary depends on both Warrant and `mnemonic-core`. This repository takes no
   `vstd` dependency.
6. Warrant's zkVM, Groth16 and escrow contracts are out of scope for swaps.

Open for the owner: policy disclosure to the counterparty, key custody (HSM,
KMS or TEE) and the policy language for version 1.

---

## 2026-10-05 — Identity encoding in `AGSWAP_INNER_V1`

Author: claude, after a review finding on PR #275.

`sender` and `recipient` use the bare base58 Ed25519 public key, not `did:key`.
Reason: `sign_cose` writes the bare key into the COSE `kid`, and sealed A2A grants
store the bare key as `reader`. Core has no `did:key` parser. Checks compare raw
32-byte keys after decoding. `did:key` stays a display and policy form.

Downstream: tasks 2 and 4 implement the comparison; the golden vectors fix it.
