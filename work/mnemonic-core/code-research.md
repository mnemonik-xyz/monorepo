# Code Research: mnemonic-core extraction

Source codebase: `/home/claude/.openclaw/workspace/mnemonic-protocol/mcp/`
Research date: 2026-04-20

---

## 1. Inter-Module Dependencies

### Dependency graph (who imports whom)

```
codec/schema   ──────────────────────────────────────────────────┐
codec/canonical  ← codec/schema                                  │
codec/hash       ← codec/canonical, codec/schema                 │
codec/sign       ← codec/canonical, codec/hash, codec/schema     │
                   + solana-sdk (Keypair, Signer, Pubkey)         │
                                                                  │
embed          (standalone: sha2, reqwest::blocking)              │
compress       ← turboquant, ndarray                              │
identity       ← solana-sdk (Keypair, Signer, Pubkey) + std::fs  │
lineage        ← rusqlite + codec/schema (ParentRef, MAX_*)      │
db             ← rusqlite, std::fs + embed (HashEmbedder in tests)│
arweave        ← reqwest, solana-sdk (Keypair, Signer), sha2     │
solana         ← reqwest, solana-sdk, tokio::time::sleep, bincode │
                                                                  │
tools.rs       ← arweave, codec/{canonical,hash,schema,sign},    │
                  compress, db, embed, identity, pricing, solana  │
mcp.rs         ← arweave, compress, db, embed, pricing,          │
                  solana, tools                                   │
payment.rs     ← db, solana + axum::http::HeaderMap              │
main.rs        ← all modules + axum, clap, tower-http, tokio     │
config.rs      (standalone, std::env only)                       │
pricing.rs     ← reqwest, tokio (via async fn) — no core modules │
```

### Key finding: tools.rs / main.rs import directly from all core modules

`tools.rs` imports from: `arweave`, `codec::*`, `compress`, `db`, `embed`, `identity`, `pricing`, `solana` — all at the same level, no intermediate aggregator module.

`main.rs` calls `identity::load_or_create_keypair`, `embed::build_embedder`, `compress::EmbeddingCompressor::new`, `db::AttestationStore::open`, `solana::SolanaClient::new`, `arweave::ArweaveClient::new` directly.

`mcp.rs` holds `McpState` struct which owns instances of `ArweaveClient`, `EmbeddingCompressor`, `AttestationStore`, `Box<dyn Embedder>`, `SolanaClient`, `PricingEngine`.

`lineage.rs` is the only core module that imports another core module (`codec::schema`). All other core modules are independent of each other.

---

## 2. WASM Blockers Per Module

### Summary table

| Module | WASM Blockers | Severity |
|--------|--------------|----------|
| `codec/schema` | None | Clean |
| `codec/canonical` | None (chrono tag 1 encoding is ok) | Clean |
| `codec/hash` | None (blake3 has wasm32 support) | Clean |
| `codec/sign` | `solana-sdk` (Ed25519/Keypair) | Moderate |
| `embed` | `reqwest::blocking::Client` (OpenAIEmbedder), `tracing` | Moderate |
| `compress` | `turboquant` + `ndarray` | Moderate (see §4) |
| `identity` | `solana-sdk`, `std::fs` (load_or_create_keypair) | High |
| `db` | `rusqlite`, `std::fs` (create_dir_all, /dev/urandom) | Blocked |
| `arweave` | `reqwest::Client` (async HTTP), `solana-sdk` | High |
| `solana` | `reqwest::Client` (async HTTP), `solana-sdk`, `tokio::time::sleep` | Blocked |
| `lineage` | `rusqlite::Connection` | Blocked |

### Per-module detail

**`codec/schema.rs`** — zero WASM blockers. Pure data types (`serde`, `std::collections::BTreeMap`), no I/O, no crypto.

**`codec/canonical.rs`** — zero WASM blockers. Uses `ciborium`, `serde_json`, `chrono` (parsing only). All three crates support `wasm32-unknown-unknown`.

**`codec/hash.rs`** — zero WASM blockers. `blake3` explicitly supports WASM32.

**`codec/sign.rs`** — uses `solana-sdk` for `Keypair` and `Signature`. `solana-sdk` compiles on WASM32 (the keypair struct is pure crypto). No `tokio`, no `std::fs`. The `solana_sdk::signature::Signature::verify` call is available on WASM. **Conditionally clean** if `solana-sdk` is gated for WASM — or the signing can be refactored to use raw `ed25519-dalek`.

**`embed.rs`** — `OpenAIEmbedder` uses `reqwest::blocking::Client` which uses `std::thread` internally — a hard WASM blocker. `FastEmbedder` (ONNX) is also non-WASM. `HashEmbedder` and the trait definition itself are clean. The fix: feature-gate the `OpenAIEmbedder` as `cfg(not(target_arch = "wasm32"))`.

**`compress.rs`** — imports `turboquant` (git dep) and `ndarray`. See §4 for turboquant WASM status. `ndarray` itself supports WASM32. No `std::fs`, no `tokio`, no network.

**`identity.rs`** — uses `solana-sdk` (ok on WASM32 for the crypto functions `sign_message`, `pubkey`), but `load_or_create_keypair` uses `std::fs::read_to_string` and `std::fs::write` — hard WASM blockers. Fix: split into two parts: crypto functions (WASM-safe) and file I/O functions (native-only, behind `#[cfg(not(target_arch = "wasm32"))]`).

**`db.rs`** — `rusqlite` requires `libsqlite3` or the `bundled` feature (compiles SQLite from C). Neither is available on `wasm32-unknown-unknown`. The full `AttestationStore` is a hard WASM blocker. Core module extraction must exclude `db.rs` from WASM target, or provide a no-op stub.

**`arweave.rs`** — uses `reqwest::Client` (async, needs tokio executor), `solana-sdk` (for signing), `sha2`, `base64`. `reqwest` can target WASM via `reqwest` with `features = ["json"]` and no `blocking`, but requires a JS executor. ANS-104 data item construction code (pure byte math) is WASM-clean. Net assessment: **WASM is possible with reqwest's wasm feature** but the async executor dependency makes it complex.

**`solana.rs`** — uses `reqwest::Client` for all JSON-RPC calls and `tokio::time::sleep` inside `confirm_tx`. Hard blocker for native WASM32 without JS glue. `solana-sdk` types used (`Keypair`, `Transaction`, `Message`) compile on WASM32. The RPC client layer is the blocker.

**`lineage.rs`** — imports `rusqlite::Connection` directly. Hard WASM blocker, same as `db.rs`.

---

## 3. Cargo.toml Analysis

Source: `/home/claude/.openclaw/workspace/mnemonic-protocol/mcp/Cargo.toml`

### MCP-binary-only dependencies (do not belong in core)

| Dependency | Reason |
|-----------|--------|
| `axum = "0.8"` | HTTP server framework |
| `axum-extra = "0.10"` | Typed headers for axum |
| `tower-http = "0.6"` | CORS middleware for axum |
| `clap = "4"` | CLI argument parsing |
| `tokio = { version = "1", features = ["full"] }` | Async runtime |
| `tokio-stream = "0.1"` | SSE streaming (axum transport) |
| `dotenvy = "0.15"` | .env loading (startup only) |
| `tracing-subscriber = "0.3"` | Log output formatter (binary concern) |

### Core library dependencies (belong in mnemonic-core)

| Dependency | Used by module(s) |
|-----------|------------------|
| `sha2 = "0.10"` | `embed`, `arweave`, `db` (fallback PRNG), `tools` (legacy verify) |
| `hex = "0.4"` | `db`, `tools` |
| `base64 = "0.22"` | `arweave`, `codec/canonical`, `tools` |
| `serde = { features = ["derive"] }` | All modules |
| `serde_json = "1"` | All modules |
| `blake3 = "1"` | `codec/hash` |
| `ciborium = "0.2"` | `codec/canonical` |
| `coset = "0.3"` | `codec/sign` |
| `chrono = { features = ["serde"] }` | `codec/canonical`, `db`, `lineage`, `tools` |
| `anyhow = "1"` | All modules |
| `thiserror = "2"` | Used in error types |
| `uuid = { features = ["v4"] }` | `db`, `tools` |
| `bs58 = "0.5"` | `identity` |
| `bincode = "1"` | `solana` (tx serialization) |
| `ndarray = "0.16"` | `compress` |
| `turboquant = { git = "..." }` | `compress` |
| `tracing = "0.1"` | `embed`, `arweave` (can be kept as optional) |
| `futures = "0.3"` | Used in async streams |

### Dependencies with split placement

| Dependency | Split |
|-----------|-------|
| `rusqlite = { features = ["bundled"] }` | `mnemonic-core` (native target only); excluded from WASM |
| `reqwest = { version = "0.12" }` | `mnemonic-core` (with `features = ["json"]` for native; `wasm` feature for WASM builds); blocking feature must be native-only |
| `solana-sdk = "2.2"` | `mnemonic-core` (works on WASM32 for crypto types) |
| `solana-client = "2.2"` | MCP-only (RPC client, requires tokio) |
| `solana-transaction-status = "2.2"` | MCP-only |
| `spl-memo = "6"` | `mnemonic-core` / `solana.rs` (pure data) |
| `fastembed = { optional }` | `mnemonic-core` (native-only feature) |
| `tokio = "1"` | MCP-only for the binary; core only needs `tokio` in `arweave`/`solana` async fns (use `async-trait` or feature-gate) |

---

## 4. turboquant-rs Readiness

### Location

Git dependency in `Cargo.toml`:
```toml
turboquant = { git = "https://github.com/sivo4kin/turboquant-rs.git", branch = "master" }
```

Cargo has checked it out locally at:
`/home/claude/.cargo/git/checkouts/turboquant-rs-1c4580d69f1953dd/09a241e/`

There is also a directory at `/home/claude/.openclaw/workspace/mnemonic-protocol/external/turboquant_plus/` but it is empty (no files).

### Version and metadata

Checked-out `Cargo.toml`:
```toml
[package]
name = "turboquant"
version = "0.1.0"
edition = "2021"
description = "TurboQuant: KV cache compression via PolarQuant + QJL — Rust port"
license = "MIT"
```

No `homepage`, `repository`, `documentation`, `keywords`, or `categories` fields — **not ready for crates.io publish** as-is.

Missing required fields for crates.io: `description` is present, but `repository`, `license-file` (or confirmed SPDX), `readme` are absent. The package has no `publish = true` flag set (defaults to publishable, but no CI or version tag infrastructure is evident).

### WASM compatibility of turboquant

The crate uses:
- `ndarray = "0.16"` — WASM32-compatible
- `rand = "0.8"` with `ChaCha8Rng` (seeded PRNG) — WASM32-compatible with `getrandom = { features = ["js"] }` transitively
- `rand_chacha = "0.3"` — WASM32-compatible
- `rand_distr = "0.4"` — WASM32-compatible
- `statrs = "0.17"` — uses pure math, should be WASM32-compatible (uses `libm`)
- No `std::fs`, `std::thread`, `std::net`, or `tokio`

**Assessment:** turboquant itself has no hard WASM blockers in its own code. The WASM32 path requires adding `getrandom = { features = ["js"] }` as a dev-dependency (for the `rand` crate's getrandom backend) in the consuming crate's WASM build. The library is WASM-compatible pending that transitive feature flag.

**Key risk:** The `compress.rs` integration uses `ndarray::Array1` and `turboquant::CompressedVectors` struct directly. These types cross the mnemonic-core API boundary and must be kept internal or re-exported.

---

## 5. Public API Surface

The following `pub` items are candidates for the `mnemonic-core` public API.

### `embed` module

```rust
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Vec<f32>;
    fn dim(&self) -> usize;
    fn provider_name(&self) -> &str;
    fn model_id(&self) -> &str;
    fn is_open_weights(&self) -> bool { false }
}
pub struct HashEmbedder { dim: usize }           // test-only embedder
pub struct OpenAIEmbedder { ... }                 // native-only (reqwest::blocking)
#[cfg(feature = "local-embed")]
pub struct FastEmbedder { ... }
pub fn build_embedder(provider: &str, api_key: &str, model: &str) -> Result<Box<dyn Embedder>, String>
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32
#[cfg(test)]
pub fn build_test_embedder() -> Box<dyn Embedder>
```

### `compress` module

```rust
pub struct CompressedEmbedding {
    pub dim: usize,
    pub bit_width: usize,
    pub mse_indices_packed: Vec<u8>,
    pub qjl_signs_packed: Vec<u8>,
    pub vector_norm: f32,
    pub residual_norm: f32,
}
impl CompressedEmbedding {
    pub fn to_bytes(&self) -> Vec<u8>
    pub fn from_bytes(data: &[u8]) -> Option<Self>
}
pub struct EmbeddingCompressor { ... }
impl EmbeddingCompressor {
    pub fn new(dim: usize, bit_width: usize, seed: u64) -> Self
    pub fn compress(&self, embedding: &[f32]) -> CompressedEmbedding
    pub fn decompress(&self, compressed: &CompressedEmbedding) -> Vec<f32>
    pub fn compression_ratio(&self) -> f64
}
```

### `identity` module

```rust
pub fn load_or_create_keypair(path: &Path) -> anyhow::Result<Keypair>  // native-only
pub fn pubkey_base58(kp: &Keypair) -> String
pub fn did_sol(kp: &Keypair) -> String
pub fn did_key(kp: &Keypair) -> String
pub fn sign_bytes(kp: &Keypair, message: &[u8]) -> Vec<u8>
pub fn verify_signature(pubkey: &Pubkey, message: &[u8], signature: &[u8]) -> bool
```

### `db` module (native-only)

```rust
pub struct AttestationStore { conn: Connection }
impl AttestationStore {
    pub fn open(path: &Path) -> anyhow::Result<Self>
    pub fn in_memory() -> anyhow::Result<Self>
    pub fn save_attestation(...) -> anyhow::Result<()>
    pub fn create_api_key(&self, owner_pubkey: &str) -> anyhow::Result<String>
    pub fn get_owner_pubkey(&self, api_key: &str) -> anyhow::Result<Option<String>>
    pub fn get_balance(&self, api_key: &str) -> anyhow::Result<Option<i64>>
    pub fn deduct_balance(&self, api_key: &str, amount: i64, description: &str) -> anyhow::Result<()>
    pub fn credit_deposit(&self, api_key: &str, amount: i64, tx_sig: &str) -> anyhow::Result<i64>
    pub fn mark_x402_nonce(&self, tx_sig: &str) -> anyhow::Result<()>
    pub fn record_attestation_cost(...) -> anyhow::Result<()>
    pub fn get_pnl_stats(&self, days: u64) -> anyhow::Result<PnlStats>
    pub fn find_by_tx(&self, tx_id: &str) -> anyhow::Result<Option<AttestationRow>>
    pub fn count(&self, signer: &str) -> anyhow::Result<i64>
    pub fn search(&self, query_embedding: &[f32], signer: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>>
}
pub struct AttestationRow { pub attestation_id, content, content_hash, solana_tx, arweave_tx, signer_pubkey: String }
pub struct SearchResult { pub attestation_id, content, content_hash, tags, solana_tx, arweave_tx, created_at: String, relevance_score: f32 }
pub struct PnlStats { pub period_days: u64, attestations, earned_micro_usdc, cost_sol_lamports, cost_micro_usdc_equiv, net_micro_usdc: i64, margin_pct, avg_sol_price_usdc: f64 }
```

### `arweave` module

```rust
pub struct ArweaveClient { base_url: String, client: reqwest::Client }
impl ArweaveClient {
    pub fn new(base_url: &str) -> Self
    pub async fn write(&self, payload: &str, keypair: &Keypair) -> anyhow::Result<String>
    pub async fn write_bytes(&self, data: &[u8], keypair: &Keypair) -> anyhow::Result<String>
    pub async fn read(&self, tx_id: &str) -> anyhow::Result<Vec<u8>>
    pub async fn mine(&self) -> anyhow::Result<()>
    pub async fn health_check(&self) -> bool
}
```

### `solana` module

```rust
pub struct SolanaClient { rpc_url: String, client: reqwest::Client }
impl SolanaClient {
    pub fn new(rpc_url: &str) -> Self
    pub async fn write_memo(&self, keypair: &Keypair, memo: &str) -> anyhow::Result<String>
    pub async fn read_memo(&self, tx_sig: &str) -> anyhow::Result<Option<serde_json::Value>>
    pub async fn airdrop(&self, pubkey: &Pubkey, lamports: u64) -> anyhow::Result<String>
    pub async fn health_check(&self) -> bool
    pub async fn get_tx_signers(&self, tx_sig: &str) -> anyhow::Result<Vec<String>>
    pub async fn verify_usdc_transfer(&self, tx_sig, recipient, usdc_mint, min_amount) -> anyhow::Result<Option<u64>>
}
```

### `lineage` module

```rust
pub fn init_lineage_schema(conn: &Connection) -> anyhow::Result<()>
pub fn record_parents(conn: &Connection, child_id: &str, parents: &[ParentRef], created_at: &str) -> anyhow::Result<()>
pub fn get_parents(conn: &Connection, artifact_id: &str) -> anyhow::Result<Vec<ParentRef>>
pub fn get_children(conn: &Connection, artifact_id: &str) -> anyhow::Result<Vec<String>>
pub fn validate_parents(conn, new_artifact_id, parents, attestation_exists) -> Result<(), String>
pub fn traverse_lineage(conn, start_id, max_depth, direction, get_node_info) -> anyhow::Result<LineageResult>
pub struct LineageResult { pub root, direction: String, depth_traversed: usize, nodes: HashMap<String, LineageNode>, edges: Vec<LineageEdge>, chain_valid: bool }
pub struct LineageNode { pub artifact_type, content_hash, producer, created_at: String, verified: bool }
pub struct LineageEdge { pub from, to: String, role: Option<String> }
```

### `codec` module

```rust
// schema
pub const MAX_PARENTS: usize = 16;
pub const MAX_DEPTH: usize = 64;
pub struct ParentRef { pub artifact_id: String, pub role: Option<String> }
pub enum ArtifactType { RagContext, RagResult, AgentState, Receipt, Memory }
pub struct ArtifactSchema { pub artifact_type, version, required_fields, optional_fields, cbor_field_order }
pub const RAG_CONTEXT_V1, RAG_RESULT_V1, AGENT_STATE_V1, RECEIPT_V1, MEMORY_V1: ArtifactSchema
pub fn get_schema(artifact_type: &str, version: u32) -> Option<&'static ArtifactSchema>
pub fn validate_artifact(artifact: &Value, schema: &ArtifactSchema) -> Result<(), String>

// canonical
pub fn to_canonical_cbor(artifact: &JsonValue, schema: &ArtifactSchema) -> Result<Vec<u8>, String>
pub fn from_canonical_cbor(bytes: &[u8]) -> Result<JsonValue, String>

// hash
pub fn hash_artifact(artifact: &Value, schema: &ArtifactSchema) -> Result<String, String>
pub fn hash_bytes(data: &[u8]) -> String
pub fn verify_hash(data: &[u8], expected_hex: &str) -> bool

// sign
pub struct SignedArtifact { pub cose_bytes: Vec<u8>, content_hash: String, canonical_cbor: Vec<u8> }
pub struct VerificationResult { pub valid, content_integrity, cose_signature, algorithm_valid: bool, content_hash, signer: String, payload: Vec<u8> }
pub fn sign_artifact(artifact: &Value, schema: &ArtifactSchema, keypair: &Keypair) -> Result<SignedArtifact, String>
pub fn verify_artifact(cose_bytes: &[u8], expected_hash: Option<&str>) -> Result<VerificationResult, String>
```

---

## 6. Existing Test Coverage

### Unit tests per module (`#[cfg(test)]` blocks)

| Module | Test count | Notes |
|--------|-----------|-------|
| `embed.rs` | 8 | Covers: determinism, dimension, normalization, model_id, cosine similarity, build rejection |
| `compress.rs` | 4 | Covers: roundtrip, serialize/deserialize, compression ratio, compressed size |
| `identity.rs` | 4 | Covers: did_sol, did_key, sign/verify, keypair file roundtrip |
| `db.rs` | 2 | Covers: save+count, semantic search ranking; imports `embed::HashEmbedder` |
| `arweave.rs` | 0 | No unit tests; arlocal/irys paths require HTTP |
| `solana.rs` | 0 | No unit tests; all methods require live RPC |
| `lineage.rs` | 9 | Covers: record parents, get children, validate parents (ok/not-found/too-many), cycle detection, no-false-cycle, empty parents, traverse ancestors |
| `codec/schema.rs` | 4 | Covers: schema lookup, validate artifact, type strings, cbor_field_order coverage |
| `codec/canonical.rs` | 7 | Covers: determinism, 1000x determinism, roundtrip, field order, optional field omission, different artifacts, timestamp as CBOR tag 1 |
| `codec/hash.rs` | 6 | Covers: determinism, blake3 identity, hash length, hash changes with content, verify_hash, consistency with canonical_cbor |
| `codec/sign.rs` | 7 | Covers: sign produces bytes, sign/verify roundtrip, detects wrong hash, verify without hash, content hash is blake3 of cbor, different keypairs same hash, all 5 schemas |

### Integration tests (`tests/`)

| File | Test count | Notes |
|------|-----------|-------|
| `integration_cbor.rs` | 5 | Full sign→serialize→verify pipeline; inlines codec helpers (binary crate can't be imported directly) |
| `proptest_canonical.rs` | 1 (proptest macro) | Property-based: random artifact payloads → canonical CBOR must be deterministic; also inlines codec functions |

### Benchmarks (`benches/`)

| File | Notes |
|------|-------|
| `decompress.rs` | Criterion benchmarks for `EmbeddingCompressor::decompress` |
| `cbor_codec.rs` | Criterion benchmarks for `to_canonical_cbor` and `hash_artifact` |

### Coverage gaps

- `arweave.rs` and `solana.rs` have zero tests — all functionality requires live endpoints
- `db.rs` has minimal tests (2); payment methods (`deduct_balance`, `credit_deposit`, `mark_x402_nonce`, `get_pnl_stats`) are untested
- Integration tests re-implement codec helpers inline because the source is a binary crate — this duplication goes away once `mnemonic-core` is a proper lib crate
- No WASM-specific tests exist anywhere

### Test framework

- Runner: `cargo test`
- Async tests: `tokio-test = "0.4"` in dev-dependencies (not currently used in test blocks; `solana.rs` tests would need `#[tokio::test]`)
- Property tests: `proptest = "1"` (used in `proptest_canonical.rs`)
- Benchmarks: `criterion = "0.5"` with `harness = false`
- Temp files: `tempfile = "3"` (used in `identity.rs` keypair roundtrip test)
