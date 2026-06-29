---
created: 2026-06-29
updated: 2026-06-29
status: draft
type: feasibility-memo
size: M
origin: arXiv 2606.23768 (Gabbay, "Cryptographic certificates of validity for trustworthy AI")
---

# Feasibility: SNARK policy-compliance certificates on the Mnemonic envelope

## TL;DR

Mnemonic today proves **authorship** (Ed25519/COSE_Sign1) and **integrity**
(canonical CBOR + blake3, anchored on Arweave/Solana). It does **not** prove the
*action it recorded was policy-compliant*. Gabbay's certificate closes that:
express a policy as a first-order-logic predicate, compile it to polynomial
constraints, and attach a SNARK proof `π` that the recorded action satisfies the
predicate. A verifier checks `π` in milliseconds — no re-execution, no access to
the private witness.

**This is buildable now**, because the cost wall (circuit size scaling with
*model* size) lives in a different layer we are *not* building (zkML inference
proofs). A policy circuit scales with **policy complexity**, not parameter count.

**It fits the existing architecture with one new capability, not a rewrite.** The
`verifiable-trajectories` feature already decided: *bind correctness proofs by
hash, never produce them in `core/`*. We keep that. The new move: SNARK
**verification** (unlike production) is cheap and stateless, so a verifier
*can* live in `core/`. That turns a bound-but-trusted `proof_ref` into an
**independently re-checked** proof — the audit-log → audit-proof step.

## Relationship to `verifiable-trajectories` (read first)

That feature shipped `STEP_V1`/`VERDICT_V1`/`TRAJECTORY_V1` behind
`trajectory-experimental`, with a three-layer model (`work/verifiable-trajectories/tech-spec.md`):

| Layer | Guarantee | Mnemonic role today |
|---|---|---|
| A | Steps ordered, unaltered | **Produces** (hash-linked chain) |
| B | Each step was a valid move | **Attests** a judge's verdict (co-sign) |
| C | Model ran faithfully on committed weights | **Binds by hash only** |

`VERDICT_V1` already carries `proof_ref` (hash of an external proof artifact) and
`proof_kind` ∈ `{prm, deterministic, zkml, tee, opml, ocp}`. A Gabbay policy
certificate is a **new `proof_kind` (`snark`/`policy`)** — *not* a new envelope.

**What this memo adds on top of that decision:**

- Layers A/B/C are about *the model and the trajectory*. A **policy certificate
  is a distinct, cheaper guarantee**: "the recorded output satisfies predicate
  `P`", independent of whether the model ran faithfully (C). It is provable today
  precisely because it does not touch the model.
- The trajectory feature *binds* `proof_ref` by hash and reports it; the verifier
  never re-runs it. For deterministic SNARK policies we can do better: **re-verify
  `π` at `verify` time** in `core/`. Cheap (ms), stateless, no model. This is the
  only net-new primitive — a verifier, never a prover.

The "no prover in `core/`" decision (`decisions.md`, 2026-06-27) is **unchanged**:
proving stays external/client-side, consistent with the non-custodial rule that
*the server signs nothing*.

## Why the circuit-size wall does not apply

Two different things can be proven, scaling on different axes:

1. **Policy over the output** (this memo). Circuit encodes the *predicate*. Size
   scales with rule complexity — hundreds to a few thousand constraints. GPT-2 vs
   a frontier model makes **no difference**; the model is not in the circuit.
   Prove in ms–seconds, verify in ms, proof ~200 B (Groth16) to tens of KB.
2. **The inference itself** (zkML — deferred, = layer C). Circuit encodes the
   *model*. Size scales with parameters. GPT-2 (124M) is the current edge at
   seconds-to-minutes; frontier models are not viable at interactive latency.

We build (1). (2) is the "natural next layer" the thread already defers — bound
by hash via the existing `proof_kind: zkml` path when it matures.

## What a policy certificate proves — and what it does NOT

- **Proves:** the recorded action data satisfies predicate `P`, bound to the
  content by a public-input commitment, with the private witness never disclosed.
- **Does NOT prove:** that the LLM honestly produced that output. A model could
  emit an output that happens to satisfy `P`. Closing that gap is layer C (zkML).
  Stated plainly so the user-facing claim never overreaches.

The honest one-liner: **"this recorded output is policy-compliant, and anyone can
re-check it"** — not "the agent behaved."

## Real-world use cases that compile to deterministic policies

A policy is cheap to prove **iff it is a deterministic function of data already
present at sign time**. Organised by the circuit primitive each reduces to.
"Data on hand?" = does the current envelope already carry the inputs.

### Tier 1 — arithmetic & ordering (cheapest; a handful of constraints)

| Use case | Predicate | Data on hand? |
|---|---|---|
| Spending cap (treasury/payment agent) | `amount ≤ approved_limit` | amount in content; limit from policy params |
| Double-entry conservation (bookkeeping) | `sum(line_items) == total` ∧ `debits == credits` | content |
| Action-budget / rate limit | `count_in_window ≤ N` | needs a counter input (witness) |
| Trade bounds (DeFi agent) | `0 ≤ slippage ≤ max` ∧ `price ∈ [floor, ceil]` | content |

> **Integer discipline:** circuits are over a finite field — no floats. Amounts
> must be minor units / fixed-point integers. This constrains *which* policies are
> clean and is a hard design rule, not a nicety.

### Tier 2 — set membership / non-membership (Merkle proof, ~log N hashes)

| Use case | Predicate | Note |
|---|---|---|
| Counterparty allowlist (payments) | `recipient ∈ approved_set` | set root in policy params |
| Sanctions screening (OFAC) | `target ∉ sanctioned_set` | non-membership |
| Tool/capability scoping | `tool_called ∈ permitted_tools[role]` | agent guardrail |
| Jurisdiction / licensing | `region ∈ licensed_regions` | |

### Tier 3 — structural / format / absence

| Use case | Predicate |
|---|---|
| PII absence in recorded output | no field matches SSN/PAN patterns |
| Mandated-disclosure fields present | required keys ⊆ content keys |
| Schema conformance | content matches approved CBOR shape |

### Tier 4 — authorization & delegation (in-circuit signature / hash-chain)

This tier is the **highest-leverage** target: RSAC 2026 named agent-to-agent
verification in delegation chains as an *unsolved* identity gap, and FINRA/SOX
require attributing each action to a specific authorized identity.

| Use case | Predicate |
|---|---|
| Authorized role | action carries a valid sig from a key in `approved_role_set` |
| Human-in-the-loop above threshold | `amount > T ⇒ valid human-approver signature present` |
| Delegation chain A→B→C | each hop proves "I hold a valid grant from my delegator ∧ my action ⊆ delegated_scope" |

### Tier 5 — temporal (composes with Solana timestamps)

| Use case | Predicate |
|---|---|
| Retention / expiry | `now ≤ created_at + retention_period` |
| Sequencing | action B's anchor postdates prerequisite A |

### Explicitly out of scope (not deterministic → zkML or trusted oracle)

"Is this toxic / medical advice / sound reasoning?" — any **learned semantic
judgment**. These need the classifier *in* the circuit (layer C / zkML) or an
oracle whose verdict you then policy-check. Naming them keeps the claim honest.

**Recommended first targets:** Tier 1 (spending cap, conservation), Tier 2
(allowlist/denylist), Tier 4 (delegation chain). Tier 4 is the differentiator —
it answers "who authorized what" across an agent chain without a central authority.

## Binding into the existing envelope (concrete)

### What carries the certificate

A policy certificate is the tuple
`{ policy_id, params_hash, public_inputs, proof_ref, backend }`:

- `policy_id` — content-addressed id of the compiled circuit + verifying key in
  the policy registry.
- `params_hash` — blake3 of the policy's public parameters (the limit, the
  allowlist root, the approved-role set). Pins *which* parameterization was used.
- `public_inputs` — the field elements the verifier needs, including the
  **content commitment** (see circularity, below).
- `proof_ref` — blake3 of the proof bytes `π`. `π` itself is stored on Arweave
  (it can be tens of KB); only its hash rides in the signed payload. This reuses
  the **bind-by-hash** decision verbatim.
- `backend` — `groth16` | `plonk` | `halo2` (determines the verify routine).

### Where it attaches

Two options; recommend **(A) for single memories, (B) for trajectories**.

**(A) `metadata.policy_cert` on `MEMORY_V1`** — zero schema change. `metadata`
is already optional and already inside the canonical CBOR that is blake3-hashed
and COSE-signed (`core/src/codec/schema.rs` MEMORY_V1; built at
`mcp/src/tools.rs:1101`). Adding a nested key changes the *value*, not the
`cbor_field_order`, so canonicalization is untouched and the certificate becomes
part of the signed, tamper-evident payload. Old attestations (no cert) keep
verifying; new ones coexist — same coexistence the `write_mode` column already
relies on.

**(B) new `proof_kind: "snark"` on `VERDICT_V1`** — for trajectory steps, reuse
the shipped `proof_ref`/`proof_kind` fields. `proof_ref` = the cert's `proof_ref`;
add `policy_id`/`params_hash`/`public_inputs` alongside. No new envelope.

> **Do NOT** store a bare `"passed": true`. A signed boolean only proves *the
> signer asserted compliance* — that is the audit **log** you already have. The
> certificate must carry `proof_ref` + `public_inputs` so a verifier re-checks
> `π`. This is the whole audit-log → audit-proof distinction.

### The circularity fix (binding without a fixed-point)

The proof must commit to the content, but the content (canonical CBOR) now
*contains* the proof — naive "public input = envelope hash" is circular. Resolve
by committing to a **stable sub-hash of the action fields only**:

1. `action_commitment = blake3(canonical_cbor_of(content, producer, created_at,
   tags))` — the pre-certificate fields, in canonical order. Stable regardless of
   the cert.
2. Generate `π` with `action_commitment ∈ public_inputs`.
3. Embed the cert in `metadata`, then canonicalize → blake3 → COSE-sign the whole
   envelope as today. The signature now covers the cert; the cert commits to the
   action; no fixed-point.

Verifier (three independent checks, any party, no re-execution):
1. **Ed25519** over COSE_Sign1 → authorship (existing `verify_artifact`).
2. **blake3** of payload == stored `content_hash` → envelope integrity (existing).
3. **SNARK**: recompute `action_commitment` from the content fields; assert it
   equals the cert's public input; fetch the verifying key for `policy_id`; run
   `backend.verify(vk, public_inputs, π)`. → policy compliance (**new**).

## Proposed `core/` module — `proof` (verifier-only)

Mirror the `Embedder` trait pattern (`core/src/embed/mod.rs`) and the
`chain_valid: Option<bool>` convention.

```
core/src/proof/mod.rs        # PolicyVerifier trait, PolicyCertificate, types
core/src/proof/registry.rs   # PolicyRegistry: policy_id -> (verifying_key, params_hash)
core/src/proof/groth16.rs    # first backend (feature = "snark-groth16"), e.g. arkworks
core/src/proof/mock.rs       # #[cfg(test)] MockVerifier (mirrors MockEmbedder)
```

- **Verifier only.** No prover in `core/` — proving stays client-side/external
  (non-custodial; `decisions.md` 2026-06-27 unchanged). `core/` gains the cheap
  half (ms pairing checks), never the expensive half.
- Feature-gated `snark-experimental` like `trajectory-experimental`; default
  `cargo build --workspace` stays green and dependency-free.
- `core/` stays native-only, one-way `core → mcp` intact (`PolicyVerifier` knows
  nothing about MCP/payment).

### `verify` extension

`VerificationResult` (`core/src/codec/sign.rs:92`) gains
`policy_valid: Option<bool>` — `None` = no cert present, `Some(false)` = proof
failed, `Some(true)` = verified. Exactly the `chain_valid` tri-state convention.
`mnemonic_verify` surfaces it as the third check.

## Phasing (waves)

- **Wave 1 — plumbing, no real crypto.** `proof` module skeleton +
  `PolicyCertificate` type + `MockVerifier` + `policy_valid: Option<bool>` wired
  through `verify`. Cert attaches to `metadata`; round-trips local mode. De-risks
  the envelope/circularity integration before any circuit exists.
- **Wave 2 — one backend, Tier 1.** Groth16 verifier + 2–3 real circuits
  (spending cap, conservation). Client-side prover lives in the SDK, not `core/`.
- **Wave 3 — Tier 2 + registry.** Merkle membership/non-membership; content-
  addressed `PolicyRegistry` with anchored verifying keys.
- **Wave 4 — Tier 4 delegation chain.** The RSAC differentiator; composes with
  trajectory `prev_hash` linkage.
- **Wave 5 — audit + conformance vectors** (golden `{policy, params, π, vk,
  public_inputs}` for byte-parity), threat model, QA gate.

## Honest risks & limitations

1. **Compliance ≠ honest computation.** Cert proves the *recorded output*
   satisfies `P`, not that the model produced it. Layer C / zkML closes that; do
   not conflate them in marketing.
2. **Trusted setup.** Groth16 needs a per-circuit ceremony. Acceptable for a
   small fixed policy registry; document each ceremony. Evolving policies favour a
   universal-setup backend (Plonk/Halo2, EZKL's stack) at the cost of larger proofs.
3. **Integer/field discipline.** No floats in-circuit — amounts as minor-unit
   integers. Limits which policies are clean (Tier-1 note).
4. **Public-input leakage.** Public inputs are visible. Don't expose a sensitive
   raw amount as a public input — commit to it and prove the relation, so the
   "no access to private inputs" claim holds.
5. **Expressiveness ceiling.** Only deterministic predicates compile cleanly.
   Semantic policies are out of scope until zkML — say so.
6. **Gabbay is one week old, single-author.** Use the FOL→polynomial framing as a
   design target, not a dependency. The buildable claim rests on mature SNARK
   tooling (arkworks/EZKL/gnark), not on that specific paper shipping.

## Open questions for the owner

- Backend: Groth16-per-policy (tiny proofs, ceremony each) vs universal-setup
  (no ceremony, bigger proofs)? Affects registry design.
- Where does the prover run first — SDK (client-side, non-custodial) or an
  opt-in operator sidecar service?
- Is this a standalone feature, or folded into `verifiable-trajectories` as the
  `proof_kind: snark` verification upgrade?
