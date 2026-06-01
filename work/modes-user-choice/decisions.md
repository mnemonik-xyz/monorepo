# Decisions — modes-user-choice

Append-only log of decisions and audit findings.

## Interview outcomes (2026-06-01, user)

**DECIDED:**
- **Payment UX** — *pay per shared artifact* (per-`participate`-write cost; local
  writes always free). Aligns with issue #28's per-sign model.
- **Retraction** — *permanent / immutable*. Once participated, the anchor is
  immutable; no un-share / tombstone in V1. Matches append-only design.

**PENDING RESEARCH (blocks finalizing participate semantics):**
- **What "participate" means** — broadcast-to-pool vs directed point-to-point
  exchange. User: "no answer yet, need deeper research on what is really on
  demand by multi-agent trustless development." → deep-research launched.
- **Delivery-guarantee definition** — which failure to protect against
  (durability / retrievability / proof-of-delivery). User: "need proper
  decision on this." → fold into same research, then re-present for sign-off.

## Open decisions (awaiting user sign-off before Wave 1)

1. **Storage invariant** — S1 (tag rows in one DB, recommended) vs S2 (separate
   DB per mode). User answered "decide in the spec"; tech-spec recommends S1.
2. **Delivery definition for V1** — pending research (see above).
3. **Participate semantics** — pending research (see above).
4. **Mode granularity** — per-request `mode` field on `sign_memory`
   (recommended) vs per-identity default vs both.

## Locked context (from user, 2026-06-01)

- Local storage (filesystem / self-hosted) ⇒ **free, no payment**. Structural,
  per whitepaper §5.7.1.
- "Participate" (share/exchange artifacts with the network) ⇒ protocol must
  **guarantee delivery**; purely-local artifacts are risky to share because they
  may be unreachable when a peer asks. This is the paid service-layer path.
- Mode must be the **user's** choice driven by intent, not an operator env var.
