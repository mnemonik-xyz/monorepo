# Decisions — modes-user-choice

Append-only log of decisions and audit findings.

## Interview outcomes (2026-06-01, user)

**DECIDED:**
- **Payment UX** — *pay per shared artifact* (per-`participate`-write cost; local
  writes always free). Aligns with issue #28's per-sign model.
- **Retraction** — *permanent / immutable*. Once participated, the anchor is
  immutable; no un-share / tombstone in V1. Matches append-only design.

**RESEARCH-BACKED RECOMMENDATIONS (deep-research 2026-06-01, see research.md —
awaiting final user sign-off):**
- **What "participate" means → broadcast-publish a verifiable public record**, not
  recipient-ACK handoff. Evidence: cross-operator exchange is dominated by
  *directed* message-passing that explicitly shares no memory (A2A "Opaque
  Execution"); the only *broadcast* pattern shipping is ERC-8004's public
  attestation/reputation registry — which is Mnemonic-shaped. Verifiable
  cross-operator *shared memory* is still aspirational, so V1 doesn't bet on it.
  Directed exchange deferred to the A2A bridge.
- **Delivery-guarantee → D1: durable write + read-back + re-verify.** Cheapest
  guarantee meaningfully stronger than a receipt; needs no online counterparty;
  fills the exact gap ERC-8004/IPFS leave open (hash committed, availability NOT
  guaranteed). Recipient-ACK (D2) is the gold standard but needs an online
  counterparty → deferred. Receipt schema forward-shaped for D2.
- **Mnemonic wedge identified:** "ERC-8004 proves a hash; Mnemonic proves the
  bytes are actually retrievable."

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
