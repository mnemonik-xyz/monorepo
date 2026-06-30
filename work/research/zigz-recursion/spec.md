---
created: 2026-06-30
updated: 2026-06-30
status: draft
type: tech-spec
target_repo: mnemonik-dev/zigz   # authored here (monorepo scope); relocate to zigz docs/
relates: ../computation-proof/decisions.md (2026-06-30 zigz recursion finding)
---

# zigz scalability: recursion, folding & proof aggregation

> Authored in the Mnemonic monorepo for scope reasons. Intended to live in the
> **zigz** repo (e.g. `docs/RECURSION.md` / a design issue). Mnemonic is the
> driving consumer but the work is general-purpose zkVM scalability.

## Goals

1. **Aggregation** — combine *k* independent zigz proofs into **one** proof
   (cheap verification at scale; "many attestations → one proof").
2. **IVC / unbounded computation** — prove a long/streaming computation with
   **bounded** proof size (long-running stateful intents in one constant-size
   proof, instead of O(batches) checkpoints).
3. **Compression / on-chain** (optional) — a tiny final proof a contract can
   verify.

**Preserve transparency** (no trusted setup) wherever possible — it is zigz's
defining property; only Track C (on-chain wrap) may trade it deliberately.

## Why (motivation, brief)

Today zigz emits **monolithic** proofs of a single RISC-V run (no recursion —
`MODULES.md`/`VERIFIER.md` list it as future). That is fine for bounded programs,
but blocks: constant-size proofs over unbounded history, and aggregating many
proofs into one. Both have direct product value for the Mnemonic correspondence
layer and for zkVM users generally.

## Current architecture — and why recursion is non-trivial *here*

zigz = **sumcheck + Lasso lookups + Binary Merkle commitments** (hash-based,
transparent, post-quantum), **BabyBear** (31-bit) field, **RISC-V RV64IM**.
Two structural facts shape every approach:

- **Hash-based commitments → no additive homomorphism.** Classic **Nova-style
  folding assumes homomorphic (Pedersen/group) commitments + R1CS** — neither
  holds here. Nova does **not** drop in.
- **Recursion bottleneck = hashing inside the VM.** Verifying a zigz proof means
  recomputing many Merkle + Fiat-Shamir hashes. If those are SHA/Keccak/Blake3,
  each hash is hundreds–thousands of RISC-V steps → recursion overhead explodes.
- **Small field.** BabyBear soundness relies on an extension field; in-VM field
  emulation adds cost. Note for any recursion-layer arithmetic.

## Track A — recursion via "verifier-as-guest" (engineering-first, recommended)

Compile the **existing O(log n) verifier** (`src/verifier/`) to a RISC-V guest and
prove its execution. No new proof system. Yields **aggregation** (verify *k*
proofs inside one guest → one proof) and **compression** (one wrap proof).

**Critical enabler — a recursion-friendly hash + precompile.** zigz already
depends on **`hash-zig` (Poseidon2)**. Poseidon2 is algebraic → far cheaper to
prove in-VM than Blake3/SHA. The linchpin is exposing it as a **Lasso lookup
precompile** (Jolt's strength is lookups) so in-VM hashing is cheap. Without this,
Track A is infeasible; with it, it is "engineering-hard," not "research-hard."

**Phases (each gated by a benchmark):**
- **A0** — make the transcript + Merkle hash pluggable; add a **Poseidon2**
  backend behind a flag (dual-hash: keep Blake3 for the base layer, Poseidon2 for
  the recursion layer if needed). Measure base-prover impact.
- **A1** — **Poseidon2 as a Lasso/precompile**; benchmark cost-per-hash *inside*
  the VM. **Gate:** in-VM hash must be cheap enough that A2 is viable.
- **A2** — **verifier-as-guest**: minimal RISC-V guest that runs the zigz verifier;
  prove one recursion step. Measure **overhead ratio** = `prove(verify-guest) /
  prove(base)`.
- **A3** — **aggregation tree**: a guest that verifies *k* proofs + a combine step
  → one proof; benchmark size/time vs *k* separate proofs.

**Success:** recursion-step proof time within a small constant of base; aggregated
proof ~constant size; verify still ms. Aggregation (A3) is the first shippable
win and covers Mnemonic's "many attestations → one proof" need.

## Track B — folding / accumulation (research-grade)

Goal: **cheap per-step accumulation**, deferring a single expensive proof to the
end (true IVC) — for unbounded streams without O(batches) artifacts.

**The fit problem:** zigz's hash commitments + sumcheck/Lasso do not match Nova's
homomorphic-commitment + R1CS assumptions. Candidate directions that *might* fit:

- **ProtoStar / ProtoGalaxy** — accumulation for high-degree gates **with
  lookups**; closest in spirit to Lasso + sumcheck.
- **Split-accumulation (BCLMS, "PCD without succinct arguments")** — accumulation
  **without** homomorphic commitments → directly relevant to hash-based zigz.
- **Sumcheck / GKR folding** and **lookup-centric IVC** (Mangrove-style) lines.

**Phases:**
- **B0** — literature review + **fit analysis**: which scheme is compatible with
  Lasso + sumcheck + Merkle over BabyBear? Output: a short feasibility note.
- **B1** — paper-design the accumulation step for **one** constraint/lookup type.
- **B2** — prototype that single accumulation step; benchmark fold-step cost vs a
  full base proof.
- **B3** — **decision gate:** pursue full IVC, or stop and rely on Track A
  aggregation + checkpoint state-chaining. Concluding "not worth it" is an
  acceptable, valuable outcome.

**Risk:** may not converge. Keep strictly off any product critical path.

## Track C — compression / on-chain wrap (optional, separable)

Wrap the final zigz proof in a succinct SNARK (Groth16/Plonk over a verifier
circuit) → tiny on-chain proof. **Reintroduces a trusted/universal setup** —
deliberately trades transparency, only when on-chain verification is required.
Independent of Tracks A/B; sequence only when a contract-verification use case is
real.

## Recommended sequencing

**A0 → A1 first** (the Poseidon2 precompile is the linchpin *and* independently
useful), then **A2 → A3 aggregation** (clearest product value). Run **B0–B1** as a
parallel research spike. **C** only on demand.

## Consumer interface (what zigz should expose)

```
aggregate(proofs: []Proof) -> Proof              // Track A3
// Track B (if pursued):
ivc_step(prev: Acc, witness: Witness) -> Acc
ivc_finalize(acc: Acc) -> Proof
```

Recursion adds **new proof variants** → they MUST be **versioned**. This ties to
the `zigz-proof-v1` freeze (Mnemonic `tech-spec.md`): an aggregated/IVC proof is a
distinct, separately-versioned format with its own conformance vectors.

## Success metrics / benchmarks

- in-VM Poseidon2 cost (cycles/hash) **before vs after** the precompile (A1 gate).
- recursion overhead = `prove(verify-guest) / prove(base)` (A2).
- aggregation: *k* → 1 proof **size + verify time** vs *k* separate proofs (A3).
- (B) fold-step cost vs full-proof cost (B2).

## Risks

- **A1 is gating** — hash-in-VM cost without the precompile kills Track A.
- BabyBear small-field recursion subtleties (extension-field soundness).
- Track B research may not converge.
- Maintenance: recursion adds proof variants + verifier surface; differential
  conformance vectors must cover every variant (Zig prover ↔ Rust re-verifier in
  Mnemonic `core/`).

## Non-goals

- Sacrificing base-layer transparency (trusted setup) outside the Track C wrap.
- Changing base prover/VM semantics.
- Putting folding (Track B) on any product critical path.

## Open questions

1. **Self-recursion** (zigz-verifier-in-zigz) vs wrapping in a different system?
2. Poseidon2 **everywhere**, or **dual-hash** (Blake3 base + Poseidon2 recursion)?
3. **Aggregation-only** (covers Mnemonic's needs today) vs committing to full IVC?
4. Who owns this track — zigz core, or a Mnemonic-funded contribution upstream?
