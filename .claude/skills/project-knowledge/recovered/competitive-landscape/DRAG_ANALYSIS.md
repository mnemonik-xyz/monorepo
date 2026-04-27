# Research Report: Decentralized RAG with Blockchain-Secured Source Reliability
## Analysis Against the Mnemonic Protocol

**Paper:** "A Decentralized Retrieval Augmented Generation System with Source Reliabilities Secured on Blockchain"
**arXiv:** 2511.07577
**Authors:** Yining Lu, Wenyi Tang, Max Johnson, Taeho Jung, Meng Jiang
**Date analyzed:** 2026-04-02

---

## 1. Paper Summary

The paper proposes a decentralized RAG (D-RAG) system where multiple independent nodes contribute document sources to augment LLM generation. The core problem: in a decentralized setting, sources vary significantly in reliability, and there is no central authority to curate them.

**The solution:** Assign and track per-source reliability scores using blockchain-based smart contracts. Retrieved documents are weighted by on-chain reliability scores before being fed to the language model.

**Key results:**
- +10.7% performance improvement over centralized RAG in real-world-like unreliable data environments (tested with Llama 3B and 8B)
- ~56% marginal cost savings via batched blockchain update operations
- Source reliability scores are tamper-proof and auditable

**Architecture components:**
- **Document nodes:** Independent participants storing and serving documents
- **Retrieval layer:** Distributed vector search across contributing nodes
- **Reliability oracle:** Smart contract that records credibility scores based on response quality feedback
- **Weighted generation:** LLM context assembled from documents weighted by on-chain reliability

---

## 2. Where Mnemonic and D-RAG Converge

### 2.1 Blockchain as a trust anchor — not as storage

Both systems make the same fundamental architectural choice: blockchain is not the storage layer, it is the **trust layer**. Documents/memories live off-chain (Arweave in Mnemonic's case; distributed nodes in D-RAG). The chain holds only a tamper-evident record — a hash and metadata.

D-RAG records: source reliability scores via smart contracts.
Mnemonic records: SHA3-256 hash of the encrypted memory blob via Solana memo.

Both systems are cheap to run on-chain precisely because they push bulk data off-chain and use the chain only for what it is uniquely good at: immutability and auditability.

### 2.2 Batched updates for cost efficiency

D-RAG achieves 56% cost savings via batched blockchain writes. This is exactly the logic behind Mnemonic's delta log design (ADR-006): rather than committing every individual memory item, writers batch deltas into Arweave blobs and commit once per batch to Solana. Per-item commits would be economically unviable at scale.

This is independent validation that batching is the right production strategy.

### 2.3 Decentralization creates a quality/reliability problem

D-RAG frames the core challenge as: "decentralization brings a challenge: the numerous independent data sources vary significantly in reliability."

Mnemonic frames the analogous challenge as the malicious collaborator threat (ADR-009): a party with legitimate write access can inject adversarial content — crafted embeddings, poisoned quantizer calibration, stale replays, payload injection.

Both papers arrive at the same diagnosis from different directions: **decentralized writes require an explicit trust model.** D-RAG solves this with on-chain reliability scoring. Mnemonic plans to solve this with per-entry signing and commitment chains (ADR-009 mitigations, not yet implemented).

### 2.4 Retrieval quality is the primary product metric

D-RAG evaluates success by whether retrieved context improves LLM answer quality. Mnemonic evaluates success by whether retrieved memories improve agent continuity and decision quality. Both are retrieval-first protocols — the on-chain layer serves the retrieval layer, not the other way around.

---

## 3. Where Mnemonic and D-RAG Diverge

### 3.1 Memory vs. document retrieval

D-RAG is a **document retrieval** system: it indexes static external documents (web pages, knowledge bases, corpora) to augment LLM generation at query time. The content is supplied by independent third parties with unknown quality.

Mnemonic is an **agent memory** system: it stores and retrieves an agent's own accumulated context across sessions. The content is produced by the agent itself (or trusted collaborators). The problem is not curating external noise — it is preserving an agent's own continuity across time and model switches.

This is a fundamental difference in the trust model. D-RAG's reliability problem is: "which of many strangers' documents should I trust?" Mnemonic's reliability problem is: "how do I prove my own memory wasn't tampered with?"

### 3.2 Compression and retrieval efficiency

D-RAG does not appear to address vector compression. Its retrieval concern is source quality, not index efficiency. At 10k+ memories, uncompressed vector search becomes impractical.

Mnemonic's core technical contribution — corpus-calibrated per-dimension scalar quantization with 2-stage compressed candidate generation + exact rerank — has no equivalent in D-RAG. This is Mnemonic's retrieval efficiency advantage and is the component with the most engineering depth (ADRs 010–016).

### 3.3 Smart contracts vs. Solana memos

D-RAG uses **smart contracts** to compute and store reliability scores. This implies on-chain computation: votes are aggregated, scores are calculated, and state is mutated by the contract.

Mnemonic uses **Solana memos** — a much simpler and cheaper primitive. Memos are write-once, append-only, with no on-chain computation. The memo contains only: hash, Arweave TX ID, encryption flag, parent hashes. All computation (retrieval, quantization, merge) happens off-chain.

**Implications:**
- D-RAG's smart contract approach enables richer on-chain logic (weighted aggregation, voting) but costs more and is harder to audit
- Mnemonic's memo approach is cheaper, faster, and simpler, but pushes reliability logic to the client
- If Mnemonic ever needs on-chain reliability scoring (e.g., for shared multi-agent pools), Solana PDAs + programs would be the equivalent path (mentioned in ADR-003 as a deferred option)

### 3.4 Encryption

D-RAG does not address encryption — documents are treated as public retrieval targets. The blockchain records credibility, not confidentiality.

Mnemonic encrypts by default (AES-256-GCM, keypair-derived, ADR-003). The on-chain hash is of the encrypted blob. This gives Mnemonic a privacy guarantee D-RAG does not attempt.

### 3.5 Quantizer state is a new problem D-RAG doesn't face

Mnemonic's `CalibratedScalarQuantizer` introduces a state artifact that has no equivalent in RAG: the quantizer's calibration (per-dimension alphas and steps) must be shared across all writers. A malicious writer cannot silently recalibrate the quantizer without breaking every other participant's compressed index.

This is a novel attack surface that D-RAG's reliability scoring model does not cover and that Mnemonic needs to harden explicitly (ADR-006: quantizer fit locked after bootstrap; ADR-009: quantization poisoning as explicit threat vector).

---

## 4. What Mnemonic Can Learn from D-RAG

### 4.1 Per-source reliability scoring is production-proven

D-RAG demonstrates that on-chain reliability scoring produces measurable quality improvements (+10.7%) in adversarial conditions. This is direct empirical support for Mnemonic's planned per-entry signing mitigation (ADR-009).

For the shared multi-agent memory case (ADR-006 concurrent writers), an on-chain reliability score per writer identity would address the malicious collaborator threat in a well-studied way. Each delta commit could include an identity signature; a smart contract (or client-side logic) could track per-writer reliability based on retrieval quality feedback.

### 4.2 The reliability oracle pattern

D-RAG's reliability oracle — a contract that takes quality feedback and updates credibility scores — is a generalization of Mnemonic's commitment chain. Mnemonic currently only records "was this state committed" (hash). D-RAG records "was this source useful." For a shared memory pool, "was this memory useful" is a natural extension — and could be tracked on-chain in the same memo or PDA without adding significant cost.

### 4.3 Batching is validated

D-RAG's 56% cost saving from batching directly validates ADR-006's delta-log design. The right unit of commitment is a batch/delta, not an individual memory item. D-RAG also validates that this doesn't sacrifice correctness — individual item quality is still attributable through signing within the batch.

### 4.4 Decentralized retrieval performance baseline

D-RAG measures end-to-end generation quality improvements rather than low-level retrieval metrics. This is a useful framing for Mnemonic's V2 demo: the right top-level metric is not recall@10 — it is "does the agent's output quality improve as its memory grows?" D-RAG provides a benchmark methodology for answering this question in a decentralized setting.

---

## 5. What Mnemonic Has That D-RAG Doesn't

| Capability | D-RAG | Mnemonic |
|-----------|-------|----------|
| Compressed shadow index (4-bit / 8-bit) | ✗ | ✅ ADR-010–016 |
| 2-stage retrieval (candidate generation + exact rerank) | ✗ | ✅ core architecture |
| Encrypted memory blobs | ✗ | ✅ ADR-003 |
| Provider-agnostic snapshot/restore | ✗ | ✅ ADR-012 |
| Session persistence (SQLite) | ✗ | ✅ ADR-014 |
| Multi-writer Merkle DAG | ✗ (simpler model) | ✅ designed, ADR-006 |
| On-chain source reliability scoring | ✅ | ✗ (designed, not built) |
| Smart contract voting / aggregation | ✅ | ✗ (memo only today) |
| Proven LLM output quality improvement | ✅ +10.7% | ✗ (not yet measured) |

---

## 6. Strategic Implications for Mnemonic

### 6.1 Strongest validation of the thesis

D-RAG is the closest published work to Mnemonic's architecture. It independently validates three of Mnemonic's core bets:
1. Blockchain belongs in the trust layer, not the storage layer
2. Off-chain decentralized storage + on-chain hash commitment is the right layering
3. Batched commits are economically necessary

### 6.2 The "reliability scoring" gap is Mnemonic's biggest open risk

D-RAG shows that in adversarial/unreliable environments, on-chain reliability tracking produces a measurable +10.7% improvement. Mnemonic's ADR-009 security model identifies the malicious collaborator as the primary V1 threat but has no implementation yet. For the Personal Research Assistant (ADR-011, V2), reliability is less critical because the user controls their own memory. But for the multi-agent shared pool case (the long-term product moat), D-RAG's approach is a direct blueprint.

### 6.3 Mnemonic's compression layer is a genuine differentiator

D-RAG does not address the cost of vector search at scale. At 10k+ memories, brute-force float32 search is ~600ms/query in Python (ADR-004). D-RAG presumably inherits this cost. Mnemonic's quantized shadow index (25% of float size, 94.2% recall@10 at 10k) is a performance advantage D-RAG cannot claim. This is worth foregrounding in any positioning: Mnemonic is not just "RAG with blockchain" — it is "compressed, verifiable, provider-agnostic agent memory."

### 6.4 End-to-end generation quality should be Mnemonic's next research gate

D-RAG measures what ultimately matters: does the system improve LLM output quality? Mnemonic has measured retrieval quality (recall@10) but not generation quality. The next research gate (post-V1) should be: given a research task across N sessions, does an agent with Mnemonic memory produce materially better outputs than one without? This is the proof that moves from "retrieval protocol" to "product."

---

## 7. Summary

D-RAG and Mnemonic are complementary rather than competing. D-RAG solves the multi-source reliability problem for public document RAG. Mnemonic solves the compression, continuity, and privacy problem for agent memory.

The most valuable finding from this analysis: **D-RAG's on-chain reliability oracle is a directly applicable design pattern for Mnemonic's planned malicious collaborator mitigations.** When Mnemonic implements per-entry signing (ADR-009), the reliability scoring model from D-RAG is the right next step — each writer's contributions can be scored by downstream retrieval quality, tracked on-chain, and used to weight or filter future retrievals.

The paper also provides a published benchmark baseline (+10.7% improvement in adversarial conditions) that Mnemonic can cite and aim to exceed when validating its own adversarial robustness.

---

## References

- Lu et al., "A Decentralized Retrieval Augmented Generation System with Source Reliabilities Secured on Blockchain," arXiv:2511.07577
- Mnemonic ADR-003: Encryption Layer
- Mnemonic ADR-006: Concurrent Writers
- Mnemonic ADR-009: Security and Privacy Model
- Mnemonic ADR-010–016: Retrieval Validation Gates
