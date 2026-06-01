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

— all resolved, see FINALIZED below.

## FINALIZED (2026-06-01, user sign-off)

1. **Storage invariant → S1** (tag rows with `write_mode` in one DB; local +
   shared coexist for one user; recall spans both). Retires CLAUDE.md's "Never
   mix in one DB" as a *conscious* change (update in same PR).
2. **Delivery definition → "anchored AND verified by recall = delivered"** (user's
   words). Supersedes the abstract "D1 read-back": the delivery proof is a
   **recall + verify round-trip against the anchored artifact** — anchor on
   Arweave/Solana, then confirm recall can retrieve it and `verify` re-checks the
   anchored bytes (hash + COSE signature). Reuses existing recall/verify
   machinery; no bespoke read-back primitive. Until that round-trip passes, the
   write is NOT "participated" (still local; no charge on failure).
3. **Participate semantics → broadcast / public publish.** Anchor signed COSE
   **plaintext** (today's shape). Anyone with the tx id can read+verify — the
   ERC-8004-style public attestation model. **No encryption / no key-based access
   in V1** (user: "Public publish"). Encrypted-share + capability-token sharing
   is a future arc, not this iteration. Directed/recipient-ACK exchange deferred
   to the A2A bridge.
4. **Mode granularity → per-request `mode` field on `sign_memory`** (default
   `local`).
5. **Payment → per shared artifact**; local always free (from interview).
6. **Retraction → permanent / immutable** (from interview).

**Clarification that reframed the guarantee (user, 2026-06-01):** anchored-on-
Arweave = "mission done" — the risk surface is *local-only* artifacts. `participate`'s
job is to move an artifact out of the local-only risk surface into the anchored
state; the only silent failure is reporting "participated" when the anchor didn't
actually land. That is exactly what "verified by recall" closes. Correction logged:
today's Arweave write is **signed, not encrypted** (`core/src/arweave/mod.rs:56`,
`core/src/wasm/mod.rs:208`) — so "public publish" is consistent with current code;
encryption would have been net-new.

## Locked context (from user, 2026-06-01)

- Local storage (filesystem / self-hosted) ⇒ **free, no payment**. Structural,
  per whitepaper §5.7.1.
- "Participate" (share/exchange artifacts with the network) ⇒ protocol must
  **guarantee delivery**; purely-local artifacts are risky to share because they
  may be unreachable when a peer asks. This is the paid service-layer path.
- Mode must be the **user's** choice driven by intent, not an operator env var.
