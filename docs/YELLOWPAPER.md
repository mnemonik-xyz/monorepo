# Mnemonic Protocol Yellow Paper: Verifiable Memory Infrastructure for AI Agents

**Draft:** v0.6
**Date:** 2 October 2026
**Status:** Working draft
**Overview for readers:** [Whitepaper](./WHITEPAPER.md). This paper combines the current source contract with explicitly labeled design proposals. The [source capability matrix](./source-capabilities.md) defines implemented scope at `26a9550`; it is not a release or live-service claim. The Russian edition describes an earlier revision.

---

## Abstract

Mnemonic represents agent memory as signed artifacts that clients can retain and verify independently of the original operator.
Client-prepared sealed memory is encrypted before delivery. The operator receives original signed ciphertext and public metadata.
Authenticated checkpoints, retained keys and accessible artifact bytes permit recovery without the old operational SQL database.
Recovery establishes ancestry to trusted heads, not global completeness or a provably newest state.

The source supports general-memory preparation and recovery, plus A2A discovery, sealed grants, completed streams and bounded storage migration.
Local integration drills use real cryptography and mocked external services; live availability and release acceptance remain separate.
Five cognitive schemas, capability-token delegation, safe-injection framing and ERC-8004 integration below are design proposals.
They are not implemented requirements or security guarantees.

---

## 1. Introduction

An agent's saved context can outlive its runtime when the client retains a portable record and the keys needed to verify and open it.
Mnemonic separates that record from the operator's search and delivery services.
Original signatures remain verifiable after byte-preserving copying; storage and discovery still determine which bytes can be found.

Embeddings accelerate search and can be rebuilt for a different model.
They do not establish truth or provide an authoritative version of the signed content.
The current formats permit recovery without requiring an embedding.

## 2. Problem Statement

Recovery must distinguish trustworthy bytes from routing hints, preserve original authorship, and expose incomplete results.
It must also keep private content on the client during preparation and opening.
These requirements differ from complete process migration or preservation of an agent's external credentials and unfinished actions.

Further design goals include richer memory types, delegated access policies and safer consumption of untrusted text.
The proposals below do not establish those features as available or prove that an LLM will obey framing markers.

## 3. Current Source Contract

### 3.1 Authorship and exact bytes

A client-prepared artifact carries the author's signature over the format's signed payload.
COSE verification checks its signature structure; it is not equivalent to signing an arbitrary bare content identifier.
A verifier must independently pin the expected author rather than trusting the envelope's embedded key alone.
Ed25519 signatures and BLAKE3 hashes establish integrity, not truth, trusted time, or key custody.

`MEMORY_V1` identifiers and parent identifiers need not be content hashes.
Verified memory graph recovery therefore pins the author and exact-envelope digest for each head and ancestor.
A2A parent identifiers bind content hashes; verification also checks context and author/recipient eligibility.
Neither graph contract makes an untrusted index authoritative for the latest head.

### 3.2 Client privacy and local persistence

The prepared sealed path encrypts and signs before sending bytes to the operator.
The SDK retains bytes in its session cache; durable persistence belongs to the caller.
The CLI persists sealed bytes locally. Local signing still requires the identity key.
Legacy server-prepared signing sends plaintext to the operator and has a different privacy boundary.
Explicit HTTP local writes are rejected; plain stdio-local rows are not signed portable artifacts.

### 3.3 Recovery and independence

Recovery checks original signatures, trusted authors, artifact kind and signed parent bindings before importing content.
Checkpoint owners and scope are supplied independently of the fetched checkpoint.
Keys, original bytes and trusted expected heads remain recovery prerequisites.
`completeToHeads` covers only the supplied heads' ancestry; omissions, forks and source errors remain visible.

An operator's SQL receipt is not evidence of artifact authorship or exact external bytes.
However, financial records remain necessary for safe payment replay and reconciliation.
Recovery of memory does not imply recovery of lost payment state or complete agent execution state.

### 3.4 Scope of design proposals

The following sections retain proposals for cognitive schemas, capability tokens, framing, batch anchors and additional storage backends.
They do not define shipped requirements. Current APIs and supported routes are listed in [source capabilities](./source-capabilities.md).

## 4. Proposed Composition Model

**Design only:** the complete pipeline in this section is not implemented. Embedding, framing and capability-token stages are not required for current sealed preparation.

Effective agentic memory infrastructure must simultaneously satisfy four conditions: it must be semantically expressive, cryptographically attributable, portable across heterogeneous runtimes, and computationally inexpensive to maintain.

Raw transformer attention states and Key-Value (KV) caches represent the wrong abstraction layer for portable memory. Transformer attention architectures are inherently model-specific, mathematically opaque, dimensionally massive, and entirely uninterpretable by humans or external verification systems.

Conversely, the Mnemonic Protocol introduces **typed memory artifacts** as the fundamental unit of state. These artifacts are compact, structurally inspectable, universally portable, and natively optimized for modern search and retrieval mechanics.

The protocol executes this design by composing two symmetrical pipelines built on top of the same cryptographic primitives: the **Sign Pipeline** for state production, and the **Share / Rehydrate Pipeline** for trust-boundary transit.

---

### 4.1 The Sign Pipeline
The Sign Pipeline processes raw execution context into a sealed, verifiable cryptographic artifact through the following sequential operations:


```text
[Raw Semantic Content]

EMBED           ──► Generate High-Dimensional Vector v ∈ ℝᵈ
QUANTIZE        ──► Apply TurboQuant Scalar Compression to v_q ∈ ℤ_𝘲ᵈ
ENCAPSULATE     ──► Bind Content, v_q, Type Meta, and Parent CID
CANONICALIZE    ──► Serialize Structure to Deterministic cCBOR
HASH            ──► Compute Content Identifier (CID) via BLAKE3
SIGN            ──► Sign the COSE_Sign1 signature structure with Ed25519
PERSIST         ──► Write Sealed Envelope to Distributed Storage Layers
```

---

### 4.2 The Share / Rehydrate Pipeline
The Share / Rehydrate Pipeline securely transfers a previously sealed memory artifact across an arbitrary trust boundary into an independent target execution environment:

```text
[COSE_Sign1 Artifact + Capability Token κ]

HANDSHAKE       ──► Authenticate Peers + Establish Ephemeral Diffie-Hellman Transit Key
AUDIT           ──► Cryptographically Verify Authorship, Integrity, Lineage, and Anchors
FILTER          ──► Prune Artifact Collection Based on Token Capability Scope κ
RANK            ──► Compute Integer Dot Products Over Quantized Vectors (v_q)
DECOMPRESS      ──► Reconstruct Selected High-Probability Candidates to float32 Precision
FRAME & INJECT  ──► Wrap Uncompressed Semantic Text in Isolation Markers and Push to LLM Context
```

The protocol reference framework utilizes **TurboQuant Scalar Quantization** to achieve these bounds. The specific implementation parameters—including exact bit-width allocations, centroid positioning algorithms, and localized retrieval execution steps—are isolated from the core protocol definition and treated as flexible runtime configuration profiles.


---

### 4.3 Pipeline Composition and Invariants

The protocol-level production and transfer mechanisms form a symmetric, closed-loop cryptographic composition. An artifact generated and signed by the Sign Pipeline within an originating execution runtime serves as the direct, immutable input to the Share / Rehydrate Pipeline of an external, receiving runtime. Both independent environments evaluate and verify the exact same canonical byte array against the same asymmetric public-key identity structure.

Copying exact signed bytes preserves their signature. Usable recovery still depends on supported formats, trusted keys, accessible ancestry and compatible client tools.

#### 4.3.1 Canonical Sign Pipeline Specification
The transformation of raw cognitive data into a sealed, verifiable state object proceeds through the following deterministic execution path:

```text
[Raw Semantic Content]

EMBED           ──► Generate High-Dimensional Vector v ∈ ℝᵈ
QUANTIZE        ──► Apply TurboQuant Scalar Compression to v_q ∈ ℤ_𝘲ᵈ
ENCAPSULATE     ──► Bind Content, v_q, Type Meta, and Parent CID
CANONICALIZE    ──► Serialize Structure to Deterministic cCBOR
HASH            ──► Compute Content Identifier (CID) via BLAKE3
SIGN            ──► Sign the COSE_Sign1 signature structure with Ed25519
PERSIST         ──► Write Sealed Envelope to Distributed Storage Layers
RECALL          ──► Query and Retrieve via Semantic Search (today: f32 cosine, local SQLite)
AUDIT           ──► Out-of-Band Verification against Producer, Lineage, and Ledger Anchors
```

#### 4.3.2 Canonical Share / Rehydrate Pipeline Specification
Moving an isolated, cryptographically sealed artifact across an arbitrary trust boundary into an active runtime context requires transiting the following sequential isolation gates:

```text
[COSE_Sign1 Envelope + Capability Token κ]

HANDSHAKE       ──► Peer Authentication & Ephemeral Diffie-Hellman Key Exchange
AUDIT           ──► Validate Authorship, Integrity, Lineage, and Anchors (Fail ──► ⊥)
FILTER          ──► Prune Artifact Collection Based on Capability Scope Token κ
RANK            ──► Compute Fast Integer Dot Products over Quantized Vector v_q
DECOMPRESS      ──► Reconstruct Selected High-Probability States to Raw float32 Precision
FORMAT          ──► Unroll Structural Attributes into Deterministic Object Formats
FRAME           ──► Wrap Formatted Semantic Payload inside Secure Isolation Markers
INJECT          ──► Pass Hydrated and Isolated Memory directly to Model Context Window
```

The two pipelines compose: an artifact signed in one runtime is the input to a share/rehydrate flow that hands it to another, and both flows verify the same canonical bytes against the same producer identity. This composition is what gives the protocol portable memory as a property rather than as an aspiration.

Compression of embeddings serves portability and durable-storage anchoring, not the local recall path: shrinking embeddings keeps artifact metadata cheap to carry across systems and to anchor. The reference implementation uses TurboQuant scalar quantization [[1]](#ref-1); the specific scheme, bit width, and recall implementation are documented separately as implementation choices.


#### 4.3.3 Data Quantization and Memory Topology
Vector embedding quantization is fundamentally designed to optimize data portability and reduce distributed ledger anchoring costs, rather than to serve localized cache recall paths. Compressing high-dimensional floating-point vectors down to bounded, low-bit integer coordinate representations minimizes the overall structural metadata footprint of each artifact envelope. This optimization makes it economically viable to transmit massive memory streams across peer-to-peer networks and batch state commitments into public, immutable consensus engines.

The protocol reference architecture utilizes TurboQuant Scalar Quantization to enforce these operational bounds. The specific parameters of this compression scheme—including target bit-width fields, centroid clustering parameters, and localized vector distance calculation routines—are decoupled from the invariant layer of the protocol contract and treated as customizable runtime engine configurations.



*Implementation status:* the current recall path ranks memories by cosine similarity over **uncompressed f32** embeddings in local SQLite. Ranking over quantized vectors ($v_q$) is a research track. See [decompression-fidelity.md](./research/decompression-fidelity.md) for the measured results (§13.2). The Share / Rehydrate Pipeline (§4.2, §4.3.2) is not implemented.

## 5. Architecture Overview

The Rust core and SDK implement artifact cryptography and verification.
Client storage/index interfaces retain and recover artifacts; operator services handle authenticated delivery, discovery and operational metadata.
Interfaces have distinct capabilities. There is no universal backend trait implemented by every client surface.

### 5.1 Protocol Layer

Supported formats define serialization, signature verification, author identity, encryption and parent bindings.
A caller checks original bytes under an independently trusted author key.
Copying those bytes to another backend preserves their signature, but does not establish availability or authorize a new reader.
Formal capability tokens and the five cognitive schemas remain proposals.

### 5.2 Integration Surfaces

The SDK provides client preparation and recovery APIs. The CLI adds local identity and ciphertext persistence.
MCP exposes HTTP and stdio operations; legacy server-prepared methods have different privacy and storage contracts.
Browser and extension compatibility must be validated for their actual built version.
See [source capabilities](./source-capabilities.md) for the supported surface and artifact combinations.

### 5.3 Implemented Storage Adapters

The SDK declares adapter capabilities explicitly: fetch, upload, discovery and retention/consistency descriptions.
Its `ArweaveStorageAdapter` supports bounded fetch; it does not upload or bypass ingestion payment gates.
Its `HttpObjectStorageAdapter` supports immutable object PUT and fetch under a configured origin.
Both enforce 1 MiB objects and a 10-second deadline; arbitrary locator hosts and redirects are rejected.
The object server must enforce its own access and retention policy.

A2A migration verifies original envelopes and copies them unchanged.
An owner-signed locator manifest binds the checkpoint, artifact authors, identifiers and exact-envelope digests to destination locators.
The standalone restore API supports those manifests; generic import still follows the Arweave gateway route.
No general-purpose storage trait implies support for every provider or artifact kind.

### 5.4 Supported Backend Matrix

| Backend | Source support | Boundary |
|---|---|---|
| Client local storage/index | Retain original artifacts and rebuild local indexes | Default SDK caches are session-only |
| Arweave/Irys | Ingestion upload/read-back; separate provider-specific discovery | Availability and index lag are external dependencies |
| Configured HTTP object origin | A2A exact-byte migration and destination restore | No deployed service, retention SLA or discovery adapter supplied |
| Historical Solana memo source | Legacy verification/discovery | No mandatory memo on new shared ingestion |
| IPFS/Filecoin/other providers | Design examples only | No supported adapter claimed |

See [storage portability](./storage-portability.md) for exact capabilities and routing restrictions.

### 5.5 Separation of Consensus Layers

**Design discussion:** batching and consensus inclusion proofs below are not general-memory delivery requirements.

Public ledger anchoring is optional. A client can verify a retained signed envelope against an independently trusted author key without consulting a public ledger.

Ledger anchors are selectively applied based on specific operational constraints:

* **Anchor Enforced:** When state existence at a precise temporal index must be audited by an adversarial party without relying on operator trust.
* **Anchor Bypassed:** Transient intra-session notes, volatile working variables, and temporary script variables bypass the ledger entirely to eliminate latency.
* **Anchor Batched:** Groups of low-value episodic frames are bundled into a single consensus commitment to minimize cost overhead.

---

### 5.6 Current External Delivery

Shared ingestion validates the signed artifact and any external parent before payment or upload.
It binds a durable operation to the author, original envelope digest, backend and locator.
It uploads original bytes, fetches the locator, and compares exact bytes before reporting verified delivery.
Receipt persistence is reported separately from external delivery.
New writes do not require a Solana memo.

A signature or successful storage fetch does not establish a trusted timestamp.
Historical Solana evidence can be checked separately when present.
A failed delivery does not imply no payment occurred: payment acceptance and delivery are independent states.
See [payment retries](./artifact-payment-retries.md).

The following batching/finality models are design proposals, not the current shared coordinator's state machine.

#### 5.6.1 Lineage-Driven Merkle Batching

The lineage DAG naturally functions as an implicit cryptographic tree topology. A batch commitment root is synthesized by assigning target child CIDs as structural parents within a derived coordination artifact:

$$\text{CID}(R_{\text{batch}}) = \text{BLAKE3}\left(\text{cCBOR}\left(\text{schema} = \text{"batch.root"}, \text{parents} = \bigcup_{i=1}^n \text{CID}(A_i)\right)\right)$$

Any structural modification to a leaf artifact invalidates the cryptographic path cascading to the batch root hash. Only the resulting `BatchRoot` identifier is anchored to the consensus ledger; leaf inclusion is verified via log-time ancestral path proofs.

#### 5.6.2 Verification Finality States

Because anchoring processes run asynchronously across distributed validation environments, artifacts explicitly transition through defined execution states:

1. `SignedUnanchored`: Cryptographically valid authorship; no public consensus timestamp.
2. `AnchorPending`: Commitment transaction broadcast to network mempools.
3. `Anchored`: Inclusion verified via a valid cryptographic consensus proof $\pi$.
4. `AnchorFailed`: Transaction drop or block reversion; fallback to local state.

---

### 5.7 Protocol Economics

Local verification of retained bytes requires no protocol transaction or account.
Clients and operators still incur computing, storage and network costs.
Service operators can publish quotes and quota rules; this paper promises no fixed price or free allocation.

The shared prepared-memory/A2A coordinator supports the Universal Paywall exact rail.
Other rails fail closed on that path. Legacy callback interfaces retain separate migration behavior.
An accepted payment can coexist with retryable or terminal delivery failure.
Retries use the same original bytes and durable operation, quote and provider receipt.
These replay guarantees depend on retained financial records and provider idempotency.
Record loss requires reconciliation, not a new charge by default.
Terminal failure records a pending operator remedy; no automatic refund or service credit is implemented.

New operation rows contain metadata rather than artifact payloads.
Historical paid staging is retained until identical client resubmission drains its payload columns.
An upgraded database cannot be described as payload-free before that drain and operational retention review.

### 5.8 Current Execution Lifecycles

- **Prepare:** construct, optionally encrypt, and sign locally; retain original bytes before requesting delivery.
- **Deliver:** validate, resolve payment/quota, upload and verify exact fetched bytes; report financial and persistence outcomes separately.
- **Recall:** use the appropriate local or legacy public index. SDK sealed recall requires a caller-supplied local embedder.
- **Recover:** verify envelopes and trusted ancestry, open locally with required keys, then rebuild the client index.
- **Migrate:** for supported A2A artifacts, copy exact envelopes and authenticate new locators without changing signed parent bindings.

## 6. Artifact Serialization and Object Model

**Design boundary:** the cognitive-schema registry below is proposed. Current `MEMORY_V1`, `SEALED_V1` and A2A schemas remain authoritative in source; no automatic five-schema migration is claimed.

Each supported artifact format defines its serialization and verification rules. The proposed schema model below must not replace current signed field order or identifier semantics.

### 6.1 The Schema Registry Matrix

The protocol organizes all historical context states into a strongly typed schema ledger. These schemas isolate core **Cognitive Memory Layers** from auxiliary **Metacognitive Context Layers**:

```text
[Canonical Schema Spaces]
   ├── Cognitive Memory Layers
   │     ├── memory.episodic    ──► Sequential Event Logs & Turn Interactions
   │     ├── memory.semantic    ──► Factual Knowledge & Structured World Assertions
   │     ├── memory.procedural  ──► Learned Routine Workflows & Tool Schemas
   │     ├── memory.working     ──► Transient Subgoals & Scratch States
   │     └── memory.identity    ──► Persona Attributes & Operational Constraints
   └── Metacognitive Context Layers
         ├── rag.context        ──► Extracted Source Context Bundles
         ├── rag.result         ──► Generated Inferences Linked to Source Context
         ├── agent.state        ──► Complete Volatile Runtime Snapshots
         ├── receipt            ──► Execution Attestations & Verification Proofs
         └── capability.token   ──► Signed Ancestral Subtree Access Authorizations

```

*Implementation status:* the five cognitive schemas are a design. They are not implemented. General memory uses `MEMORY_V1` and `SEALED_V1`; A2A has separate typed envelopes. A future engine would map legacy `memory` blocks to `memory.episodic`.

---

### 6.2 Deterministic Encoding and Serialization Strategy

An artifact's raw information is completely invariant across versions. Any structural modifications, field insertions, or attribute deprecations dictate an explicit schema version iteration, preventing state drift within established historical sequences.

The proposed model uses deterministic serialization to produce matching bytes from matching inputs:

```text
[Raw Payload Attributes]

LEXICOGRAPHICAL SORT ──► Order Dictionary Map Keys by Byte Value (K_i < K_i+1)
CANONICALIZE         ──► Transform Attributes into Deterministic cCBOR Layout
HASH                 ──► Execute BLAKE3 over Canonical Bytes to Generate CID

```

1. **Lexicographical Map Sorting:** All dictionary keys ($K_i$) within the artifact header and payload blocks are explicitly sorted by their literal byte sequence value prior to transport encoding:

$$K_i < K_{i+1}$$

2. **Canonical Bit Allocation:** The sorted structure is written directly to the wire format using the **Concise Binary Object Representation (CBOR)** specification outlined in Request for Comments (RFC) 8949 Section 4.2. This rule eliminates non-deterministic data variables (such as variable-length integer byte packing or unstable floating-point layouts).
3. **Content Identifier (CID) Derivation:** The resulting byte block ($S_{\text{canonical}}$) serves as the exclusive input parameter for the hashing engine, yielding an unalterable structural reference:

$$\text{CID}(A) = \text{BLAKE3}(S_{\text{canonical}})$$

Through this design, the Mnemonic Object Model transitions from a basic variable cache into a hardened, multi-layered attestation framework. The system securely binds the operational lineage of agent workflows—weaving together cognitive role configurations, retrieved vector sources, execution state receipts, and tokenized authorization parameters into a cohesive, cryptographically verifiable history.


## 7. Memory Composition and Multi-Runtime Sharing

While the fundamental serialization rules establish the layout of an isolated memory artifact, the execution of multi-runtime workflows requires a rigorous framework for composition and cross-boundary transport. This section specifies the mechanisms governing cognitive state policy enforcement, decentralized authorization via cryptographic capabilities, secure runtime handshakes, and safe rehydration boundaries designed to neutralize semantic exploit vectors.
 For data fields, execution sequences, and mathematical equations see [Memory Composition and Sharing Specification](./spec/memory-composition.md).

---

**Implementation status of this section:**

| Subsection | Status |
|---|---|
| §7.1 Five cognitive schemas | Design; not a required field of every current artifact |
| §7.2 Scoped/revocable capability tokens | Design; delivered decryption keys cannot be revoked from their recipients |
| §7.3 Multi-step sharing handshake | Design; existing cryptographic wraps and A2A embedded grants do not implement the full handshake |
| §7.4 Full rehydration pipeline | Design; current recovery verifies, opens and rebuilds supported local indexes |
| §7.5 Safe-injection framing | Design; model behavior is not guaranteed |
| §7.6 Portability | Original signatures survive byte-preserving copying; supported adapters and recovery inputs still matter |

Current client-prepared sealed content stays encrypted at the operator.
Hosted sealed storage, recall-session creation and new general-grant publication are retired (HTTP 410).
SDK private recall uses a supplied local embedder; CLI general sealed recall is unsupported.
Legacy public/owner recall remains a separate server-index contract, not independent discovery or private sealed search.

A2A supports embedded recipient grants and completed sealed streams.
General grant reads remain, but the new shared coordinator does not publish independent grant artifacts.
See [source capabilities](./source-capabilities.md) rather than inferring surface support from the proposed protocol below.

### 7.1 Cognitive Typing Semantics

The explicit classification of memory artifacts into five discrete primitive categories (`memory.episodic`, `memory.semantic`, `memory.procedural`, `memory.working`, `memory.identity`) represents a core semantic contract rather than metadata labeling. Each category dictates a distinct operational lifecycle profile:

| Cognitive Kind | Retention Boundary | Retrieval Score Weight | Access Authorization Posture | Injection Vulnerability Vector |
| :--- | :--- | :--- | :--- | :--- |
| `memory.working` | Ephemeral (Session Bounded) | High Recency Bias | Restricted to Executing Thread | Minimal |
| `memory.episodic` | Linear Append-Only Log | Balanced (Cosine Similarity) | Selectively Shared via Capability | Moderate (Untrusted Data Input) |
| `memory.semantic` | Multi-Generational | High Semantic Density | Open Across Authorized Orgs | Moderate (Extracted Context) |
| `memory.procedural` | Version-Controlled Lifecycle | Functional Pattern Match | Immutable Attestation Paths | High (Executable Tool Logic) |
| `memory.identity` | Permanent Operator Invariant | Absolute Precedence | Owner Sovereign (Non-Delegable) | Critical (System Instruction Hijack) |

Because the cognitive role is locked inside the signed artifact envelope, downstream execution engines parse type classifications natively, enforcing appropriate handling policies without out-of-band coordination.

---

### 7.2 Cryptographic Capability Tokens

Cross-runtime data synchronization is authorized non-interactively using **Capability Tokens** (`capability.token`). A capability token functions as a standalone, content-addressed artifact signed by the data owner's public key. The structural boundary of a token is defined as:

$$\kappa = \langle \text{Subject}, \mathcal{S}_{\text{scope}}, \mathcal{P}_{\text{perms}}, T_{\text{exp}}, \Sigma_{\text{issuer}} \rangle$$

Where $\mathcal{S}_{\text{scope}}$ defines explicit access constraints mapping over specific lineage subtrees, cognitive categories, or metadata attribute filters.

To maintain low-latency out-of-band execution, the protocol optimizes for short-lived tokens governed by a strict Time-to-Live ($TTL$) constraint, mitigating the need for global, real-time online validation checks. When long-lived access authorizations are required, the token payload forces an explicit network policy rule requiring nodes to evaluate token hashes against an immutable ledger revocation map or cryptographic nullifier accumulator set ($\mathcal{R}$):

$$\text{Valid}(\kappa) \iff \text{Verify}(\Sigma_{\text{issuer}}) \equiv \text{True} \;\wedge\; \text{CurrentTime}() < T_{\text{exp}} \;\wedge\; \text{CID}(\kappa) \notin \mathcal{R}$$

---

### 7.3 The Trust-Boundary Sharing Handshake

The proposed, unimplemented sharing handshake would authenticate participating runtimes and protect transit. Its intended properties are:

1. **Mutual Identity Authentication:** Exchange and verification of asymmetric peer keypairs via decentralized identifier schemas.
2. **Dynamic Scope Intersection:** Calculation of the active read boundary, computed as the strict intersection of the token's explicit capability scope ($\mathcal{S}_{\text{scope}}$) and the sender's real-time localized disclosure policies.
3. **Ephemeral Tunnel Confidentiality:** Derivation of a symmetric encryption key via an ephemeral **Elliptic-Curve Diffie-Hellman (ECDH)** key exchange to protect data bytes in transit.

Upon completion, both nodes sign a mutual transfer receipt artifact which is appended directly to the lineage Directed Acyclic Graph (DAG), providing a clear audit trail of the transfer event.

---

### 7.4 The Deterministic Rehydration Pipeline

Ingested artifacts migrating into a target execution environment must transit a linear compilation pipeline. This process is strictly deterministic and repeatable: given identical inputs and state parameters, separate node implementations generate matching memory configurations.

```text
[Ingested Signed Envelopes]

VERIFY      ──► Validate Authorship Signatures, Lineage Links, and Consensus Proofs
FILTER      ──► Prune Artifact Collections Outside of Active Capability Scope κ
RANK        ──► Score Vector Proximity using Accelerated Integer Dot Products over v_q
DECOMPRESS  ──► Hydrate Selected Top-M TurboQuant Vectors back to float32 Precision
FORMAT      ──► Map cCBOR Struct Attributes into Target Text Templates
FRAME       ──► Add reference-data framing markers (model compliance is not guaranteed)
INJECT      ──► Push Securely Framed Memory block directly to the Active Model Context

```

This proposed sequence delays decompression until formatting. It does not describe the current local recall implementation, which ranks uncompressed embeddings.

---

### 7.5 Safe Injection (Context Framing)

Because untrusted historical memory sequences can easily mimic instructions, naive concatenation of historical text strings directly into an LLM's context window exposes the target entity to control-flow hijacking. The proposed framing operator labels reference data at the rehydration boundary; it does not prevent a model from obeying malicious content:

$$f_{\text{frame}}(D, R) = \big[ \text{Tag}_{\text{begin}}(R, \alpha) \;\parallel\; D \;\parallel\; \text{Tag}_{\text{end}}(R) \big]$$

The markers request that a consuming model treat the enclosed text as reference data. Their effectiveness and any role-specific policy require independent evaluation.

Framing requires cooperation from the consuming runtime. The proposed attestation would record a claim about processing, not prove safe model behavior or assign responsibility for an exploit.

---

### 7.6 Continuous Cross-Runtime Portability

Signed authorship and parent bindings belong to the artifact rather than the service that stores it. Verification can be repeated after migration.

Original signatures remain valid when exact bytes move. Recovery still requires supported formats, accessible ancestry, trusted checkpoints and keys. A new runtime may need to rebuild or re-embed its local search index; complete execution-state migration is not provided.


## 8. Cryptographic Trust Model and Security Boundaries

The current implementation verifies signatures and supported parent/encryption bindings.
Capability-token delegation, safe framing and the full sharing handshake are separate proposals, not assumptions of the current verifier.

### 8.1 Implemented Security Boundaries

- **Integrity:** envelope verification detects changes to signed bytes under the expected key.
- **Authorship:** callers independently pin trusted authors; embedded keys alone do not establish trust.
- **Parent validation:** A2A verifies signed parent hash, context and eligible author/recipient before child delivery.
- **Memory graph binding:** generic memory recovery requires author and exact-envelope pins for ancestors as well as heads.
- **Private preparation:** the client-prepared sealed request excludes plaintext and decryption keys. Public fields, sizes and access timing remain visible.
- **Storage independence:** exact copied envelopes retain their original signatures. Supported routing and byte availability remain operational prerequisites.
- **Recovery bounds:** source errors, missing parents and forks are reported. Completeness is relative to trusted heads only.

COSE signatures authenticate signed structures, not arbitrary bare IDs.
No deployed tokenized-isolation, dual-signed sharing receipt or general batched-timestamp guarantee is implied.

### 8.2 Explicit Non-Guarantees

Signatures do not prove semantic truth, possession of an uncompromised key, or a trusted creation time.
An index may omit new heads; exhaustion cannot establish global completeness.
A signed timestamp is an author's claim unless separately supported by validated external evidence.

General sealed memory uses XChaCha20-Poly1305 content encryption and HPKE recipient wraps with key commitment checks.
The verifier accepts supported canonical CBOR inner memory and the existing SDK JSON representation.
A2A sealing has its own signed binding and recipient-card validation contract.
See [sealed A2A](./sealed-a2a.md) for its completed-stream boundaries.

Required decryption keys cannot be recovered from signatures alone.
Readers who already obtained keys or plaintext can retain copies; grants are not retroactively revocable.
Hosted decryption sessions are retired. Legacy server-prepared signing still exposes content to that operator before signing.

Current recovery does not impose prompt-injection framing or prove that embeddings or search results are truthful or complete.
It does not restore live process state, resolve all multi-writer conflicts, or guarantee downstream model behavior.
Financial-state loss is separate from recoverable memory-state loss.

### 8.3 Extension Boundary

Future formats and backends need explicit versioning, declared capabilities and acceptance evidence.
Current signatures do not establish compatibility with an unimplemented extension.

## 9. Proposed ERC-8004 Integration

**Unimplemented design:** this section is a historical integration proposal, not a deployed registry, interoperability certification, price quote or acceptance result.

*Implementation status:* this section is a design. None of the ERC-8004 paths (§9.2–§9.5) or the `did:mnemonic` method are implemented. The external specification is maintained separately at [EIP-8004](https://eips.ethereum.org/EIPS/eip-8004); this paper does not certify its current status or compatibility.

The Mnemonic Protocol is engineered with the explicit intent to extend the **ERC-8004 ("Trustless Agents")** framework, serving as its definitive off-chain **Signed-Memory and Lineage Trust Extension**. By interfacing directly alongside decentralized identity singletons, machine micropayment protocols, and peer-to-peer messaging layers, the protocol introduces a fully compatible, content-addressed state-plane substrate to the Web3 agent ecosystem.

### 9.1 Technical Division of Labor and Core Thesis

The ERC-8004 standard defines three public, on-chain registries to govern decentralized machine networks: **Identity** (ERC-721 tokenized credentials), **Reputation** (subjective network performance signals), and **Validation** (consensus-driven work audits). To retain strict execution efficiency and gas cost viability on execution layers, ERC-8004 leaves long-term context retention, vector-space indexing, and context window isolation out of scope.

Mnemonic introduces a critical fourth trust category: **Cryptographically Verifiable Agent Memory**. An agent’s historical memory—what it learned, when, from which data ingredients, and under whose authority—represents a more robust security signal than flat network uptime metrics or subjective client ratings.

```text
[THE FOUR PILLARS OF ARCHITECTURAL SYNCHRONIZATION]

  ERC-8004 ON-CHAIN LANDSCAPE             MNEMONIC OFF-CHAIN STATE PRIMITIVE
 ┌──────────────────────────────┐       ┌──────────────────────────────┐
 │       Identity Registry      │ ────► │     Sovereign Operator ID    │ (did:mnemonic resolve
 │ (Who the agent is on-chain)  │       │     (Registration File Card) │  to asymmetric keys)
 ├──────────────────────────────┤       ├──────────────────────────────┤
 │      Reputation Registry     │ ────► │    Evidence-Based Feedback   │ (Attested feedback links
 │ (How other nodes rate it)    │       │    (Non-Repudiable Logs)     │  to source CIDs)
 ├──────────────────────────────┤       ├──────────────────────────────┤
 │      Validation Registry     │ ────► │  Lightweight Vector Audit    │ (Out-of-band execution of
 │ (Whether its work was checked)│      │  (Deterministic Lineage Check)  pipeline verifications)
 └──────────────────────────────┘       └──────────────────────────────┘

```

The unified division of labor is unambiguous: **ERC-8004** makes autonomous agents discoverable and rateable on the network plane; **Mnemonic** allows those same agents to cryptographically prove what they remember, produced, used, and shared over time.

---

### 9.2 Path P1: The Validation Registry — Mnemonic as a Cryptographic State Oracle

Unlike compute-heavy stake-secured re-execution or hardware-dependent Trusted Execution Environment (TEE) validation tracks, memory validation relies entirely on pure, low-overhead cryptography. It would validate the supplied history relative to independently trusted heads, without proving global completeness.

#### 9.2.1 On-Chain Request Interface

When an agent submits an output trajectory for audit under an ERC-8004 workflow, it commits a validation transaction targeting the asymmetric Mnemonic Validator Singleton:

```solidity
validationRequest(
    validatorAddress = 0xMnemonicValidatorAddress,
    agentId = 42,
    requestURI = "ipfs://bafybeiccanonicalmemorybatch...",
    requestHash = keccak256(canonical_cbor_payload_bytes)
)

```

#### 9.2.2 Off-Chain Validation Calculus

The decentralized cluster of Mnemonic Validator Nodes processes the transaction out-of-band, executing an optimized Rust engine verification loop to assess the memory block against a strict set of architectural checks:

1. **Signature Provenance:** Enforces that every sequential `COSE_Sign1` data block resolves to a valid cryptographic key identifier (`did:key` or `did:sol`).
2. **Content Hash Determinism:** Recomputes the **BLAKE3** hash over the lexicographically sorted Concise Binary Object Representation (CBOR) payload to confirm byte-level data integrity.
3. **Lineage Graph Continuity:** Confirms that ancestral parent references contain no gaps, unlinked leaf groupings, or Directed Acyclic Graph (DAG) loops.
4. **Temporal Monotonicity:** Verifies that internal artifact metadata timestamps increase monotonically along the lineage progression vector.

#### 9.2.3 Cryptographic Finality Scoring

To safeguard the system against adversarial exploits, cryptographic validation acts as a binary gate for state corruption. If any signature fails or a payload mismatch is identified, the evaluation drops instantly to a terminal reject state ($\bot$). If cryptographic integrity holds true, the system scores the trace based on contextual completeness and ledger-anchored finality:

| Numeric Audit Score | Operational Evaluation Meaning |
| --- | --- |
| **`100`** | Absolute Verification: Signatures, hashes, and lineage paths match perfectly, backed by valid ledger consensus anchors. |
| **`75`** | Latent Validation: Cryptographic integrity is fully confirmed; specific public ledger anchors are currently in flight within network mempools. |
| **`50`** | Segmented Lineage: Basic signatures verify, but interior history gaps or untraceable historical deletion events are detected. |
| **`0`** ($\bot$) | Terminal Core Failure: Forged signatures, corrupted cCBOR structures, or broken hash chains encountered. |

---

### 9.3 Path P2: The Identity Registry — The `did:mnemonic:` Resolution Vector

To bridge Ethereum's on-chain tokenized credentials with off-chain cryptographic memory graphs, the protocol introduces a specialized identifier method: `did:mnemonic:`.

This decentralized identifier maps directly to the ERC-8004 Identity Registry, resolving through an agent's on-chain token state to locate its external `agentURI` file card. The discovery document updates its standard `services` array and `supportedTrust` metrics to explicitly advertise its data-plane capabilities to network crawlers:

```json
{
  "services": [
    {
      "name": "Mnemonic",
      "endpoint": "https://mcp.mnemonik.xyz/mcp",
      "version": "v0.2",
      "capabilities": ["sign", "recall", "verify", "anchor"]
    }
  ],
  "supportedTrust": ["reputation", "tee-attestation", "signed-memory", "lineage-attestation"],
  "mnemonic": {
    "public_key": "8xGzM8F7k...Kp9",
    "attestation_count": 156,
    "last_anchor_slot": 123456789
  }
}

```

#### 9.3.1 ERC-721 Ownership Transfer Invariant

If an ERC-8004 Agent Identity NFT is transferred to a new wallet address on-chain, the historical memory graph forks cleanly at the exact block height of the transfer transaction. The new operator inherits the historical ancestral root as an immutable reference foundation, but cannot retroactively modify prior records because they lack the previous operator's private signing key.

---

### 9.4 Path P3: The Reputation Registry — Evidence-Based Attestation Loops

Standard reputation ranking frameworks are vulnerable to Sybil manipulation, fake review insertion, and arbitrary down-voting. Mnemonic transitions the ERC-8004 Reputation Registry into an objective, evidence-backed verification loop.

When a client submits an execution rating or performance signal to the registry, it must include a signed `mnemonic_attestation` payload block. This metadata explicitly binds the rating to the exact content-addressed data nodes generated during the task execution sequence:

```json
{
  "tag1": "memory-verified",
  "mnemonic_attestation": {
    "feedback_artifact_cid": "bafybeifb...",
    "cose_signature": "dGVzdF9zaWduYXR1cmU...",
    "signer_did": "did:mnemonic:sol:8xGzM8F7k...",
    "artifacts_used": ["blake3:4a8f9c...", "blake3:9e2b1c..."]
  }
}

```

An adversarial node attempting to pollute the reputation registry must generate authentic `COSE_Sign1` envelopes and unbroken lineage links that track to real-world context inputs. This requirement significantly increases the economic and computational cost of execution spoofing.

---

### 9.5 Infrastructure Cost and Settlement Mechanics

The multi-tiered integration plan coordinates its network payment routines using the **x402 Internet-Native Payment Standard**, tracking transaction execution costs across three distinct economic actors:

* **State Generation (The Operator):** Memory signing and local index querying remain free when self-hosted, or incur tiny USDC micro-fees when routed through dedicated cloud nodes.
* **Ledger Consensus Anchoring (The Operator):** The marginal cost of writing public slot commitments is minimized via Merkle tree batching inside the lineage DAG, shifting ledger settlement expenses to an asynchronous optimization background path.
* **Trust Validation Auditing (The Agent):** When an autonomous agent requires an official validation score logged to the ERC-8004 schema layer to unlock an escrow account or win a high-value task route, the agent pays a competitive micro-fee (~100$\mu$USDC per artifact) to the verifying validator nodes.


## 10. Use Cases

Mnemonic supports a family of agent-memory patterns. Each item below links to a deep-dive document. For data fields, execution sequences and equations, see [Usecases](./usecases.md). Most of these patterns depend on sharing (§7), which is not implemented yet.

1. [Shared Memory Layer](./usecases/shared-memory-layer.md)
2. [Provenance and Attestation Layer](./usecases/provenance-attestation-layer.md)
3. [Trust and Reputation Layer](./usecases/trust-reputation-layer.md)
4. [Portable Memory Wallet](./usecases/portable-memory-wallet.md)
5. [Settlement-Aware Memory Infrastructure](./usecases/settlement-aware-memory-infrastructure.md)
6. [Task Memory Ledger](./usecases/task-memory-ledger.md)
7. [Shared Project Memory Namespace](./usecases/shared-project-memory-namespace.md)
8. [Artifact Attestation Service](./usecases/artifact-attestation-service.md)
9. [Agent Continuity Layer](./usecases/agent-continuity-layer.md)
10. [Reliability Oracle for Orchestration](./usecases/reliability-oracle-for-orchestration.md)


## 11. Relationship to Storage and Search

Vector indexes serve retrieval; signed artifacts serve provenance and integrity checks.
A client can rebuild an index from supported verified artifacts without treating the old index as authoritative.
Different embedding models can require re-embedding rather than reuse of old vectors.

External storage supplies bytes. It does not establish the expected author or completeness of a recovered history.
Discovery services supply candidate locators that must be verified before import.
The currently implemented backend routes are narrower than the architectural examples in this paper.
ERC-8004 integration remains a proposal under §9.

## 12. Current Source Status

The [source capability matrix](./source-capabilities.md) is the implementation boundary at `26a9550`.
It distinguishes artifact kinds, client operations, backend adapters and unsupported routes.
Source support is not a claim about published packages or a hosted deployment.

### 12.1 Implemented Scope

- Rust core and WASM verification, signing, encryption and supported artifact recovery.
- CLI local sealed persistence; SDK client preparation and caller-owned index integration.
- Shared prepared-memory and A2A delivery with exact read-back, metadata receipts and durable financial operations.
- Independently authenticated checkpoints and encrypted client backups.
- Bounded A2A discovery, verified import, recipient opening, completed streams and external-parent continuation.
- A2A exact-byte storage migration through declared Arweave-fetch and configured HTTP-object adapters.
- Historical Solana verification/discovery, legacy public indexing and server-prepared signing with explicit privacy boundaries.

MCP transport and tool exposure depend on the build and deployment.
Use the [tool reference](./tools.md) and deployed capability response; do not infer capabilities from a fixed tool count in a paper.
Hosted local writes are rejected. The client's default local sealed cache is not hosted persistence.

### 12.2 Unsupported or Design-Only Scope

The five cognitive schemas, formal capability tokens, full sharing handshake, safe-injection framing and general memory batch anchoring remain proposals.
No automatic legacy-to-cognitive-schema conversion is claimed.
General-memory migration, independent grant migration and actual SSE stream migration are outside the current locator-manifest API.
Published-package drills, live provider discovery/retention and deployment acceptance require separate recorded evidence.

## 13. Empirical Evaluation and Performance Metrics

This section reports only measured values. Each value names the benchmark that produced it. Values that nobody has measured yet are marked "not measured".

**Test machine:** shared cloud virtual machine, 4 vCPU, Intel Xeon @ 2.10 GHz, Linux. Measured on 24 September 2026 with `criterion` (release profile). A shared machine adds noise; treat differences below ~10% as noise.

---

### 13.1 Serialization, Hashing and Signing Latency

Source: `cargo bench -p mnemonic-core --bench cbor_codec`. Median time per operation:

| Step | 100 B content | 500 B | 2,000 B | 10,000 B |
| :--- | :--- | :--- | :--- | :--- |
| Canonical CBOR (`to_canonical_cbor`) | 1.06 µs | 1.25 µs | 1.53 µs | 1.43 µs |
| BLAKE3 hash of the CBOR bytes | 0.35 µs | 0.66 µs | 1.21 µs | 3.77 µs |
| COSE_Sign1 + Ed25519 sign | 25.6 µs | 27.9 µs | 33.1 µs | not measured |
| Full pipeline (CBOR + hash + sign) | 20.8 µs | 20.9 µs | 30.5 µs | not measured |

The full pipeline is faster than signing alone for small inputs. This is within the noise of the shared machine. The main result: one memory is encoded, hashed and signed in about 20–35 µs. Verification (hash recompute + signature check) is **not measured** yet.

---

### 13.2 TurboQuant Compression and Retrieval Fidelity

Sources: `cargo bench -p mnemonic-core --bench decompress_fidelity` (synthetic) and `--bench decompress_fidelity_real --features local-embed` (real). Method and full tables: [decompression-fidelity.md](./research/decompression-fidelity.md). "Top-K recall" is the overlap of the top 10 results before and after compression.

**Real embeddings** — model `all-MiniLM-L6-v2` (384 dimensions), 60 sentences, 10 queries:

| Bits per dimension | Size reduction | Mean cosine | Top-10 recall |
| :--- | :--- | :--- | :--- |
| 4 | 7.68× (87%) | 0.974 | **94%** |
| 3 | 10.11× (90%) | 0.919 | 91% |
| 2 | 14.77× (93%) | 0.787 | 83% |

**Synthetic worst case** — random uniform vectors, 1,536 dimensions: 4-bit gives 7.92× and **80%** Top-10 recall. Random vectors have many near-ties, so this is a lower bound.

The real corpus is small. A standard benchmark set (MTEB or BEIR) is needed for a headline number.

**Effect on the product today:** none. Recall ranks over uncompressed f32 embeddings (§4.3.3). Compressed bytes serve as proof of existence only.

---

### 13.3 External Delivery Cost and Latency

Current prepared delivery uploads original bytes and verifies read-back; it does not add a mandatory Solana memo.
Production delivery latency and end-to-end settled cost are not established by the local drills.
Prices and quota depend on the configured operator and accepted quote.
Historical pricing formulas are not a current user-facing quote or a promise about every payment rail.

Batch anchoring remains a separate design proposal. It does not describe the shared ingestion coordinator.

---

### 13.4 x402 Payment Overhead

**Not measured.** An earlier draft gave 14 ms and 42 ms. No benchmark or log in the repository supports those numbers, so they are removed.

---

### 13.5 Fault Handling (tested behaviour)

These statements are covered by automated tests:

* **Tampered payload:** changing bytes inside a COSE_Sign1 envelope makes verification fail (`core/tests/integration_cbor.rs`, `test_tampered_cose_detected`).
* **Lineage cycles:** writing an artifact that would create a cycle is refused with `CYCLE_DETECTED` (`core/src/lineage/mod.rs`).
* **Failed delivery:** payment and delivery status remain separate. Retry uses the original envelope and durable operation; settled payment can coexist with failed delivery. Terminal failures need an operator remedy. See [payment evidence](./artifact-payment-retries.md).
* **Recovery and migration:** local SDK/WASM HTTP drills cover SQL loss, exact-byte destination recovery and independent operator continuation. External services are mocked; see [acceptance evidence](../work/sealed-memories/a2a-acceptance-reconciliation.md).

---

## 14. Limitations and Open Questions

### 14.1 Key Loss and Retained Copies

Deleting a key can prevent future opening only when no usable copy of that key or plaintext remains.
Recipients, backups and external storage can retain copies outside the author's control.
Signatures and public metadata remain visible even when content cannot be opened.
The implementation does not guarantee deletion of all copies or make a legal-compliance determination.

### 14.2 Asynchronous Multi-Writer Consistency and Convergence Semantics

Current A2A recovery preserves forks rather than resolving every multi-writer conflict. Capability-scoped authorization handshakes and convergence rules remain design proposals. Because the protocol relies on immutable records, concurrent mutations cannot utilize destructive multi-master overwrites.

Diverging state traces are modeled as explicit branch splits inside the lineage tree. When two nodes simultaneously publish updates over a common base ancestor, the system requires the generation of a multi-parent merge block:

$$A_{\text{merge}} = \text{cCBOR}\left( \text{schema} = \text{"branch.merge"}, \; \text{parents} = [\text{CID}(A_{\alpha}), \text{CID}(A_{\beta})] \right)$$

Future research tracks focus on refining deterministic topological sorting algorithms and integrating specialized **Observed-Remove Conflict-Free Replicated Data Type (OR-CRDT)** set primitives directly into the `RecallIndex` trait layer. This strategy will enable decentralized agent networks to converge on unified historical orderings without relying on centralized consensus locks.

---

### 14.3 Semantic Disambiguation and Vector Space Poisoning

The protocol's retrieval layer is vulnerable to adversarial **Vector Space Poisoning** strategies. A malicious actor with authorized write permissions to an agent's `memory.episodic` or `rag.context` ledger can inject high-density, repetitive text payloads optimized to map near the central coordinates of critical operational models.

```text
[VECTOR SPACE POISONING MATRIX]

  Normal Coordinate Pool          Adversarial Injection Dense Clusters
 ┌──────────────────────────────┐       ┌──────────────────────────────┐
 │   Sparse, contextually relevant│     │ High-density, uniform vectors │
 │   historical memory points.  │       │ designed to crowd out Top-K  │
 └──────────────────────────────┘       └──────────────────────────────┘
                │                                      │
                ▼                                      ▼
     Standard Recall: Accurate              Poisoned Recall: Agent Blinded

```

During the `rank` and `decompress` phases of the rehydration pipeline, these uniform adversarial clusters crowd out real-world memories, effectively "blinding" the model's attention matrix to its true historical records. Mitigating this risk requires the formulation of advanced multidimensional outlier-detection matrices and structural density-filtering boundaries within the baseline ranking engine.

---

### 14.4 Fact Mutation and Factual Contradiction Management

The processing mechanics for updating `memory.semantic` records remain an active development frontier. When an agent experiences a new interaction that directly invalidates a previously signed factual assertion, treating the old artifact as broken or invalid breaks the historical record.

The protocol does not execute structural updates via in-place state mutation. Instead, fact modifications must be registered as **Causal Supersedence Attestations**. The newer block points directly back to the older artifact's identity hash, adding a structured contradiction signal. The downstream rehydration pipeline is responsible for parsing this conflict trail, giving the model the cognitive context required to resolve semantic state changes at runtime.

---

### 14.5 Framing-Compliance Ecosystem Standards

Safe-injection markers and framing-compliance attestations are unimplemented proposals. Even signed markers would authenticate a framing claim, not prove that a consuming model respected it.

A possible framing-attestation registry would require a separate specification and evaluation across consuming runtimes. No implemented registry, interoperability result or release commitment is claimed here.

---

### 14.6 Cross-Surface Interoperability Testing Matrix

Achieving complete, production-grade interoperability mandates a comprehensive conformance suite capable of testing the full lifecycle across all distribution layers:

* Evaluating data layout execution parity between native Rust environments and compiled **WebAssembly (WASM)** runtimes running within browser sidecars.
* Enforcing uniform trait behaviors and error handling metrics across distinct execution surfaces including the core library, standalone command-line utilities, embeddable SDK blocks, and networked Model Context Protocol (MCP) servers.
* Simulating edge-case failures across decentralized infrastructures, testing node behaviors during permanent storage dropouts, consensus mempool transaction drops, and corrupt local caching events.


## 15. Remaining Work

Live discovery, release artifacts and deployment drills remain separate from passing local tests.
Additional artifact/backend support must declare its capabilities and provide recovery evidence before being described as available.
Formal capability tokens, cognitive schemas, framing, batching and ERC-8004 integration remain design work without release commitments here.
The active [product work](../work/protocol-product/README.md) records scope and acceptance.

## 16. Conclusion

Mnemonic provides signed artifacts, private client preparation and recovery tools that can operate without the original operator's SQL history.
Recoverability depends on retained bytes, keys, independently trusted checkpoints and supported adapters.
Exact-byte migration preserves original signatures; it does not establish global completeness, trusted time, or permanent availability.
Current scope and evidence are listed in [source capabilities](./source-capabilities.md).

---

## References


<a id="ref-1"></a>
1. *[TurboQuant: Online Vector Quantization with Near-Optimal Distortion Rate](https://arxiv.org/abs/2504.19874).*  Zandieh, A. and Mirrokni, V. arXiv:2504.19874.


<a id="ref-2"></a>
2. *[Sublinear Verifiable Recall: An Inverted-File Cascade for Compressed Embedding Retrieval in the Mnemonic Protocol](https://www.researchgate.net/publication/404381758_Sublinear_Verifiable_Recall_An_Inverted-File_Cascade_for_Compressed_Embedding_Retrieval_in_the_Mnemonic_Protocol).*

<a id="ref-3"></a>
3. *[Portable Agent Memory: A Protocol for Cryptographically-Verified Memory Transfer Across Heterogeneous AI Agents](https://arxiv.org/abs/2605.11032).* arXiv:2605.11032.


<a id="ref-4"></a>
4. *[ERC-8004: Trustless Agents](https://eips.ethereum.org/EIPS/eip-8004).*
