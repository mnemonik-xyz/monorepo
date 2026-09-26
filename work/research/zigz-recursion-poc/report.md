---
created: 2026-07-01
type: research-report
relates: ../zigz-recursion/spec.md
poc_source: ../zigz/examples/recursion_poc/
zigz_snapshot: mnemonik-dev/zigz @ 85c7f77
---

# zigz recursion PoC — can zigz prove verification-flavored statements?

**Question (from `../zigz-recursion/spec.md`, Track A).** Recursion via
"verifier-as-guest" means running zigz's own verifier *inside* a zigz guest and
proving it. Its cost is dominated by **hashing in-VM** (Merkle + Fiat-Shamir over
Poseidon2). This PoC measures that cost directly, on real hardware, to decide
whether Track A is viable — and separately confirms that *verification
arithmetic* (the non-hash part) is cheap in-VM.

**Verdict up front:** verification arithmetic is cheap; **the real Poseidon2
permutation costs ~25.6k RISC-V steps in-VM and ~228 s to prove a SINGLE
permutation.** Naive verifier-as-guest recursion is therefore **not viable
without a Poseidon2 precompile** — which is exactly the A1 gate the spec named.
The PoC is a *pass* for "zigz can prove verification logic" and a decisive
*"precompile required"* for the hashing path.

## What was built

Two freestanding rv64im guests (+ a native host that proves/verifies/measures),
under `../zigz/examples/recursion_poc/`, wired as a `zig build recursion_poc` step:

1. **`verify_kernel_guest.zig`** — runs sumcheck-style *verification* arithmetic
   over BabyBear (P = 2013265921): for K rounds, evaluate a degree-2 univariate at
   0/1/challenge, check `g(0)+g(1) == claim`, Fiat-Shamir fold to the next claim,
   commit `ok` + `final_claim`. This is "proof-verification as a provable
   statement" — the atom of recursion.
2. **`hash_cost_guest.zig`** — runs the **real Poseidon2 permutation** N times
   in-VM and commits the digest. Uses hash-zig's allocator-free
   `poseidon2` core (vendored into `examples/recursion_poc/vendored_poseidon2/`
   so it cross-compiles freestanding). This is authoritative because **zigz's own
   zkVM commitment scheme uses Poseidon2** (`src/commitments/polynomial_commit.zig`
   → `CommitmentSchemePoseidon2`), so in-VM Poseidon2 cost *is* the recursion gate.

## Measured results (2026-07-01, zig 0.15.2, ReleaseSmall guests, single machine)

### PoC 1 — verify-kernel (verification arithmetic is cheap)

| K rounds | steps | proof size | prove | verify | result |
|---:|---:|---:|---:|---:|:--|
| 1 | 408 | 34 KB | 3.7 s | 68 ms | ACCEPT ✓ |
| 4 | 1,506 | 46 KB | 14.7 s | 93 ms | ACCEPT ✓ |
| 16 | 5,898 | 96 KB | 60 s | 197 ms | ACCEPT ✓ |

~**366 steps per sumcheck round**; verify stays 68–197 ms. Verification *logic*
runs fine in-VM — the substrate can prove "I checked a proof."

### PoC 2 — hash-cost (THE GATE: real Poseidon2 in-VM)

| N permutations | steps | proof size | prove | verify | result |
|---:|---:|---:|---:|---:|:--|
| 0 (baseline) | 198 | 31 KB | 1.9 s | 51 ms | ACCEPT ✓ |
| **1** | **25,821** | 242 KB | **228 s** | 489 ms | ACCEPT ✓ |
| 4, 8, 64 | — | — | **did not finish** | — | prohibitive |

**One Poseidon2 permutation ≈ 25,821 − 198 = 25,623 RISC-V steps in-VM.**
Proving a *single* permutation took ~228 s; N=4 (~100k steps) and N=8/64 never
completed (they were the multi-GB, 40-min runaway processes — killed).

## What this means for recursion (Track A)

zigz's verifier does **O(log n) Poseidon2 hashes** (Merkle openings +
Fiat-Shamir), plus field arithmetic. Extrapolating from the measured
~25.6k steps/permutation:

- Recursively verifying even a *small* proof (say 20–40 in-circuit hashes) →
  **~0.5–1M+ RISC-V steps of hashing alone** → tens of minutes to hours to prove,
  and multi-GB memory. **Not viable** as-is.
- The field-arithmetic half is cheap (PoC 1), so the wall is **entirely the
  hashing**, exactly as the spec predicted.

**Conclusion: Track A (verifier-as-guest recursion) requires a Poseidon2
precompile** — expose Poseidon2 as a Lasso lookup / dedicated constraint so a
permutation costs a handful of constraints instead of ~25.6k emulated RISC-V
steps. That precompile (spec step **A1**) is not optional; it is the gate. Until
it exists, use the **checkpoint state-chaining** substitute for unbounded intents
(decisions.md), which needs no recursion.

## Honest limitations

- The Poseidon2 core is hash-zig's KoalaBear Plonky3 instance, vendored for
  freestanding compilation; zigz's commitment uses the same family. The *cost
  order of magnitude* (~10^4 steps/perm) is the load-bearing result and is robust;
  exact constants depend on instance width (16 vs 24) and S-box implementation.
- N≥4 was not run to completion (prohibitive); the per-permutation cost is derived
  from the N=0→N=1 delta. A precompiled build would be needed to measure large N.
- These are single-machine, unoptimized-VM numbers. A production prover would be
  faster in absolute terms but the *ratio* (hash-in-VM ≫ field-op-in-VM) stands.

## Next steps

1. **Prototype the A1 Poseidon2 precompile** (Lasso lookup) in zigz; re-run
   PoC 2 to measure steps/permutation *with* the precompile — the go/no-go for
   Track A. Target: from ~25.6k steps → low hundreds.
2. If A1 passes, build the minimal **verifier-as-guest** (PoC 1 + a precompiled
   Merkle/FS check) and measure the recursion overhead ratio (spec A2).
3. Keep folding (Track B) as a parallel research track; it does not depend on A1.

## Reproduce

```
cd work/research/zigz && zig build recursion_poc   # ~5 min (verify-kernel + hash N∈{0,1})
```
(needs zig 0.15.2 + hash-zig/ssz in zig's cache; the poseidon2 core is vendored.)
