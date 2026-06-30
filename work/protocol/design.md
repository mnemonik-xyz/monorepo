---
created: 2026-06-30
updated: 2026-06-30
status: draft-v0
type: protocol-design
---

# Mnemonic Protocol — design v0

## What this is (one sentence)

Mnemonic is a **protocol for self-verifying intent→action objects**: signed,
content-addressed records that anyone can verify with an embeddable library,
with a cheap batched **anchor** as the only shared facility. No nodes, no
consensus, no token.

## Scope & non-goals (owner decision 2026-06-30)

- **Protocol now, business/network later.** Build the format + verifier +
  anchor. Do **not** build nodes, consensus, gossip, incentives, or a token.
- The design must **not foreclose** a future network, but must not build toward
  it. Anything requiring multi-party agreement is out of v0.
- The existing MCP server is a **convenience relay** (sign / store / anchor /
  recall), never a consensus participant.

## Roles

| Role | Does | Holds |
|---|---|---|
| **Principal** | signs an Intent (a typed mandate) | identity key |
| **Agent** | retrieves memory, acts, produces Action + correspondence proof | identity key |
| **Verifier** | checks objects — **anyone** (auditor, counterparty, contract, browser) | nothing; runs the library |
| **Relay** (optional) | stores bytes + serves them by hash; batches anchors | no authority; untrusted |

The Relay has **no power**: it cannot forge, reorder, or censor without
detection, because every object is signed + content-addressed.

## Objects (all signed, all content-addressed by blake3)

- **Intent** — principal-signed typed mandate (AP2-aligned). `intent_hash`.
- **Action** — agent-signed; references `intent_hash` + `knowledge_refs`
  (hashes of memories retrieved at decision time) + the correspondence
  certificate (`proof_kind`, `proof_ref`, public inputs).
- **Bundle** — a batch of objects laid out in order; its Merkle root is what
  gets anchored. One anchor covers thousands of objects.
- **Anchor receipt** — proof that a root existed before time `T` (see Anchoring).

Immutability + content-addressing is the load-bearing property: a bundle with
hash `H` is identical everywhere, so **no two parties ever need to agree on
state** — they each verify `H` locally. This is why there are no nodes.

## The flow

```
Principal ──signs──▶ INTENT ─┐ (intent_hash published / shared out-of-band)
                             │
Agent: recall memory ─▶ build ACTION + collect Evidence + zigz proof π
                             │
   bundle = { intent_ref, action, π, knowledge_refs }  ──signed, blake3─┐
                             │                                          │
                  store bytes (tiered, cheap)            anchor ROOT (batched, cheap)
                             │                                          │
Anyone ◀─ fetch bundle by hash ─ run library verifier ─ accept/reject (5 checks)
```

The five verifier checks (all local, no re-execution, no private witness):
`intent_sig` · `action_sig` · `intent_link` · `correspondence_proof` (zigz) ·
`evidence_proof`. See `work/computation-proof/tech-spec.md`.

## Where the verifier lives: nowhere and everywhere

The verifier is the **pure-Rust + WASM library** (`core/correspondence`). It is a
**function, not a place.** The relying party embeds it — auditor CLI, counterparty
service, browser (wasm), later possibly an on-chain contract. There is **no
"verifier node"** to run or trust. This is the whole point of the ZK choice:
verification is cheap and trustless, so it runs at the edge.

## Anchoring: a pluggable interface, not a chain (owner decision 2026-06-30)

The mistake to avoid is treating any single chain as "the anchor." The protocol
commits to **anchoring a batched Merkle root** (proving existence + time), via a
trait — not to Solana specifically.

```rust
trait Anchor {
    fn anchor(&self, root: [u8;32]) -> Result<AnchorReceipt>;
    fn verify(&self, root: [u8;32], r: &AnchorReceipt) -> Result<AnchoredTime>;
}
```

Backends (pick per deployment / per object):

| Backend | Cost | Latency | Trust | Role |
|---|---|---|---|---|
| **OpenTimestamps → Bitcoin** | **free** (batched into 1 BTC tx) | ~hours | credibly neutral, no token | **default** for a neutral protocol |
| **Solana SPL Memo** (current) | cheap, needs SOL | seconds | one chain's liveness | fast-path / low-latency option |
| **RFC-3161 TSA** (eIDAS) | ~free | instant | centralized CA | regulated / enterprise |
| **none/local** | free | — | none | dev / offline |

**Decision:** default the protocol to **OpenTimestamps→Bitcoin**; **demote Solana
from "the anchor" to one backend**; keep RFC-3161 for enterprises. The current
Solana memo (`v3`/`v5`) becomes a backend behind the trait, not removed.

**Anchor ≠ storage.** The anchor holds only a 32-byte root, never data. One
batched root amortizes the chain cost ~1000× across the objects under it.

**On "calendars" (early instinct, corrected).** The instinct was notarization —
"a timestamp anyone can see." Correct goal, wrong substrate: a calendar is
mutable + centralized, so it cannot *be* the anchor. It may serve as a
**human-facing pointer** that links to an anchored root — never the source of
truth. The cryptographic timestamp lives in OTS/TSA/Solana.

## Storage: retrievability is a verifiable protocol guarantee (owner decision 2026-06-30)

The protocol must guarantee **the payload can be obtained** — not just that it
matches its hash. Two distinct properties:

- **Integrity** — bytes match `H`. Content-addressing gives this for free.
- **Availability** — the bytes can actually be *retrieved*. Content-addressing
  gives this **not at all**. This is the property the protocol must add.

**Why this is load-bearing, not a tier:** in an audit the **producer is the
adversary**. An evidence record the subject can delete or refuse to serve is not
evidence. So the custodian of an auditable payload **cannot be the producer** —
it must be a party that serves it even when the producer doesn't want it served.

**Mechanism:** every anchored object carries a **durability commitment** — a
verifiable declaration of *how* its payload is guaranteed obtainable. The verifier
checks the commitment's **class** meets the relying party's bar.

| Class | Mechanism | Guarantee | For |
|---|---|---|---|
| **D0** self-custody | producer holds bytes | none | private / dev — **invalid for audit** |
| **D1** accountable relays | *k* signed relay receipts ("store `H`, serve on demand") | relay honesty+liveness; failure to serve = provable breach | cheap redundancy, no external network |
| **D2** provable storage | Filecoin deal + Proof-of-Spacetime | cryptographic proof it is *still* stored | renewable, ongoing cost |
| **D3** permanent | Arweave, pay-once (ANS-104 batched) | retrievable "forever" | audit-grade |

**Principles:**

1. **Bind to existing guarantee networks; do not build one.** Arweave/Filecoin
   are *backends* behind a `Store` trait — using them is not running a node
   network, so "protocol now, no nodes" holds.
2. **Bundling answers "Arweave too expensive."** One ANS-104 bundle tx holds
   thousands of individually-addressable objects → D3 at ~$0.0004/bundle
   (measured in `verifiable-trajectories`). Batch, don't abandon.
3. **Minimize what needs the strong class.** The *verifiable core* (Action +
   zigz proof, tens of KB) goes in D2/D3 and must be retrievable; bulky context
   (full memory text, embeddings) may be D0/D1. Pay the strong guarantee only for
   the small thing that proves something.

**Decision:** retrievability is a protocol-level guarantee via durability
commitments. **D0 is invalid for audit objects.** Audit-grade default =
**D3-batched** (Arweave ANS-104 bundle) or **D2** (Filecoin); self-custody is
allowed only for private, non-audit objects. The commitment is part of the object
and is verified.

## What is actually shared / global (kept minimal)

- **A clock** — the anchor (existence + time + order). Hard requirement.
- **Durable custody** — a custodian that isn't the producer, holding the payload
  under a verifiable durability commitment (see Storage). Hard requirement for
  audit objects; provided by existing backends (Arweave/Filecoin) or accountable
  relays — not a new global facility we build.
- **Discovery** (optional) — "how do I find an agent's objects?" A plain index
  service or DHT; **not** consensus. Deferred; can be added without touching the
  object format.
- **Revocation / freshness** (optional) — handled by `expiry` inside the Intent,
  or a small revocation list later. No global mutable state in v0.

## Deferred (explicitly not in v0)

Nodes; consensus; gossip; incentives / staking; a token; a global registry;
on-chain verification (needs a STARK→SNARK wrap — separate later decision).

## Open questions

1. Default anchor = OpenTimestamps→Bitcoin (proposed). Accept, or keep Solana
   default for latency and make OTS opt-in?
2. Intent distribution: how does a verifier obtain the Intent referenced by an
   Action — embedded in the bundle, or fetched by `intent_hash` from a store?
3. Discovery: needed in v0 at all, or purely out-of-band (you hand someone a
   bundle hash) until business phase?
4. Identity: stay with `did:sol` key format, or move to a chain-neutral DID now
   that Solana is demoted to one anchor backend among several?
