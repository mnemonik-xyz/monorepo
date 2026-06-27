---
created: 2026-06-27
updated: 2026-06-27
status: draft
type: feature
size: L
---

# Tech Spec: Verifiable Trajectories

## Goal

Let a verifier prove, offline and without model weights, that an agent's sequence
of steps was **good** = (a) ordered + tamper-evident (chain integrity) **and**
(b) each step carries an independently-signed correctness verdict (verdict
binding). Mnemonic owns the *commitment + lineage* layer (OCP's "confirm
inclusion"); it **binds** correctness proofs (PRM / zkML / opML / TEE / OCP) by
hash, it never produces them.

**Storage principle (owner decision, decisions.md 2026-06-27):** keep everything,
decentralized, no DB on the MCP side. The unit of storage write is a *bundle*,
not a step — Merkle batching applied one layer down — which is what makes
keep-everything affordable. The on-chain `batch_root` and the Arweave bundle
manifest are the **same root**: the order-preserving Merkle root over the
bundle's ordered step hashes. One inclusion proof simultaneously proves
"step is at this position in the chain" and "step is in this anchored bundle."

## Layer mapping (why this is the right scope)

"Prove the steps were good" decomposes into three guarantees at three layers:

| Guarantee | Producer | Mnemonic role |
|---|---|---|
| A. Steps happened in this order, unaltered | hash-linked signed chain | **Produces** |
| B. Each step was a valid/quality move | PRM / judge / deterministic check | **Attests the verdict** (co-sign) |
| C. Model ran faithfully on committed weights | zkML / opML / TEE | **Binds by hash only** |

Building C in `core/` would violate the native-only, one-way `core → mcp`
dependency rule (CLAUDE.md) and lose to dedicated zkML projects (100×–10,000×
overhead). We ship A + B.

## What exists today (substrate audit)

- `codec/canonical.rs`, `codec/hash.rs` — deterministic CBOR + blake3. ✓
- `codec/sign.rs` — COSE_Sign1 / Ed25519 over canonical CBOR; `verify_artifact`. ✓
- `lineage/mod.rs` — DAG + cycle detection; `chain_valid: Option<bool>` exists
  but is **always `None`**. Materialize it.
- `merkle.rs` — `commitment_root` / `prove` / `verify`, domain-separated
  (0x00 leaf / 0x01 node), odd-node promotion. Set-semantics (sorts+dedups),
  per-owner. Add an **order-preserving** sibling for trajectories.
- `arweave/mod.rs` — already constructs + signs **single** ANS-104 data items and
  uploads via Irys; `read(tx_id)`. Missing: multi-item **bundles** and a GraphQL
  **tag-query** read path. Extend, don't replace.
- `storage/traits.rs` — `AttestationStore` trait; only impl is `SqliteStore`.
  The trait is the backend seam. Add `ArweaveStore` as the canonical impl;
  demote `SqliteStore` to optional local cache.
- `mcp/src/tools.rs` — `correlation_id` + deferred-sign + `check_pending`: an
  existing "commit now, sign later" mechanism. Reuse for decoupled prove.

## Design

### Schemas (`core/src/codec/schema.rs`, feature `trajectory-experimental`)

Add three `ArtifactType`s + constants, following the existing `cbor_field_order`
discipline. **Do not mutate** any existing schema.

- `STEP_V1` (`type: "step"`): required `artifact_id, type, schema_version,
  content, producer, created_at`; trajectory fields `trajectory_id`, `seq` (u64),
  `prev_hash` (Option<hex>, None only for `seq == 0`); optional `verdict_hash`,
  `parents`, `metadata`, `tags`. `prev_hash` is inside the signed CBOR payload —
  that is what makes the link tamper-evident.
- `VERDICT_V1` (`type: "verdict"`): required `step_hash, status, judge,
  created_at` (+ envelope fields); `status` ∈ {`pass`,`concern`,`reject`};
  optional `score` (f32), `proof_ref` (hash of external zkML/TEE/OCP artifact),
  `proof_kind` ("prm"|"deterministic"|"zkml"|"tee"|"opml"|"ocp"), `rationale`.
  Signed by the **judge identity**, which MUST differ from the step `producer`.
- `TRAJECTORY_V1` (`type: "trajectory"`): required `trajectory_id, step_count,
  batch_root, producer, created_at` (+ envelope); optional `verdict_coverage`
  (f32), `chain_valid` (bool), `is_final` (bool), `prev_root` (checkpoint
  root-of-roots link). The anchored summary; `batch_root` = the bundle manifest
  root and the `proofHash` analog.

### Chain verifier (`core/src/lineage/`)

`verify_chain(store: &dyn AttestationStore, trajectory_id) -> ChainVerification` —
**backend-agnostic**: on the canonical path `store` is an `ArweaveStore`, so the
stateless MCP verifies straight from the permaweb.
1. Fetch steps for `trajectory_id` ordered by `seq`; assert dense `0..n`.
2. For each `i>0`: `step[i].prev_hash == step[i-1].content_hash`.
3. For each step: recompute canonical CBOR → blake3 == `content_hash`;
   `verify_artifact(cose_bytes, Some(content_hash))`.
4. Across checkpoints: each `TRAJECTORY_V1.prev_root` links to the prior anchored
   root (root-of-roots) — verify the chain of checkpoint roots is unbroken.
5. Materialize `chain_valid: Some(bool)`. Return `{chain_valid, broken_at,
   verified_steps, total_steps}`.

### Order-preserving Merkle root (`core/src/merkle.rs`)

`trajectory_root(ordered_step_hashes) -> [u8;32]` + `trajectory_prove(ordered,
index)`; reuse `verify`. Leaf index = `seq`, NO sort/dedup (contrast
`commitment_root`). Same domain-separation constants. **This root is the Arweave
bundle's manifest root** — the bundle's data items are laid out in `seq` order so
the two coincide by construction. Checkpoints compose via a root-of-roots.

### Verdict binding & coverage

`verdict_coverage` = fraction of steps with ≥1 `pass`/`concern` verdict whose
`judge != producer` and signature verifies. `mnemonic_verify_trajectory` reports
`safe_to_settle = chain_valid && verdict_coverage == 1.0 && no reject verdicts` —
the ERC-8301 "every preceding stage has a valid proof before a high-value action"
invariant, computed at verify time (interactivity preserved).

### Storage — Arweave bundles, stateless MCP

- **Canonical store = `ArweaveStore`** (impl of `AttestationStore`): packs a
  checkpoint's steps (full content + COSE) as ANS-104 data items into one bundle,
  laid out in `seq` order, tagged `trajectory_id` / `seq` / `owner_pubkey` /
  `content_hash`. Extends the existing single-item ANS-104 code in `arweave/`.
- **Stateless MCP:** no server DB on the canonical path. Fetch from Arweave
  (GraphQL tag query), re-derive root, `verify_chain`, check coverage.
- **BYO wallet:** the Arweave signer is caller-supplied; user owns the bytes.
- **Checkpoint roots:** flush a bundle + anchor an interim `TRAJECTORY_V1` every
  N steps / on a timer; each carries `prev_root`; the final root supersedes.
  Anchor = Solana SPL Memo today, OpenTimestamps→Bitcoin chain-neutral option.
- **Recall, no server DB:** exact retrieval via Arweave GraphQL tags; semantic
  recall = fetch embeddings + cosine **client-side** in V1.

`SqliteStore` remains only as an opt-in offline/local cache, never canonical.

### MCP tools (`mcp/src/tools.rs`)

- `mnemonic_attest_step {trajectory_id, content, seq?, mode?}` — auto-fills
  `prev_hash` from `store.trajectory_head`; signs `STEP_V1`; buffers into the
  current bundle. On the Nth step (or timer) flushes the bundle + anchors an
  interim checkpoint root. `local` mode = client cache only; `participate` =
  bundle pushed to Arweave + anchored.
- `mnemonic_attest_verdict {step_hash, status, score?, proof_ref?, proof_kind?,
  rationale?}` — signs `VERDICT_V1` with the caller (judge) keypair; rejects
  `judge == producer`; deferred attach via `correlation_id`.
- `mnemonic_verify_trajectory {trajectory_id}` — `verify_chain` +
  `trajectory_root` over all checkpoints; returns `{chain_valid, broken_at,
  verdict_coverage, batch_root, proofs[], safe_to_settle}`. The trajectory
  summary is written as an Arweave artifact + anchor, never a server table.

## Testing

- Golden vectors: a 3-step trajectory + verdicts → `{step_json,
  canonical_cbor_hex, cose_hex, bundle_manifest_root_hex, inclusion_proof}`,
  published for byte-parity (`references/conformance.md`).
- Property tests: reorder ⇒ `chain_valid=false`; tamper ⇒ hash mismatch; verdict
  by producer ⇒ rejected; missing `seq` ⇒ dense-range fail; broken `prev_root`
  ⇒ checkpoint-chain fail; inclusion proof verifies iff step in bundle.
- `cargo build --workspace` (no feature) stays green — proves gating.

## Tasks / waves

- **Wave 1** — Task 1: schemas in `schema.rs`.
- **Wave 2** (disjoint files) — Task 2: chain verifier + root-of-roots
  (`lineage/`); Task 3: order-preserving root = bundle manifest (`merkle.rs`);
  Task 4: `ArweaveStore` bundles + GraphQL, stateless verify (`arweave/`,
  `storage/`).
- **Wave 3** — Task 5: MCP tools + checkpoint flush + decoupled prove.
- **Wave 4** — Task 6: conformance vectors + threat model (incl. bundler trust).
- **Wave 5** — Task 7: audit (code/security/test) + pre-deploy QA gate.

## Out of scope (V1)

zkML/opML/TEE proof generation; PRM scoring itself; on-chain ERC-8274 verifier
contract (inherits chain-pluggable anchor / ERC-8004 backlog); decentralized
semantic vector index (V1 does client-side cosine).
