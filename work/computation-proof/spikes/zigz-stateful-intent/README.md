# Spike: stateful multi-action intent on zigz (measured)

**Question:** can an intent *more complex than a single transaction* — stateful,
over a sequence of actions — be expressed, proven, and verified on zigz?
**Answer (measured 2026-06-30, real build + run): yes.**

## What the guest proves

A monthly payment **mandate** over a *sequence* of payments, jointly:
- `Σ amounts ≤ cap` — **stateful aggregate** (running total carried across actions)
- each `amount ≤ max_single` — per-action arithmetic
- every `vendor ∈ allowlist` — membership (linear scan)
- timestamps **non-decreasing** — sequencing / temporal

This is the seed of Wave 2's `payment_mandate_v1` guest (see `../../tech-spec.md`,
`../../v1-agentic-payments.md`).

## Measured results (zig 0.15.2, zigz @ `mnemonik-dev/zigz`, ReleaseSmall guest)

| Scenario | committed `ok` | steps | proof size | prove | verify |
|---|---|---|---|---|---|
| compliant (4 pays, Σ=900≤1000) | 1 ✓ | 210 | ~31 KB | 1536 ms | 41 ms |
| over aggregate cap (Σ=1100) | 0 ✓ | 169 | ~31 KB | 1530 ms | 40 ms |
| off-allowlist vendor (99) | 0 ✓ | 127 | ~31 KB | 788 ms | 35 ms |
| out-of-order timestamps | 0 ✓ | 128 | ~31 KB | 788 ms | 36 ms |
| compliant (50 pays) | 1 ✓ | 2096 | ~53 KB | 23666 ms | 89 ms |

## Honest interpretation

- **Stateful multi-action intents are provable today** — the aggregate `Σ ≤ cap`
  is real state carried across the sequence, and all four constraint kinds are
  evaluated correctly (compliant → 1; each violation → 0).
- **Verify is cheap and ~flat** (35–89 ms, O(log n)) — ideal for "verify
  everywhere."
- **Proof size grows sub-linearly** (~31 KB → ~53 KB from 210 → 2096 steps).
- **Proving is the bottleneck, ~linear in steps.** 50 payments → 2096 steps →
  ~24 s on this *unoptimized, experimental* VM (lookups dominate: 1936 Lasso
  proofs). Fine for v1-sized intents (a handful of actions, 1–2 s); the cost at
  50+ actions is the empirical motivation for the recursion/aggregation R&D track
  (`../../../zigz-recursion/spec.md`) and for checkpoint-batching long intents.

## How to reproduce

Needs the zigz repo + zig 0.15.2. Drop `payment_mandate_guest.zig` into
`examples/payment_mandate_guest/src/main.zig`, `payment_mandate.zig` into
`examples/payment_mandate.zig`, add a `payment_mandate` build step mirroring the
`fibonacci` block in `build.zig` (reusing `riscv_target` + `zigz_io_mod`), then:

```
zig build payment_mandate
```

Source files in this dir are the exact spike code.
