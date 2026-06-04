# Research: what multi-agent trustless systems demand from shared/exchanged memory

> ⚠️ **NEEDS REVIEW (flagged 2026-06-01).** Background research, still largely useful,
> but it fed conclusions that predate (a) the user's reframing into a *transparent
> self-host ↔ remote cost spectrum* (incl. durable **cloud** for private cross-device
> reuse, not only public on-chain broadcast) and (b) the discovery of the shipped
> `work/chrome-extension/`. Re-read against **`user-spec.md` (canonical)** before
> using any conclusion.

**Date:** 2026-06-01 · **Method:** deep-research (5 parallel angles → claims →
cross-verification). Feeds the two open questions in `decisions.md`:
*what "participate" means* and *what "delivery guarantee" means*.

> Fetch caveat: many primary docs (eips.ethereum.org, github.com, ipfs/arweave
> docs, arxiv PDFs) returned HTTP 403 to the fetcher; high-confidence claims rest
> on canonical source repos + cross-angle corroboration. Low-confidence/
> aspirational claims are tagged.

## Q1 — Broadcast vs Directed: what's actually in demand

The dominant pattern for **agent-to-agent exchange/handoff is DIRECTED
point-to-point**; the only **broadcast** pattern with real traction is
**publishing a verifiable public record** (attestation/reputation), not a shared
memory pool.

| System | Model | "Memory" sharing? |
|---|---|---|
| **A2A** (Google→Linux Foundation, 50+ partners) | Directed: POST a Task to one agent's AgentCard endpoint | **None** — "Opaque Execution": agents share no internal state/memory; only `contextId` grouping [0.9] |
| **ACP** (IBM/LF) | Directed: POST to a named agent's `/runs`, poll `run_id` | No durable shared memory; Run state machine only [0.9] |
| **AGNTCY/SLIM** (Cisco) | Directed default (name-addressed) + optional pub/sub | Transport, not memory [0.85] |
| **ERC-8004** (Ethereum, mainnet ~Jan 2026) | **Hybrid**: Validation *directed* (`validationRequest(validator, agentId,…)`); Reputation *broadcast* (public registry anyone reads via `readAllFeedback`) [0.9] | Commits hash+URI of off-chain attestations |
| **Letta / Mem0 / Zep / cognee** | Shared memory blocks/namespaces/graphs | **Intra-org only** — trust boundary already collapsed; a plain shared DB [0.7–0.95] |

**Key findings:**
- Cross-operator coordination is being built as **directed message-passing that
  explicitly refuses to share memory** (A2A "Opaque Execution") [conf 0.9, two
  independent angles].
- True broadcast shared-memory pools exist **only inside one deployment/org**,
  where the trust boundary is already gone and crypto-verification is moot [0.85].
- **[ASPIRATIONAL]** "Verifiable, signed, cross-operator *memory* pool" appears
  almost only in 2025–26 research papers, not shipped roadmaps; the verifiable-
  agent demand that *does* exist targets **identity / credentials / audit-logs**,
  not a shared memory store [conf 0.6–0.75, agent self-flagged as unproven].
- BUT the one broadcast pattern that **is shipping** is exactly Mnemonic-shaped:
  ERC-8004's **public registry of signed, hash-committed, off-chain attestations**
  that anyone can discover, fetch, verify, and aggregate (reputation/validation)
  [0.9]. This is "publish a verifiable record," not "hand memory to a peer."

## Q2 — What "delivered" must provide

Three tiers, increasing strength and cost:

1. **Proof-of-receipt** (txid / HTTP 200): network *accepted* bytes. ~Free.
   Proves admission, **not** durability or retrievability.
2. **Durable-existence** (Arweave endowment, a pin): bytes *intended* to persist.
   Cheap, economically-backed, but **probabilistic** [0.85].
3. **Provable-retrievability** (Filecoin PoSt, Sia storage proofs, academic PoR):
   repeated cryptographic challenge proving data is *still there and recoverable
   now*. The only tier defending against silent post-write loss — **expensive**
   [0.9].

**What real protocols treat as "delivered":**
- They almost never require a **recipient ACK**. x402 proves "paid" via an
  **on-chain tx hash** (external anchor, not payee signature) [0.93]; ACP/A2A
  treat a **terminal state transition** (`completed`) as success [0.85].
- A recipient counter-signature is the non-repudiation **gold standard** but is
  expensive — it needs an **online, responsive counterparty**, which conflicts
  with the async/fire-and-forget patterns these protocols favor [0.9].
- **ERC-8004 explicitly does NOT guarantee retrievability** — it commits
  hash+URI and even *omits* the hash for IPFS (CID is self-verifying); off-chain
  availability is "the publisher's responsibility, not a protocol guarantee"
  [0.85]. **"A gap that durable storage like Arweave is positioned to fill."**
- Arweave's own tooling warns a `200` ≠ "seeded" — so a **read-back + re-hash +
  signature re-verify is the cheap approximation of proof-of-retrievability**:
  it collapses "we wrote it" into "we just retrieved the exact signed bytes
  back" [0.85, storage angle].

## Synthesis → recommendation for Mnemonic

The two answers reinforce each other and point one way:

**"Participate" V1 = broadcast-publish a durably-anchored, publicly-verifiable,
discoverable attestation** — the ERC-8004 reputation/validation analog that is
actually shipping — **not** a recipient-ACK point-to-point handoff. Directed
exchange (recipient ACK) is real for *task handoff* but belongs with the A2A
bridge later (`work/a2a-bridge/`), and verifiable *shared memory* across
operators is still aspirational, so we don't bet V1 on it.

**Delivery guarantee V1 = D1 (durable write + read-back + re-verify).** It is the
cheapest thing meaningfully stronger than a receipt, needs **no online
counterparty**, and **directly fills the exact gap ERC-8004/IPFS leave open**
(hash committed, availability unguaranteed). That gap is Mnemonic's wedge:
*"ERC-8004 proves a hash; Mnemonic proves the bytes are actually retrievable."*

**Forward-shape, don't build yet:** receipt schema reserves room for D2
(recipient ACK, optional `acks[]`) and directed exchange via the A2A bridge.

## Confidence / honesty notes
- Directed-dominates and A2A-shares-no-memory: **high** (multi-angle).
- ERC-8004 hybrid + retrievability-gap: **high** (primary repo).
- "Verifiable cross-operator shared memory is aspirational": **medium**, agent
  self-flagged — this is why V1 leans on the *attestation-broadcast* demand that
  is shipping, not the *shared-memory* demand that isn't.
