---
created: 2026-06-30
updated: 2026-06-30
status: draft
type: feature
size: XL
supersedes-extends: work/verifiable-trajectories/
---

# Tech Spec: Agent Provenance — proving an agent's actions correspond to its intent

## One-line goal

Let any party prove, **offline and without model weights**, that an agent's
executed trajectory *corresponds to the task/intent it was given* — not merely
that the steps were ordered and individually plausible, but that they were
**rooted in a signed mandate, stayed inside its authorized scope, and satisfied
its stated constraints**.

This is the missing leg. `work/verifiable-trajectories/` (shipped, behind
`trajectory-experimental`) already proves two of the three guarantees an
auditable agent needs. This feature adds the third — **intent binding** — and
re-frames `safe_to_settle` from "the steps were good" to "the steps did *the
thing that was asked*."

## The frame: Delta vs. Mnemonic, and the gap we close

| Question | Layer | Owner today |
|---|---|---|
| What did the agent **know**? (memory authentic, untampered) | memory | Mnemonic `MEMORY_V1` ✓ |
| Did the steps happen **in order, unaltered**? | chain integrity | Mnemonic `STEP_V1` ✓ |
| Was each step a **valid move**? | per-step verdict | Mnemonic `VERDICT_V1` ✓ |
| Did the agent do **the right thing vs. the task it was given**? | **intent correspondence** | **GAP — this feature** |

Delta Network answers the last row with a **centralized, closed-source SP1
zkVM + zkTLS** pipeline: user signs a typed Intent, agent submits a Proposal,
a hosted prover proves the Proposal satisfies the Intent's policy *before funds
move*. Mnemonic's answer is deliberately different on three axes the owner named:

- **Decentralized** — correspondence is a **re-runnable deterministic policy
  evaluation** over hash-committed evidence, not a hosted prover you must trust.
  Anyone can re-execute the check from the permaweb record and get the same
  verdict. zk is *bound by hash* only where evidence must stay private — exactly
  the composition stance already locked in (`decisions.md`, verifiable-traj.).
- **Efficient** — no zkVM in `core/`. One small signed mandate, deterministic
  predicate eval (microseconds), one order-preserving Merkle root, one anchor
  write per checkpoint. zk proving cost is always someone else's, bound by ref.
- **Provenance, not just a gate** — Delta gates *before* the action; Mnemonic
  produces the immutable, anchored, independently-verifiable *record* that the
  action matched intent, and ALSO exposes a gate (`safe_to_settle`) for callers
  who want pre-action enforcement. Both, not either.

The two stacks compose: a Mnemonic `COMPLIANCE_V1` can *bind a Delta SP1 proof
by hash* as its `proof_ref`. Mnemonic orders, anchors, and makes recallable what
Delta (or a TEE, or a deterministic checker) proves.

## What exists today (substrate audit — verified against code)

- `codec/{canonical,hash,sign}.rs` — deterministic CBOR → blake3 → COSE_Sign1
  (Ed25519). `verify_artifact(cose, Some(hash)) -> {valid, signer}`. ✓ reuse verbatim.
- `codec/schema.rs` — immutable versioned schemas + `cbor_field_order`; trajectory
  schemas gated behind `trajectory-experimental`, **pre-GA** (not frozen in
  decisions.md) — so we may extend them in place this iteration.
- `trajectory/` — `verify_chain` (dense `seq`, `prev_hash` linkage, content hash,
  signature==producer), `verdict_coverage` (independent judge), `build_report`
  → `TrajectoryReport { chain_valid, batch_root, safe_to_settle, ... }`,
  `verify_checkpoint_chain` (root-of-roots). **Pure**: codec + merkle only, runs
  in wasm and against any `TrajectoryStore`. ✓ extend, don't replace.
- `merkle.rs` — `trajectory_root` (order-preserving) + `trajectory_prove`;
  `commitment_root` (set semantics) for recall. Domain-separated. ✓ reuse.
- `trajectory/store.rs` — `TrajectoryStore` trait (`steps_for_trajectory`,
  `verdicts_for_step`, `trajectory_head`); `ArweaveStore` is the canonical impl,
  `SqliteTrajectoryStore`/`InMemory` are caches. ✓ extend with mandate/compliance reads.
- `trajectory/reconstruct.rs` — `step_from_cose` / `verdict_from_cose`:
  **non-custodial** — the server verifies + parses client-signed envelopes, signs
  nothing. ✓ same pattern for mandate/compliance.
- `lineage/` — DAG + `validate_parents` (MAX_PARENTS=16) + cycle detection;
  `ParentRef { artifact_id, role }`. ✓ mandate becomes a typed parent of step 0.
- `solana/` anchor JSON `{h, a, m, v}` (`v=3`). ✓ add `r` (root) / `i`
  (mandate id) for an anchored trajectory checkpoint.

The substrate is ~85% present. This feature is **three new schemas, one pure
policy evaluator, one pure correspondence verifier, and the tool wiring** — all
behind the existing experimental gate.

## Design

### New artifact 1 — `MANDATE_V1` (the intent / task; the genesis root)

The signed task statement, authored by the **principal** (the delegating
authority: a human, an orchestrator, or a parent agent) and authorizing a
**subject** (the executing agent's pubkey). The principal is the COSE signer.

- required: `artifact_id`, `type="mandate"`, `schema_version`, `content` (the
  natural-language intent — lives in the standard `content` slot so
  `content_hash` commits to it, exactly like `POST_V1`), `principal` (signer
  pubkey), `subject` (authorized executing-agent pubkey), `created_at`.
- optional: `constraints` (array of typed predicates, see below), `capabilities`
  (`{ tools: [..], spend_limit: { currency, amount }, expires_at }`), `nonce`,
  `expires_at`, `parents`, `metadata`, `tags`.
- The mandate's `content_hash` is the **anchor of correspondence**: every step's
  chain ultimately roots in it, and every `COMPLIANCE_V1` names it.

A `constraint` is a small, total, machine-checkable predicate:
```
{ id: "price", kind: "number", path: "$.order.price", op: "lte", value: 150 }
{ id: "size",  kind: "string", path: "$.order.size",  op: "eq",  value: "10" }
{ id: "tone",  kind: "subjective", description: "reply stays professional" }
```
`kind: subjective` constraints are NOT machine-decidable; they require a signed
`COMPLIANCE_V1` from an independent checker (LLM-judge / human). All other kinds
are re-evaluated deterministically by the policy evaluator below.

### New artifact 2 — `EVIDENCE_V1` (the zkTLS analog; optional, per step)

Binds the external data a step relied on, so constraint evaluation has a
hash-committed input. This is Mnemonic's decentralized analog to Delta's zkTLS
Evidence Layer — Mnemonic does not *produce* the TLS proof, it commits + orders it.

- required: `artifact_id`, `type="evidence"`, `schema_version`, `step_hash`
  (the step that consumed it), `source` (endpoint/source id), `response_hash`,
  `producer`, `created_at`.
- optional: `request_hash`, `payload` (the structured facts, when public),
  `attestation_kind` (`zktls`|`notary`|`tee`|`none`), `attestation_ref` (hash of
  the external authenticity proof), `tags`.

### New artifact 3 — `COMPLIANCE_V1` (the correspondence verdict)

The evolution of `VERDICT_V1` from "was this step a good move" to "does this
trajectory satisfy *this mandate*." Signed by an independent **checker**
(`checker != subject` always; SHOULD also `!= principal`).

- required: `artifact_id`, `type="compliance"`, `schema_version`, `mandate_hash`,
  `trajectory_root` (the order-preserving root it judges), `compliant` (bool),
  `checker` (signer), `created_at`.
- optional: `constraint_results` (array `{ constraint_id, satisfied: bool,
  evidence_hash, method }`), `proof_ref` (hash of an external SP1/zkML/TEE proof
  — *e.g. a Delta proof*), `proof_kind`, `rationale`, `tags`.

### Rooting steps in the mandate (no breaking schema change)

The genesis step (`seq == 0`) sets `prev_hash = mandate.content_hash` instead of
`null`, and adds `parents: [{ artifact_id: <mandate_id>, role: "mandate" }]`.
This makes the *entire* hash-linked chain tamper-evidently rooted in the intent:
swap or edit the mandate and the genesis link breaks. `STEP_V1` needs **no new
field** — `prev_hash` and `parents` already exist. `TRAJECTORY_V1` gains one
optional field `mandate_hash` (allowed: experimental, pre-GA) so the anchored
summary names the intent it fulfills.

### Pure policy evaluator (`core/src/trajectory/policy.rs`, new)

A tiny, **total, dependency-free** evaluator — the decentralized, re-runnable
heart of correspondence. `evaluate(constraint, evidence_json) -> ConstraintOutcome`:
- supports `kind` ∈ {`number`, `string`, `bool`, `set`, `exists`} with ops
  (`eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `in`, `nin`, `matches` for a
  bounded/anchored regex, `present`); JSONPath-lite `path` resolution over the
  committed evidence object.
- `kind: subjective` → `ConstraintOutcome::NeedsChecker`.
- Total and deterministic: malformed path/type → `Unsatisfied { reason }`, never
  panics, no I/O, no clock, no allocation surprises. Same input → same verdict on
  every machine. This is what makes the check re-runnable by any verifier and
  removes the need to trust a prover.

### Pure correspondence verifier (`core/src/trajectory/correspondence.rs`, new)

`verify_correspondence(mandate, steps, evidence, compliance) ->
CorrespondenceReport` — pure (codec + merkle + policy), wasm-safe:

1. **Mandate authenticity** — `verify_artifact(mandate.cose)` valid &&
   `signer == mandate.principal`; not past `expires_at`.
2. **Rooted in mandate** — `steps[0].prev_hash == mandate.content_hash`.
3. **Authorized subject** — every `step.producer == mandate.subject` (delegation
   chains: `subject` may be a set; V1 = single pubkey).
4. **Capability conformance** — each step's declared action (tool + spend, read
   from step metadata) ∈ `mandate.capabilities`; cumulative spend ≤ `spend_limit`.
5. **Constraint satisfaction** —
   - machine constraints: **re-evaluate** via `policy::evaluate` against the
     bound `EVIDENCE_V1` payload/hash. Re-runnable; needs no checker.
   - subjective constraints: require ≥1 `COMPLIANCE_V1` with
     `constraint_results[id].satisfied == true`, valid signature,
     `checker != subject`.
6. **Compliance binding** — every `COMPLIANCE_V1.mandate_hash ==
   mandate.content_hash` && `trajectory_root` matches the recomputed
   `merkle::trajectory_root`.

Returns `CorrespondenceReport { rooted_in_mandate, authorized,
capabilities_respected, constraints: [{id, satisfied, method}], unmet:[..],
checker_independent, compliant }`.

### `safe_to_settle`, re-framed

`build_report` gains an intent-aware path. When a mandate is present:
```
safe_to_settle = chain_valid                       // ordering, untampered (existing)
              && verdict_coverage.full && !has_reject   // per-step quality (existing)
              && rooted_in_mandate                  // genesis links to the intent
              && authorized && capabilities_respected
              && all machine-constraints satisfied
              && no unmet subjective constraints
              && compliant                          // the COMPLIANCE_V1 binding
```
Mandate-less trajectories keep today's behavior exactly (backward compatible).
This boolean is the decentralized analog of Delta's "Verifier approved the
proposal" — but computed by any party, from the permaweb, with no hosted service.

### Storage & decentralization (inherits verifiable-trajectories decisions)

No change to the storage philosophy: stateless MCP, Arweave ANS-104 bundles as
canonical, anchor chain for root timestamps, user keychain for identity, BYO
wallet. Mandate + evidence + compliance are additional tagged data items in the
trajectory's bundle (`mandate_hash`, `trajectory_id`, `type`). The anchored
`TRAJECTORY_V1` checkpoint's Solana memo extends `{h,a,m,v}` with `r` (batch
root) and `i` (mandate hash) so the on-chain anchor names both the intent and the
fulfilling root. `TrajectoryStore` gains `mandate_for(trajectory_id)` and
`compliance_for(trajectory_root)` reads.

### MCP tools (`mcp/src/`, behind `trajectory-experimental`)

- `mnemonic_sign_mandate { content, subject, constraints?, capabilities?,
  expires_at? }` → non-custodial: principal signs `MANDATE_V1` client-side,
  server verifies + stores, returns `mandate_hash`. The trajectory genesis.
- `mnemonic_attest_step` — extended: accepts `mandate_hash`; genesis auto-roots
  `prev_hash := mandate_hash` and adds the `role:"mandate"` parent.
- `mnemonic_attest_evidence { signed }` — store a client-signed `EVIDENCE_V1`.
- `mnemonic_attest_compliance { signed }` — store a client-signed `COMPLIANCE_V1`;
  rejects `checker == subject`; deferred attach via existing `correlation_id`.
- `mnemonic_verify_trajectory` — extended output: `correspondence` report +
  intent-aware `safe_to_settle`.

## Testing

- Golden vectors: a 3-step **intent-bound** trajectory (mandate + 3 steps + 1
  evidence + 1 compliance) → frozen `{mandate_hash, genesis_prev_hash,
  batch_root, correspondence_report}`, published for byte-parity.
- Policy evaluator: table-driven over every `kind × op`, incl. malformed
  path/type totality (no panic), regex anchoring/bound.
- Property tests: edit mandate ⇒ `rooted_in_mandate=false`; step by a non-subject
  ⇒ `authorized=false`; spend over limit ⇒ `capabilities_respected=false`;
  unsatisfied numeric constraint ⇒ `compliant=false` from re-evaluation alone
  (no checker needed); subjective constraint w/o compliance ⇒ `unmet`;
  `checker==subject` ⇒ rejected.
- `cargo build --workspace` (no feature) stays green — gating proof.

## Tasks / waves

- **Wave 1** — Task 1: `MANDATE_V1` / `EVIDENCE_V1` / `COMPLIANCE_V1` in
  `schema.rs`; `mandate_hash` on `TRAJECTORY_V1`.
- **Wave 2** (disjoint files) — Task 2: `policy.rs` evaluator; Task 3:
  `correspondence.rs` verifier + `build_report` intent path; Task 4: store reads
  (`mandate_for`, `compliance_for`) + Arweave tags + anchor `{r,i}`.
- **Wave 3** — Task 5: MCP tools + non-custodial reconstruct for mandate/compliance.
- **Wave 4** — Task 6: golden vectors + threat-model addendum (mandate-swap,
  subject-spoof, capability-overreach, evidence-forgery, checker-collusion).
- **Wave 5** — Task 7: audit (code/security/test) + pre-deploy QA gate.

## Out of scope (V1)

zkVM/zkML/zkTLS proof *generation* (bound by hash only — compose with Delta /
TEEs); on-chain ERC-8274 verifier contract (inherits ERC-8004 / anchor-pluggable
backlog); multi-subject delegation chains (single `subject` in V1); a non-trivial
constraint DSL beyond the bounded predicate set (kept deliberately small so it
stays total and re-runnable).
