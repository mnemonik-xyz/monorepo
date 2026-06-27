# Threat Model

Per-boundary threat models for Mnemonic. Each entry: attack → the check that
defeats it, and whether it is an **integrity** failure (cryptographically
defeated) or an **availability** failure (only *detectable*, not preventable).

---

## Verifiable Trajectories boundary

Scope: the `trajectory-experimental` surface — `STEP_V1` / `VERDICT_V1` /
`TRAJECTORY_V1` artifacts, `trajectory::verify_chain` / `verdict_coverage`, the
order-preserving Merkle `batch_root`, and the Arweave-bundle storage path. The
verifier is pure and backend-agnostic, so every check below runs client-side
with no trust in the server.

| # | Attack | Defeating check | Class |
|---|---|---|---|
| 1 | **Step reordering** — replay steps in a different order to fake a cleaner reasoning path | `verify_chain` requires `prev_hash[i] == content_hash[i-1]`, and `prev_hash` is inside the signed CBOR payload; the order-preserving `trajectory_root` also changes under reorder | Integrity |
| 2 | **Content tampering** — edit a step after the fact | `verify_chain` recomputes `blake3(canonical_cbor)` and compares to `content_hash`; the COSE_Sign1 signature over the payload fails | Integrity |
| 3 | **Verdict forgery** — fabricate a passing verdict | `verdict_coverage` verifies each verdict's COSE signature and that `signer == judge`; an unsigned/altered verdict does not count | Integrity |
| 4 | **Judge substitution / self-judging** — the producing agent signs its own "pass" | Coverage ignores any verdict where `judge == step.producer`; only independent judges count | Integrity |
| 5 | **Step omission / dense-range break** — drop an inconvenient step | `verify_chain` requires a dense `seq` range `0..n`; a gap sets `chain_valid=false` with `broken_at` at the gap | Integrity |
| 6 | **Batch-root mismatch** — anchor a root that doesn't match the steps | The root is recomputed from the steps' content hashes; inclusion proofs verify against the recomputed root, not the claimed one | Integrity |
| 7 | **Cross-trajectory replay** — reuse a step/verdict from another trajectory | Step `trajectory_id` and `seq` are in the signed payload; a step signed for trajectory A does not satisfy B's chain | Integrity |
| 8 | **Broken checkpoint root-of-roots** — splice two unrelated checkpoints | `verify_checkpoint_chain` requires each `prev_root == previous batch_root`; a spliced checkpoint breaks the chain | Integrity |
| 9 | **Settle-gate bypass** — act on a trajectory with a `reject` or missing coverage | `safe_to_settle = chain_valid && full coverage && no reject`; a single independent valid `reject`, or any uncovered step, forces `false` | Integrity |
| 10 | **Bundler misbehavior** — Irys/Bundlr reorders or drops data items within a bundle | The anchored manifest root is the order-preserving root over `seq`-ordered hashes; the verifier re-derives it, so reordering/dropping is detected as #1/#5 | Integrity (detect) |
| 11 | **Recall withholding** — a gateway hides steps to make a trajectory look complete or to hide a `reject` | Hiding a step breaks the dense range (#5); hiding a `reject` cannot create a *false positive* settle, but a withholding gateway *can* deny service. Mitigation: query multiple Arweave gateways / pin independently | Availability |
| 12 | **Anchor censorship** — the anchor chain refuses the root | The root is small and chain-pluggable (Solana SPL Memo / OpenTimestamps→Bitcoin); a censoring chain is swappable. Until anchored, the trajectory is unverifiable-in-time (detectable, not forgeable) | Availability |

### Residual / out-of-scope

- **Correctness of the model computation** (layer C): Mnemonic binds an external
  proof (`VERDICT_V1.proof_ref` + `proof_kind`) by hash but does **not** verify
  zkML/opML/TEE math. A verdict attests that *a judge* passed the step, not that
  the underlying inference was faithful. Trusting the verdict = trusting the
  judge identity.
- **Judge collusion**: an independent judge that is honest-but-bribed can sign a
  `pass` on a bad step. Defeated only by judge diversity / reputation, which is
  an ERC-8004 Reputation-Registry concern, not this layer.
- **Key compromise**: a stolen producer or judge key forges valid signatures.
  Out of scope here; covered by the identity/keychain boundary.
