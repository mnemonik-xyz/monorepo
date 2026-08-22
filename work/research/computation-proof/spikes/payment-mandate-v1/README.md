# Spike: payment_mandate_v1 — evidence-bound payment mandate on zigz (measured)

**Question:** can the stateful payment-mandate intent be hardened so the proof is
about *reality* — what the merchant actually saw — rather than the agent's own
self-claim?
**Answer (measured 2026-06-30, real build + run): yes.**

This is the hardened successor to the first spike
(`../zigz-stateful-intent/`). It keeps every clause of that mandate and adds one
new clause: **evidence binding**.

## What this adds vs the first spike: the EVIDENCE-BINDING clause (the "Delta lesson")

The original guest proved a sequence of payments jointly satisfied a mandate, but
every fact it checked — vendor, amount — came from the *same* untrusted source
(the agent's claimed action). A correct proof over fabricated inputs is still a
proof over fabricated inputs: it certifies the agent's story is internally
consistent, not that it matches what happened.

The **Delta lesson**: bind each action to an independent, merchant-authenticated
**EVIDENCE** value carried on the input tape. The per-payment record grows from

```
(vendor, amount, ts)            →   (vendor, amount, ts, ev_vendor, ev_amount)
```

and the guest loop adds one clause, while keeping all the originals:

```zig
if (vendor != ev_vendor or amount != ev_amount) { ok = 0; }
```

Now the proof asserts: *the agent's claimed (vendor, amount) for every action
equals the value the merchant authenticated.* The agent can no longer pass the
mandate by lying about what it paid or to whom — the proof is about reality.

In production `ev_vendor`/`ev_amount` would be derived inside the guest from a
merchant-signed receipt (signature check + field extraction); here they are
supplied directly on the tape to isolate and measure the binding clause itself.

### Clauses proven (v1)

- `Σ amounts ≤ cap` — **stateful aggregate** (running total carried across actions)
- each `amount ≤ max_single` — per-action arithmetic
- every `vendor ∈ allowlist` — membership (linear scan)
- timestamps **non-decreasing** — sequencing / temporal
- `(vendor, amount) == (ev_vendor, ev_amount)` — **evidence binding (NEW)**

## Measured results (zig 0.15.2, zigz @ `mnemonik-dev/zigz`, ReleaseSmall guest)

Run 2026-06-30 on this machine via `zig build payment_mandate`.

| Scenario | committed `ok` | steps | proof size | prove | verify |
|---|---|---|---|---|---|
| compliant (4 pays, Σ=900≤1000) | 1 ✓ | 256 | ~32248 B | 1541 ms | 42 ms |
| OVER CAP (Σ=1100>1000) | 0 ✓ | 204 | ~31684 B | 1540 ms | 42 ms |
| OFF-ALLOWLIST (vendor 99) | 0 ✓ | 151 | ~31108 B | 1538 ms | 41 ms |
| OUT-OF-ORDER ts | 0 ✓ | 152 | ~31120 B | 1532 ms | 40 ms |
| **EVIDENCE MISMATCH (a≠ev_a)** | **0 ✓** | 152 | ~31120 B | 1526 ms | 41 ms |
| compliant (50 pays) | 1 ✓ | 2648 | ~58352 B | 23824 ms | 96 ms |

Every row verified to `Accept`, and every committed `ok` matched its expectation
(compliant → 1, each violation → 0). The new **EVIDENCE MISMATCH** scenario
(action #2 claims amount 100 while its evidence says 300; all other clauses pass)
correctly trips `ok=0`.

## Honest interpretation

- **Evidence binding is cheap.** Adding the clause adds two tape reads and one
  comparison per action; the mismatch scenario proves in ~1.5 s like the other
  small cases. The hardening that makes the proof meaningful costs almost nothing.
- **The binding clause is independently observable.** EVIDENCE MISMATCH passes the
  cap, single-payment, allowlist, and ordering checks — only the
  evidence-binding clause fails it. That isolates the new behavior cleanly.
- **All prior properties are preserved.** Stateful `Σ ≤ cap`, per-action cap,
  membership, and ordering still trip `ok=0` exactly as in the first spike.
- **Verify stays cheap and ~flat** (40–96 ms, O(log n)) — ideal for "verify
  everywhere." Proof size grows sub-linearly (~31 KB → ~58 KB, 256 → 2648 steps).
- **Proving is the bottleneck, ~linear in steps.** 50 payments → 2648 steps →
  ~24 s on this *unoptimized, experimental* VM (lookups dominate). Fine for
  v1-sized intents (a handful of actions, ~1.5 s); the cost at 50+ actions
  remains the motivation for the recursion/aggregation R&D track and for
  checkpoint-batching long intents.

## How to reproduce

Needs the zigz repo + zig 0.15.2.

1. Drop `payment_mandate_v1_guest.zig` into
   `examples/payment_mandate_guest/src/main.zig`.
2. Drop `payment_mandate_v1_host.zig` into `examples/payment_mandate.zig`.
3. Ensure a `payment_mandate` build step exists in `build.zig` mirroring the
   `fibonacci` block (reusing `riscv_target` + `zigz_io_mod`). The first spike's
   step works unchanged — only the example sources differ.
4. Build + run:

```
export PATH="<zig-0.15.2-dir>:$PATH"
zig build payment_mandate
```

The per-scenario summary lines (`grep ok=`) print committed `ok`, steps, proof
size, and prove/verify timings. The source files in this dir are the exact
hardened spike code.
