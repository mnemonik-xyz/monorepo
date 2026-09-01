# How Mnemonic Works

Companion to [WHITEPAPER §5.3 Pipeline Walkthrough](./WHITEPAPER.md#53-pipeline-walkthrough-sign--recall--verify). Where the whitepaper sketches the sign / recall / verify flows at protocol granularity, this document maps each step onto the actual `mnemonic-core` and `mnemonic-mcp` modules so contributors can navigate the codebase, understand the dependency direction, and reason about operational concerns such as lock discipline, storage modes, and payment gating.

## Module map

The repository is a Cargo workspace (`resolver = "2"`) with two members. The dependency graph is strictly one-way: `mcp/` depends on `core/`; `core/` never references `mcp/`.

| Module | Responsibility |
|---|---|
| `mnemonic-core::codec` | Canonical CBOR encoding, blake3 hashing, COSE_Sign1 sign/verify, schema registry (`memory`, `rag.context`, `rag.result`, `agent.state`, `receipt`). |
| `mnemonic-core::embed` | `Embedder` trait plus providers: `OpenAIEmbedder`, `FastEmbedder` (behind `local-embed`, ships an ONNX model on first run), `MockEmbedder` (`#[cfg(test)]` only). |
| `mnemonic-core::compress` | TurboQuant scalar quantization at 2/3/4 bits per dimension; default 4. |
| `mnemonic-core::identity` | Ed25519 keypair load/generate, base58 encoding, `did:sol` and `did:key` derivation. |
| `mnemonic-core::storage` | `AttestationStore` trait and `SqliteStore` implementation; `LineageStore` trait; SQL lives in `core/src/storage/sqlite.rs`. |
| `mnemonic-core::arweave` | Full-mode persistence: ANS-104 bundle builder, Irys upload, deep hash + Avro encoding. |
| `mnemonic-core::solana` | Full-mode anchoring: `SolanaClient` for SPL Memo writes/reads. |
| `mnemonic-core::lineage` | Parent-child artifact DAG with cycle detection and BFS traversal (`Direction::{Ancestors, Descendants, Both}`). |
| `mnemonic-mcp` | JSON-RPC 2.0 dispatcher (`mcp.rs`), the MCP tools (`tools.rs`; 8 by default, 11 with `trajectory-experimental`), Axum bootstrap (`main.rs`), payment gating (`payment.rs`), pricing engine (`pricing.rs`), env-driven config (`config.rs`). |

## End-to-end walkthrough — sign_memory

Implemented in `mcp/src/tools.rs::sign_memory`.

1. **Embed.** Call the active `Embedder` to produce a full-precision f32 vector. Provider is selected at startup from `EMBED_PROVIDER`; the server aborts if no embedder is available. Embed quality dictates recall quality, so it must match between sign time and query time.
2. **Compress.** Run the embedding through `compress::EmbeddingCompressor` (TurboQuant, default 4 bits/dim). The compressed bytes ride along inside artifact metadata as portable proof-of-existence; they are not used for local recall.
3. **Build artifact.** Assemble the canonical JSON shape: `artifact_id`, `type`, `schema_version`, `content`, `producer` (DID-sol), `created_at`, `tags`, embedding metadata.
4. **Canonicalize.** `codec::canonical::to_canonical_cbor` produces a deterministic byte sequence with stable field ordering. Determinism is required so the hash is reproducible across runtimes.
5. **Hash.** `codec::hash::hash_bytes` computes blake3 over the canonical CBOR. This is the artifact's identity.
6. **Sign.** The CBOR payload is wrapped in a COSE_Sign1 envelope. **Who holds the signing key depends on the caller** — the operator's key never signs content authored by another identity:
   - **Inline (server-signed).** Taken only when the writer *is* the operator: the stdio / single-tenant path with no JWT, or a JWT whose subject equals the operator's own pubkey. `codec::sign::sign_artifact` signs under the server's Ed25519 identity and the flow continues to step 7.
   - **Deferred (client-signed).** Taken for every JWT write owned by a different identity — *including* an explicit `mode: "local"`. The server parks the canonical bundle and returns `{status: "awaiting_signature", correlation_id, approve_url, content_hash, expires_in: 300}`. The client signs locally (browser approval, or a headless `POST /api/sign-callback`), and only then does step 7 run. Nothing is persisted or anchored until the callback lands; bundles expire after 300 seconds. `mnemonic_check_pending` resolves the `correlation_id` to the final state.
7. **Persist.**
   - **Local mode:** write COSE bytes plus the uncompressed embedding to `SqliteStore`; return synthetic `local:` tx IDs.
   - **Full mode:** upload COSE bytes to Arweave via the Irys client; submit an SPL Memo on Solana carrying `{"h": blake3, "a": arweave_tx, "v": 2}`; record both tx IDs alongside the row in SQLite. Cost is captured in `attestation_costs` for P&L tracking.

## End-to-end walkthrough — recall

Implemented in `mcp/src/tools.rs::recall` over `core/src/storage/sqlite.rs`.

1. Embed the query with the same provider used at sign time.
2. Cosine-score the query vector against every uncompressed f32 embedding in the local `memory_embeddings` table.
3. Return the top-k rows ordered by score, joined to their `attestations` row metadata.

Recall is intentionally local: SQLite read plus an in-process scan, no chain calls. Uncompressed f32 wins here because cosine similarity is sensitive to small magnitude shifts and TurboQuant compressed bytes are optimized for portability and inner-product approximation, not for being the canonical retrieval index. The compressed form on Arweave is proof-of-existence; the uncompressed form in SQLite is the search index.

## End-to-end walkthrough — verify

Implemented in `mcp/src/tools.rs::verify`.

- **Artifact-only path.** Read the COSE_Sign1 envelope (locally, or by fetching the Arweave object in full mode), decode to canonical CBOR, recompute blake3 over the bytes, and check the recomputed hash matches the stored value. Then validate the COSE_Sign1 signature against the producer's claimed Ed25519 public key. Result is `verified`, `tampered`, or `not_found`.
- **Chain-anchored path (full mode).** In addition to the above, fetch the SPL Memo on Solana referenced by `solana_tx`, parse its `{h, a, v}` payload, and confirm that the on-chain hash and Arweave tx ID match the local row. This adds independent timestamped existence to the integrity and authorship checks.

## Operational notes

- **Storage modes** (`STORAGE_MODE`). `local`: SQLite only, synthetic `local:` tx IDs, free, offline; suitable for development and single-node use. `full`: COSE bytes to Arweave and an SPL Memo to Solana, requiring a funded Ed25519 keypair on a live RPC. `STORAGE_MODE` sets the operator's *capability and default*, **not** a global switch — the write mode is a per-request choice via the optional `mode: "local" | "participate"` field on `mnemonic_sign_memory` (requests that omit it fall back to the operator default, so shipped legacy clients keep working). Rows are tagged with a `write_mode` column and `recall` spans both modes for one owner, so a single owner's `local` and `participate` writes coexist in one DB by design. A `participate` write only succeeds after the anchored bytes pass a recall+verify round-trip; on failure the row is demoted to `local` and no payment is charged. See [tools.md § Write modes](./tools.md#write-modes-local-vs-participate).
- **Payment modes** (`PAYMENT_MODE`, HTTP transport in `full` mode only). `none` | `balance` (Bearer-token API key checked against the live pricing engine) | `x402` (HTTP 402 challenge, retry with `X-Payment` header) | `both`. Only `mnemonic_sign_memory` is paid, and only for `participate` writes; `whoami`, `recall`, `verify`, `prove_identity`, `check_pending`, and `publish_post` are free.
- **Lock discipline.** `rusqlite::Connection` is `!Send`. Always wrap `SqliteStore` in `std::sync::Mutex` in async contexts and never hold the lock across an `.await`. Tool handlers explicitly take `&std::sync::Mutex<SqliteStore>` and scope their guards before any IO.
- **TurboQuant bit width.** Default 4 bits per dimension. Never change for an existing database — old and new compressed embeddings become incomparable, breaking any cross-node comparison and the artifact metadata commitment.

## Architectural rules (audit-enforced)

- Payment methods (`create_api_key`, `deduct_balance`, `credit_deposit`, `mark_x402_nonce`, `record_attestation_cost`, `get_pnl_stats`, `get_owner_pubkey`, `verify_usdc_transfer`) live only in `mcp/src/payment.rs`. None in `core/`.
- `verify_usdc_transfer` is a standalone function taking `&SolanaClient`, not a method on it.
- `pricing.rs` lives in `mcp/`, never in `core/`.
- No `HashEmbedder` anywhere; `MockEmbedder` is allowed only inside `#[cfg(test)]` blocks.
- `core/` has zero references to anything in `mcp/`. The dependency graph is one-way.

## Pointers

- [tools.md](./tools.md) — full MCP tool reference: inputs, outputs, auth, write modes.
- [WHITEPAPER.md](./WHITEPAPER.md) — §4 Core Insight, §5 Architecture Overview (including §5.3 Pipeline Walkthrough), §6 Artifact Model, §7 Trust Model, §11 Current Implementation Status.
- [research/condensed-principles.md](./research/condensed-principles.md) — TurboQuant design principles distilled.
- [usecases/](./usecases/) — concrete agent-memory use-case roles for the protocol.
- [competitive-landscape/](./competitive-landscape/) — positioning vs decentralized RAG, zkTAM, V3DB, and adjacent directions.
- [problems/](./problems/) — open issues and unresolved questions.
