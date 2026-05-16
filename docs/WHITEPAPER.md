# Mnemonic Protocol: Verifiable Memory Infrastructure for AI Agents

**Draft:** v0.2  
**Date:** May 2026  
**Status:** Working draft  

---

## Abstract

AI agents accumulate operational context across sessions, tools, and providers — preferences learned over time, factual knowledge extracted from interactions, procedures refined through use, working state maintained across turns, and persistent self-descriptions that shape their behavior. This memory is valuable, but it remains fragile: bound to single providers, locked in proprietary formats, unverifiable by outside parties, and unable to follow the operator that produced it from one runtime to another. As agents become more autonomous and operate across longer time horizons, the absence of a memory layer with cryptographic provenance becomes a coordination problem rather than a convenience problem.

Mnemonic Protocol is a verifiable memory layer for AI agents. It treats memory as a portable, signed artifact — content-addressed, typed, lineage-linked, signed by the operator's cryptographic identity, and independently verifiable by any party that holds it. The protocol distinguishes five kinds of memory artifact — episodic, semantic, procedural, working, and identity — each with its own schema and semantics. Memory belongs to the operator who signed it; it can be shared between runtimes through an explicit handshake mediated by capability tokens and brought into a target runtime through a defined rehydration pipeline that includes safe-injection framing to prevent memory-mediated prompt injection across trust boundaries. Because memory is bound to operator identity rather than to any specific runtime, an agent built up under one model provider can switch providers and continue from the accumulated state.

Mnemonic is independent of where artifacts are stored and how they are anchored. Storage may be local, hosted, or on-chain; anchoring may use any backend that produces a verifiable inclusion proof linking a content-addressed hash to a publicly observable timestamp. Two protocol-level commitments hold across every deployment: verification is free for any party by design, and self-hosting is always available for any operator. These commitments are what make the protocol's trustlessness claim credible — neither the protocol's authors nor any specific operator can gate verification or prevent independent operation. Mnemonic fits underneath agent coordination protocols: A2A makes agents interoperable in motion, the Model Context Protocol makes agents interoperable in capability, and Mnemonic makes them coherent over time. The core thesis is that trustless agents cannot work without trustless agentic memory.

---

## 1. Introduction

AI agents forget. Their working context disappears when a session ends, when a provider restarts, when a model changes, or when a workflow moves between tools. Even when a system stores memory, that memory is usually controlled by a single provider, shaped by a private database, and unverifiable by outside parties.

For simple assistants, this is inconvenient. For agents that produce research, compliance artifacts, security findings, financial decisions, or operational plans, it is a trust problem. The question is not only whether an agent can remember, but whether anyone can verify what the agent remembered, when it remembered it, who wrote it, and whether the record was later modified.

The common instinct is to make context windows longer or to snapshot more internal model state. That helps with continuity inside one runtime, but it does not solve portability or auditability. Raw attention state is model-specific, opaque, expensive to move, and hard for independent systems to interpret. A proprietary chat history is more readable, but still belongs to the platform that stores it.

Mnemonic starts from a different unit: the typed memory artifact. A memory artifact is human-readable content with a declared cognitive role — episodic, semantic, procedural, working, or identity (see §7.1) — linked to embeddings, metadata, cryptographic identity, and a verifiable commitment trail. It can be recalled by meaning, inspected by people, signed by agents, and independently checked by other systems.

Memory belongs to the operator who signed it, not to the runtime that produced it. The same artifacts are valid across model providers, agent frameworks, and physical machines: an operator that built up memory under one runtime can switch runtimes and continue from the accumulated state without re-signing prior records.

Where those artifacts physically live is a separate concern from the protocol's guarantees. Storage may be local, hosted, on-chain, or any combination — backends are pluggable, not a binary "fast local vs. trustworthy decentralized" choice. Authorship and integrity hold regardless of backend; the backend layer adds availability, latency, and the strength of third-party timestamp claims (see §5).

This shift matters for multi-agent systems. A2A protocols can move messages and tasks between agents, but they do not by themselves provide durable, portable, attestable memory. Mnemonic fits underneath coordination protocols as memory infrastructure: the substrate that lets agents remain coherent over time.

## 2. Problem Statement

Current agent memory systems typically fall into three categories:

1. **Context windows** are fast and accurate but temporary.
2. **Application-native memory** can persist but is locked to one product or provider.
3. **External vector stores** support retrieval but usually do not prove provenance, integrity, ordering, or non-repudiation.

These systems are useful, but they do not provide a portable trust layer. If an agent moves from one runtime to another, its accumulated state may not move with it. If a memory record changes, consumers may not notice. If an agent produces an answer based on prior context, downstream systems may not be able to audit which context existed at the time.

Two further problems compound the trust gap and are not addressed by any of the three categories above.

**Memory has cognitive structure that flat stores erase.** An agent's working state, its accumulated facts about the world, the procedures it has learned, the events it has witnessed, and its persistent self-description play different cognitive roles and warrant different retention, retrieval, and sharing semantics. Systems that treat memory as a homogeneous bag of strings cannot apply per-kind lifecycle, cannot scope access by kind, and cannot make different safety decisions about, for example, transient working state versus durable identity. The cognitive distinctions exist whether or not the storage layer recognizes them; surfacing them in the protocol is what allows correct downstream handling.

**Memory crosses trust boundaries, and the transfer itself is unsolved.** When one agent or runtime hands memory to another, three problems appear at once. Authorization: the receiver should only see what they are entitled to see, and the entitlement should be expressible, transferable, and revocable without trusting a central authority. Verifiable transit: the receiver needs to confirm that what arrived is exactly what the owner signed, that lineage holds, and that any timestamp claims survive the hop. Injection safety: memory content can resemble instructions, and a naive paste of received memory into a target runtime's context creates a memory-mediated prompt injection surface. None of the three current categories provide primitives for any of these.

A verifiable agent memory system needs the following properties:

- **Persistence:** memory survives sessions, providers, and runtime changes.
- **Semantic recall:** memory can be retrieved by meaning, not only by keyword.
- **Provenance:** each memory is linked to a cryptographic identity.
- **Integrity:** tampering can be detected independently.
- **Portability:** memory is not trapped inside one vendor or framework.
- **Cognitive typing:** memory is distinguishable by role (episodic, semantic, procedural, working, identity) so per-kind semantics can apply.
- **Capability-scoped sharing:** access is authorized by signed, scoped, revocable grants — not by ambient trust in a central operator.
- **Safe injection across runtimes:** transferred memory enters target runtimes through a defined pipeline that prevents memory-mediated prompt injection.
- **Economic viability:** storage and verification costs remain low enough for real agent workflows, and verification in particular remains free by design.

## 3. Protocol Contract

This section enumerates the protocol-level invariants that any compliant Mnemonic deployment must provide. They are protocol commitments, not implementation choices: they hold whether the backend is local, hosted, on-chain, or hybrid, and whether any specific operator is running or not. Concrete implementation status is in §12; specific backend choices, parameters, and costs are in companion documents.

- **Typed, signed, content-addressed artifacts.** Memory is encoded deterministically, hashed by content, and signed by the operator's cryptographic identity (see §5.1).
- **Cognitive typing.** Memory artifacts declare a kind — episodic, semantic, procedural, working, or identity — and per-kind semantics apply downstream (see §7.1).
- **Lineage as a first-class structure.** Parent–child relationships across artifacts are content-addressed, verifiable, and traversable, supporting provenance audits and Merkle-batched anchoring (see §5.6.1).
- **Storage-agnostic protocol layer.** Authorship, integrity, lineage, and authorization hold regardless of which backend stores the bytes (see §5.3).
- **Anchoring as a separable property.** Third-party timestamp is an opt-in addition over signature-based authorship and integrity, not a baseline requirement (see §5.6).
- **Capability-scoped sharing.** Cross-runtime access to memory is authorized by signed, scoped, revocable capability tokens (see §7.2).
- **Safe rehydration across runtimes.** Memory entering a target runtime traverses a defined verify → filter → rank → compress → format → frame → inject pipeline that prevents memory-mediated prompt injection (see §7.4, §7.5).
- **Free verification.** Any party may verify any artifact they hold, with no operator gate (see §5.7.1).
- **Free self-hosting.** Any operator may run a complete node and participate in the protocol without paying any other operator (see §5.7.1).
- **Operator pluralism.** No operator is structurally privileged; verification is independent of which operator produced or stored the artifact (see §5.7.3).

Forward-looking work — what extends this contract beyond v1 — is consolidated in §15 Roadmap.

## 4. Core Insight

Agent memory should be semantically meaningful, cryptographically attributable, portable across runtimes, and operationally cheap.

Raw transformer state is the wrong abstraction for portable memory. Attention caches are model-specific, opaque, large, and difficult for humans or independent systems to interpret. Typed memory artifacts are smaller, inspectable, portable, and compatible with retrieval systems.

Mnemonic applies this principle through two pipelines that share the same protocol primitives: one for producing memory, one for transferring it across a trust boundary into another runtime.

The **sign pipeline** turns content into a verifiable artifact:

```text
content
  -> embed
  -> compress
  -> build typed artifact (declared cognitive kind)
  -> canonicalize to CBOR
  -> hash canonical bytes with blake3
  -> sign with Ed25519 as COSE_Sign1
  -> persist to one or more backends
  -> recall by meaning
  -> verify against producer, lineage, and (optionally) anchor
```

The **share / rehydrate pipeline** moves a signed artifact from one runtime to another:

```text
signed artifact + capability token
  -> sharing handshake (authenticate, scope-check, encrypted transit)
  -> verify (authorship, integrity, lineage, anchor)
  -> filter (per capability scope)
  -> rank, compress, format
  -> frame (safe-injection markers)
  -> inject into target runtime context
```

The two pipelines compose: an artifact signed in one runtime is the input to a share/rehydrate flow that hands it to another, and both flows verify the same canonical bytes against the same producer identity. This composition is what gives the protocol portable memory as a property rather than as an aspiration.

Compression of embeddings serves portability and durable-storage anchoring, not the local recall path: shrinking embeddings keeps artifact metadata cheap to carry across systems and to anchor. The specific compression scheme, bit width, and recall implementation are documented separately as implementation choices.

## 5. Architecture Overview

Mnemonic is structured in two horizontal layers and a vertical taxonomy of surfaces over them. The two layers are a **protocol layer** that defines artifact format, identity, and verification semantics, and a **backend layer** that determines where artifacts physically live. The protocol layer is storage-agnostic: every guarantee about authorship, integrity, lineage, and authorization holds regardless of which backend is chosen. Backends differ in availability, latency, cost, and the strength of third-party timestamp claims. Surfaces (§5.2) are the consumer-facing entry points into the protocol layer; any surface can drive any backend, so storage is a user choice per artifact, not a property of the surface or the protocol.

### 5.1 Protocol Layer (storage-independent)

The protocol layer comprises six primitives, none of which assume anything about where bytes are persisted:

- **Canonical encoding** — deterministic CBOR per RFC 8949 §4.2.
- **Content addressing** — blake3 over canonical bytes.
- **Signing envelope** — COSE_Sign1 with Ed25519, producer identity bound via DID (`did:key`, `did:sol`, or other supported methods).
- **Schema registry** — typed, versioned artifact schemas including `memory.episodic`, `memory.semantic`, `memory.procedural`, `memory.working`, `memory.identity`, `rag.context`, `rag.result`, `agent.state`, `receipt`, and `capability.token`.
- **Lineage DAG** — content-addressed parent→child references with cycle detection and directional traversal.
- **Capability tokens** — signed, scoped authorizations over lineage subtrees.

A verifier needs the artifact bytes, the producer's public key, and (optionally) a capability token. It does not need to know which backend served those bytes. This is the core property that makes Mnemonic portable: an artifact verified locally and the same artifact verified after retrieval from on-chain storage return identical integrity and authorship results.

### 5.2 Consumer Surfaces

The protocol layer is exposed through several surfaces, each suited to a different runtime context. All surfaces consume the same protocol primitives (§5.1) and target the same `StorageBackend` abstraction (§5.3), so any surface can drive any backend the user configures.

- **`core`** — the protocol library, shipped as both a native Rust crate and a WebAssembly module. Higher surfaces are built on `core`; it has no opinion on storage or transport.
- **`cli`** — command-line client for terminals and scripts. Local-first by default; the user may configure cloud or on-chain backends per artifact.
- **`sdk`** — embeddable library for applications. Self-host capable: an SDK consumer may produce its own on-chain anchor proofs directly, without routing through any hosted service.
- **`mcp`** — networked MCP server with HTTP and stdio transports. The default surface for agent runtimes that speak MCP; also exposes the operator payment surface for hosted services (§5.7).
- **`browser-extension`** — in-browser client built on the WASM build of `core`. Fully functional in the browser: local, cloud, and on-chain backends are all reachable.

Which surface to use is an ergonomic choice driven by the embedding context. Which backend to target is an independent choice made by the user per artifact, driven by durability, trust, and cost requirements. The protocol layer constrains neither, and no surface restricts which backends are reachable.

### 5.3 The `StorageBackend` Abstraction

Backends implement a small trait:

```rust
trait StorageBackend {
    async fn put(&self, artifact: &SignedArtifact) -> Result<BackendRef>;
    async fn get(&self, reference: &BackendRef) -> Result<SignedArtifact>;
    async fn list(&self, filter: &Filter) -> Result<Vec<BackendRef>>;
    fn capabilities(&self) -> BackendCapabilities;
}
```

`BackendCapabilities` declares what guarantees a backend provides: `durability`, `third_party_timestamp`, `censorship_resistance`, `random_access_latency`, `cost_per_write_micro_usdc`. Higher surfaces consult these to surface trade-offs to the user. Search is abstracted separately (`RecallIndex` trait) so semantic recall does not bind to any specific storage backend.

### 5.4 Backend Implementations

| Backend | Durability | 3rd-party timestamp | Latency | Cost/write | Primary use |
|---|---|---|---|---|---|
| **Local** (e.g. SQLite) | Operator only | None | <10 ms | 0 | Development, single-agent, working memory |
| **Cloud** (e.g. object store / managed DB) | Provider SLA | Provider-trusted | 10–100 ms | Low | Production teams, single-org trust boundary |
| **Content-addressed P2P** (e.g. IPFS/Filecoin) | Network-dependent | Weak (DHT) | 100 ms–s | Very low | Public content, opportunistic availability |
| **Permanent storage network** (e.g. Arweave) | Permanent (pay-once) | Implicit (block order) | 1–10 s to settle | Low | Long-lived attestations, audit records |
| **On-chain anchor** | Anchors only (paired with durable storage) | Strong | seconds | Low per anchor (batched) | Trustless timestamp for high-value artifacts |
| **Hybrid** | Composite | Composite | Composite | Composite | Common configuration in practice |

The table gives the protocol-relevant categories; specific backend identifiers and operational parameters are tracked in implementation documentation. Storage selection happens per artifact: the user (or the application configured by the user) picks one or more backends from the available categories based on the artifact's durability, trust, and cost requirements. Many deployments combine a local index for hot access with one or more durable backends for long-lived or third-party-verifiable artifacts.

### 5.5 Why On-Chain Is Optional

On-chain anchoring is the only backend category that provides independent third-party timestamp without trusting any single operator. It is therefore valuable, but it is also the most expensive: every on-chain transaction costs something. Mnemonic treats on-chain anchoring as one tool among several:

- **Anchor when** an artifact's existence at a specific time must be verifiable by a party that does not trust the operator.
- **Do not anchor when** local or cloud durability is sufficient — the vast majority of working memory, draft state, and intra-session artifacts fall here.
- **Batch anchor when** many low-value artifacts can share a single timestamp (see §5.6).

The protocol's guarantees about authorship and integrity require no chain at all — only signatures and canonical encoding. Chain anchoring adds **third-party timestamp** and **non-repudiation against operator collusion**, which are valuable but separable properties.

### 5.6 Anchoring

Anchoring produces a verifiable claim that an artifact existed at or before a given time, checkable by any party without trusting the artifact's producer or the operator that stored it. Signatures alone give authorship and integrity; anchoring adds independent timestamp and non-repudiation against operator backdating. These are separable properties, and the protocol exposes them separately rather than collapsing them into a single boolean.

Anchoring is the only protocol operation whose marginal cost is bounded below by an external system. The protocol minimizes that cost through batching and remains agnostic to the choice of anchor backend.

#### 5.6.1 Merkle batching via the lineage DAG

The lineage DAG (§5.1) provides what batched anchoring requires. A batch root is a derived artifact whose parents are the artifacts in the batch:

```
BatchRoot = derive(
    schema     = "batch.root",
    parent_ids = [H_1, H_2, ..., H_N]
)
```

Because the batch root's canonical bytes contain its parents' content-addressed ids, and its own id is the hash of those bytes, modifying any leaf breaks the chain up to the root. The lineage DAG functions as a Merkle commitment without additional structure. Only the batch root is anchored; inclusion of any single artifact is proved via the lineage path. Batches compose into trees for logarithmic inclusion proofs at large N. Flat versus tree batching is a deployment parameter, not a protocol commitment.

The amortized per-artifact anchoring cost decreases proportionally with batch size. The user chooses batch parameters per their latency tolerance for anchor finality.

#### 5.6.2 Backend-agnostic anchor proofs

The protocol accepts any anchor backend that produces a verifiable inclusion proof linking a content-addressed hash to a publicly observable timestamp. The contribution of a backend is captured in an `anchor_proof` record: a backend identifier, a backend-specific reference, the anchored hash, and the timestamp.

Backends differ in finality latency, cost per anchor, trust assumption, and timestamp granularity. The protocol takes no position on the right trade-off across these axes. The reference implementation supports specific backends; their identification and operational parameters are specified in implementation documentation rather than in this protocol document.

#### 5.6.3 Self-anchoring

The protocol distinguishes between *producing* an anchor and *accepting* an anchor proof. Any user can submit an anchor transaction independently of any hosted service and present the resulting proof to the protocol; verification uses the same logic regardless of who produced the anchor. This is the structural property that keeps the protocol open — hosted anchoring is convenience, not a gate. The `sdk` and `browser-extension` surfaces both support direct self-anchoring; `mcp` additionally supports anchoring on behalf of users that prefer a hosted path.

#### 5.6.4 Verification states

Anchoring is asynchronous. The protocol defines explicit states reflecting whether an artifact has settled, is in flight, or remains unanchored: **signed but not anchored**, **anchor pending**, **anchored**, **anchor failed**. A verifier always knows which state an artifact is in. The protocol never claims an anchor exists when it does not, and never claims an absence of anchor when one is in flight.

#### 5.6.5 What anchoring does and does not guarantee

Anchoring adds existence at or before a known time, tamper-evidence for all artifacts under the anchored root, and non-repudiation against operator backdating. Anchoring does not provide content availability, authorization to read, correctness of the artifact's claims, or sub-settlement-time finality. Signed-but-unanchored artifacts remain fully valid for authorship and integrity. Anchoring upgrades verifiability against operator collusion and adds a public timestamp.

### 5.7 Protocol Economics

The protocol takes positions on what must remain free and what may be paid, but takes no position on how any particular operator prices the services they offer. The result is an economic model in which the protocol itself is unmonetizable while the services built on top of it are freely monetizable. This separation is intentional: it is what allows the protocol to remain a public good even when individual operators are commercial.

#### 5.7.1 Operations the protocol guarantees are free

Two operations are free by protocol design and cannot be gated by any operator:

- **Verification.** Any party may verify the authorship, integrity, lineage, and anchoring of any artifact they hold. Verification requires no operator service, no account, and no permission. This is the foundation of the protocol's trustlessness claim — if verification can be gated, the protocol's guarantees become contingent on whoever holds the gate.
- **Self-hosting.** Any user may run a complete node, sign and verify locally, and participate in the protocol without paying any other operator. The full set of protocol operations is available to any user running their own implementation across the `cli`, `sdk`, and `browser-extension` surfaces.

These two guarantees are structural protections against capture. They mean that no party — including the protocol's original authors — can hold the protocol's core functionality hostage.

#### 5.7.2 Operations operators may charge for

Operations whose cost is bounded below by real compute, storage, bandwidth, or external-system fees may be charged for by the operator providing them. These are service-layer choices and the protocol takes no position on whether they are priced, subsidized, or free at any particular operator:

- Producing anchors against backends that have non-zero cost.
- Hosting durable storage on behalf of users.
- Running embedding, recall, or query workloads at scale.
- Operating high-availability or dedicated infrastructure.
- Providing managed identity, capability, or audit services.

Charging is a feature of operator deployment, not of the protocol. An operator may choose to offer any of these for free, at cost, or at a margin. Users uncomfortable with one operator's pricing are free to use another, run their own, or operate without that service entirely.

#### 5.7.3 Operator pluralism

The protocol does not privilege any operator. There is no canonical hosted service, no preferred node, no central registry of authorized operators. Any party may run a Mnemonic node, accept artifacts from other operators, anchor on behalf of users, and charge for any of these services. Verification of an artifact is independent of which operator produced it.

This pluralism is what makes the free-tier guarantees credible. An operator that abuses its position — by raising prices, degrading service, or restricting access — does not control the protocol; users can migrate to other operators or to self-hosting without losing accumulated state, because all artifacts are portable by signature and verifiable independently.

#### 5.7.4 Payment as a protocol primitive

The protocol provides primitives for operators that wish to charge for services: payment-gated tool calls, balance-tracking accounts, and standard micropayment patterns over established settlement rails. These primitives are tools, not mandates. An operator that wishes to provide free service simply does not enable payment gating; an operator that wishes to charge enables it. The same protocol code supports both modes.

Operators that charge are responsible for the economics of their own services: pricing, free quotas, subsidies, revenue allocation, and treasury management. These are commercial decisions of individual operators and are not specified by the protocol.

#### 5.7.5 What this economic model rules out

The combination of free verification, free self-hosting, operator pluralism, and optional payment rules out several common failure modes of protocol economics:

- **Verification capture.** No operator can charge for the act of checking whether an artifact is genuine.
- **Publishing capture.** No operator can prevent a user from signing and storing artifacts; the user can always run their own node.
- **Lock-in.** Artifacts produced under one operator are verifiable by any other operator and by self-hosted nodes. There is no operator-specific format or operator-specific verification path.
- **Single-operator dependency for protocol liveness.** Even if a particular hosted operator becomes unavailable, the protocol continues to function across all other operators and self-hosters.

The protocol does not rule out commercial activity. It rules out commercial activity that depends on holding the protocol itself hostage.

#### 5.7.6 Implementation-level economics

Specific pricing, free-tier quotas, subsidy models, treasury accounting, and revenue strategy are operator-level concerns documented separately from the protocol specification. Different implementations and different hosted operators will make different choices. The protocol document specifies only what those choices must respect: verification stays free, self-hosting stays available, and no operator becomes structurally privileged over others.

### 5.8 Pipeline Walkthrough (sign / recall / verify)

The protocol exposes three primary operational flows: signing a new memory, recalling stored memories by meaning, and verifying that a held artifact is genuine.

**Sign.** A request carrying content and metadata traverses a fixed pipeline. The configured embedder produces a full-precision embedding vector; the vector is compressed so a small form can be carried inside artifact metadata for portability and durable-storage anchoring. The artifact — content, declared cognitive kind, producer identity, timestamp, tags, embedding metadata — is encoded to canonical CBOR with stable field ordering, hashed by content, and signed under a COSE_Sign1 envelope with the operator's Ed25519 identity. The signed artifact is then persisted to whichever backends the user has selected for that artifact: a local index for fast access, and any subset of cloud, content-addressed network, or on-chain backends for durability and third-party verifiability. When the user requests an anchor, the content hash and a backend reference are submitted to the chosen anchor backend; the resulting proof is committed alongside the artifact.

**Recall.** A query is embedded with the same provider used at sign time, scored against the stored full-precision embeddings by cosine similarity, and the top-k matches are returned ordered by score. Recall reads from the local index regardless of where else the artifacts are stored: the local index is the hot path for retrieval, while remote backends serve durability and third-party verification. Compressed embeddings in artifact metadata serve portability and cross-node proof-of-existence, not retrieval.

**Verify.** Verification reads the artifact bytes from whichever backend holds them, recomputes the content hash over the canonical-encoded payload, and validates the signature against the claimed producer identity. If the artifact carries an anchor proof, the verifier additionally confirms that the anchor exists on the claimed anchor backend and references the same content hash. The result is one of `verified`, `tampered`, or `not_found`.

For a deeper walkthrough including module boundaries and lock discipline, see [docs/how-it-works.md](./how-it-works.md).

## 6. Artifact Model

Mnemonic artifacts are typed, versioned, and canonical.

The schema registry distinguishes five kinds of memory artifact, each with its own schema and semantics (see §7.1), alongside the artifact types produced by surrounding agent workflows and the capability artifacts that authorize sharing:

- `memory.episodic` — time-ordered events, observations, and interactions
- `memory.semantic` — factual assertions about the world, typically as structured claims
- `memory.procedural` — learned skills, routines, and workflows
- `memory.working` — transient goals, subgoals, scratch state, and pending actions
- `memory.identity` — persistent persona attributes, preferences, communication style, and operational policies
- `rag.context` — retrieved context bundles
- `rag.result` — generated results derived from retrieved context
- `agent.state` — state snapshots
- `receipt` — operational receipts
- `capability.token` — signed, scoped authorizations over lineage subtrees (see §7.2)

Each schema defines required fields, optional fields, and stable canonical CBOR field ordering. Published schemas are immutable within a version. Changes require version bumps. The earlier flat `memory` schema is retained as a deprecated alias and resolves to `memory.episodic` for backward compatibility; new artifacts should use one of the five typed kinds directly.

This model lets Mnemonic evolve beyond single memory items into a general attestation layer for agent workflows: typed memory by cognitive role, retrieved context, generated results, state snapshots, receipts, capability authorizations, and lineage-linked artifacts.

## 7. Memory Composition and Sharing

The artifact model in §6 gives a single signed memory its shape. §7 specifies how those artifacts compose into multi-runtime workflows: how cognitive role drives per-kind semantics, how access is scoped via capability tokens, how memory crosses trust boundaries through a defined handshake, and how it enters a target runtime through a rehydration pipeline that includes safe-injection framing. The whitepaper states the protocol-level claims; structures, exchange details, stage interfaces, and marker grammars are specified in [docs/spec/memory-composition.md](./spec/memory-composition.md).

### 7.1 Cognitive Typing

The five `memory.*` kinds declared in §6 (`episodic`, `semantic`, `procedural`, `working`, `identity`) are not stylistic tags. They reflect distinct cognitive roles and warrant different retention, retrieval, sharing, and safety semantics. The kind is part of the canonical artifact and is verified alongside content, so downstream tooling reads the declared kind and applies kind-appropriate policy without renegotiating with the producer. Per-kind defaults — retention horizons, retrieval scoring, sharing posture, framing strictness — are specified in the memory-composition spec.

### 7.2 Capability Tokens

Cross-runtime access to memory is authorized by signed, scoped, revocable capability tokens. A capability token is itself a typed artifact (`capability.token`), content-addressed, and verifiable by anyone holding it. Tokens carry a subject, a scope (over lineage subtrees, kinds, tag predicates, or explicit ids), a permission set, and an expiry; delegation is supported through a chain of authority; revocation is a counter-signed attestation. The protocol does not require online revocation checks on every use — short-lived tokens are the preferred pattern for high-value operations, and longer-lived tokens carry explicit online-check policy in their metadata. Token structure, scope grammar, and revocation semantics are specified in the memory-composition spec.

### 7.3 Sharing Handshake

Memory crosses a trust boundary through a defined handshake between two runtimes. The handshake produces mutual authentication, an effective scope as the intersection of the capability token's scope and the sender's policy at handshake time, an encrypted transport for the artifact bytes, and a co-signed share receipt anchored in the lineage DAG so the transfer itself becomes auditable. The handshake is the protocol-level boundary between memory at rest and memory in flight. Exchange details, transport bindings, and receipt structure are specified in the memory-composition spec.

### 7.4 Rehydration Pipeline

When a signed artifact enters a target runtime, it traverses a defined pipeline: **verify → filter → rank → compress → format → frame → inject**. The pipeline is deterministic and replayable: two implementations given the same inputs and configuration produce identical output at every stage, which makes the entire rehydration auditable from the source artifacts plus the recorded capability evaluation. Stage-by-stage interfaces, the ranker abstraction, and compression budgets are specified in the memory-composition spec.

### 7.5 Safe Injection (Framing)

Memory content can resemble instructions, and naively concatenating retrieved memory into a target runtime's prompt creates a memory-mediated prompt injection surface. The protocol's framing layer wraps retrieved memory in safe-injection markers that declare provenance and posture (reference content, not instruction). Identity-kind memory carries stricter framing than other kinds, reflecting its higher potential as an injection vector. Framing is a protocol-level contract enforced at the rehydration boundary; it is not a unilateral guarantee, because honoring the markers depends on receiving-runtime cooperation, and compliance is itself an attestation that target runtimes publish. Marker grammars, per-kind strictness, and compliance-attestation structure are specified in the memory-composition spec.

### 7.6 Portability Across Runtimes

The portability claim composes the prior subsections. A signed artifact whose authorship and integrity verify locally remains verifiable after transfer; capability tokens and the sharing handshake establish what may move and on what terms; the rehydration pipeline transforms transferred bytes into target-runtime context without losing provenance; framing protects the receiving runtime from memory-mediated injection. The result is that operator identity, not runtime identity, is what binds memory together over time. An operator that accumulated memory under one runtime and one provider may switch and continue from the accumulated state without re-signing prior records.

## 8. Trust Model

Mnemonic's trust model separates what the protocol guarantees from what it does not. The guarantees rest on signatures and canonical encoding alone; backend choices, anchoring, and capability tokens are layered above to add third-party timestamp, scoped sharing, and auditable transfer. Everything outside the guarantee list is explicitly out of scope for v1.

The protocol guarantees:

- **Integrity** — artifacts are content-addressed over canonical encoded bytes; tampering after signing is detectable by anyone holding the artifact.
- **Authorship** — artifacts are signed by an Ed25519 operator identity; the producer is independently verifiable.
- **Lineage verifiability** — parent–child relationships across artifacts are content-addressed; tampering with any artifact in a lineage chain breaks the chain up to the root.
- **Backend-independent verification** — verification holds regardless of which backend served the bytes. The same artifact retrieved from a local index, from a content-addressed network, or after on-chain anchoring returns the same authorship and integrity result.
- **Optional third-party timestamp** — when an anchor is requested, existence at or before the anchor time is verifiable by any party that does not trust the operator (§5.6).
- **Capability-scoped sharing** — cross-runtime access is authorized by signed, scoped, revocable capability tokens; receivers can independently verify the authorization chain (§7.2).
- **Auditable transfer** — the sharing handshake produces a co-signed share receipt anchored in the lineage DAG; the share event itself is an attestable artifact (§7.3).
- **Safe-injection contract** — retrieved memory is wrapped in markers that declare provenance and posture; the contract is honored when target runtimes publish framing-compliance attestations (§7.5).
- **Free verification** — verification requires no operator service, no account, and no permission (§5.7.1).

The protocol does not guarantee:

- **Correctness of memory content** — the protocol verifies who wrote what and when, not whether the content is true.
- **Completeness of memory history** — operators may sign a subset of their state; the protocol does not detect selective omission.
- **At-rest encryption of memory content** — the canonical artifact format is plaintext under the operator's signature; transport encryption is provided by the sharing handshake (§7.3), but at-rest encryption is an operator-level concern.
- **Receiving-runtime safety beyond the framing contract** — if a target runtime does not honor framing markers, the protocol cannot prevent memory-mediated prompt injection unilaterally. Compliance is observable via the target runtime's framing-compliance attestation.
- **Concurrent multi-writer shared namespaces** — the protocol authorizes pairwise transfers via capability tokens, but does not yet specify conflict resolution, ordering, or convergence semantics for namespaces with multiple concurrent writers.
- **ZK proofs of embedding correctness** — there is no protocol-level proof that an embedding was computed faithfully from the claimed content under the claimed model.
- **ZK proofs of retrieval correctness** — there is no protocol-level proof that a returned top-k is the true top-k against a committed corpus.

These boundaries are intentional. The v1 protocol commits only to what signatures and canonical encoding can enforce, plus what capability tokens and the sharing handshake can authorize and attest. ZK proofs of computation, multi-writer convergence, and at-rest encryption are credible extensions, but they add cost and complexity that the v1 scope deliberately defers (§15 Roadmap).

## 9. Positioning In The Agent Stack

Mnemonic is not a replacement for A2A protocols, orchestration systems, or vector databases.

A2A protocols handle discovery, coordination, task exchange, and message passing. Mnemonic fits underneath that layer as durable memory, provenance, portability, and trust infrastructure.

In one sentence:

> A2A makes agents interoperable in motion; Mnemonic makes them coherent over time.

## 10. Use Cases

Mnemonic supports a family of agent-memory patterns. The 10 subsections below are short summaries; each links to a deep-dive document under `docs/usecases/`.

### 10.1 Shared Project Memory Namespace

Multiple A2A agents read from and write to a shared project-level memory namespace, so findings, decisions, contradictions, and source references accumulate on the project rather than inside any single agent. New agents joining the workflow retrieve accumulated context instead of starting from zero.
[See deep-dive in docs/usecases/shared-project-memory-namespace.md.]

### 10.2 Shared Memory Layer

Mnemonic acts as a persistent shared memory substrate underneath A2A coordination, surviving sessions, providers, and runtime changes while offering semantic retrieval and verifiable provenance. This replaces fragile context windows, ad-hoc databases, and vendor-locked memory with a portable common surface.
[See deep-dive in docs/usecases/shared-memory-layer.md.]

### 10.3 Provenance And Attestation Layer

Mnemonic records what an agent produced, what inputs it used, when it produced the output, and how the output connects to earlier artifacts, turning opaque message passing between agents into auditable knowledge production. Downstream consumers can independently check authorship, integrity, and timestamped existence of each claim.
[See deep-dive in docs/usecases/provenance-attestation-layer.md.]

### 10.4 Trust And Reputation Layer

Historical memory and contribution records can power trust signals — which agents are reliable in a domain, whose outputs are reused, which contributors are noisy or adversarial — that orchestrators use beyond declared capabilities. Mnemonic links agent identity, memory entries, downstream usage, and validation outcomes into a durable reputation surface.
[See deep-dive in docs/usecases/trust-reputation-layer.md.]

### 10.5 Portable Memory Wallet

Memory belongs to the agent or its operator rather than a provider: an operator can write memory while running on Claude, switch the runtime to GPT or a local model, and continue working from the same attested store without re-signing or re-attesting prior records. Memory snapshots are portable, verifiable, rehydratable, and independent from a single inference provider.
[See deep-dive in docs/usecases/portable-memory-wallet.md.]

### 10.6 Settlement-Aware Memory Infrastructure

Networked memory services need metering and payment; Mnemonic already supports balance and x402-style HTTP payment flows so agents can autonomously pay for memory writes, recall, and verification. This evolves into agent-payable memory infrastructure where verification remains open and paid operations sustain node operators.
[See deep-dive in docs/usecases/settlement-aware-memory-infrastructure.md.]

### 10.7 Task Memory Ledger

Each task exchanged in an A2A workflow leaves a durable record — request hash, assigned agent, summary, intermediate notes, output, artifact references, completion status, ordering anchors — that subsequent agents can retrieve. This prevents repeated context loss across the many short-lived tasks typical in multi-agent execution.
[See deep-dive in docs/usecases/task-memory-ledger.md.]

### 10.8 Artifact Attestation Service

Mnemonic attests, indexes, and retrieves artifacts produced by A2A workflows — reports, code patches, evidence bundles, recommendations, structured outputs — by storing artifact hash, producing identity, upstream references, and semantic summary. Consumers can later prove who produced an artifact, when, and from which inputs.
[See deep-dive in docs/usecases/artifact-attestation-service.md.]

### 10.9 Agent Continuity Layer

When an agent moves across runtimes, providers, or infrastructure because of cost, model upgrades, framework migration, or compliance, Mnemonic preserves prior memory items, project context, artifact history, and decisions so the agent retains accumulated context. Continuity is decoupled from the specific platform the agent runs on today.
[See deep-dive in docs/usecases/agent-continuity-layer.md.]

### 10.10 Reliability Oracle For Orchestration

Orchestrators query Mnemonic for memory-backed trust signals — accepted vs rejected outputs, downstream reuse, citation quality, contradiction rate, reviewer corrections — to route work beyond stated capabilities. Mnemonic holds the historical evidence needed to answer reliability questions about agents and contributions.
[See deep-dive in docs/usecases/reliability-oracle-for-orchestration.md.]

## 11. Related Work

Mnemonic sits at the intersection of:

- agent memory systems
- vector databases and RAG infrastructure
- decentralized storage
- blockchain commitments
- verifiable computation
- machine-native payments

The closest research and product directions include decentralized RAG, trustless agentic memory, ZK embedding proofs, verifiable ANN retrieval, and source reliability oracles. Mnemonic's current bet is pragmatic: hash commitments and signed artifacts are cheaper and deployable today, while ZK embedding or retrieval proofs remain credible future extensions.

## 12. Current Implementation Status

The current canonical implementation is the Rust MCP server in this repository.

Implemented today:

- HTTP and stdio MCP transports.
- Five Mnemonic tools.
- Ed25519 server identity, with DID-sol and DID-key derivation exposed through `mnemonic_whoami`.
- Canonical CBOR artifact encoding.
- COSE_Sign1 artifact signing.
- blake3 hashing for current artifacts.
- TurboQuant compression of embeddings (2–4 bits per dimension) carried in artifact metadata.
- SQLite local recall over full embeddings.
- Local lineage index: parent–child artifact DAG with cycle detection and directional BFS traversal (`Ancestors`, `Descendants`, `Both`).
- `local` and `full` storage modes.
- Optional Solana and Arweave persistence in full mode.
- Payment modes: `none`, `balance`, `x402`, and `both`.

Not current implementation behavior:

- End-to-end encrypted snapshots.
- Compressed shadow-index retrieval as the local recall path.
- Multi-party shared namespaces.
- Reliability oracle.
- On-chain node registry.
- Agent SDK abstraction.
- ZK proof of embedding or retrieval correctness.

## 13. Evaluation Plan

A production-grade whitepaper should include empirical results for:

- Artifact signing and verification latency.
- Local recall quality across realistic corpora.
- Embedding provider behavior (`fastembed`, OpenAI, future open embedders).
- Compression ratios and reconstruction error.
- Full-mode persistence latency and cost.
- Payment-gated HTTP overhead.
- Failure modes: missing Arweave data, missing Solana anchors, tampered artifacts, stale local rows.

Historical prototype documents include retrieval and compression benchmarks, but this paper should only publish results that match the current Rust implementation or are clearly labeled as prior research.

## 14. Limitations And Open Questions

Open areas before broad production deployment:

- Security and privacy boundaries.
- Encryption architecture and key recovery.
- Memory write semantics: append, merge, overwrite, contradiction handling.
- Lifecycle policy: pruning, compaction, export, deletion, retention classes.
- Multi-writer consistency and shared namespace authorization.
- Robustness to noisy, duplicate, contradictory, or adversarial memories.
- Product packaging: local tool, SDK, node network, hosted service, or hybrid.
- Compliance and governance for sensitive memory data.

## 15. Roadmap

The roadmap below is organized around a single positioning, locked in 2026-05-01: **Mnemonic is verifiable memory for trustless agents.** Phases 1–3 deliver the core memory primitive; Phases 4–5 compose it into the trustless-agent stack (A2A + ERC-8004) so signed memory becomes a first-class layer underneath multi-agent coordination and on-chain identity.

### Phase 1: Practical Verifiable Memory

- Harden the Rust MCP implementation.
- Document the artifact format and verification model.
- Improve local developer experience.
- Keep local mode fast, free, and offline.
- Ship core protocol primitives as both a native Rust crate and a WebAssembly module, so identical verification logic runs in servers, browsers, and embedded agents.
- Provide a browser-based demo client that recalls and verifies signed memory artifacts without a server dependency.

### Phase 2: Product-Grade Memory Semantics

- Define memory write/update/delete policy.
- Add lifecycle and compaction primitives.
- Specify export and recovery guarantees.
- Clarify privacy and encryption boundaries.

### Phase 3: Shared And Portable Memory

- Introduce shared namespaces.
- Add multi-writer consistency semantics.
- Support portable memory restore workflows.
- Expand provenance artifacts beyond simple memory items.
- Anchor-layer pluggability ("Phase 3α") — narrowed subset that decouples the storage path from Solana SPL Memo, so any chain (or no chain) can serve as anchor. Pulled forward as a prerequisite of Phase 5.

### Phase 4: Trust And Settlement Network

- Add node discovery and operator economics.
- Mature x402-style agent payment flows.
- Introduce reliability scoring for shared-memory contributors.
- Explore ZK proofs for embedding correctness and retrieval correctness.

### Phase 5: Trustless-Agent Stack Integration

This phase locks in the "verifiable memory for trustless agents" positioning by composing Mnemonic's signed-memory primitive into the multi-agent and on-chain identity standards that shipped during 2026.

- **A2A bridge V1.** First-class binding to Google's Agent2Agent protocol (v1.0.0-rc, JSON-RPC + SSE). Three new CBOR schemas (`A2A_TASK_V1`, `A2A_MESSAGE_V1`, `A2A_ARTIFACT_V1`), an adapter crate, two new MCP tools (`mnemonic_attest_a2a`, `mnemonic_recall_a2a`), reference middleware sidecar, and a published byte-for-byte conformance suite. Turns six A2A-shaped use cases (`task-memory-ledger`, `shared-memory-layer`, `shared-project-memory-namespace`, `artifact-attestation-service`, `provenance-attestation-layer`, `reliability-oracle-for-orchestration`) from aspirational into executable.
- **ERC-8004 follow-on.** Integration with the on-chain trustless-agent registries (Identity / Reputation / Validation), live on Ethereum mainnet since 2026-01-29. Four paths: validator-as-a-service on the Validation Registry, agent registration-file binding, Mnemonic-attested entries in the Reputation Registry, and `did:mnemonic:` resolver for cross-ecosystem identity reconciliation. Mnemonic occupies a third trust category — *signed-memory* — distinct from the TEE and crypto-economic validators already filling.
- **AgentCard `x-mnemonic` extension.** Publishes the agent's Ed25519 attestation key alongside its A2A capability declaration, so any A2A-native verifier can locate the Mnemonic verification key without a separate lookup. Composes with A2A's existing AgentCard JWS authentication.
- **Threat model and conformance.** Publishes the first formal threat-model document covering the A2A and ERC-8004 boundaries (canonicalization mismatches, replay, identity substitution, contextId forking) and a portable conformance vector suite (`@mnemonik-xyz/conformance`) so any third-party implementation in any language can prove byte-for-byte parity.
- **Reference deployment — Hermes Agent (Nous Research).** In parallel with the standards-track work above, Mnemonik lands as a first-class Memory Provider in the Hermes agent runtime — the first deployed multi-agent system to ship cryptographically verifiable memory alongside conventional retrieval-only providers. The Hermes integration also enables verifiable RL trajectory datasets (each ShareGPT turn carries a Solana tx + Arweave tx + producing-agent DID), demonstrating the positioning in front of named users while the standards-track integrations mature.

After Phase 5: Mnemonic is the only primitive in the trustless-agent stack that gives cryptographic provenance over content the agent itself claims to remember, cross-vendor temporal coherence via lineage, and semantic recall over that signed history — composable underneath A2A, anchored through ERC-8004's existing on-chain commitments without binding the agent's signing identity to a wallet.

Detailed plan, task breakdown, and decision rationale: `work/a2a-bridge/` (`user-spec.md`, `tech-spec.md`, `tasks/`, `backlog.md`, `decisions.md`, `research/positioning-trustless-agents.md`).

## 16. Conclusion

Agents need more than longer context windows. They need memory that persists, travels, and can be verified.

Mnemonic Protocol provides a practical foundation: semantic memory items encoded as canonical, signed artifacts; local recall for developer usability; optional external persistence for independent verification; and an MCP interface that works with today's agent clients.

The long-term goal is broader: memory that agents can own, share, pay for, audit, and carry across the agent ecosystem. Trustless agents cannot work without trustless agentic memory. Mnemonic is the memory layer for that stack.

---

## References

1. Model Context Protocol. https://modelcontextprotocol.io/
2. Arweave Protocol. https://arweave.org/
3. Solana Documentation. https://solana.com/docs
4. COSE: CBOR Object Signing and Encryption. RFC 9052.
5. BLAKE3 Cryptographic Hash Function. https://github.com/BLAKE3-team/BLAKE3
6. Zandieh, A. and Mirrokni, V. *TurboQuant: Online Vector Quantization with Near-Optimal Distortion Rate.*
7. Coinbase. *x402: HTTP 402 Payment Required for Machine-to-Machine Payments.* https://x402.org/
8. [Mnemonic Protocol Foundational Paper](./research/paper.pdf)

---

## Glossary

**A2A:** Agent-to-Agent. A broad category of protocols and patterns for agent discovery, coordination, task exchange, and message passing. Mnemonic is complementary: A2A moves work between agents; Mnemonic gives agents durable memory across time.

**Arweave:** A decentralized permanent storage network. In Mnemonic full mode, Arweave can store signed artifact bytes so memory records remain available outside a single provider database.

**blake3:** A modern cryptographic hash function used by the current Mnemonic artifact format. Mnemonic hashes canonical CBOR bytes with blake3 so artifact integrity can be checked deterministically.

**CBOR:** Concise Binary Object Representation. A compact binary serialization format. Mnemonic uses canonical CBOR so the same artifact fields serialize into the same byte sequence, which is required for stable hashing and signing.

**Canonical Encoding:** A serialization rule that produces one deterministic byte representation for the same logical data. Without canonical encoding, two equivalent JSON-like objects could produce different bytes and therefore different hashes.

**COSE:** CBOR Object Signing and Encryption. A standards family for signing and encrypting CBOR-encoded data.

**COSE_Sign1:** A COSE structure for a single digital signature over CBOR data. Mnemonic signs artifacts as COSE_Sign1 objects so verifiers can check that a memory artifact was produced by the holder of the corresponding Ed25519 key.

**DID (Decentralized Identifier):** A self-describing identifier scheme that can be resolved without a central registry. Mnemonic derives both a `did:sol` and a `did:key` from the server's Ed25519 keypair so the same identity is addressable on-chain and off-chain.

**Ed25519:** A public-key signature algorithm. Mnemonic uses Ed25519 keypairs for agent identity and artifact signing.

**Lineage / Artifact DAG:** A directed acyclic graph linking child artifacts to their parents. Mnemonic maintains a local lineage index so a recalled memory can be traced back through the artifacts it was derived from, supporting provenance audits and chain verification.

**MCP:** Model Context Protocol. The interface layer Mnemonic uses to expose memory tools to agent clients over HTTP or stdio.

**Semantic Memory Item:** The core unit of Mnemonic memory: human-readable content plus embedding, metadata, identity, and verification data. It is portable in a way raw model attention state is not.

**Solana Anchor:** A Solana transaction or memo used to timestamp and externally commit to artifact data. In full mode, Mnemonic can use Solana as an ordering and verification layer.

**TurboQuant:** A scalar quantization scheme that compresses embedding vectors to 2–4 bits per dimension. Mnemonic uses TurboQuant to shrink embeddings by up to roughly 32× so they remain cheap to anchor on durable storage and to transmit between systems.

**x402:** A machine-native payment pattern based on HTTP 402 Payment Required. Mnemonic's HTTP payment layer supports x402-style flows for agent-payable memory services.

