---
created: 2026-06-27
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
- `lineage/mod.rs` — `artifact_lineage(child_id, parent_id, role, created_at)`,
  cycle detection, `Direction`. `chain_valid: Option<bool>` exists but is
  **always `None`** ("full verification done separately"). ✗ to materialize.
- `merkle.rs` — `commitment_root` / `prove` / `verify`, domain-separated
  (0x00 leaf / 0x01 node), odd-node promotion. Roots **all** of an owner's
  hashes; not per-trajectory. Extend.
- `storage/sqlite.rs` — `attestations` table; `write_mode` (local|participate).
  No `seq` / `prev_hash` / `trajectory_id` / `verdict_hash`. Add.
- `mcp/src/tools.rs` — `correlation_id` + deferred-sign + `check_pending`: an
  existing "commit now, sign later" mechanism. Reuse for decoupled prove.

## Design

### Schemas (`core/src/codec/schema.rs`, feature `trajectory-experimental`)

Add three `ArtifactType`s + schema constants, following the existing
`cbor_field_order` discipline. **Do not mutate** any existing schema.

- `STEP_V1` (`type: "step"`): required `artifact_id, type, schema_version,
  content, producer, created_at`; trajectory fields `trajectory_id`, `seq`
  (u64), `prev_hash` (Option<hex>, None only for `seq == 0`); optional
  `verdict_hash`, `parents`, `metadata`, `tags`. `prev_hash` participates in the
  signed CBOR payload — that is what makes the link tamper-evident.
- `VERDICT_V1` (`type: "verdict"`): required `artifact_id, type, schema_version,
  step_hash, status, judge, created_at`; `status` ∈ {`pass`,`concern`,`reject`};
  optional `score` (f32), `proof_ref` (hash of external zkML/TEE/OCP artifact),
  `proof_kind` ("prm"|"deterministic"|"zkml"|"tee"|"opml"|"ocp"), `rationale`.
  Signed by the **judge identity**, which MUST differ from the step `producer`
  (enforced at attest time).
- `TRAJECTORY_V1` (`type: "trajectory"`): required `artifact_id, type,
  schema_version, trajectory_id, step_count, batch_root, producer, created_at`;
  optional `verdict_coverage` (f32), `chain_valid` (bool), `is_final` (bool).
  This is the anchored summary; `batch_root` is the `proofHash` analog.

### Chain verifier (`core/src/lineage/`)

New `verify_chain(store, trajectory_id) -> ChainVerification`:
1. Load steps for `trajectory_id` ordered by `seq`; assert dense `0..n`.
2. For each `i>0`: assert `step[i].prev_hash == step[i-1].content_hash`.
3. For each step: recompute canonical CBOR → blake3, assert `== content_hash`;
   `verify_artifact(cose_bytes)` for signature.
4. Materialize `chain_valid: Some(bool)` in `LineageResult` for trajectory walks.
Return `{chain_valid, broken_at: Option<seq>, verified_steps, total_steps}`.

### Per-trajectory Merkle root (`core/src/merkle.rs`)

Add `trajectory_root(ordered_step_hashes: &[String]) -> [u8;32]` and
`trajectory_prove` / reuse `verify`. **Order-preserving** (unlike the existing
set-semantics `commitment_root` which sorts+dedups) — leaf index = `seq`. Keep
the same domain-separation constants. Document the divergence in `decisions.md`.

### Verdict binding & coverage

`verdict_coverage` = fraction of steps with ≥1 `pass`/`concern` verdict whose
`judge != producer` and signature verifies. Invariant for high-value gating:
`is_final` may be set early, but `mnemonic_verify_trajectory` reports
`safe_to_settle = chain_valid && verdict_coverage == 1.0 && no reject verdicts`.
This is the ERC-8301 "every preceding stage has a valid proof before a high-value
action" invariant, computed at verify time.

### Storage (`core/src/storage/sqlite.rs`)

Migration (idempotent `ALTER TABLE ... ADD COLUMN`, matching existing migration
style): add `trajectory_id TEXT`, `seq INTEGER`, `prev_hash TEXT`,
`verdict_hash TEXT` to `attestations`; new index on `(trajectory_id, seq)`. New
`trajectory_roots(trajectory_id PK, batch_root, step_count, chain_valid,
verdict_coverage, anchored_tx, created_at)`. No backfill — legacy rows have NULL
trajectory fields and are unaffected.

### MCP tools (`mcp/src/tools.rs`)

- `mnemonic_attest_step` — like `sign_memory` but takes `trajectory_id`, `seq`,
  auto-fills `prev_hash` from the store's current head for that trajectory.
- `mnemonic_attest_verdict` — sign a `VERDICT_V1` with the caller's (judge)
  keypair; reject if `judge == step.producer`; supports deferred attach via
  `correlation_id`.
- `mnemonic_verify_trajectory` — runs `verify_chain` + builds `trajectory_root`,
  returns `{chain_valid, broken_at, verdict_coverage, batch_root, proofs[],
  safe_to_settle}`. Only `participate` mode anchors the root.

## Testing

- Golden vectors: a 3-step trajectory with verdicts → `{step_json,
  canonical_cbor_hex, cose_hex, batch_root_hex, inclusion_proof}`, published for
  byte-parity (ties into the missing `references/conformance.md`).
- Property tests: reordering any step ⇒ `chain_valid=false`; tampering content ⇒
  hash mismatch; verdict signed by producer ⇒ rejected; omission of a `seq` ⇒
  dense-range assertion fails; inclusion proof verifies iff step in trajectory.
- `cargo build --workspace` (no feature) stays green — proves gating.

## Tasks / waves

- **Wave 1** — Task 1: schemas (STEP/VERDICT/TRAJECTORY) in `schema.rs`.
- **Wave 2** (parallel, disjoint files) — Task 2: chain verifier (`lineage/`);
  Task 3: trajectory Merkle root (`merkle.rs`); Task 4: storage + migrations.
- **Wave 3** — Task 5: MCP tools (+ decoupled prove via `correlation_id`).
- **Wave 4** — Task 6: conformance golden vectors + `references/threat-model`
  (trajectory boundary).
- **Wave 5** — Task 7: audit (code/security/test) + pre-deploy QA gate.

## Out of scope (V1)

zkML/opML/TEE proof generation; PRM scoring itself; on-chain ERC-8274 verifier
contract (inherits chain-pluggable anchor / ERC-8004 backlog); semantic recall
over steps.
