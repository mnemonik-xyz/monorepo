# Architecture

## Purpose
Technical architecture overview for AI agents. Helps agents understand HOW the system is built.

---

## Tech Stack

**`core/` — Rust library (dual-target)**
- Language: Rust stable, 2021 edition
- WASM export: wasm-bindgen + wasm-pack
- Why: single codebase compiles to native (for MCP server) and WASM (for webapp). No code duplication.
- WASM embedding: when building the WASM webapp, include a local embedding model that runs in-browser (e.g. `ort` web backend or `transformers.js`) so users don't need an AI provider API key.

**`mcp/` — MCP server (Rust binary)**
- HTTP server: axum
- Protocol: JSON-RPC 2.0 over stdio + HTTP
- Depends on: `core/` as Cargo workspace member

**`webapp/` — Demo web app (TypeScript + React)**
- Bundler: Vite
- UI: React + Tailwind CSS
- Local model: Ollama API (Qwen2.5-3B)
- Core logic: WASM build of `core/` (no backend needed)
- Deploy: Cloudflare Pages (static)

**`docs/` — Protocol documentation**
- Format: Markdown
- Contents: whitepaper, protocol spec, ADRs, Mermaid/PlantUML diagrams, roadmap

---

## Project Structure

The root repo is a **git submodule container**. `core/`, `mcp/`, and `webapp/` are each independent git repositories registered as submodules. `docs/` is a regular directory inside the root repo.

`core/` (submodule `mnemonic-core`) — Rust library crate. `src/` is split by responsibility: `codec/` holds SHA-256 hashing, schema encoding, canonical CBOR serialisation, and COSE_Sign1 signing; `embed/` holds the Embedder trait and two provider implementations (fastembed, openai); `compress/` holds TurboQuant scalar quantisation; `identity/` holds Ed25519 keypair generation, DID derivation, and signing; `storage/` holds the `AttestationStore` and `LineageStore` traits with a SQLite implementation; `arweave/` holds the ANS-104 bundle builder with deep hash and Avro encoding; `solana/` holds SPL Memo writer and reader; `lineage/` holds the directed acyclic graph of parent–child relationships between attestations and exposes a `Direction` enum (`Ancestors` / `Descendants` / `Both`) for BFS traversal.

`mcp/` (submodule `mnemonic-mcp`) — MCP server binary. `src/` is split by concern: `main.rs` bootstraps Axum and McpState; `mcp.rs` is the JSON-RPC 2.0 dispatcher; `tools.rs` implements the five MCP tools; `payment.rs` implements the dual payment gate; `pricing.rs` is the dynamic pricing engine; `config.rs` reads configuration from env vars; `oauth.rs` is the OAuth 2.1 + PKCE server (RFC 8414 metadata, RFC 7591 dynamic client registration, HS256 JWT issuance, Bearer-auth middleware, `tower_governor` rate limiter); `pending.rs` is the `PendingBundles` LRU+TTL+per-user-cap store backing the browser-mediated signing flow; `api.rs` exposes `GET /api/pending/{id}` and `POST /api/sign-callback` (capability-based auth via `correlation_id`); `cors_policy.rs` is the predicate that allows first-party + Anthropic + Cursor + OpenAI origins plus localhost loopback for dev; `lib.rs` re-exports the modules so integration tests can link against the binary; `bin/mint-test-jwt.rs` is a CLI helper used by CI smoke and Playwright e2e to mint test JWTs.

`webapp/` (submodule `mnemonic-webapp`) — TypeScript + React demo app. `src/` contains React components, Tailwind styles, and a `wasm/` subfolder that imports the compiled WASM package from `core/`. Routes (`src/App.tsx`): `/` Landing, `/install` Install (Cursor / VS Code / Claude.ai deeplink emitter + IdentityPanel for keypair generate / import / export / clear), `/chat` Chat demo, `/sign/:correlationId` Sign (browser-mediated COSE signing for `mnemonic_sign_memory`), `/oauth/consent` Consent (OAuth 2.1 challenge sign-and-redirect). The sign and consent pages call WASM exports `sign_cose_payload` / `sign_challenge` against the locally-stored keypair (`localStorage["mnemonic.identity"]`) — private keys never leave the browser. `webapp/scripts/build-wasm.sh` runs `wasm-pack build core --target web --features wasm`; the npm `build` script chains `build:wasm && tsc -b && vite build`. Playwright e2e in `webapp/e2e/` covers install deeplinks (`install.spec.ts`), OAuth flow (`oauth-flow.spec.ts`), and deferred-signing (`deferred-sign-flow.spec.ts`); helpers in `_helpers.ts` mint test JWTs via the `mint-test-jwt` binary.

Top-level repo files added during the AI-tools integration phase: `smithery.yaml` (Smithery hosted-MCP catalogue manifest pointing at `https://mcp.mnemonik.xyz/mcp`) and `mcp/deploy/nginx-mcp-subdomain.conf` (in-tree source for the `mcp.mnemonik.xyz` server block deployed to `/etc/nginx/sites-available/mnemonic-mcp`).

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
- `jsonwebtoken` — HS256 JWTs for OAuth Bearer tokens (Decision 11 of `mnemonic-integrations`)
- `tower-http` (CORS) + `tower_governor` (per-IP rate limiter on `/oauth/*`)
- `serde_urlencoded` — `/oauth/token` accepts both `application/json` and `application/x-www-form-urlencoded` (Cursor/VS Code use form, Claude.ai uses JSON)

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

**Smithery (hosted MCP catalogue)**
- Purpose: discoverability for the hosted `mcp.mnemonik.xyz` deployment
- Manifest: `smithery.yaml` at repo root (transport=streamable-http, advertised endpoint `/mcp`, OAuth `authorization_servers` pointer)
- Submission: manual via Smithery dashboard; CI does not auto-publish

**Cursor / VS Code / Claude.ai (MCP clients)**
- Purpose: install the hosted MCP server via deeplink
- Auth: OAuth 2.1 + PKCE against `mcp.mnemonik.xyz/oauth/*`. Cursor and VS Code POST `/oauth/token` as `application/x-www-form-urlencoded`; Claude.ai POSTs the same endpoint as `application/json` and registers itself via `POST /oauth/register` (RFC 7591 dynamic client registration). Claude.ai POSTs JSON-RPC to apex `/`; Cursor and VS Code POST to `/mcp`. Both paths are wired to the same handler.

**OpenAI (optional)**
- Purpose: higher-quality embeddings
- Auth: `OPENAI_API_KEY` env var
- Provider priority (per `core/src/embed/mod.rs` `build_embedder`): fastembed (open, verifiable) > openai (proprietary but semantic). When `EMBED_PROVIDER=openai`, fallback chain is openai → fastembed → Err.

---

## Data Flow

**sign_memory:** Content is embedded, TurboQuant-compressed, SHA-256 hashed, uploaded to Arweave as a signed ANS-104 bundle item, anchored on Solana via SPL Memo, then saved to SQLite with the full-precision embedding.

**recall:** Query is embedded with the same provider, then scored against all SQLite embeddings via cosine similarity, returning the top-k results ordered by score.

**verify:** The Solana SPL Memo is fetched to extract the Arweave tx ID and expected hash; the Arweave payload is fetched and its content re-hashed; the result is `verified`, `tampered`, or `not_found`.

**local mode:** Same flow as above but skips Arweave upload and Solana tx. Synthetic IDs prefixed `local:`.

**Browser-mediated `sign_memory` (Decision 12 of `mnemonic-integrations`):** when the MCP server runs in hosted mode, it does not hold the user's signing key. `tools.rs::sign_memory` builds the canonical-CBOR payload server-side, stores it in `pending::PendingBundles` keyed by a UUID `correlation_id` bound to the OAuth user's pubkey, and returns a JSON-RPC response containing `https://mnemonik.xyz/sign/{correlation_id}` plus an expiry. The webapp `Sign.tsx` page fetches the bundle via `GET /api/pending/{id}` (capability-based auth — possession of the unguessable `correlation_id` is the capability), runs `sign_cose_payload` in WASM against the locally-stored keypair, and POSTs the COSE_Sign1 envelope to `/api/sign-callback`. The server validates the envelope, persists the attestation tagged with `owner_pubkey`, and (in `full` mode) writes Arweave + Solana asynchronously. `PendingBundles` enforces a TTL, an LRU cap, and a per-user-pubkey limit so an authenticated client cannot flood the queue.

**OAuth 2.1 + PKCE flow (Decision 11):** MCP clients hit `GET /.well-known/oauth-authorization-server` for RFC 8414 metadata, then either `POST /oauth/register` (Claude.ai DCR) or use a static client_id. They redirect the user to `GET /oauth/authorize?...` which serves a bootstrap HTML page that loads `/oauth/consent`; the React Consent page asks WASM `sign_challenge` to sign the PKCE-bound challenge bytes with the local Ed25519 key and POSTs the raw signature back to `/oauth/authorize` (raw Ed25519, not COSE_Sign1 — the challenge is a single binary blob, no canonical-CBOR wrapping required). The server verifies the signature against `challenge_bytes`, issues an HS256 JWT (1h TTL, claims bound to the user pubkey via `sub`), and redirects to the client's callback. The token endpoint accepts both `application/json` (Claude.ai) and `application/x-www-form-urlencoded` (Cursor / VS Code).

---

## Data Model

**Database:** SQLite (`~/.mnemonic/attestations.db`)

**attestations** — one row per memory item. Key fields: `attestation_id` UUID PK, `content` text, `content_hash` SHA-256 hex, `tags` JSON array, `solana_tx`, `arweave_tx`, `signer_pubkey`, `created_at`, `owner_pubkey` (added by `migrate_owner_pubkey_columns` — OAuth-resolved tenant scope; legacy rows are NULL and won't match any caller, enforcing tenant isolation per Decision 13).

**memory_embeddings** — full-precision vectors for cosine search. Key fields: `attestation_id` FK, `embedding` BLOB (f32 array), `dim`, `provider`.

**api_keys** — OAuth/Bearer credentials. Key fields: `api_key` PK, `owner_pubkey` (deposit account, legacy), `oauth_pubkey` (links the row to the OAuth user pubkey from `sub` — added by the same migration), `balance_micro_usdc`, `created_at`.

**attestation_costs** — P&L tracking, full mode only. Key fields: `attestation_id` FK, `irys_lamports`, `sol_tx_fee_lamports`, `sol_price_usdc`, `charge_micro_usdc`.

Schema is applied on first DB connection via `CREATE TABLE IF NOT EXISTS`; the `owner_pubkey` / `oauth_pubkey` columns are added idempotently by `migrate_owner_pubkey_columns` (SQLite has no `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, so the helper inspects `pragma_table_info` first). No general migration tooling for MVP. Private keys are never stored in SQLite — loaded from a keypair file at runtime (server) or `localStorage["mnemonic.identity"]` (browser).
