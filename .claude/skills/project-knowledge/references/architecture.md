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

The repo is a Cargo workspace with two members (`core` and `mcp`) plus a TypeScript `webapp/` and a `docs/` directory.

`core/src/` is split into modules by responsibility: `embed/` holds the Embedder trait and three provider implementations (fastembed, openai, hash); `compress/` holds TurboQuant scalar quantisation; `identity/` holds Ed25519 keypair generation, DID derivation, and signing; `attest/` holds the SHA-256 hashing and AttestationRecord type; `storage/` holds the SQLite store; `arweave/` holds the ANS-104 bundle builder including deep hash and Avro encoding; `solana/` holds the SPL Memo writer and reader; `wasm/` holds all `#[wasm_bindgen]` exports and is only compiled for the `wasm32-unknown-unknown` target.

`mcp/src/` is split by concern: `main.rs` bootstraps the Axum server and McpState; `mcp.rs` is the JSON-RPC 2.0 dispatcher; `tools.rs` implements the five MCP tools; `payment.rs` implements the dual payment gate (balance + x402); `pricing.rs` is the dynamic pricing engine using Irys and CoinGecko; `config.rs` reads all configuration from environment variables.

`webapp/src/` contains React components, Tailwind styles, and a `wasm/` subfolder that imports the compiled WASM package from `core/`.

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
- Fallback chain: openai → fastembed → hash

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
