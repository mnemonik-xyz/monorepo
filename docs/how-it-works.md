# How Mnemonic Works

Companion to [Yellow paper §5.3 Pipeline Walkthrough](./YELLOWPAPER.md#53-pipeline-walkthrough-sign--recall--verify). Where the whitepaper sketches the sign / recall / verify flows at protocol granularity, this document maps each step onto the actual `mnemonic-core` and `mnemonic-mcp` modules so contributors can navigate the codebase, understand the dependency direction, and reason about operational concerns such as lock discipline, storage modes, and payment gating.

## Module map

The repository is a Cargo workspace (`resolver = "2"`) with two members. The dependency graph is strictly one-way: `mcp/` depends on `core/`; `core/` never references `mcp/`.

| Module | Responsibility |
|---|---|
| `mnemonic-core::codec` | Canonical CBOR encoding, blake3 hashing, COSE_Sign1 sign/verify, schema registry (`memory`, `rag.context`, `rag.result`, `agent.state`, `receipt`). |
| `mnemonic-core::embed` | `Embedder` trait plus providers: `OpenAIEmbedder`, `FastEmbedder` (behind `local-embed`, ships an ONNX model on first run), `MockEmbedder` (`#[cfg(test)]` only). |
| `mnemonic-core::compress` | TurboQuant scalar quantization at 2/3/4 bits per dimension; default 4. |
| `mnemonic-core::identity` | Ed25519 keypair load/generate, base58 encoding, `did:sol` and `did:key` derivation. |
| `mnemonic-core::sealed` | `seal_memory`, `open_memory`, `open_with_key`, `make_grant`, `open_grant`, `link_fragment`: sealed write / open / grant / bearer-link operations. Uses `core/src/encrypt.rs` (XChaCha20-Poly1305 + HPKE) and `core/src/identity/recall_key.rs`. |
| `mnemonic-core::storage` | `AttestationStore` trait and `SqliteStore` implementation; `LineageStore` trait; SQL lives in `core/src/storage/sqlite.rs`. Sealed rows store outer CBOR in the `sealed_blob` column; `content` is empty. |
| `mnemonic-core::arweave` | Full-mode persistence: ANS-104 bundle builder, Irys upload, deep hash + Avro encoding. |
| `mnemonic-core::solana` | Full-mode anchoring: `SolanaClient` for SPL Memo writes/reads. |
| `mnemonic-core::lineage` | Parent-child artifact DAG with cycle detection and BFS traversal (`Direction::{Ancestors, Descendants, Both}`). |
| `mnemonic-mcp` | JSON-RPC 2.0 dispatcher (`mcp.rs`), the MCP tools (`tools.rs`; 12 by default, 15 with `trajectory-experimental`), Axum bootstrap (`main.rs`), payment gating (`payment.rs`), pricing engine (`pricing.rs`), env-driven config (`config.rs`). |

## End-to-end walkthrough — sign_memory

Implemented in `mcp/src/tools.rs::sign_memory`.

1. **Embed.** Call the active `Embedder` to produce a full-precision f32 vector. Provider is selected at startup from `EMBED_PROVIDER`; the server aborts if no embedder is available. Embed quality dictates recall quality, so it must match between sign time and query time.
2. **Compress.** Run the embedding through `compress::EmbeddingCompressor` (TurboQuant, default 4 bits/dim). The compressed bytes ride along inside artifact metadata as portable proof-of-existence; they are not used for local recall.
3. **Build artifact.** Assemble the canonical JSON shape: `artifact_id`, `type`, `schema_version`, `content`, `producer` (DID-sol), `created_at`, `tags`, embedding metadata.
4. **Canonicalize.** `codec::canonical::to_canonical_cbor` produces a deterministic byte sequence with stable field ordering. Determinism is required so the hash is reproducible across runtimes.
5. **Hash.** `codec::hash::hash_bytes` computes blake3 over the canonical CBOR. This is the artifact's identity.
6. **Sign.** Only an `anchored` write gets a COSE_Sign1 envelope. The author of the memory signs it. The operator key never signs content from a different identity:
   - **Local write (no signature).** The server stores the blake3 hash only. Nothing signs the row, so no keychain prompt occurs. Over HTTP, a JSON Web Token (JWT) write with an explicit `mode: "local"` takes this path. The JWT subject owns the row.
   - **Inline (agent-signed).** The stdio path with no JWT, in `anchored` mode. The operator key is the identity of the local agent. `codec::sign::sign_artifact` signs, then step 7 runs. HTTP requests never take this path.
   - **Deferred (client-signed).** Each JWT write in `anchored` mode, and each JWT write without a `mode` field. The server parks the canonical bundle and returns `{status: "awaiting_signature", correlation_id, approve_url, content_hash, expires_in: 300}`. The client signs locally (browser approval, or a headless `POST /api/sign-callback`). Then step 7 runs. Nothing is persisted or anchored until the callback lands. Bundles expire after 300 seconds. `mnemonic_check_pending` resolves the `correlation_id` to the final state.
7. **Persist.**
   - **Local mode:** write the content hash plus the uncompressed embedding to `SqliteStore`; return synthetic `local:` tx IDs.
   - **Full mode:** upload COSE bytes to Arweave via the Irys client; submit an SPL Memo on Solana carrying `{"h": blake3, "a": arweave_tx, "v": 2}`; record both tx IDs alongside the row in SQLite. Cost is captured in `attestation_costs` for P&L tracking.

## End-to-end walkthrough — sealed write (client-side paths)

Implemented in `core/src/sealed/api.rs::seal_memory`, called from the local
MCP server, CLI, browser extension, and webapp before any network call.

1. **Generate K.** `OsRng` produces 32 bytes. This is the content key.
2. **Derive X25519 key.** The author's Ed25519 identity key is mapped to X25519
   via `VerifyingKey::to_montgomery`. The keychain is opened at this step.
3. **Seal.** The inner `MEMORY_V1` canonical CBOR (content, tags, embedding)
   is padded to the next multiple of 256 bytes and encrypted with
   `XChaCha20Poly1305` under `K` and a 24-byte `OsRng` nonce.
4. **Key commitment.** `kc = blake3::derive_key("mnemonic sealed v1 key commitment", K)`.
5. **Wrap.** `K` is wrapped to the author's X25519 public key using HPKE
   (X25519HkdfSha256 / HkdfSha256 / ChaCha20Poly1305, RFC 9180). One wrap is
   written to `SEALED_V1.wraps`. The wrap holds `enc` (ephemeral sender key,
   32 bytes) and the wrapped key (48 bytes).
6. **Zeroize K.** `K` is zeroized immediately after the wrap.
7. **Build SEALED_V1.** Assemble outer artifact: `artifact_id`, `type`,
   `schema_version`, `alg`, `nonce`, `ct`, `kc`, `wraps`, `created_at`, `producer`.
8. **Sign (anchored only).** COSE_Sign1 over the outer canonical CBOR. The
   signature is over the ciphertext, not the plaintext.
9. **Persist.** `SqliteStore` stores the outer CBOR in the `sealed_blob` column.
   The `content` column is empty. The uncompressed f32 embedding is not stored
   in SQLite for sealed rows (it would invert to approximate plaintext).

**Sealed recall (client-side).** The client fetches the `sealed_blob` from
the server, calls `open_memory(sealed_blob, x25519_secret)`, verifies `kc`,
and decrypts. Search scores are computed locally over the decrypted inner
embedding.

**Grant flow (`mnemonic_share`).** `make_grant(k, reader_x25519_pubkey)` wraps
`K` to the reader using HPKE with a fresh ephemeral key. The signed `GRANT_V1`
record is stored on the server. The reader calls `open_grant(grant_cbor, their_x25519_secret)`
to recover `K`, then `open_with_key(sealed_blob, k)` to decrypt.

---

## End-to-end walkthrough — recall

Implemented in `mcp/src/tools.rs::recall` over `core/src/storage/sqlite.rs`.

1. Embed the query with the same provider used at sign time.
2. Select the candidate rows. An authenticated caller gets only its own rows, private and public. An anonymous caller gets only rows with `visibility = 'public'`, from all owners. A private row goes only to its owner. Sealed rows (`sealed_blob IS NOT NULL`) are counted separately as `sealed_hidden` in the anonymous path and excluded from the public vector index.
3. Cosine-score the query vector against the uncompressed f32 embedding of each candidate row. Sealed rows whose inner artifact was decrypted by the recall key contribute their decrypted embedding.
4. Return the top-k rows ordered by score, joined to their `attestations` row metadata. Sealed rows that were decrypted carry `sealed: true`; undecrypted sealed rows are counted in `sealed_hidden`.

Recall is intentionally local: SQLite read plus an in-process scan, no chain calls. Uncompressed f32 wins here because cosine similarity is sensitive to small magnitude shifts and TurboQuant compressed bytes are optimized for portability and inner-product approximation, not for being the canonical retrieval index. The compressed form on Arweave is proof-of-existence; the uncompressed form in SQLite is the search index.

## Public Ledger listing and recall — `GET /artifacts`

Implemented in `mcp/src/api.rs::artifacts_handler` and `mcp/src/chain_stats.rs`. The webapp Ledger page uses this route. The route needs no authentication (available now).

Query parameters:

| Parameter | Meaning |
|---|---|
| `q` | Recall text. If you omit it, the route returns the newest rows first. |
| `limit` | Maximum number of rows in the page. |
| `source` | `all` (default), `on_node` (local writes) or `on_chain` (anchored writes). |

Privacy rule (available now): the route returns only SQLite rows with `visibility = 'public'`. This applies to the listing and to recall, for each `source` value. The route never returns a private row. A row with no `visibility` value (a legacy row) counts as private. A local write is always private, so `source=on_node` shows only memories published with `mnemonic_publish_post`. An anchored `anchored` write is always public, because its content is plain text on Arweave (owner decision D-8). Each row has `plaintext_on_arweave`: `true` when the content is plain text on Arweave. Sealed (encrypted) anchored writes are planned.

Chain recovery (available now): when the operator sets `CHAIN_STATS_WALLETS`, the server reads its anchored memories from Solana memos and Arweave. It keeps them in memory as a snapshot. SQLite does not store this snapshot.

Recall with `q` (available now):

1. The server embeds `q` one time.
2. SQLite rows get a cosine score against their f32 embeddings.
3. Each snapshot item gets a cosine score against the embedding in its signed payload. The server decodes `metadata.embedding_f32` or `metadata.embedding_compressed` (TurboQuant) into an in-memory index. The server builds this index again after each snapshot refresh.
4. A snapshot item without a usable embedding matches only if its content or tags contain `q`. The match ignores letter case. These items get a score of 1.0, equal to a perfect cosine match.
5. The server sorts all matches by score and returns the first `limit` rows.

A snapshot item that has the same `arweave_tx` or `content_hash` as a SQLite row does not appear. If that SQLite row is public, the SQLite row appears instead. If that SQLite row is private, neither appears. A snapshot item with no SQLite row appears, because its bytes are already public on Arweave. With `source=on_node`, the result contains no snapshot items.

The same rules apply to a listing without `q`.

Each recall row has a `match` field: `semantic` (cosine score) or `text` (text match). A listing without `q` has no `match` field.

## End-to-end walkthrough — verify

Implemented in `mcp/src/tools.rs::verify`.

- **Artifact-only path.** Read the COSE_Sign1 envelope (locally, or by fetching the Arweave object in full mode), decode to canonical CBOR, recompute blake3 over the bytes, and check the recomputed hash matches the stored value. Then validate the COSE_Sign1 signature against the producer's claimed Ed25519 public key. Result is `verified`, `tampered`, or `not_found`.
- **Chain-anchored path (full mode).** In addition to the above, fetch the SPL Memo on Solana referenced by `solana_tx`, parse its `{h, a, v}` payload, and confirm that the on-chain hash and Arweave tx ID match the local row. This adds independent timestamped existence to the integrity and authorship checks.

## Operational notes

- **Storage modes** (`STORAGE_MODE`). `local`: SQLite only, synthetic `local:` tx IDs, free, offline; suitable for development and single-node use. `full`: COSE bytes to Arweave and an SPL Memo to Solana, requiring a funded Ed25519 keypair on a live RPC. `STORAGE_MODE` sets the operator's *capability and default*, **not** a global switch — the write mode is a per-request choice via the optional `mode: "local" | "anchored"` field on `mnemonic_sign_memory` (requests that omit it fall back to the operator default, so shipped legacy clients keep working). Rows are tagged with a `write_mode` column and `recall` spans both modes for one owner, so a single owner's `local` and `anchored` writes coexist in one DB by design. An `anchored` write only succeeds after the anchored bytes pass a recall+verify round-trip; on failure the row is demoted to `local` and no payment is charged. See [tools.md § Write modes](./tools.md#write-modes-local-vs-anchored).
- **Payment modes** (`PAYMENT_MODE`, HTTP transport in `full` mode only). `none` | `balance` (Bearer-token API key checked against the live pricing engine) | `x402` (HTTP 402 challenge, retry with `X-Payment` header) | `both`. Only `mnemonic_sign_memory` is paid, and only for `anchored` writes; `whoami`, `recall`, `verify`, `prove_identity`, `check_pending`, and `publish_post` are free. On `x402`, an agent key that is linked to a Google account first gets a free daily quota of `anchored` writes (available now; see [tools.md § Free daily quota](./tools.md#free-daily-quota)). The quota counts per Google account, per client IP address and globally, and only for signed envelopes up to `MNEMONIC_FREE_ANCHOR_MAX_BYTES` (default 16 KiB). If a write fails after the chain write started, the server gives back the account and IP counters but not the global counter. Paid writes need no Google account.
- **Client IP behind a proxy** (`TRUSTED_PROXIES`, available now). The rate limiters and the per-IP free quota use the real client IP address. The server reads `X-Forwarded-For` only when the TCP peer is a trusted proxy, and it takes the rightmost untrusted hop. IPv6 addresses are grouped by /64.
- **Content size** (`MNEMONIC_MAX_CONTENT_BYTES`, available now). `mnemonic_sign_memory` refuses content above 32 KiB (default and maximum) on every transport.
- **Lock discipline.** `rusqlite::Connection` is `!Send`. Always wrap `SqliteStore` in `std::sync::Mutex` in async contexts and never hold the lock across an `.await`. Tool handlers explicitly take `&std::sync::Mutex<SqliteStore>` and scope their guards before any IO.
- **TurboQuant bit width.** Default 4 bits per dimension. Never change for an existing database — old and new compressed embeddings become incomparable, breaking any cross-node comparison and the artifact metadata commitment.

## Architectural rules (audit-enforced)

- Payment methods (`create_api_key`, `deduct_balance`, `credit_deposit`, `claim_x402_nonce`, `release_x402_nonce`, `record_attestation_cost`, `get_pnl_stats`, `get_owner_pubkey`, `verify_usdc_transfer`) live only in `mcp/src/payment.rs`. None in `core/`.
- `verify_usdc_transfer` is a standalone function taking `&SolanaClient`, not a method on it.
- `pricing.rs` lives in `mcp/`, never in `core/`.
- No `HashEmbedder` anywhere; `MockEmbedder` is allowed only inside `#[cfg(test)]` blocks.
- `core/` has zero references to anything in `mcp/`. The dependency graph is one-way.

## Pointers

- [tools.md](./tools.md) — full MCP tool reference: inputs, outputs, auth, write modes.
- [YELLOWPAPER.md](./YELLOWPAPER.md) — §4 Core Insight, §5 Architecture Overview (including §5.3 Pipeline Walkthrough), §6 Artifact Model, §7 Trust Model, §11 Current Implementation Status.
- [research/condensed-principles.md](./research/condensed-principles.md) — TurboQuant design principles distilled.
- [usecases/](./usecases/) — concrete agent-memory use-case roles for the protocol.
- [competitive-landscape/](./competitive-landscape/) — positioning vs decentralized RAG, zkTAM, V3DB, and adjacent directions.
- [problems/](./problems/) — open issues and unresolved questions.
