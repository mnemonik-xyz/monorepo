# Architecture

## Purpose
Technical architecture overview for AI agents. Helps agents understand HOW the system is built.

---

## Tech Stack

**`core/` — Rust library (dual-target)**
- Language: Rust stable, 2021 edition
- WASM export: wasm-bindgen + wasm-pack
- Why: single codebase compiles to native (for MCP server) and WASM (for webapp). No code duplication.

**`mcp/` — MCP server (Rust binary)**
- HTTP server: axum
- Protocol: JSON-RPC 2.0 over stdio + HTTP
- Depends on: `core/` as Cargo workspace member

**`webapp/` — Demo web app (TypeScript + React)**
- Bundler: Vite
- UI: React + Tailwind CSS
- Local model: Ollama API (Qwen2.5-7B-Instruct)
- Core logic: WASM build of `core/` (no backend needed)
- Deploy: Cloudflare Pages (static)

**`docs/` — Protocol documentation**
- Format: Markdown
- Contents: whitepaper, protocol spec, ADRs, Mermaid/PlantUML diagrams, roadmap

---

## Project Structure

The root repo is a **git submodule container**. `core/`, `mcp/`, and `webapp/` are each independent git repositories registered as submodules. `docs/` is a regular directory inside the root repo.

`core/` (submodule `mnemonic-core`) — Rust library crate. `src/` is split by responsibility: `codec/` holds SHA-256 hashing, schema encoding, canonical CBOR serialisation, and COSE_Sign1 signing; `embed/` holds the Embedder trait and two provider implementations (fastembed, openai); `compress/` holds TurboQuant scalar quantisation; `identity/` holds Ed25519 keypair generation, DID derivation, and signing; `storage/` holds the `AttestationStore` and `LineageStore` traits with a SQLite implementation; `arweave/` holds the ANS-104 bundle builder with deep hash and Avro encoding; `solana/` holds SPL Memo writer and reader; `lineage/` holds the directed acyclic graph of parent–child relationships between attestations.

`mcp/` (submodule `mnemonic-mcp`) — MCP server binary. `src/` is split by concern: `main.rs` bootstraps Axum and McpState; `mcp.rs` is the JSON-RPC 2.0 dispatcher; `tools.rs` implements the five MCP tools; `payment.rs` implements the dual payment gate; `pricing.rs` is the dynamic pricing engine; `config.rs` reads configuration from env vars.

`webapp/` (submodule `mnemonic-webapp`) — TypeScript + React demo app. `src/` contains React components, Tailwind styles, and a `wasm/` subfolder that imports the compiled WASM package from `core/`.

---

## Key Dependencies

**`core/`:**
- `fastembed` — local ONNX embedding runner (all-MiniLM-L6-v2, 384-dim)
- `wasm-bindgen` — Rust↔JS FFI bridge
- `solana-sdk` — Ed25519 keypair, SPL Memo instruction
- `rusqlite` (bundled) — SQLite without system dependency
- `sha2` — SHA-256 hashing
- `ed25519-dalek` — standalone signing for ANS-104 items
- `reqwest` — async HTTP (Arweave/Irys, CoinGecko)

**`mcp/`:**
- `axum` — async HTTP server
- `tokio` — async runtime
- `serde_json` — JSON-RPC 2.0 serialisation
- `uuid` — attestation IDs

---

## External Integrations

**Arweave / Irys**
- Purpose: permanent storage for attestation payloads
- Auth: ANS-104 bundle item signed with agent Ed25519 keypair
- Production: `https://uploader.irys.xyz` / Local dev: `http://localhost:1984`

**Solana**
- Purpose: immutable timestamp anchor via SPL Memo
- Auth: same Ed25519 keypair
- Configurable RPC

**Ollama (webapp only)**
- Purpose: local LLM chat for demo
- Auth: none (localhost:11434)

**OpenAI (optional)**
- Purpose: higher-quality embeddings
- Auth: `OPENAI_API_KEY` env var
- Fallback chain: openai → fastembed → Err

---

## Data Flow

**sign_memory:** Content is embedded, TurboQuant-compressed, SHA-256 hashed, uploaded to Arweave as a signed ANS-104 bundle item, anchored on Solana via SPL Memo, then saved to SQLite with the full-precision embedding.

**recall:** Query is embedded with the same provider, then scored against all SQLite embeddings via cosine similarity, returning the top-k results ordered by score.

**verify:** The Solana SPL Memo is fetched to extract the Arweave tx ID and expected hash; the Arweave payload is fetched and its content re-hashed; the result is `verified`, `tampered`, or `not_found`.

**local mode:** Same flow as above but skips Arweave upload and Solana tx. Synthetic IDs prefixed `local:`.

---

## Data Model

**Database:** SQLite (`~/.mnemonic/attestations.db`)

**attestations** — one row per memory item. Key fields: `attestation_id` UUID PK, `content` text, `content_hash` SHA-256 hex, `tags` JSON array, `solana_tx`, `arweave_tx`, `signer_pubkey`, `created_at`.

**memory_embeddings** — full-precision vectors for cosine search. Key fields: `attestation_id` FK, `embedding` BLOB (f32 array), `dim`, `provider`.

**attestation_costs** — P&L tracking, full mode only. Key fields: `attestation_id` FK, `irys_lamports`, `sol_tx_fee_lamports`, `sol_price_usdc`, `charge_micro_usdc`.

Schema is applied on first DB connection via `CREATE TABLE IF NOT EXISTS`. No migration tooling for MVP. Private keys are never stored in SQLite — loaded from a keypair file at runtime only.
