# Mnemonic Protocol: Verifiable Memory Infrastructure for AI Agents

**Draft:** v0.1  
**Date:** April 2026  
**Status:** Working draft  

---

## Abstract

AI agents increasingly operate across long-running tasks, tools, sessions, and providers, but their memory remains fragile. Context windows are temporary, vendor-native memory is hard to audit, and conventional vector stores provide persistence without cryptographic provenance. As a result, an agent can claim that it remembered a fact, produced an artifact, or acted on prior context, but external parties have no standard way to verify what was stored, who stored it, when it existed, or whether it was later changed.

Mnemonic Protocol introduces a verifiable memory layer for AI agents. It treats memory as a portable, signed artifact rather than an opaque database row: something an agent can recall, carry across systems, and prove has not been silently changed. Memories can run fully locally for speed and development, or be persisted to decentralized infrastructure so third parties can independently verify integrity, authorship, and timestamped existence.

Mnemonic is exposed through the Model Context Protocol (MCP), making it usable by current agent clients over HTTP or stdio. The current implementation is a Rust MCP server with five tools for identity, memory signing, verification, challenge signing, and recall. The broader protocol roadmap extends this foundation toward shared project memory, provenance attestations, portable memory wallets, settlement-aware memory services, and multi-agent trust infrastructure.

The core thesis is simple: trustless agents cannot work without trustless agentic memory. A2A protocols make agents interoperable in motion; Mnemonic makes them coherent over time.

---

## 1. Introduction

AI agents forget. Their working context disappears when a session ends, when a provider restarts, when a model changes, or when a workflow moves between tools. Even when a system stores memory, that memory is usually controlled by a single provider, shaped by a private database, and unverifiable by outside parties.

For simple assistants, this is inconvenient. For agents that produce research, compliance artifacts, security findings, financial decisions, or operational plans, it is a trust problem. The question is not only whether an agent can remember, but whether anyone can verify what the agent remembered, when it remembered it, who wrote it, and whether the record was later modified.

The common instinct is to make context windows longer or to snapshot more internal model state. That helps with continuity inside one runtime, but it does not solve portability or auditability. Raw attention state is model-specific, opaque, expensive to move, and hard for independent systems to interpret. A proprietary chat history is more readable, but still belongs to the platform that stores it.

Mnemonic starts from a different unit: the semantic memory item. A memory item is human-readable content linked to embeddings, metadata, cryptographic identity, and a verifiable commitment trail. It can be recalled by meaning, inspected by people, signed by agents, and independently checked by other systems.

This shift matters for multi-agent systems. A2A protocols can move messages and tasks between agents, but they do not by themselves provide durable, portable, attestable memory. Mnemonic fits underneath coordination protocols as memory infrastructure: the substrate that lets agents remain coherent over time.

## 2. Problem Statement

Current agent memory systems typically fall into three categories:

1. **Context windows** are fast and accurate but temporary.
2. **Application-native memory** can persist but is locked to one product or provider.
3. **External vector stores** support retrieval but usually do not prove provenance, integrity, ordering, or non-repudiation.

These systems are useful, but they do not provide a portable trust layer. If an agent moves from one runtime to another, its accumulated state may not move with it. If a memory record changes, consumers may not notice. If an agent produces an answer based on prior context, downstream systems may not be able to audit which context existed at the time.

A verifiable agent memory system needs six properties:

- **Persistence:** memory survives sessions, providers, and runtime changes.
- **Semantic recall:** memory can be retrieved by meaning, not only by keyword.
- **Provenance:** each memory is linked to a cryptographic identity.
- **Integrity:** tampering can be detected independently.
- **Portability:** memory is not trapped inside one vendor or framework.
- **Economic viability:** storage and verification costs remain low enough for real agent workflows.

## 3. Design Goals

Mnemonic is designed around the following goals:

### 3.1 Current Implementation Goals

- Provide a working MCP server that existing agent clients can use today.
- Support local development with no blockchain dependency.
- Produce deterministic, typed, signed artifacts using canonical CBOR and COSE_Sign1.
- Use blake3 hashes over canonical artifact bytes for current artifacts.
- Store searchable local state in SQLite.
- Support optional full-mode persistence through Arweave and Solana.
- Keep verification available as a first-class operation.

### 3.2 Protocol Roadmap Goals

- Make memory portable across models, providers, agents, and hosts.
- Support shared project memory namespaces for multi-agent workflows.
- Extend memory records into provenance attestations for artifacts and decisions.
- Provide settlement-aware memory infrastructure that agents can pay for directly.
- Add stronger privacy, lifecycle, and multi-writer semantics before broad multi-party deployment.
- Ship core protocol primitives as both a native Rust crate and a WebAssembly module, so identical verification logic runs in servers, browsers, and embedded agents.
- Provide a browser-based demo client that recalls and verifies signed memory artifacts without a server dependency.

## 4. Core Insight

Agent memory should be semantically meaningful, cryptographically attributable, and operationally cheap.

Raw transformer state is the wrong abstraction for portable memory. Attention caches are model-specific, opaque, large, and difficult for humans or independent systems to interpret. Semantic memory items are smaller, inspectable, portable, and compatible with retrieval systems.

Mnemonic applies this principle through a layered pipeline:

```text
content
  -> embed
  -> compress
  -> build typed artifact
  -> canonicalize to CBOR
  -> hash canonical bytes with blake3
  -> sign with Ed25519 as COSE_Sign1
  -> persist locally or externally
  -> recall and verify through MCP
```

The current implementation uses full embeddings in SQLite for recall. Compressed embeddings are produced and embedded in artifact metadata, supporting portability and future cross-node verification work. Compression uses TurboQuant scalar quantization to 2–4 bits per dimension, yielding up to roughly 32× size reduction so embeddings remain cheap to carry across systems and to anchor on durable storage. Historical research explored compressed candidate generation plus exact reranking; that remains a protocol design direction rather than the current local recall path.

## 5. Architecture Overview

Mnemonic has two layers:

- **`mnemonic-core`** provides protocol primitives: canonical CBOR encoding, blake3 hashing, COSE_Sign1 signing, identity (Ed25519 keypairs with DID-sol and DID-key derivation), embedding, TurboQuant compression, storage traits, Solana integration, Arweave integration, and lineage helpers (a parent–child artifact DAG with directional traversal).
- **`mnemonic-mcp`** exposes those primitives through MCP over HTTP and stdio, plus payment and pricing logic for networked operation.

The MCP server exposes five tools:

- `mnemonic_whoami`
- `mnemonic_sign_memory`
- `mnemonic_verify`
- `mnemonic_prove_identity`
- `mnemonic_recall`

Storage is selected by mode:

- **Local mode:** SQLite only, synthetic local transaction IDs, no payment gate.
- **Full mode:** signed artifact bytes can be persisted to Arweave, anchored on Solana, and indexed locally in SQLite.

### 5.3 Pipeline Walkthrough (sign / recall / verify)

The MCP server exposes three primary operational flows. Each is a thin orchestration layer over `mnemonic-core` primitives.

**`mnemonic_sign_memory`.** A request carrying content text and tags traverses a fixed pipeline. The active embedder (FastEmbed local, OpenAI remote, or `MockEmbedder` in tests) produces a full-precision f32 vector. The vector is run through the TurboQuant scalar quantizer at the configured bit width (2, 3, or 4 bits per dimension; default 4) so the compressed form can be carried inside artifact metadata. The artifact — content, producer DID, timestamp, tags, embedding metadata — is encoded to canonical CBOR with stable field ordering, hashed with blake3, and signed as a COSE_Sign1 envelope using the server's Ed25519 identity. In `local` mode the COSE bytes plus the uncompressed embedding are written to SQLite and the operation returns synthetic `local:` transaction IDs at zero cost. In `full` mode the COSE bytes are uploaded to Arweave (via Irys) and the blake3 hash plus Arweave tx ID are anchored on Solana via an SPL Memo instruction; both real tx IDs are then committed alongside the row in the SQLite `AttestationStore`.

**`mnemonic_recall`.** Recall is local-only by design. The query string is embedded with the same provider used at sign time, then scored against every stored uncompressed f32 embedding in SQLite via cosine similarity, and the top-k rows are returned ordered by score. The compressed bytes that traveled to Arweave are not consulted here: they are proof-of-existence artifacts for third-party verification, not a retrieval index. Recall therefore costs one embed call and one full table scan, with no chain reads.

**`mnemonic_verify`.** Verification reads the stored artifact (locally, or by fetching the COSE bytes from Arweave in full mode), recomputes the blake3 hash over the canonical CBOR payload, and validates the COSE_Sign1 signature against the claimed Ed25519 producer identity. In full mode the verifier additionally confirms that the on-chain SPL Memo on Solana exists and references the same blake3 hash and Arweave tx ID. The result is one of `verified`, `tampered`, or `not_found`.

For a deeper walkthrough including module boundaries and lock discipline, see [docs/how-it-works.md](./how-it-works.md).

## 6. Artifact Model

Mnemonic artifacts are typed, versioned, and canonical.

The current schema registry includes:

- `memory`
- `rag.context`
- `rag.result`
- `agent.state`
- `receipt`

Each schema defines required fields, optional fields, and stable canonical CBOR field ordering. Published schemas are immutable within a version. Changes require version bumps.

This model lets Mnemonic evolve beyond single memory items into a general attestation layer for agent workflows: retrieved context, generated results, state snapshots, receipts, and lineage-linked artifacts.

## 7. Trust Model

Mnemonic currently guarantees:

- **Integrity:** current artifacts are hashed over canonical CBOR bytes.
- **Authorship:** artifacts are signed by an Ed25519 identity.
- **Local verifiability:** local-mode records can be checked against SQLite state.
- **External verifiability:** full-mode records can be checked against persisted artifacts and chain anchors when available.
- **Version awareness:** current CBOR+COSE artifacts and legacy JSON/SHA-256 artifacts are handled separately.

Mnemonic does not yet guarantee:

- End-to-end encryption in the active MCP sign/verify path.
- Correctness of the memory content itself.
- Completeness of an agent's memory history.
- ZK proof that an embedding was computed faithfully.
- ZK proof that a retrieval result is the true top-k from a committed corpus.
- Safe multi-party shared memory semantics.

These limitations are intentional to state clearly: Mnemonic V1 prioritizes practical memory integrity and provenance before more expensive proof systems.

## 8. Positioning In The Agent Stack

Mnemonic is not a replacement for A2A protocols, orchestration systems, or vector databases.

A2A protocols handle discovery, coordination, task exchange, and message passing. Mnemonic fits underneath that layer as durable memory, provenance, portability, and trust infrastructure.

In one sentence:

> A2A makes agents interoperable in motion; Mnemonic makes them coherent over time.

## 9. Use Cases

Mnemonic supports a family of agent-memory patterns. The 10 subsections below are short summaries; each links to a deep-dive document under `docs/usecases/`.

### 9.1 Shared Project Memory Namespace

Multiple A2A agents read from and write to a shared project-level memory namespace, so findings, decisions, contradictions, and source references accumulate on the project rather than inside any single agent. New agents joining the workflow retrieve accumulated context instead of starting from zero.
[See deep-dive in docs/usecases/shared-project-memory-namespace.md.]

### 9.2 Shared Memory Layer

Mnemonic acts as a persistent shared memory substrate underneath A2A coordination, surviving sessions, providers, and runtime changes while offering semantic retrieval and verifiable provenance. This replaces fragile context windows, ad-hoc databases, and vendor-locked memory with a portable common surface.
[See deep-dive in docs/usecases/shared-memory-layer.md.]

### 9.3 Provenance And Attestation Layer

Mnemonic records what an agent produced, what inputs it used, when it produced the output, and how the output connects to earlier artifacts, turning opaque message passing between agents into auditable knowledge production. Downstream consumers can independently check authorship, integrity, and timestamped existence of each claim.
[See deep-dive in docs/usecases/provenance-attestation-layer.md.]

### 9.4 Trust And Reputation Layer

Historical memory and contribution records can power trust signals — which agents are reliable in a domain, whose outputs are reused, which contributors are noisy or adversarial — that orchestrators use beyond declared capabilities. Mnemonic links agent identity, memory entries, downstream usage, and validation outcomes into a durable reputation surface.
[See deep-dive in docs/usecases/trust-reputation-layer.md.]

### 9.5 Portable Memory Wallet

Memory belongs to the agent or its operator rather than a provider: an operator can write memory while running on Claude, switch the runtime to GPT or a local model, and continue working from the same attested store without re-signing or re-attesting prior records. Memory snapshots are portable, verifiable, rehydratable, and independent from a single inference provider.
[See deep-dive in docs/usecases/portable-memory-wallet.md.]

### 9.6 Settlement-Aware Memory Infrastructure

Networked memory services need metering and payment; Mnemonic already supports balance and x402-style HTTP payment flows so agents can autonomously pay for memory writes, recall, and verification. This evolves into agent-payable memory infrastructure where verification remains open and paid operations sustain node operators.
[See deep-dive in docs/usecases/settlement-aware-memory-infrastructure.md.]

### 9.7 Task Memory Ledger

Each task exchanged in an A2A workflow leaves a durable record — request hash, assigned agent, summary, intermediate notes, output, artifact references, completion status, ordering anchors — that subsequent agents can retrieve. This prevents repeated context loss across the many short-lived tasks typical in multi-agent execution.
[See deep-dive in docs/usecases/task-memory-ledger.md.]

### 9.8 Artifact Attestation Service

Mnemonic attests, indexes, and retrieves artifacts produced by A2A workflows — reports, code patches, evidence bundles, recommendations, structured outputs — by storing artifact hash, producing identity, upstream references, and semantic summary. Consumers can later prove who produced an artifact, when, and from which inputs.
[See deep-dive in docs/usecases/artifact-attestation-service.md.]

### 9.9 Agent Continuity Layer

When an agent moves across runtimes, providers, or infrastructure because of cost, model upgrades, framework migration, or compliance, Mnemonic preserves prior memory items, project context, artifact history, and decisions so the agent retains accumulated context. Continuity is decoupled from the specific platform the agent runs on today.
[See deep-dive in docs/usecases/agent-continuity-layer.md.]

### 9.10 Reliability Oracle For Orchestration

Orchestrators query Mnemonic for memory-backed trust signals — accepted vs rejected outputs, downstream reuse, citation quality, contradiction rate, reviewer corrections — to route work beyond stated capabilities. Mnemonic holds the historical evidence needed to answer reliability questions about agents and contributions.
[See deep-dive in docs/usecases/reliability-oracle-for-orchestration.md.]

## 10. Related Work

Mnemonic sits at the intersection of:

- agent memory systems
- vector databases and RAG infrastructure
- decentralized storage
- blockchain commitments
- verifiable computation
- machine-native payments

The closest research and product directions include decentralized RAG, trustless agentic memory, ZK embedding proofs, verifiable ANN retrieval, and source reliability oracles. Mnemonic's current bet is pragmatic: hash commitments and signed artifacts are cheaper and deployable today, while ZK embedding or retrieval proofs remain credible future extensions.

## 11. Current Implementation Status

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

## 12. Evaluation Plan

A production-grade whitepaper should include empirical results for:

- Artifact signing and verification latency.
- Local recall quality across realistic corpora.
- Embedding provider behavior (`fastembed`, OpenAI, future open embedders).
- Compression ratios and reconstruction error.
- Full-mode persistence latency and cost.
- Payment-gated HTTP overhead.
- Failure modes: missing Arweave data, missing Solana anchors, tampered artifacts, stale local rows.

Historical prototype documents include retrieval and compression benchmarks, but this paper should only publish results that match the current Rust implementation or are clearly labeled as prior research.

## 13. Limitations And Open Questions

Open areas before broad production deployment:

- Security and privacy boundaries.
- Encryption architecture and key recovery.
- Memory write semantics: append, merge, overwrite, contradiction handling.
- Lifecycle policy: pruning, compaction, export, deletion, retention classes.
- Multi-writer consistency and shared namespace authorization.
- Robustness to noisy, duplicate, contradictory, or adversarial memories.
- Product packaging: local tool, SDK, node network, hosted service, or hybrid.
- Compliance and governance for sensitive memory data.

## 14. Roadmap

The roadmap below is organized around a single positioning, locked in 2026-05-01: **Mnemonic is verifiable memory for trustless agents.** Phases 1–3 deliver the core memory primitive; Phases 4–5 compose it into the trustless-agent stack (A2A + ERC-8004) so signed memory becomes a first-class layer underneath multi-agent coordination and on-chain identity.

### Phase 1: Practical Verifiable Memory

- Harden the Rust MCP implementation.
- Document the artifact format and verification model.
- Improve local developer experience.
- Keep local mode fast, free, and offline.

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

After Phase 5: Mnemonic is the only primitive in the trustless-agent stack that gives cryptographic provenance over content the agent itself claims to remember, cross-vendor temporal coherence via lineage, and semantic recall over that signed history — composable underneath A2A, anchored through ERC-8004's existing on-chain commitments without binding the agent's signing identity to a wallet.

Detailed plan, task breakdown, and decision rationale: `work/a2a-bridge/` (`user-spec.md`, `tech-spec.md`, `tasks/`, `backlog.md`, `decisions.md`, `research/positioning-trustless-agents.md`).

## 15. Conclusion

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

