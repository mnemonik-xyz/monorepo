# Web Research: Decentralized Trustless RAG Protocols
## Validation Research for Mnemonic Protocol

**Date:** 2026-04-02
**Queries:** Decentralized trustless RAG, verifiable vector search on-chain, zkTAM, V3DB, compressed agent memory Arweave Solana

---

## Executive Summary

The market thesis is real, articulated, and being built by multiple teams independently. ICME explicitly states: **"Trustless agents can't work without trustless agentic memory."** This is the same thesis Mnemonic is built on. The ecosystem is early, fragmented, and no single system is winning. Mnemonic's combination of compression + hash commitment + provider-agnostic snapshot sits in a gap that none of the current systems fill.

---

## 1. Key Systems Found

### 1.1 zkTAM / Kinic-CLI (ICME Labs) — closest direct peer

**Source:** [Trustless Agents can't work without Trustless Agentic Memory](https://blog.icme.io/trustless-agents-cant-work-without-trustless-agentic-memory/)

The most direct peer to Mnemonic found in the search. ICME is building **zkTAM** (Trustless Agentic Memory) using zkML (zero-knowledge machine learning). Their stack:

- **Vectune** — vector database running on WASM-compatible data availability layers, built with ideas from freshVamana (a disk-optimized graph-based ANN index). RAM usage kept within the DA layer.
- **JOLT Atlas** — zkML framework extended to tackle embedding models. Generates ZK proofs that embeddings are correct (the embedding provider ran the model faithfully on the stated input).
- **Internet Computer Protocol (ICP)** — default DA layer: cheap storage, vetKey encryption, cross-chain signing (tECDSA).
- **Kinic-CLI** — CLI tool for creating decentralized memory stores, add/delete/update memories with ownership, generate verifiable embeddings.

**What zkTAM proves:** (1) the embedding was computed correctly (ZK proof of embedding model execution), (2) the memory store contains what it claims (integrity proof).

**What zkTAM does NOT do:**
- No vector compression (4/8-bit quantization)
- No provider-agnostic snapshot/restore across different embedding models
- No fast compressed candidate retrieval + exact rerank cascade
- ICP-specific (not Arweave + Solana)

**Mnemonic vs. zkTAM:**

| | Mnemonic | zkTAM |
|---|---|---|
| Trust model | Hash commitment (cheaper, faster) | ZK proof of embedding (stronger, expensive) |
| Compression | ✅ 4/8-bit shadow index | ✗ |
| Provider portability | ✅ Snapshot/restore any embedder | ✗ |
| Storage | Arweave + Solana | ICP |
| Encryption | AES-256-GCM | vetKey (ICP-native) |
| Status | Pre-V1, prototype validated | Live (Kinic-CLI shipped) |

**Strategic note:** zkTAM proves more (embedding correctness via ZK) but costs dramatically more in proving time and is ICP-locked. Mnemonic's hash commitment is weaker in ZK sense but orders of magnitude cheaper and chain-agnostic. The key question: do users need proof of *embedding correctness*, or proof of *memory integrity*? Mnemonic bets the latter is sufficient for V1.

---

### 1.2 V3DB — verifiable ANN retrieval with ZK proofs

**Source:** Search results + [GitHub: zk-IVF-PQ](https://github.com/TabibitoQZP/zk-IVF-PQ)

V3DB is a verifiable, versioned vector-search service with audit-on-demand ZK proofs for ANN-retrieval:
- Commits to corpus snapshots (IVF-PQ index)
- Standardizes a 5-step query pipeline
- When challenged, produces a ZK proof that the top-k result is *exactly* what the committed semantics would return
- Built on Plonky2 — achieves 40% lower peak memory than circuit-only baseline, millisecond-level verification time
- Avoids costly in-circuit sorting via multiset equality/inclusion checks + boundary conditions

**Relevance to Mnemonic:**
- V3DB is an ANN retrieval verifier, not an agent memory system
- Mnemonic commits the memory blob (SHA3 hash); V3DB proves the retrieval result
- V3DB-style proofs are the answer to Mnemonic's open question Q3 (Section 9.2): "Is there a practical ZK proof that a retrieval result came from a committed blob?"
- Combining Mnemonic's commitment + V3DB's retrieval proof would give end-to-end trustless agent memory

---

### 1.3 D-RAG variants — two distinct papers

Two independent "D-RAG" papers found:

**Paper 1:** Lu et al. (2025), arXiv:2511.07577 — already analyzed in `DRAG_ANALYSIS.md`.

**Paper 2:** NSF-funded D-RAG — [Privacy-Preserving Framework for Decentralized RAG using Blockchain](https://par.nsf.gov/biblio/10578004-rag-privacy-preserving-framework-decentralized-rag-using-blockchain). This version uses **permissioned blockchain + privacy-preserving consensus protocol** for expert verification of data before it enters the RAG store. More conservative (permissioned, expert-gated) vs. Lu et al.'s open reliability scoring.

Both papers establish decentralized RAG as an active research area funded by major institutions (NSF).

---

### 1.4 JOLT Atlas + ERC-8004 — agent coordination + verification stack

**Sources:** [JOLT Atlas paper](https://arxiv.org/html/2602.17452), [Trustless Agents with zkML](https://blog.icme.io/trustless-agents-with-zkml/)

An emerging two-layer architecture for trustless agents:
- **ERC-8004** — coordination: how agents find each other, establish identity, build reputation
- **JOLT Atlas** — verification: ZK proofs of inference execution

Together: agents can prove they executed the right model on the right inputs, enabling trustless agent-to-agent interaction. JOLT Atlas supports classification, embedding, automated reasoning, and small LMs. Uses folding scheme for fast, space-efficient ZKPs that run in browsers and constrained devices.

**Relevance to Mnemonic:**
- Mnemonic's per-entry signing (ADR-009) + reliability oracle (ADR-018) is the lightweight version of this stack
- A future Mnemonic integration could use JOLT Atlas to prove that each memory item was embedded by the stated model — resolving the embedding correctness gap vs. zkTAM
- ERC-8004 agent identity is compatible with Mnemonic's Solana keypair model

---

### 1.5 Arweave + Solana — validated production infrastructure

**Sources:** [Arweave on Solana](https://solana.com/ecosystem/arweave), [Arweave-Solana SDK](https://github.com/labscommunity/arweave-storage-sdk), [Permanent Storage guide](https://www.quicknode.com/builders-guide/tools/permanent-decentralized-storage-by-arweave)

- Arweave and Solana have an official partnership for decentralized ledger data storage
- Arweave "pay-once, store forever" model is mature infrastructure used in production by Solana validators
- The `arweave-storage-sdk` supports paying for Arweave storage using Solana stablecoins — a direct fit with Mnemonic's economic model
- A 2025 paper on Blockweave (Arweave-based) reports: 48-byte on-chain metadata, 7200 TPS, 99% storage savings — validates the efficiency of Mnemonic's approach

**Strategic note:** Mnemonic's Arweave + Solana choice is not a novel bet — it is backed by production deployments at scale. This is infrastructure, not research.

---

## 2. Market Signal: The Thesis Is Established

Three independent sources articulate the same thesis as Mnemonic, in the same terms:

1. **ICME (2025):** *"Trustless agents can't work without trustless agentic memory."* — direct match to Mnemonic's founding thesis.
2. **D-RAG (NSF-funded, 2025):** Decentralized RAG is framed as necessary infrastructure for AI agent trust.
3. **Blockchain Agent Economy (arXiv:2602.14219):** Blockchain infrastructure for autonomous AI agents is active academic research with multiple teams building coordination + verification primitives.

No system has yet combined: compression + hash commitment + provider-agnostic portability + open embedder. This is Mnemonic's gap.

---

## 3. Competitive Positioning Update

| System | What it proves | Compression | Provider-portable | Chain | Status |
|---|---|---|---|---|---|
| zkTAM (ICME) | Embedding correctness (ZK) | ✗ | ✗ (ICP-locked) | ICP | Live |
| V3DB | Retrieval correctness (ZK) | ✗ | N/A | None (service) | Research |
| D-RAG Lu et al. | Source reliability | ✗ | ✓ | Ethereum/generic | Paper |
| D-RAG NSF | Data quality (permissioned) | ✗ | Partial | Permissioned | Paper |
| Mnemonic | Memory integrity (hash) | ✅ 4/8-bit | ✅ | Arweave+Solana | Pre-V1 |

**Mnemonic's unique position:** The only system combining compression with decentralized verifiable agent memory. zkTAM is the closest peer but is proof-heavy, ICP-locked, and uncompressed. V3DB solves a different layer (retrieval verification, not memory storage).

---

## 4. Implications for Mnemonic

### 4.1 Immediate validation
- The market thesis ("trustless agent memory is necessary infrastructure") is articulated and funded by multiple independent teams. Mnemonic is not a speculative bet — it is in an established research and product direction.
- Arweave + Solana as the storage/commitment layer is validated production infrastructure, not an experimental choice.

### 4.2 Key differentiator to emphasize
Compression is Mnemonic's clearest technical gap vs. all competitors. No other system in this space addresses the cost/performance tradeoff of storing embeddings at scale. This should be the lead technical claim in all positioning.

### 4.3 ZK upgrade path is well-defined
- Near term: hash commitment (Mnemonic V1) — cheapest, fastest, sufficient for single-user
- Medium term: reliability oracle (ADR-018) — per-writer quality tracking, no ZK needed
- Long term: JOLT Atlas integration for embedding correctness proofs — closes the gap vs. zkTAM
- Aspirational: V3DB-style retrieval correctness proofs — full end-to-end trustless agent memory

This is a credible and sequential upgrade path, not a vague future vision.

### 4.4 ICP vs. Arweave+Solana
zkTAM chose ICP for cheap storage + vetKey encryption + cross-chain signing. Mnemonic chose Arweave + Solana. Key difference: Arweave's permanence guarantee is stronger (pay-once endowment model); Solana's throughput is higher than ICP for commitment anchoring. ICP's vetKey is a compelling encryption primitive but requires ICP lock-in. Mnemonic's AES-256-GCM with HKDF is simpler and chain-agnostic.

### 4.5 "Trustless agents" framing is the right marketing angle
ICME's framing — "trustless agents can't work without trustless agentic memory" — is the correct positioning hook. Mnemonic should adopt this language explicitly. The progression is: trustless inference (JOLT Atlas) + trustless memory (Mnemonic) + trustless coordination (ERC-8004) = fully trustless agent stack.

---

## 5. References

- [Trustless Agents can't work without Trustless Agentic Memory — ICME](https://blog.icme.io/trustless-agents-cant-work-without-trustless-agentic-memory/)
- [Trustless Agents with zkML — ICME](https://blog.icme.io/trustless-agents-with-zkml/)
- [Jolt Atlas: Verifiable Inference via Lookup Arguments (arXiv:2602.17452)](https://arxiv.org/html/2602.17452)
- [D-RAG: Privacy-Preserving Framework — NSF](https://par.nsf.gov/biblio/10578004-rag-privacy-preserving-framework-decentralized-rag-using-blockchain)
- [D-RAG: Blockchain-Secured Source Reliability (arXiv:2511.07577)](https://arxiv.org/abs/2511.07577)
- [V3DB / zk-IVF-PQ GitHub](https://github.com/TabibitoQZP/zk-IVF-PQ)
- [The Agent Economy: Blockchain for Autonomous AI Agents (arXiv:2602.14219)](https://arxiv.org/html/2602.14219v1)
- [Arweave on Solana Ecosystem](https://solana.com/ecosystem/arweave)
- [Arweave Storage SDK (Solana/EVM payments)](https://github.com/labscommunity/arweave-storage-sdk)
