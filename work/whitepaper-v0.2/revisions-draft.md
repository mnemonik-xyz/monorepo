# Mnemonic Protocol Whitepaper — Revisions Draft

**Status:** Working draft of revised sections, May 2026
**Source:** Design conversation, May 2026
**Scope:** Revised Abstract; new §5 Architecture with §5.1–§5.6; new §7 Memory Composition and Sharing; notes on existing sections requiring alignment.

---

## How to use this document

This file contains drafted revisions to the Mnemonic whitepaper. Each section below carries a header indicating whether it is **revised**, **new**, or a **note about existing content** that needs alignment.

The revisions follow four discipline rules:

1. The whitepaper describes the protocol in general terms.
2. Implementation details, specific backend names, costs, and numeric parameters live in companion documents (`docs/implementation/*`, `ECONOMICS.md`, ADRs, pricing pages), not in the whitepaper.
3. The whitepaper takes positions on protocol properties, not on operator-specific service offerings.
4. Verification and self-hosting are guaranteed free across all deployments; this is structural, not promotional.

---

## REVISED — Abstract

AI agents accumulate operational context across sessions, tools, and providers — preferences learned over time, factual knowledge extracted from interactions, procedures refined through use, working state maintained across turns, and persistent self-descriptions that shape their behavior. This memory is valuable, but it remains fragile: bound to single providers, locked in proprietary formats, unverifiable by outside parties, and unable to follow the operator that produced it from one runtime to another. As agents become more autonomous and operate across longer time horizons, the absence of a memory layer with cryptographic provenance becomes a coordination problem rather than a convenience problem.

Mnemonic Protocol is a verifiable memory layer for AI agents. It treats memory as a portable, signed artifact — content-addressed, typed, lineage-linked, signed by the operator's cryptographic identity, and independently verifiable by any party that holds it. The protocol distinguishes five kinds of memory artifact — episodic, semantic, procedural, working, and identity — each with its own schema and semantics. Memory belongs to the operator who signed it; it can be shared between runtimes through an explicit handshake mediated by capability tokens and brought into a target runtime through a defined rehydration pipeline that includes safe-injection framing to prevent memory-mediated prompt injection across trust boundaries. Because memory is bound to operator identity rather than to any specific runtime, an agent built up under one model provider can switch providers and continue from the accumulated state.

Mnemonic is independent of where artifacts are stored and how they are anchored. Storage may be local, hosted, or on-chain; anchoring may use any backend that produces a verifiable inclusion proof linking a content-addressed hash to a publicly observable timestamp. Two protocol-level commitments hold across every deployment: verification is free for any party by design, and self-hosting is always available for any operator. These commitments are what make the protocol's trustlessness claim credible — neither the protocol's authors nor any specific operator can gate verification or prevent independent operation. Mnemonic fits underneath agent coordination protocols: A2A makes agents interoperable in motion, the Model Context Protocol makes agents interoperable in capability, and Mnemonic makes them coherent over time. The core thesis is that trustless agents cannot work without trustless agentic memory.

---

## NEW — §5 Architecture Overview

Mnemonic is structured in two layers: a **protocol layer** that defines artifact format, identity, and verification semantics, and a **backend layer** that determines where artifacts physically live. The protocol layer is storage-agnostic. Every guarantee about authorship, integrity, lineage, and authorization holds regardless of which backend is chosen. Backends differ only in availability, latency, cost, and the strength of third-party timestamp claims.

### §5.1 Protocol Layer (storage-independent)

The protocol layer comprises six primitives, none of which assume anything about where bytes are persisted:

- **Canonical encoding** — deterministic CBOR per RFC 8949 §4.2.
- **Content addressing** — blake3 over canonical bytes.
- **Signing envelope** — COSE_Sign1 with Ed25519, producer identity bound via DID (`did:key`, `did:sol`, or other supported methods).
- **Schema registry** — typed, versioned artifact schemas including `memory.episodic`, `memory.semantic`, `memory.procedural`, `memory.working`, `memory.identity`, `rag.context`, `rag.result`, `agent.state`, `receipt`, and `capability.token`.
- **Lineage DAG** — content-addressed parent→child references with cycle detection and directional traversal.
- **Capability tokens** — signed, scoped authorizations over lineage subtrees.

A verifier needs the artifact bytes, the producer's public key, and (optionally) a capability token. It does not need to know which backend served those bytes. This is the core property that makes Mnemonic portable: a Mnemonic artifact verified locally and the same artifact verified after retrieval from on-chain storage return identical integrity and authorship results.

### §5.2 The `StorageBackend` Abstraction

Backends implement a small trait:

```rust
trait StorageBackend {
    async fn put(&self, artifact: &SignedArtifact) -> Result<BackendRef>;
    async fn get(&self, reference: &BackendRef) -> Result<SignedArtifact>;
    async fn list(&self, filter: &Filter) -> Result<Vec<BackendRef>>;
    fn capabilities(&self) -> BackendCapabilities;
}
```

`BackendCapabilities` declares what guarantees a backend provides: `durability`, `third_party_timestamp`, `censorship_resistance`, `random_access_latency`, `cost_per_write_micro_usdc`. Higher layers consult these to route artifacts. Search is abstracted separately (`RecallIndex` trait) so semantic recall does not bind to any specific storage backend.

### §5.3 Backend Implementations

| Backend | Durability | 3rd-party timestamp | Latency | Cost/write | Primary use |
|---|---|---|---|---|---|
| **Local** (SQLite) | Operator only | None | <10 ms | 0 | Development, single-agent, working memory |
| **Cloud** (S3 / object store / Postgres) | Provider SLA | Provider-trusted | 10–100 ms | Low | Production teams, single-org trust boundary |
| **IPFS / Filecoin** | Network-dependent | Weak (DHT) | 100 ms–s | Very low | Public content, opportunistic availability |
| **Arweave** | Permanent (pay-once) | Implicit (block order) | 1–10 s to settle | Low | Long-lived attestations, audit records |
| **On-chain anchor** | Anchors only (paired with durable storage) | Strong | seconds | Low per anchor (batched) | Trustless timestamp for high-value artifacts |
| **Hybrid** | Composite | Composite | Composite | Composite | Default for real deployments |

Most deployments run **hybrid**: local SQLite as the hot path for every artifact, with cloud or on-chain backends layered on for artifacts requiring durability or independent verification. The choice per artifact is governed by policy and capability, not by the protocol.

### §5.4 Why On-Chain Is Optional

On-chain anchoring is the only backend that provides independent third-party timestamp without trusting any single operator. It is therefore valuable, but it is also the most expensive: every on-chain transaction costs something. Mnemonic treats on-chain anchoring as one tool in the kit:

- **Anchor when** an artifact's existence at a specific time must be verifiable by a party that does not trust the operator.
- **Do not anchor when** local or cloud durability is sufficient — the vast majority of working memory, draft state, and intra-session artifacts fall here.
- **Batch anchor when** many low-value artifacts can share a single timestamp (see §5.5).

The protocol's guarantees about authorship and integrity require no chain at all — only signatures and canonical encoding. Chain anchoring adds **third-party timestamp** and **non-repudiation against operator collusion**, which are valuable but separable properties.

### §5.5 Anchoring

Anchoring produces a verifiable claim that an artifact existed at or before a given time, checkable by any party without trusting the artifact's producer or the operator that stored it. Signatures alone give authorship and integrity; anchoring adds independent timestamp and non-repudiation against operator backdating. These are separable properties, and the protocol exposes them separately rather than collapsing them into a single boolean.

Anchoring is the only protocol operation whose marginal cost is bounded below by an external system. The protocol minimizes that cost through batching and remains agnostic to the choice of anchor backend.

#### §5.5.1 Merkle batching via the lineage DAG

The lineage DAG (§5.1) provides what batched anchoring requires. A batch root is a derived artifact whose parents are the artifacts in the batch:

```
BatchRoot = derive(
    schema     = "batch.root",
    parent_ids = [H_1, H_2, ..., H_N]
)
```

Because the batch root's canonical bytes contain its parents' content-addressed ids, and its own id is the hash of those bytes, modifying any leaf breaks the chain up to the root. The lineage DAG functions as a Merkle commitment without additional structure. Only the batch root is anchored; inclusion of any single artifact is proved via the lineage path. Batches compose into trees for logarithmic inclusion proofs at large N. Flat versus tree batching is a deployment parameter, not a protocol commitment.

The amortized per-artifact anchoring cost decreases proportionally with batch size. Operators choose batch parameters according to their latency tolerance for anchor finality.

#### §5.5.2 Backend-agnostic anchor proofs

The protocol accepts any anchor backend that produces a verifiable inclusion proof linking a content-addressed hash to a publicly observable timestamp. The contribution of a backend is captured in an `anchor_proof` record: a backend identifier, a backend-specific reference, the anchored hash, and the timestamp.

Backends differ in finality latency, cost per anchor, trust assumption, and timestamp granularity. The protocol takes no position on the right trade-off across these axes. The reference implementation supports specific backends; their identification and operational parameters are specified in implementation documentation rather than in this protocol document.

#### §5.5.3 Self-anchoring

The protocol distinguishes between *producing* an anchor and *accepting* an anchor proof. Any operator can submit an anchor transaction independently of any hosted service and present the resulting proof to the protocol; verification uses the same logic regardless of who produced the anchor. This is the structural property that keeps the protocol open — hosted anchoring is convenience, not a gate.

#### §5.5.4 Verification states

Anchoring is asynchronous. The protocol defines explicit states reflecting whether an artifact has settled, is in flight, or remains unanchored: **signed but not anchored**, **anchor pending**, **anchored**, **anchor failed**. A verifier always knows which state an artifact is in. The protocol never claims an anchor exists when it does not, and never claims an absence of anchor when one is in flight.

#### §5.5.5 What anchoring does and does not guarantee

Anchoring adds existence at or before a known time, tamper-evidence for all artifacts under the anchored root, and non-repudiation against operator backdating. Anchoring does not provide content availability, authorization to read, correctness of the artifact's claims, or sub-settlement-time finality. Signed-but-unanchored artifacts remain fully valid for authorship and integrity. Anchoring upgrades verifiability against operator collusion and adds a public timestamp.

### §5.6 Protocol Economics

The protocol takes positions on what must remain free and what may be paid, but takes no position on how any particular operator prices the services they offer. The result is an economic model in which the protocol itself is unmonetizable while the services built on top of it are freely monetizable. This separation is intentional: it is what allows the protocol to remain a public good even when individual operators are commercial.

#### §5.6.1 Operations the protocol guarantees are free

Two operations are free by protocol design and cannot be gated by any operator:

- **Verification.** Any party may verify the authorship, integrity, lineage, and anchoring of any artifact they hold. Verification requires no operator service, no account, and no permission. This is the foundation of the protocol's trustlessness claim — if verification can be gated, the protocol's guarantees become contingent on whoever holds the gate.
- **Self-hosting.** Any operator may run a complete node, sign and verify locally, and participate in the protocol without paying any other operator. The full set of protocol operations is available to any operator running their own implementation.

These two guarantees are structural protections against capture. They mean that no party — including the protocol's original authors — can hold the protocol's core functionality hostage.

#### §5.6.2 Operations operators may charge for

Operations whose cost is bounded below by real compute, storage, bandwidth, or external-system fees may be charged for by the operator providing them. These are service-layer choices and the protocol takes no position on whether they are priced, subsidized, or free at any particular operator:

- Producing anchors against backends that have non-zero cost.
- Hosting durable storage on behalf of users.
- Running embedding, recall, or query workloads at scale.
- Operating high-availability or dedicated infrastructure.
- Providing managed identity, capability, or audit services.

Charging is a feature of operator deployment, not of the protocol. An operator may choose to offer any of these for free, at cost, or at a margin. Users uncomfortable with one operator's pricing are free to use another, run their own, or operate without that service entirely.

#### §5.6.3 Operator pluralism

The protocol does not privilege any operator. There is no canonical hosted service, no preferred node, no central registry of authorized operators. Any party may run a Mnemonic node, accept artifacts from other operators, anchor on behalf of users, and charge for any of these services. Verification of an artifact is independent of which operator produced it.

This pluralism is what makes the free-tier guarantees credible. An operator that abuses its position — by raising prices, degrading service, or restricting access — does not control the protocol; users can migrate to other operators or to self-hosting without losing accumulated state, because all artifacts are portable by signature and verifiable independently.

#### §5.6.4 Payment as a protocol primitive

The protocol provides primitives for operators that wish to charge for services: payment-gated tool calls, balance-tracking accounts, and standard micropayment patterns over established settlement rails. These primitives are tools, not mandates. An operator that wishes to provide free service simply does not enable payment gating; an operator that wishes to charge enables it. The same protocol code supports both modes.

Operators that charge are responsible for the economics of their own services: pricing, free quotas, subsidies, revenue allocation, and treasury management. These are commercial decisions of individual operators and are not specified by the protocol.

#### §5.6.5 What this economic model rules out

The combination of free verification, free self-hosting, operator pluralism, and optional payment rules out several common failure modes of protocol economics:

- **Verification capture.** No operator can charge for the act of checking whether an artifact is genuine.
- **Publishing capture.** No operator can prevent a user from signing and storing artifacts; the user can always run their own node.
- **Lock-in.** Artifacts produced under one operator are verifiable by any other operator and by self-hosted nodes. There is no operator-specific format or operator-specific verification path.
- **Single-operator dependency for protocol liveness.** Even if a particular hosted operator becomes unavailable, the protocol continues to function across all other operators and self-hosters.

The protocol does not rule out commercial activity. It rules out commercial activity that depends on holding the protocol itself hostage.

#### §5.6.6 Implementation-level economics

Specific pricing, free-tier quotas, subsidy models, treasury accounting, and revenue strategy are operator-level concerns documented separately from the protocol specification. Different implementations and different hosted operators will make different choices. The protocol document specifies only what those choices must respect: verification stays free, self-hosting stays available, and no operator becomes structurally privileged over others.

### §5.7 Pipeline Walkthrough

*Note: The existing §5.3 "Pipeline Walkthrough" content from the v0.1 whitepaper moves here unchanged. It already describes the sign / recall / verify flow correctly across local and full modes and does not require rewriting.*

---

## NEW — §7 Memory Composition and Sharing

*(Existing §7 "Trust Model" becomes §8; subsequent sections shift down by one.)*

Memory is not a homogeneous blob. An agent's working state, its accumulated facts about the world, the procedures it has learned, the events it has witnessed, and its persistent self-description all play different cognitive roles and warrant different lifecycle, retrieval, and access semantics. The protocol exposes this structure directly through typed memory artifacts, and provides primitives for authorizing and transferring memory across the runtimes that operate on it.

This section describes those primitives. It does not specify retrieval algorithms, scoring functions, or implementation parameters — only the protocol contracts that any compliant implementation must respect.

### §7.1 Memory composition

The protocol distinguishes five kinds of memory artifact, each with its own schema in the registry:

- **Episodic memory** records time-ordered events, observations, and interactions. It captures what happened.
- **Semantic memory** records factual assertions about the world, typically as structured claims with confidence scores. It captures what is believed to be true.
- **Procedural memory** records learned skills, routines, and workflows. It captures how to do things.
- **Working memory** records transient goals, subgoals, scratch state, and pending actions. It captures what is currently being attempted.
- **Identity memory** records persistent persona attributes, preferences, communication style, and operational policies. It captures who the operator is.

These distinctions are not merely organizational. Each kind has different retention semantics, different retrieval characteristics, different sharing implications, and different requirements for safe injection into a target runtime. A protocol that treats memory as a homogeneous store cannot make these distinctions; a protocol that recognizes them can route, scope, and transfer each kind appropriately.

The five kinds compose. A semantic fact may be derived from an episodic observation; a procedural skill may reference identity preferences; working state may draw on all four others. The lineage DAG (§5.1) captures these derivation relationships across kinds.

### §7.2 Capability tokens

Memory is owned by the identity that signed it. By default, no other party may read it. Sharing requires explicit authorization, expressed as a capability token: a signed artifact whose schema declares it as a permission grant rather than a memory entry.

A capability token carries four pieces of information:

- **Scope** — which artifacts the token applies to, expressed in terms of the lineage DAG (a specific artifact, a subtree under a specific root, all artifacts of a given kind, or all artifacts matching a tag predicate).
- **Permissions** — which operations the token authorizes (read, derive, redact, export, rehydrate, or operations specific to the schema being authorized).
- **Audience** — the identity authorized by the token, expressed as a DID. A token bound to a specific audience cannot be reused by another party.
- **Expiry** — the time after which the token is no longer valid.

Tokens are signed by the issuing identity using the same envelope used for all other signed artifacts (§5.1), and are verified by the same machinery. There is no separate token format and no separate verifier.

Tokens are themselves artifacts. They have content-addressed ids, they appear in the lineage DAG, they may be anchored, and they may be revoked by issuing a successor token that explicitly invalidates them. A holder of a token can verify the token's authenticity and current validity using only the protocol's standard verification primitives.

### §7.3 The sharing handshake

A party wishing to access memory held by another operator does not have direct access to that memory; the protocol mediates the transfer through a handshake whose contract is the following:

1. The requester obtains the content-addressed id of the desired artifact and the owner's identity through any means external to the protocol. The protocol does not specify a discovery mechanism in v1.
2. The requester transmits to the owner their public key, a signed challenge proving control of the corresponding private key, and any capability token they hold authorizing the request.
3. The owner verifies the challenge signature, evaluates the capability token against the requested artifact, and applies any additional policy.
4. If the request is authorized, the owner serializes the artifact along with the lineage ancestors required for its verification, compresses the result, encrypts to the requester's public key using established key-exchange primitives, and transmits the resulting bundle.
5. The requester decrypts, decompresses, and verifies the artifact through the standard verification pipeline.

The protocol specifies the handshake contract but not the transport. Memory may be transferred over any channel capable of carrying an encrypted byte string. The contract is enforced by what the parties produce and verify, not by where the bytes travel.

### §7.4 Rehydration

A verified artifact is not yet useful to a target runtime. To act on memory, the runtime requires the memory rendered into its active context, in a form that respects its token budget, instruction hierarchy, and safety constraints. The protocol calls this transformation rehydration.

Rehydration is a pipeline of stages, each of which any compliant implementation must perform:

1. **Verify** — confirm authorship, integrity, lineage, and (where applicable) anchoring of the artifact and its transitive ancestors.
2. **Filter** — exclude artifacts not authorized by the capability tokens presented for this rehydration.
3. **Rank** — order remaining artifacts by relevance to the current task. The ranking function is implementation-defined; the protocol requires only that its inputs include the artifact, the task context, and the artifact's metadata.
4. **Compress** — fit the ranked set into the target runtime's context budget, preserving high-ranked content verbatim and condensing lower-ranked content according to the implementation's strategy.
5. **Format** — render the compressed content in a form appropriate for the target runtime's conventions.
6. **Frame** — wrap the formatted content in structural markers that the target runtime treats as data rather than as instructions (§7.5).
7. **Inject** — place the framed content into the target runtime's context in a position consistent with the runtime's instruction hierarchy.

Each stage has defined inputs, outputs, and failure semantics. An implementation may parameterize any stage but may not skip stages or reorder them; the contract exists because each stage protects properties that downstream stages rely on.

### §7.5 Safe injection

Memory transferred from one runtime to another can carry, intentionally or otherwise, content that resembles instructions. A naive injection that places transferred memory directly into a runtime's input stream creates an attack surface in which the source memory can hijack the target runtime's behavior.

The protocol requires that all rehydrated memory be wrapped in typed boundary markers and accompanied by an explicit directive identifying the content as data rather than as instructions. Content within those markers is escaped at the boundary level to prevent boundary breakout, at the role-marker level to prevent role hijacking, and at the instruction-pattern level to neutralize known injection vectors. Content that does not match the schema declared for its boundary is quarantined and excluded from the framed output.

The specific framing syntax is implementation-defined; the requirement that framing be present and enforced is protocol-defined. An implementation that omits framing does not produce protocol-compliant rehydration output.

### §7.6 Portability model

Memory belongs to the identity that signed it. That identity may operate any number of runtimes — different model providers, different agent frameworks, different physical machines — and the protocol guarantees that memory signed under that identity remains verifiable, transferable, and rehydratable across all of them without re-signing or re-attesting prior records.

This is the protocol's portable-memory property: an operator who builds up memory under one runtime can switch runtimes and continue from the accumulated state, because the runtime is a consumer of the operator's memory, not its owner.

The portability model in v1 covers a single owning identity operating across multiple runtimes. Multi-operator scenarios, in which different owning identities share memory across organizational boundaries, are a strict extension of this model: the same primitives — typed artifacts, capability tokens, the handshake, rehydration — apply, but with additional discovery, registry, and cross-identity trust concerns that v1 leaves out of scope.

---

## Existing sections requiring alignment edits

The following existing whitepaper sections were not redrafted in this conversation. They contain framing that does not match the new structure and need light edits before the revised whitepaper is internally consistent.

### §1 Introduction

Currently references "local for development, decentralized for verification" as the storage model. Should be aligned with the storage-agnostic framing of the new §5: storage is one of several pluggable backends, not a binary mode.

### §2 Problem Statement

Currently emphasizes the technical fragility of agent memory but does not surface the cognitive structure of memory or the sharing problem. Should be expanded slightly to motivate the §7 primitives (cognitive typing, capabilities, rehydration, safe injection).

### §3 Design Goals

The "Current Implementation Goals" and "Protocol Roadmap Goals" subsections list cognitive memory structure, capability tokens, and rehydration as future work. Several of these are now core protocol primitives per §7 and should move to the current-goals list. The list of cryptographic primitives in 3.1 is implementation detail and should be trimmed.

### §4 Core Insight

Currently describes the protocol pipeline as `embed → compress → canonicalize → hash → sign → persist → recall → verify`. This is the *sign* pipeline. The full protocol contract now also includes the *share/rehydrate* pipeline introduced in §7. The core insight section should mention both flows.

### §6 Artifact Model

Currently lists schemas (`memory`, `rag.context`, `rag.result`, `agent.state`, `receipt`). Should be updated to reflect the cognitive memory taxonomy from §7.1 (`memory.episodic`, `memory.semantic`, `memory.procedural`, `memory.working`, `memory.identity`) and `capability.token`. The `memory` schema becomes a deprecated alias or is replaced by the five typed kinds.

### §8 Trust Model (was §7)

Should be cross-referenced with §5.6 (Protocol Economics) commitments. The "Mnemonic does not yet guarantee" list should be updated to remove items now addressed by §7 (capability-scoped access, multi-runtime portability) and to keep items genuinely out of scope (multi-party shared namespaces, ZK proofs).

### §9 Use Cases (was §9)

Several use cases — Shared Project Memory Namespace (§9.1), Portable Memory Wallet (§9.5), Agent Continuity Layer (§9.9) — now read as deployments of §7 primitives rather than as standalone features. They should be tightened to reference §7 and §5 rather than restate primitives.

### §11 Current Implementation Status (was §11)

The "Implemented today" list is accurate per the May 2026 monorepo state. The "Not current implementation behavior" list should be updated to include capability tokens, the rehydration pipeline, safe-injection framing, and the sharing handshake as not-yet-implemented protocol primitives. This keeps the gap between protocol specification and current implementation honest.

---

## Companion documents this whitepaper references

The whitepaper is intentionally agnostic about implementation-level details. Those details should live in companion documents in the repository:

| Document | Contents |
|---|---|
| `docs/implementation/anchoring.md` | Specific anchor backends supported, their cost characteristics, latency, and trust assumptions |
| `docs/implementation/schemas/` | Concrete field definitions for each schema in the registry, including the five memory kinds and capability tokens |
| `docs/implementation/framing.md` | Concrete framing syntax for safe injection, escape rules, and quarantine semantics |
| `docs/implementation/crypto.md` | Specific cryptographic primitives, including key-exchange derivation for the sharing handshake |
| `ECONOMICS.md` | Specific pricing, free-tier quotas, subsidy plan, treasury accounting (operator-specific, not protocol-specific) |
| `docs/adr/` | Architecture decision records covering specific implementation choices (default anchor backend, etc.) |
| `docs/operator-guide.md` | Deployment guidance, including batch size selection, backend configuration, and operator-level economic choices |

---

*End of revisions draft. Suggested next step: review this document, integrate the new sections into the existing whitepaper file, perform the alignment edits listed above on the existing sections, and commit as a v0.2 whitepaper.*

---

## Provenance

- **Attestation ID:** `1ee91ba4-6e1e-4ea1-9843-af7885139177`
- **Content hash (blake3 of canonical CBOR):** `33cac0e59ddf021df6bb682440a20af314c8b1a31a71f6e9f6a5200209203e23`
- **Solana tx:** `3trZxfswfFRVCoXzHUHwETsewSt6CZ5BP3NjcCPXHXwU8kQ297yKq3VvCJDDw8ocVpw9v4L2R2q2Ad6W9VWimBza`
- **Arweave tx:** `CubJzDPLBaLF7fo67KB9RWXgex1QV8RsaWPWWkEaLoT3`
- **Producer:** `2jdrmxfLqiCtRNL1u5ZdLaKKdq6Wg8iPqvJSohQhLq4x`
- **Signed at:** 2026-05-16T04:07:03Z
