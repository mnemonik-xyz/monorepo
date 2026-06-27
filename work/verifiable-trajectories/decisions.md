# Decisions — verifiable-trajectories

Append-only log. Each entry: date, who, what, why, what changes downstream.

---

## 2026-06-27 — Feature folder created

Author: claude (from analysis of "Toward a Verifiable Architecture for Agentic
Cognition" report + substrate audit of `core/src/{lineage,codec,merkle,storage}`).

Origin: request to let Mnemonic "prove the sequences of steps executed by agents
were good." Audit found the substrate is ~70% present (blake3 + COSE_Sign1 +
lineage DAG + `merkle.rs`), with four concrete gaps: no ordered hash-linked
chain (`seq`/`prev_hash`), `chain_valid` never materialized, no verdict binding,
Merkle root is per-owner not per-trajectory.

---

## 2026-06-27 — Decision: scope to layers A+B, bind layer C by hash

"Good" = A (chain integrity) + B (per-step verdict) + C (computational
correctness). We **produce** A, **attest** B (co-sign a judge's verdict),
**bind** C by hash only. No zkML/opML/TEE prover in `core/`.

Why: C carries 100×–10,000× overhead and belongs to dedicated projects; building
it in `core/` violates the native-only + one-way `core → mcp` dependency rule
(CLAUDE.md). Mnemonic's defensible position is OCP's "confirm inclusion" leg,
which `merkle.rs` already implements. Composition, not competition (mirrors the
Hindsight + ERC-8004 Validation-Registry stance in `protocol-integrations.md`).

Downstream: `VERDICT_V1.proof_ref` + `proof_kind` carry the external proof hash;
Mnemonic signs that the verdict exists and is linked, not that the model math is
sound.

---

## 2026-06-27 — Decision: trajectory Merkle root is order-preserving

The new `trajectory_root` does NOT reuse `commitment_root`'s sort+dedup set
semantics. Leaf index = `seq`, order preserved, so reordering changes the root.

Why: the whole point is to prove ordering. Set-semantics would make a reordered
trajectory produce the same root. Same domain-separation constants (0x00 leaf /
0x01 node, odd-node promotion) are reused so a single verifier handles both. The
two functions coexist with different invariants — documented here so a future
reader does not "unify" them and silently break ordering proofs.

---

## 2026-06-27 — Decision: judge identity must differ from step producer

`mnemonic_attest_verdict` rejects a `VERDICT_V1` whose `judge` pubkey equals the
attested step's `producer`. Verdict coverage only counts verdicts from a distinct
verifying identity.

Why: a verdict an agent signs over its own step is worthless as a correctness
signal — exactly the "signed hallucination" failure the report names. Mirrors the
operator-keypair-never-signs-others'-content routing rule already in
`mcp/src/tools.rs`.

---

## 2026-06-27 — Decision: decoupled prove reuses correlation_id, not a new mechanism

The ERC-8301 `onAgentStep` / `onAgentProve` split (commit step now, attach proof
later) is implemented on top of the existing `correlation_id` + deferred-sign +
`check_pending` plumbing, not a new state machine.

Why: that plumbing already models "commit now, finalize later." `safe_to_settle`
(chain_valid && coverage==1.0 && no rejects) is computed at verify time rather
than enforced as a write-time lock, preserving live interactivity.

---

## 2026-06-27 — Decision: experimental gating

All three schemas + tools land behind cargo feature `trajectory-experimental`
until V1 GA is declared here. Default `cargo build --workspace` stays green
without the feature. Mirrors the `a2a-experimental` gating precedent.

---

## 2026-06-27 — Decision: storage = Arweave bundles, stateless MCP (owner answers)

Resolves the "where do we store it" open question. Owner answers: keep
everything; decentralized, no DB on the MCP side; checkpoint roots.

**Principle.** The unit of storage write is a *bundle*, not a step — the same
Merkle-batch insight applied one layer down. This is what makes "keep
everything" affordable.

1. **Steps → Arweave ANS-104 bundles.** A checkpoint's worth of steps (full
   content + COSE) is packed into one bundled Arweave transaction. Each step
   stays individually addressable (data-item id derivable from its blake3 hash),
   but cost is one write per checkpoint, not per step (~1000× reduction;
   ~$0.0004 for a 25-step task). Arweave is pay-once permanent + decentralized
   and already a dependency. "Keep everything" is the retention policy; bundling
   is what makes it viable.
2. **Stateless MCP.** No SQLite on the server. The MCP process is a
   bundler-relay + verifier. Source of truth = permaweb (content) + anchor chain
   (root timestamps) + user keychain (identity, `core/src/identity`). Given a
   `trajectory_id` the server fetches from Arweave, re-derives the root, runs
   `verify_chain`, checks coverage — holding nothing. SqliteStore stays only as
   an optional local-mode cache / offline dev backend, never the canonical store.
3. **BYO storage / sovereignty.** The user supplies their own Arweave wallet
   (or IPFS pin / Filecoin deal). Protocol defines the bundle format; user owns
   the bytes. Keeps the `AttestationStore` trait as the seam — `ArweaveStore`
   becomes a first-class impl alongside `SqliteStore`.
4. **Checkpoint roots → anchor chain.** Per owner answer: anchor an interim
   batch root every N steps / on a timer; each checkpoint references the prior
   (root-of-roots chain); final root supersedes. Anchor = Solana SPL Memo today,
   OpenTimestamps→Bitcoin as the chain-neutral option (inherits chain-pluggable
   anchor work).
5. **Recall without a server DB.** Exact retrieval via Arweave GraphQL tag index
   (`trajectory_id`, `seq`, `owner_pubkey`). Semantic recall = fetch embeddings,
   cosine **client-side** in V1. Decentralized vector index deferred.

**On "publish to calendars" (owner brainstorm).** The instinct is the
timestamp/notarization one, already satisfied better by Solana SPL Memo /
OpenTimestamps. Calendars are mutable + centralized → strictly worse as the
record. Allowed only as a human-facing pointer surface (event body links the
anchored root), never the source of truth. Not in V1.

## 2026-06-27 — Implementation landed (Waves 1–5 core)

Author: claude. Built behind `trajectory-experimental` (core + mcp); default
`cargo build --workspace` unaffected. 29 new tests green; fmt + clippy clean in
both feature configs.

**Deviation from spec (justified): a dedicated `core/src/trajectory/` module
instead of putting `verify_chain` in `lineage/`.** `lineage/` is native-only and
SQLite/`LineageStore`-coupled; the decentralized direction needs the verifier to
be pure (codec + merkle only) so the *same* chain/coverage checks run
client-side (wasm) and against any backend. `chain_valid` is materialized here
via `ChainVerification` rather than on `LineageResult`. The `lineage` DAG is
untouched.

**What shipped + tested:**
- Task 1 — `STEP_V1` / `VERDICT_V1` / `TRAJECTORY_V1` (schema.rs). ✅
- Task 2 — `trajectory::verify_chain` (dense seq, prev_hash linkage, content
  hash, signature+producer), `verdict_coverage` (independent-judge), checkpoint
  `verify_checkpoint_chain`, `build_report`/`safe_to_settle`. ✅
- Task 3 — order-preserving `merkle::trajectory_root` / `trajectory_prove`. ✅
- Task 4 (local path) — `SqliteTrajectoryStore` (sanctioned local cache) +
  `InMemoryTrajectoryStore`. ✅
- Task 5 (logic) — `mcp::trajectory_tools::{attest_step, attest_verdict,
  verify_trajectory}` pure handlers, unit-tested (auto prev_hash/seq, self-judge
  rejection, settle gate, proofs JSON). ✅
- Task 6 — golden vectors (frozen genesis hash + batch root, ignored SDK
  emitter), `references/threat-model.md`, crypto-design note. ✅
- Task 7 — QA gate run (fmt/clippy/tests, both configs). ✅

**Remaining (needs live infra, not validatable in sandbox):**
- Canonical `ArweaveStore`: extend `arweave/mod.rs` single ANS-104 data item →
  multi-item bundle + GraphQL tag read. Needs Irys + network.
- Wire the three tools into `mcp/src/mcp.rs` JSON-RPC dispatch + `tools/list`
  manifest, and construct/hold a `SqliteTrajectoryStore` (or `ArweaveStore`) in
  server state. Needs a running server to E2E.
The handler functions are dispatch-ready; only registration + the Arweave write
path remain.

**Downstream.** Task 4 pivots from "SQLite columns" to "Arweave-bundle step
store + stateless verify"; SqliteStore demoted to optional cache.

**Substrate note (corrects the first cut).** `core/src/arweave/mod.rs` ALREADY
constructs + signs single ANS-104 data items via Irys (deep-hash path included).
So the work is an *extension* — single item → multi-item bundle + a GraphQL
tag-query read path — not a net-new dependency. Likewise the on-chain
`batch_root` and the bundle manifest root are unified: the bundle's data items
are laid out in `seq` order, so the order-preserving `trajectory_root` (Task 3)
IS the manifest root, and one inclusion proof serves both chain-position and
bundle-membership. Checkpoints compose via a root-of-roots (`prev_root` on each
`TRAJECTORY_V1`). Cost model and bundler-trust assumptions to be detailed in the
threat model (Task 6).
