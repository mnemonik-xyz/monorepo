---
created: 2026-07-01
type: consolidated-research-report
subject: zigz zkVM as Mnemonic's proving substrate
status: research complete for the architecture decision; one open item (A1 precompile)
---

# zigz research — consolidated report

Synthesis of all zigz investigation for the Mnemonic correspondence layer. Pulls
together the capability spikes, the performance measurements, and the recursion
PoC into one verdict, and names the single remaining research question.

## Executive verdict

- **zigz is viable as the policy prover** (Job A): it proves bounded, stateful,
  multi-action, evidence-bound policies in seconds, with cheap verification. The
  test-case range is broad **as long as hashing stays in the Rust verifier, not
  the guest**.
- **Recursion / aggregation (Job B) is NOT viable as-is**: the real Poseidon2
  permutation costs **~25.6k RISC-V steps in-VM**, so verifier-as-guest recursion
  is prohibitive **without a Poseidon2 Lasso precompile** (the "A1" gate).
- **Decision:** ship zigz as the policy prover; defer recursion behind A1; handle
  unbounded intents with checkpoint state-chaining (no recursion needed).

The one open research question is therefore: **does an A1 Poseidon2 precompile
bring per-permutation cost from ~25.6k steps down to the low hundreds?** That is
the go/no-go for recursion, and the natural next research effort.

## 1. What zigz is

Owner's own Zig zkVM, Jolt-inspired: **sumcheck + Lasso lookups + Binary Merkle
commitments** (hash-based → **transparent, no trusted setup**, post-quantum),
**BabyBear** field, **RISC-V RV64IM**. Verifier is O(log n) and already hardened
against the Jolt "unfaithful-claims" Fiat-Shamir bug. Its zkVM commitment scheme
uses **Poseidon2** (via hash-zig) — which is why in-VM Poseidon2 cost gates
recursion.

## 2. Capability — can it prove complex statements? (YES)

Measured with a real guest proving a **stateful, multi-action, evidence-bound
payment mandate** (Σ ≤ cap running total + per-action cap + allowlist membership +
non-decreasing timestamps + evidence binding):

| Scenario | result | correct? |
|---|---|---|
| compliant (4 pays) | ok=1 | ✓ |
| over aggregate cap | ok=0 | ✓ |
| off-allowlist vendor | ok=0 | ✓ |
| out-of-order timestamps | ok=0 | ✓ |
| evidence mismatch (Delta lesson) | ok=0 | ✓ |

→ "Intent more complex than a tx" is real: the policy is a **program over a
sequence carrying state**, and every constraint class evaluates correctly.
Source: `computation-proof/spikes/{zigz-stateful-intent, payment-mandate-v1}/`.

## 3. Performance — measured (zig 0.15.2, single machine, ReleaseSmall guests)

| Program | steps | proof size | prove | verify |
|---|---:|---:|---:|---:|
| tiny demo (4 steps) | 4 | ~7 KB serialized | 48 ms | 11 ms |
| mandate, 4 payments | 256 | ~32 KB | 1.5 s | 42 ms |
| mandate, 50 payments | 2,648 | ~58 KB | 24 s | 96 ms |

**Verify is ms and ~flat (O(log n)); proof size grows sub-linearly; prove time is
~linear in steps** and is the scaling cost. For v1-sized intents (a handful of
actions) this is 1–2 s — fine. Long intents → checkpoint-batch.

## 4. Recursion — measured, verdict = precompile required

PoC ran the **real Poseidon2 permutation** in-VM (`zigz-recursion-poc/`):

| PoC | measurement | reading |
|---|---|---|
| verify-kernel (verification arithmetic) | K=1→16: 408→5,898 steps; verify 68–197 ms | verification *logic* is cheap in-VM |
| hash-cost (real Poseidon2) | **N=1: 25,821 steps, ~228 s to prove**; N≥4 prohibitive | **~25.6k steps per permutation** |

zigz's verifier does O(log n) Poseidon2 hashes → recursively verifying even a
small proof is ~1M+ steps → hours. **Track A (verifier-as-guest) needs the A1
Poseidon2 precompile.** Field arithmetic is not the problem; hashing is the whole
wall. Full detail + honest limitations: `zigz-recursion-poc/report.md`; the R&D
plan (Tracks A/B/C): `zigz-recursion/spec.md`.

## 5. Architecture role

- **zigz guest** = policy logic ONLY; ~zero in-guest hashing.
- **Rust `core/correspondence`** = hashing, commitment binding, and cheap
  re-verification of the zigz proof (Wave 2's pure-Rust verifier).
- **`mnemonic-prover`** = drives zigz to produce the proof + evidence.
- Unbounded intents → checkpoint state-chaining. Recursion/aggregation/on-chain
  wrap → behind A1. See `computation-proof/architecture.md` (diagrams 2, 4).

## 6. Open research question (the only one left)

**A1 — prototype a Poseidon2 precompile (Lasso lookup) in zigz**, then re-run the
hash-cost PoC. Success = permutation cost drops from ~25.6k steps to low hundreds,
which would make verifier-as-guest recursion (and aggregation, and the on-chain
wrap) viable. This is real zkVM engineering (weeks), scoped in
`zigz-recursion/spec.md` Track A. **Not a blocker for shipping the policy prover.**

Secondary (parallel, research-grade): Track B folding fit-analysis
(ProtoStar / split-accumulation vs zigz's hash-commitment + sumcheck stack).

## 7. Artifact index

- `zigz/` — vendored zkVM snapshot (@ 85c7f77) + `examples/recursion_poc/`.
- `computation-proof/feasibility.md` — proof-size + primitives study.
- `computation-proof/spikes/zigz-stateful-intent/` — first stateful spike + results.
- `computation-proof/spikes/payment-mandate-v1/` — evidence-bound hardened spike.
- `computation-proof/architecture.md` — the viable-architecture flow diagrams.
- `zigz-recursion/spec.md` — recursion/folding R&D plan (Tracks A/B/C).
- `zigz-recursion-poc/report.md` — the recursion measurements + verdict.
- `computation-proof/decisions.md` — dated decision log (backend, verifier, waves).
