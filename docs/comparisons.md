# Comparisons

> Honest framing of how Mnemonic Protocol relates to the adjacent ecosystem. Composable where possible, distinct where it has to be. No strawmen — these are real, well-built projects, and several of them can be used *with* Mnemonic, not against it.

---

## Memory-only systems

These give you persistence; none of them give you third-party-verifiable persistence.

| | letta | zep | mem0 | cognee | **Mnemonic** |
|---|---|---|---|---|---|
| Persistent across sessions | yes | yes | yes | yes | **yes** |
| Semantic recall | yes | yes | yes | yes | **yes** |
| Cryptographically signed | no | no | no | no | **yes (COSE_Sign1)** |
| Content-addressed (hash) | no | no | no | no | **yes (blake3)** |
| Independently verifiable (no vendor required) | no | no | no | no | **yes** |
| On-chain anchor available | no | no | no | no | **yes (Solana SPL Memo)** |
| Open protocol (vs. open-source product) | product | product | product | product | **protocol** |
| Standardized envelope format | proprietary | proprietary | proprietary | proprietary | **canonical CBOR + COSE** |

**When you'd reach for letta / zep / mem0 / cognee:** you want the highest-level "give my agent memory" UX, you're not worried about audit, and you trust the vendor.

**When you'd reach for Mnemonic:** you need someone *outside* your system to be able to verify what was remembered, when, and by whom. Compliance-shaped agents, multi-vendor setups, audit trails, on-chain reputation systems.

**Composable note:** you can run any of letta / zep / mem0 / cognee *on top of* Mnemonic — use them for the agent's working memory and pipe the entries you want to be auditable through Mnemonic. The signed envelopes don't compete with the recall UX; they back-stop it.

---

## Communication-only protocols

These give you message-passing between agents; none of them give you continuity *of* the message-passing.

| | A2A (Google) | MCP (Anthropic) | ACP (IBM/Linux Found.) | **Mnemonic** |
|---|---|---|---|---|
| Inter-agent message-passing | yes | (via tool calls) | yes | no |
| Persistent state across sessions | no | no | no | **yes** |
| Cryptographic provenance | AgentCard JWS only | none in-protocol | none in-protocol | **end-to-end on every memory** |
| On-chain anchoring | no | no | no | **yes** |
| Built-in semantic recall | no | no | no | **yes** |
| Adoption (May 2026) | growing fast | dominant in IDE/agent tooling | early | growing |

**When you'd reach for A2A:** you have multiple agents that need to negotiate task delegation in real time.

**When you'd reach for MCP:** you want any AI client (Claude / Cursor / VS Code) to call your service as a tool with one install.

**When you'd reach for Mnemonic:** you need any of those interactions to be *remembered* in a way someone else can verify.

**Composable note:** Mnemonic's A2A bridge (`work/a2a-bridge/`) turns A2A `Task`/`Message`/`Artifact` events into signed Mnemonic attestations indexed by `contextId`. Mnemonic is exposed *as* an MCP server, so MCP clients use it natively. ACP-binding is on the roadmap once that protocol stabilizes. Mnemonic is composable with all three; it competes with none of them.

---

## On-chain trust frameworks

These give you a registry where attestations live; none of them dictate the *shape* of what gets attested.

| | ERC-8004 (Ethereum) | Verifiable Credentials (W3C) | Solana NFT proofs | **Mnemonic** |
|---|---|---|---|---|
| On-chain registry | yes | bring-your-own | NFT contract | **off-chain envelope + on-chain anchor** |
| Specifies envelope format | no | JSON-LD | NFT metadata | **CBOR + COSE_Sign1** |
| Specifies signing algorithm | no | flexible | flexible | **Ed25519 (today), pluggable** |
| Identity model | ERC-721 NFT per agent | DIDs | wallet pubkey | **Ed25519 pubkey + DIDs** |
| Recall by semantic similarity | no | no | no | **yes** |

**When you'd reach for ERC-8004:** you want an on-chain registry of agent identities, reputation, and validations — without specifying *what* the validations actually contain.

**When you'd reach for Mnemonic:** you need the actual attestation primitive that ERC-8004's Validation Registry points at via `responseURI` + `responseHash`.

**Composable note:** Mnemonic's ERC-8004 integration registers it as a `signed-memory-attestation` validator. The off-chain Mnemonic envelope is exactly the shape ERC-8004's `responseURI` + `responseHash` fields expect. They are designed to fit together; Mnemonic is one of the cleanest off-chain primitives for ERC-8004 that exists today.

---

## Vector databases

These store and retrieve embeddings; none of them sign them.

| | Pinecone / Weaviate / Qdrant / pgvector | **Mnemonic** |
|---|---|---|
| Stores embeddings | yes | yes |
| Semantic search | yes | yes |
| Cryptographically signed entries | no | **yes** |
| Independently verifiable | no | **yes** |
| On-chain anchor | no | **yes** |
| Tuned for billion-scale recall | yes | not optimized for that yet |

**When you'd reach for a vector DB:** you have hundreds of millions of embeddings and recall throughput is the bottleneck.

**When you'd reach for Mnemonic:** verifiability is the bottleneck. Mnemonic's recall is fine for thousands of attestations per pubkey today and scales linearly with infrastructure; the differentiator is the signing + anchor layer that vector DBs do not provide.

**Composable note:** you can run any vector DB downstream of Mnemonic — Mnemonic's CBOR envelopes embed the raw f32 embeddings, and any consumer can feed those into Pinecone / Weaviate / pgvector for downstream recall while preserving the signed origin.

---

## Identity primitives

These tell you *who* an agent is; Mnemonic uses them, doesn't replace them.

| | DID (W3C) | ENS / SNS / Solana DID | OAuth + JWT | Mnemonic identity |
|---|---|---|---|---|
| Public key as identity | yes | indirect (resolves to pubkey) | indirect (signed by issuer) | **yes (Ed25519)** |
| On-chain registration | optional | yes | no | **optional (`did:sol:`)** |
| Self-sovereign | yes | yes | depends on issuer | **yes** |
| Signs application data, not just identity | depends | no | no | **yes** |

Mnemonic's identity layer interoperates with `did:key:`, `did:sol:`, and `did:web:` resolvers. The pubkey *is* the identity; the DID format is just a serialization choice.

---

## Take-away

Mnemonic Protocol is **a primitive, not a product layer**. It is composable with letta / zep / mem0 / cognee (above them), with A2A / MCP / ACP (alongside them), with ERC-8004 (under it), and with any vector DB or identity scheme.

The thing it gives you that nothing else gives you: **memory that someone outside your system can verify.**
