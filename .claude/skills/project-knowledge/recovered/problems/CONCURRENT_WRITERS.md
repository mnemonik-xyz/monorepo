# Concurrent Writers in Mnemonic — Research Report

**Date:** 2026-03-31
**Addresses:** ADR-006, Gap #9
**Status:** Research complete, architecture recommended

---

## 1. The Core Question: Should Memory Be Shared At All?

**Yes, and it's arguably what justifies the blockchain layer.** Without sharing, you don't need a blockchain — a local SQLite + S3 backup does the same job cheaper. Shared memory is the product differentiator.

---

## 2. High-Value Shared Memory Use Cases

### A. Multi-Agent Collaboration (Highest Value)

Multiple AI agents working on the same project (e.g., a coding agent, a research agent, and a planning agent) benefit enormously from shared memory. Agent A discovers a fact; Agent B can retrieve it without re-deriving it. This is the strongest use case.

**Value:** Eliminates redundant work, enables specialization.

### B. Team Knowledge Bases (High Value)

A team of humans, each with their own agent, shares a common memory pool. When one person's agent learns something, the whole team benefits. This is a "multiplayer Notion with semantic search" backed by on-chain provenance.

**Value:** Organizational knowledge capture with cryptographic audit trail.

### C. Organizational Memory with Access Tiers (Medium Value)

Different departments share some memories but not others. Requires encryption with per-party access (the current single-keypair AES-256-GCM does not support this).

**Value:** Prevents information silos while maintaining confidentiality.

### D. Federated Learning Over Agent Experiences (Speculative)

Agents in different organizations share anonymized memory patterns (not raw content) to improve collective retrieval. Technically interesting but far from the current architecture.

### E. Public Knowledge Commons (Niche)

Agents contribute to a shared, unencrypted memory pool that anyone can read. Think "Wikipedia for agents." The on-chain commitment provides provenance.

**Value:** Public good, but limited commercial value.

### Product Implications

- Turns Mnemonic from a single-agent tool into a collaboration primitive
- On-chain commitments provide auditability that centralized alternatives (Pinecone, Weaviate) cannot offer
- The append-only nature of Arweave gives you a "memory git" with full history
- Multi-party access justifies the complexity of on-chain infrastructure

---

## 3. The Hard Technical Problem: The Quantizer Breaks Under Merge

The `CalibratedScalarQuantizer.fit()` in `pseudocode.py` computes per-dimension 98th-percentile absolute values across the *entire* corpus:

```python
for j in range(dim):
    values = [abs(v[j]) for v in vectors]
    ordered = sorted(values)
    idx = int(0.98 * (len(ordered) - 1))
    alpha = ordered[idx]
```

This is a **non-decomposable aggregate**. Two writers calibrating independently produce incompatible quantizer states — their packed codes can't be merged.

**Consequences:**

- Packed codes from writer A are not compatible with the quantizer from writer B
- Merging two quantized indices requires re-fitting the quantizer on the union corpus and re-quantizing all vectors
- Every merge = full index rebuild

### Possible Mitigations

**Option 1 — Shared quantizer, private deltas (recommended).** All writers share a single quantizer state calibrated on a bootstrap corpus or periodically recalibrated. New memories are quantized with the shared quantizer. Merging packed codes is trivial (just concatenate). The quantizer drifts slowly as the corpus grows, but 98th-percentile statistics are robust to small additions. Recalibrate periodically (every N merges or when saturation rate exceeds a threshold).

**Option 2 — Quantizer-free deltas.** Deltas contain only raw embeddings (float32). The quantized index is built locally by each reader after fetching all deltas. Moves quantization from write-time to read-time. Trade-off: deltas are 4-8x larger (float32 vs uint8), but merge is trivial.

**Option 3 — Approximate decomposable statistics.** Replace the 98th-percentile with a decomposable statistic like max absolute value (fully decomposable) or maintain a t-digest/quantile sketch per dimension. Two sketches can be merged. Makes the quantizer state itself CRDT-compatible, at the cost of slightly different calibration.

---

## 4. Five Concurrency Models Evaluated

### A. Last-Write-Wins (LWW) with Snapshot Overwrite

Each writer builds a full snapshot independently. Whichever commit lands on Solana last is the canonical state. This is what the current system implicitly does — `commit.mjs` serializes the entire memory blob and commits its hash.

- **Trade-off:** Simple, zero coordination. But writer B's commit silently discards everything writer A added since the last common ancestor. With 9 writers, you lose 8/9 of concurrent work.
- **Verdict:** Unacceptable for shared memory. Acceptable only if each writer has their own isolated memory namespace.

### B. Event Sourcing (Append-Only Log of Deltas) — RECOMMENDED

Instead of committing full snapshots, each writer commits a *delta* (new memories added since last known state). The canonical state is reconstructed by replaying all deltas in Solana slot order.

- **On-chain:** Each Solana memo becomes a delta reference: `{parent_hashes, delta_arweave_tx, num_new_memories}`
- **Arweave:** Each upload is a delta blob, not a full snapshot
- **Merge:** Deterministic — replay deltas in slot order. No conflicts because memories are append-only
- **Trade-off:** Simple merge semantics. But quantizer state cannot be incrementally updated from deltas alone. Reading requires reconstructing from the full chain of deltas, or periodically compacting into a snapshot.

### C. CRDTs (Conflict-Free Replicated Data Types)

CRDTs work for the memory set but break for the quantizer.

**What works:** The set of `MemoryItem` records is a natural G-Set (grow-only set). Each memory has a unique `memory_id`. Adding a memory is commutative, associative, and idempotent: `union(A, B)`.

**What breaks:** The `CalibratedScalarQuantizer` state depends on corpus statistics (see section 3). This is not a CRDT-compatible operation.

- **Verdict:** Partial fit. Could be combined with event sourcing (CRDT for the memory set layer, event sourcing for the index layer).

### D. Operational Transformation (OT)

OT is designed for ordered character-level edits (Google Docs). Memory items are not ordered character streams.

- **Verdict:** Wrong abstraction. Too much complexity for no benefit over event sourcing.

### E. Lock-Based (Pessimistic Concurrency via Solana PDA)

A Solana PDA acts as a mutex. Writers acquire the lock before committing. Only one writer commits at a time.

- **Implementation:** A Solana program with a PDA storing `{current_owner: Pubkey, lock_slot: u64}`. Writer calls `acquire_lock` instruction, commits, then calls `release_lock`.
- **Trade-off:** Correct and simple to reason about. But with 9 concurrent writers and ~400ms slots, throughput is capped. If each writer needs 2-3 seconds to build a snapshot + upload to Arweave + commit, effective throughput drops to one commit every few seconds. Writers queue up.
- **Verdict:** Viable for low-write scenarios (2-3 writers, infrequent commits). At 9 writers, contention is a bottleneck.

### Comparison Table

| Model | Fit for Mnemonic? | Why |
|-------|-------------------|-----|
| Last-Write-Wins | No | 9 writers = lose 8/9 of concurrent work |
| **Event Sourcing** | **Best fit** | Append-only deltas, replay in Solana slot order |
| CRDTs | Partial | Works for memory set, breaks for quantizer state |
| Operational Transform | No | Designed for text editing, wrong abstraction |
| Lock-based | Marginal | Caps throughput with 9 writers |

---

## 5. Solana-Specific Constraints and Opportunities

### What Solana Gives For Free

- **Total ordering:** Global total order within a slot (~400ms). Transactions in the same slot have a deterministic order set by the leader. Across slots, ordering is by slot number. No need to build your own Lamport clock or vector clock.
- **Concurrent capacity:** 9 writers submitting concurrently can all land in the same slot (Solana handles ~4000 TPS). No bottleneck.
- **PDA-based access control:** Derive PDAs like `[b"mnemonic", agent_pubkey, memory_namespace]`. This gives per-agent memory namespaces and on-chain ACLs.
- **Atomic CAS:** A Solana program can enforce that a commit's `parent_hash` matches the PDA's `latest_hash`, rejecting stale commits (optimistic concurrency).
- **Public verifiability** of commit sequence.
- **~400ms finality** for ordering decisions.

### What Solana Does NOT Give

- **Conflict resolution** — merge logic is off-chain
- **Storage** — memo data is in transaction logs, not account state. Arweave holds the actual blobs
- **Efficient reads of historical commit chains** — need an indexer or maintain your own chain

### Memo Size

The Memo program accepts up to ~566 bytes of UTF-8 data per instruction. Current memo payload in `commit.mjs` is ~200-300 bytes of JSON. Adding `parent_hashes` for DAG structure: each SHA3-256 hash = 64 hex chars. Two parents = 128 chars. Fits comfortably.

---

## 6. Arweave-Specific Constraints

### Immutability

Every upload is permanent. No "update" or "delete." This is a feature for event sourcing — every delta is an immutable record.

### No Update-In-Place

Eliminates snapshot-based approaches unless you accept that every commit is a new full snapshot (storage cost grows linearly with commit frequency). With 9 writers committing frequently, full snapshots get expensive. Delta-based uploads keep per-commit size proportional to the delta.

### Confirmation Timing

Arweave confirmation takes ~2 minutes (block time). Solana confirmation takes <1 second. The current code uploads to Arweave first, then commits the hash to Solana. With concurrent writers, both uploads succeed independently on Arweave, both hashes land on Solana. No conflict at the storage layer.

### Cost

Arweave charges per byte, permanently (~$1.50/MB). A 10k-memory index with 1536-dim 8-bit quantization is ~15MB as a full snapshot. 9 concurrent writers doing full snapshots every minute = catastrophic cost. Deltas of 10-50 new memories each = kilobytes per commit = negligible.

### Merge Strategy Implications

- **Full snapshot per commit:** 9 writers = 9x storage per commit cycle. Wasteful.
- **Delta per commit:** 9 writers = 9 small deltas. Efficient.
- **Compaction:** Periodically, one writer creates a new full snapshot subsuming all prior deltas. Old deltas remain on Arweave forever (immutable) but are no longer needed for reads.

---

## 7. Encryption for Multi-Party Access

The current design (`encrypt.mjs`) uses AES-256-GCM with a key derived via HKDF from the Solana keypair's first 32 bytes. This is single-owner encryption — only the keypair holder can decrypt.

### Options for Multi-Party Access

**Option 1 — Shared secret.** All 9 writers share the same AES key. Simple. No revocation. If one party is compromised, everyone is compromised. Acceptable for trusted teams only.

**Option 2 — Per-recipient key wrapping (recommended).** The blob is encrypted with a random data encryption key (DEK). The DEK is then encrypted separately for each authorized party using their public key. The wrapped DEKs are stored alongside the ciphertext.

```
Proposed format:
"MENC" (4B) + version (1B) + num_recipients (2B) +
[recipient_pubkey (32B) + wrapped_DEK (48B)] * num_recipients +
iv (12B) + tag (16B) + ciphertext

Overhead per recipient: 80 bytes
Overhead for 9 writers: 720 bytes (negligible)
```

Adding a party = wrap the DEK with their public key. Revoking = re-encrypt with new DEK + new wrapped keys. This is the PGP/age model — well-understood, no on-chain program needed.

**Option 3 — On-chain access control via PDA.** A Solana program maintains a PDA with an access control list. Writers check on-chain whether they are authorized. Encryption key derived from shared secret stored off-chain but referenced on-chain.

**Option 4 — Threshold encryption / MPC.** DEK split via Shamir's secret sharing. K-of-N parties must cooperate to decrypt. Most secure, most complex. Overkill unless threat model demands it.

---

## 8. Hash Chain Integrity

The current system commits `SHA3-256(encrypted_blob)` to Solana. With multiple concurrent writers, two approaches for history verification:

**Linear chain:** Each commit includes `parent_hash`. Two writers pointing to the same parent = fork. Fork resolution: accept both branches, merge at read time (event sourcing). Or reject one (optimistic concurrency — commit fails if parent is stale).

**Merkle DAG (recommended):** Each commit includes multiple parent hashes (like a git merge commit). Naturally handles concurrent writes. This is what IPLD/Ceramic use.

---

## 9. Real-World Precedents

### Ceramic Network (now ComposeDB)

- IPLD (InterPlanetary Linked Data) for content-addressed data structures
- Each "stream" is a DAG of commits, each signed by the controller
- Anchored to Ethereum (batched via Merkle tree)
- Default conflict resolution: last-write-wins; custom resolution supported
- **Lesson:** Separates data layer (IPFS/Arweave) from ordering layer (Ethereum). Exactly the Mnemonic architecture.
- **Limitation:** LWW default is too simple for quantized indices.

### OrbitDB

- Peer-to-peer database on IPFS, uses CRDTs for merge
- "Feed" type = append-only log — maps directly to event sourcing model
- No blockchain anchoring — the gap Mnemonic fills
- **Lesson:** CRDT merge for sets works for the memory item layer.
- **Limitation:** No notion of quantized indices or corpus-dependent state.

### Textile Threads (now Tableland)

- Multi-writer append-only logs with IPFS storage
- DAG of records, signed by participants, ACL-based write control
- **Lesson:** Closest precedent to what Mnemonic needs. Multiple authorized writers append to a shared log. DAG handles concurrent appends.

### Iroh (by n0/number0)

- Rust-based CRDT document sync on BLAKE3 content addressing
- "Authors" and "namespaces" — each namespace is a key-value store with multi-value register CRDT
- **Lesson:** Namespace model maps to Mnemonic's per-agent memory spaces. Multi-value register (keep all concurrent writes, app resolves) is pragmatic.

### Nostr

- Event-based protocol: signed JSON events, append-only, relayed to multiple servers
- No merge logic at protocol level — apps build state from event stream
- **Lesson:** Pure event sourcing. Each "memory commit" could be a signed event containing the delta reference.
- **Limitation:** No ordering guarantee. Solana fills this gap.

### Pattern Across All Precedents

All successful decentralized multi-writer systems converge on:

1. Content-addressed immutable blobs for storage
2. Signed, append-only logs for writes
3. DAG structure (not linear chain) for concurrent writes
4. External ordering layer (blockchain or consensus) for global time
5. Application-level merge logic for domain-specific conflict resolution

Mnemonic already has (1) Arweave, (2) partially via `commit.mjs`, and (4) Solana. It needs to add (3) DAG-structured commit history and (5) merge strategy for quantizer state.

---

## 10. Recommended Architecture

### Event-Sourced Delta Log with Shared Quantizer and Periodic Compaction

**Write path:**
1. Writer creates new memories + float32 embeddings
2. Quantizes with shared quantizer
3. Packs into delta blob: new `MemoryItem` records + embeddings + packed codes
4. Encrypts delta with random DEK, wraps DEK per-recipient
5. Uploads encrypted delta to Arweave
6. Commits to Solana: `{parent_hashes: [...], delta_arweave_tx, num_new, encrypted: true}`

**Shared quantizer:**
- Calibrated on initial corpus
- Stored as separate Arweave blob, referenced by hash
- All writers quantize with this quantizer
- Recalibrated during compaction

**Read path:**
1. Fetch latest compaction snapshot + all subsequent deltas
2. Replay deltas in Solana slot order
3. Quantized shadow index = union of all packed codes (compatible via shared quantizer)

**Compaction (periodic):**
1. Designated writer fetches all deltas since last compaction
2. Merges memory items
3. Re-fits quantizer on full corpus
4. Re-quantizes all vectors
5. Uploads new snapshot to Arweave
6. Commits new snapshot hash to Solana
7. Old deltas become historical

**Encryption:**
- Per-recipient key wrapping (80B/recipient)
- Adding party = wrap DEK with their public key
- Revoking party = re-encrypt with new DEK + new wrapped keys

**Access control:**
- Solana PDA per memory namespace stores authorized writer pubkeys
- Commit program verifies signer is in ACL before accepting memo

---

## 11. Technical Restrictions Summary

| Restriction | Impact | Mitigation |
|-------------|--------|------------|
| Quantizer is corpus-dependent | Can't merge packed codes from different quantizers | Shared quantizer + periodic recalibration |
| Full-precision store is source of truth | Quantized index is always rebuildable | Treat shadow index as a cache |
| Encryption is single-owner | Can't share without key sharing | Per-recipient key wrapping (80B/recipient) |
| Arweave is immutable | Can't update snapshots in place | Delta-based uploads + compaction |
| Memo size ~566B | Must fit parent hashes + metadata | DAG references fit (hash = 64 hex chars) |
| Arweave confirmation ~2min | Race between upload and commit | Upload first, commit hash after (current approach works) |
| Quantizer drift | Shared quantizer becomes stale as corpus grows | Recalibrate during compaction; 98th-percentile is robust to small additions |

---

## 12. What This Architecture Gives Up

- Real-time merge (readers see slightly stale index until they replay deltas)
- Sub-second consistency across all 9 writers
- Simplicity of single-writer model

## 13. What This Architecture Preserves

- **Correctness:** Full-precision embeddings are never lost
- **Auditability:** Full commit history on Solana
- **Privacy:** Per-recipient encryption
- **Scalability:** Delta-based Arweave uploads are cheap
- **Simplicity:** No CRDTs for the hard parts, just event replay
- **Future-proofing:** Compatible with the 2-stage retrieval architecture (ADR-002, ARCHITECTURE.md)
