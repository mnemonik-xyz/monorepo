# Memvid — Technical Analysis

> **Scope.** Technical due-diligence on Memvid v2 (the `mnemonik-dev/memvid` mirror tracks the public `memvid/memvid` repo, currently at `2.0.139`), compared against the Mnemonic Protocol. Written 2026-05-21.

## 1. What Memvid is

Memvid is a **single-file portable memory layer for AI agents**, written in Rust, published as `memvid-core` on crates.io and as SDKs for Node, Python, plus a `memvid-cli`. Everything an agent needs — raw content, compressed payloads, full-text index, vector index, time index, and a manifest table-of-contents — lives inside one `.mv2` file. No sidecar `.wal`, `.lock`, `.shm`, no external vector DB, no server.

The slogan is **"infrastructure-free memory"**: the file is the database. Move the file → move the memory.

Important historical note that often misleads readers: **Memvid v1 used QR-codes-in-video frames** as the physical encoding (which is why it's called *memvid*). That approach is now deprecated. v2 is a conventional binary file format and the "video" framing only survives metaphorically in the **"Smart Frames"** terminology (append-only, immutable records grouped into segments).

## 2. File format: `.mv2` (v2.1)

```
┌────────────────────────────┐
│ Header (4 KB)              │  magic MV2\0, version, footer offset, WAL ptrs, TOC checksum
├────────────────────────────┤
│ Embedded WAL (1–64 MB)     │  crash-recovery journal sized by file capacity
├────────────────────────────┤
│ Data Segments              │  Smart Frames: payload + blake3/sha256 checksum + tags
├────────────────────────────┤
│ Lex Index (Tantivy)        │  optional, BM25 full-text
├────────────────────────────┤
│ Vec Index (HNSW)           │  optional, cosine sim on 384-d embeddings
├────────────────────────────┤
│ Time Index                 │  chronological order for timeline / time-travel
├────────────────────────────┤
│ TOC (Footer)               │  segment catalog, per-segment SHA-256 checksums
└────────────────────────────┘
```

Key invariants the spec promises:

1. **Single-file guarantee** — no sidecar files, ever.
2. **Append-only frames** — once committed, a frame is immutable; deletes are tombstones.
3. **Determinism** — same API calls → identical bytes.
4. **Crash safety** — WAL replays on open if a write was interrupted; checkpoints flush at 75% WAL fill or every 1000 txns.
5. **Self-describing** — TOC carries everything needed to parse the file.

Frames carry `uri` (`mv2://path/...`), `title`, `created_at`, encoding (Raw / Zstd / LZ4), payload, **payload SHA-256 checksum**, and a tag map. Each segment in the TOC also has its own SHA-256.

## 3. What's inside the crate (≈31 kLOC Rust)

The `src/` tree gives a more honest picture of scope than the README:

| Module | Purpose |
|---|---|
| `memvid/` | Public façade — `Memvid` struct, mutation (put/commit), search orchestration, ask/RAG, timeline, mesh (graph), planner, builder, lifecycle, doctor (repair), acl |
| `io/` | header codec, embedded WAL, time index, manifest WAL, temporal index |
| `lex.rs` | Tantivy full-text index built into the file |
| `vec.rs`, `vec_pq.rs` | HNSW vector index + product-quantised variant; SIMD-accelerated distance |
| `text_embed.rs` | Local ONNX embedders (BGE-small/base, Nomic, GTE-large) |
| `api_embed.rs` | OpenAI cloud embeddings |
| `clip.rs` | CLIP visual embeddings (image search) |
| `whisper.rs` | Audio transcription via Candle (small/tiny/q8k Whisper) |
| `reader/` | PDF (pdf-extract / pdfium / extractous), DOCX, PPTX, XLS/XLSX extractors |
| `extract*.rs`, `text.rs`, `structure.rs`, `symspell_cleanup.rs` | document ingestion + structure-aware chunking + PDF word-spacing repair |
| `analysis/` | auto-tagging, NER (DistilBERT-NER ONNX), temporal normalization (`"last Tuesday"`) |
| `triplet/`, `graph_search.rs`, `types/logic_mesh.rs` | SPO triplet extraction → entity-relationship graph baked into the file |
| `enrich/`, `enrichment_worker.rs` | background enrichment pipeline producing "memory cards" |
| `replay/` | time-travel debugging — checkpoints + action log for agent sessions |
| `encryption/` | optional `.mv2e` capsules: AES-256-GCM, Argon2 KDF, password-based |
| `signature.rs` | Ed25519 signing — but only over **tickets** (capacity grants) and **model manifests**, not over user content |
| `pii.rs`, `audit.rs`, `acl.rs` | PII detection, audit reports, ACL (visibility / read principals / roles) |
| `doctor.rs`, `lockfile.rs`, `lock.rs` | repair tool, OS-level file locks |
| `types/` | ~25 type modules — schema, ACL, adaptive retrieval, reranker, sketch (SimHash), ticket, verification |

**Feature flags** are pervasive — defaults are `lex + pdf_extract + simd`. Heavier features (`vec`, `clip`, `whisper`, `encryption`, `temporal_enrich`, `parallel_segments`, `logic_mesh`, `replay`, `api_embed`, `pdfium`, `symspell_cleanup`) are opt-in so a minimal build stays small.

## 4. What problem it actually solves

**Memvid's value proposition is operational, not cryptographic.**

* **No infra to deploy.** A traditional RAG stack means a vector DB (Pinecone/Weaviate/Qdrant), a metadata store (Postgres), a full-text index (ES/Tantivy), an embedding service, and orchestration glue. Memvid collapses all of that into a file you `cp` between machines.
* **Portable per-agent memory.** Ship the file with the agent — to another laptop, another tenant, another LLM provider — and the indexes travel with it. No re-indexing, no schema migration.
* **Crash-safe by construction.** Embedded WAL + atomic checkpoint + per-segment SHA-256 + footer scan. The `doctor` command rebuilds corrupted indexes from the surviving frames.
* **Multi-modal in one file.** Text, PDFs, Office docs, images (CLIP), audio (Whisper), structured tables, all addressable by `mv2://` URI in the same store.
* **"Time-travel" / "Smart Recall"** — timeline queries, `as_of_frame`/`as_of_ts` parameters on search, a `replay` feature for re-running agent sessions, temporal phrase parsing.
* **High retrieval quality claims** — the README claims +35% over SOTA on LoCoMo (long-horizon conversational eval), +76% multi-hop, +56% temporal, P50 0.025 ms / P99 0.075 ms search latency. These come from their own benchmarks; reproduce-yourself harness ships in `benches/`, but the headline numbers are vendor-published.

## 5. Is the approach efficient?

**Yes — for what it's designed to do.**

* **Storage efficiency** — Zstd/LZ4 per-frame compression, optional product quantisation on HNSW, BGE-small at 384 dims (~120 MB model) keeps the indexes lean.
* **Read latency** — Tantivy segment search + HNSW + memory-mapping (`mmap = "0.9"`); SIMD distance kernels (`wide`) for vector scoring. The submillisecond P99 claim is plausible for small-to-medium files.
* **Write throughput** — WAL-batched commits, `parallel_segments` feature does multi-threaded ingestion via `crossbeam-channel` + `rayon`.
* **Footprint** — a default build (lex + pdf + simd) is small; the heavy ML features are opt-in.

**Where the model strains:**

* **Concurrency.** It's a single-writer file. The `lock.rs`/`lockfile.rs` give OS-level read/shared / write/exclusive locks. Multi-writer is not the use case.
* **Scale.** "Single file" doesn't scale horizontally — at TB-class data you'd shard, and Memvid doesn't have shard orchestration. The WAL caps at 64 MB even for ≥10 GB files; very-large workloads would push it.
* **Index rebuild cost.** Append-only means tombstones accumulate; the `doctor` and `maintenance` modules exist precisely to repack.
* **Heavy on Rust deps.** Tantivy, HNSW, ONNX Runtime, Candle, Symphonia, lopdf/pdf-extract — full-feature builds are large and have native-compile gotchas (extractous needs GraalVM; pdfium needs the C++ lib).

## 6. Is Memvid a Mnemonic competitor?

**No — they sit in different layers of the agent-memory stack and address different problems.**

| Dimension | Memvid | Mnemonic |
|---|---|---|
| **Primary problem** | "I need a portable, infra-free RAG store" | "I need *verifiable* memory whose authenticity any third party can re-check" |
| **Layer** | Local data engine (file-as-DB) | Attestation / provenance layer over storage |
| **Persistence** | The `.mv2` file on disk | SQLite + Arweave (durable) + Solana SPL Memo (timestamp anchor) |
| **Cryptography** | Ed25519 only on **tickets** and **model manifests**; blake3/SHA-256 only as data-integrity checksums | Ed25519 over **every memory item**: blake3 hash → canonical CBOR → **COSE_Sign1** envelope; identity = DID-sol / DID-key |
| **Verifiability** | "Did the file get corrupted on disk?" (yes, via segment SHA-256). No third-party signature verification of who wrote what. | "Did *that agent* really write *this content* at *that time*, and has anyone touched it since?" — verifiable by anyone with the public key |
| **Anchor** | None | Arweave (full signed bytes via ANS-104) + Solana (immutable timestamp) |
| **Retrieval** | First-class — Tantivy BM25 + HNSW + reranker + graph + timeline + sketch + CLIP + Whisper | Cosine similarity over decompressed f32 embeddings; deliberately minimal (the protocol is not in the retrieval-quality race) |
| **Storage model** | Append-only frames inside one binary file | One row per attestation in SQLite + remote anchor IDs |
| **Compression** | Zstd / LZ4 per frame; PQ on vectors | TurboQuant scalar quantisation (2–4 bits/dim) on embeddings, primarily so the compressed vector fits on-chain |
| **Server** | None — library only | MCP server (`mnemonic-mcp`) exposing 5 JSON-RPC tools; OAuth 2.1 + PKCE; payment gate (balance / x402); browser-mediated signing |
| **Interface to agents** | Native Rust + Node/Python/CLI SDKs | MCP (Cursor, Claude Desktop, Claude.ai, VS Code) |
| **Multi-tenant** | ACL fields on frames; single-writer file | OAuth-resolved `owner_pubkey` tenant isolation; multi-user by design |
| **Payment / settlement** | Not a concern | First-class — x402 + USDC, dynamic pricing engine |
| **Decentralisation** | None — local file | Public anchors on permissionless chains |

**The honest framing:** if you want a great local-only RAG engine, Memvid is more featureful than Mnemonic and out of Mnemonic's scope to compete with. If you want third-party-verifiable provenance and a settlement-aware MCP service for agent memory, Memvid doesn't address it — Mnemonic does.

They are **complementary**, not substitutes. You could very plausibly use Memvid *as the retrieval engine underneath* a Mnemonic-attested workflow.

## 7. Can we reuse the approach in Mnemonic?

There are specific, well-bounded pieces of Memvid that map cleanly onto Mnemonic's roadmap. Listed roughly by impact ÷ effort.

### 7.1 High value, fits today's architecture

**(a) Single-file packaging for the "portable memory wallet" use case.**
`docs/usecases/portable-memory-wallet.md` describes operator-owned memory across providers. Mnemonic currently stores attestations in `~/.mnemonic/attestations.db` (SQLite) plus on-chain. Adopting a `.mv2`-like envelope as the **export/import format** would give us a single-file "memory wallet" the user can carry between machines. Memvid's TOC + per-segment checksum + footer scan are a battle-tested pattern; we don't need WAL/Tantivy/HNSW inside it, but the envelope idea is directly reusable.

**(b) Embedded full-text search via Tantivy.**
Today `recall` is cosine-only over embeddings. A lexical lane (Tantivy BM25 on `attestations.content` + tags) would noticeably improve recall on keyword-heavy queries at very small footprint cost. Tantivy is `mmap`-friendly and already feature-gated in Memvid in a way we can copy.

**(c) HNSW vector index in the SQLite-adjacent file.**
Right now `recall` is brute-force cosine over every row's BLOB. HNSW would scale recall to 10⁵–10⁶ memories per agent without re-architecting. Either via the same `hnsw` crate Memvid uses, or by writing the HNSW segment to a sibling blob in the same SQLite file (use `sqlite_blob` API).

**(d) Replay / time-travel for attested sessions.**
Memvid's `replay/` module is exactly the shape of our "Agent Continuity Layer" use case (`docs/usecases/agent-continuity-layer.md`) — checkpoints, action log, divergence detection. The format/types in `src/replay/` can be lifted (Apache-2.0) and adapted to live on top of `lineage::` parent-child graphs.

**(e) PII detection, ACL, audit.**
`pii.rs`, `acl.rs`, `audit.rs` are self-contained. Mnemonic's MCP server gates writes through OAuth scope; adding frame-level ACL fields (`acl_read_principals`, `acl_visibility`) would give us tenant-internal sharing without bolting on a new system.

**(f) Doctor / repair tool.**
The `doctor` pattern (scan footer chain, rebuild missing index from surviving frames) is exactly the kind of operational hygiene that helps a long-running attestation DB. We have `migrate_owner_pubkey_columns`; we don't have a generalised `mnemonic-doctor`. Worth a small spike.

### 7.2 Architecturally aligned but bigger lift

**(g) "Smart frame" framing of attestations.**
Today attestations are flat rows. A frame-and-segment layout would let us batch many attestations into a single Arweave upload (cost win) while still letting individual COSE_Sign1 envelopes be extracted and verified. This is essentially "Mnemonic v2 file format" and would be a multi-week design effort, but it composes naturally with point (a).

**(h) Multi-modal (CLIP / Whisper) on attested content.**
If we want attestations over images and audio (e.g. an agent attesting "I observed this image"), Memvid's CLIP + Whisper integration is a working reference. Likely post-MVP.

**(i) Graph / triplet extraction.**
`triplet/` + `graph_search.rs` materialise an entity-relationship graph during ingestion. This is one possible substrate for the "Reliability Oracle" use case (cross-attestation entity reasoning).

### 7.3 Where reuse is a trap

* **Do not adopt their signature schema as-is.** `signature.rs` signs **JSON-serialised payloads** (`serde_json::to_vec` over a `TicketSignaturePayload` struct). That's not canonical — field ordering, whitespace, number normalisation are all unspecified. Mnemonic deliberately moved to **deterministic CBOR + COSE_Sign1** for exactly this reason. Memvid's signing is fine for internal ticket grants but is not safe as a general attestation envelope.
* **Do not adopt their identity model.** Memvid has no notion of agent identity beyond "the signer of this ticket". Mnemonic's DID-sol/DID-key model is a deeper construct; merging them would be a downgrade.
* **Do not adopt the QR-video legacy.** The README explicitly deprecates Memvid v1. Anything you read elsewhere about "AI memory in MP4 files" is the old design and irrelevant.
* **Don't take `.mv2e` encryption verbatim if you want cross-platform WASM.** AES-GCM + Argon2 in Rust is fine, but our `core/` compiles to WASM and the crypto deps need WASM-compatible variants — re-validate the dependency tree before adopting.

### 7.4 Direct license check

Apache-2.0. Compatible with our Apache-2.0 / MIT dual-licensing posture. Reuse needs attribution and a copy of the licence; code can be lifted with a clear `// Adapted from memvid/memvid (Apache-2.0)` header.

## 8. Bottom line

* **Memvid is a category-best local-file AI memory engine.** It's polished, fast, multi-modal, crash-safe, and operationally simpler than any vector-DB-plus-glue stack.
* **It is not what Mnemonic is.** Mnemonic is a *verifiability* protocol — cryptographic provenance signed by the agent's own key, anchored on permissionless chains, exposed via a settlement-aware MCP server. Memvid has zero of that and isn't trying to.
* **They compose well.** Memvid is a reasonable retrieval substrate underneath Mnemonic-attested content; nothing in their designs is mutually exclusive.
* **Concrete reuse wins**, in priority order:
  1. Tantivy lexical lane in `recall`
  2. HNSW vector index in `recall`
  3. `.mv2`-style single-file export envelope for the Portable Memory Wallet use case
  4. Replay/checkpoint pattern for the Agent Continuity Layer
  5. PII + ACL fields on attestations
  6. `mnemonic-doctor` repair CLI
* **Do not copy their signature/identity layer** — it is JSON-serialised and not provenance-grade.
