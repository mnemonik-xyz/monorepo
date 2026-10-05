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

---

## 2026-10-05 — Integration boundary: no swap code in `mnemonic-core`

Author: claude, at the owner's request ("rework it that way").

Decision: Mnemonik provides transport, identity, sealing, signing and anchoring.
Each integration defines its own message types, state machines and policies in
its own package. A new integration needs no change to `mnemonic-core`. A change
enters core only when it is generic.

Consequences:

- `core/src/codec/agswap/` is dropped. The swap message kinds, the swap intent and
  the transcript state machine move to the Warrant swap specification
  (`swap-core`).
- `AGSWAP_INNER_V1` is replaced by the generic `SIGNED_INNER_V1` in
  `core/src/codec/a2a/inner.rs`, behind `a2a-experimental`. Its `protocol` field
  is chosen by the caller; its `body` is opaque to Mnemonik. The identity encoding
  decision above applies unchanged.
- Mnemonik scope is now three generic items: the signed sealed message (M1), a
  WASM and SDK COSE verify (M2) and a key-custody `Signer` (M3). Tasks 1 to 6
  were rewritten accordingly.
- The `Signer` keeps Ed25519, `alg = EdDSA` and the base58 `kid`. It does not
  start `work/multi-suite-signing/`, which stays last in
  `work/DECOUPLING-SEQUENCE.md`. Golden fixtures must stay byte-identical.

Supersedes the scope of the 2026-10-05 entries above where they place swap code
in core.
