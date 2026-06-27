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

`core/` (submodule `mnemonic-core`) — Rust library crate. `src/` is split by responsibility: `codec/` holds SHA-256 hashing, schema encoding, canonical CBOR serialisation, and COSE_Sign1 signing; `embed/` holds the Embedder trait and two provider implementations (fastembed, openai); `compress/` holds TurboQuant scalar quantisation; `identity/` holds Ed25519 keypair generation, DID derivation, and signing; `storage/` holds the `AttestationStore` and `LineageStore` traits with a SQLite implementation; `arweave/` holds the ANS-104 bundle builder with deep hash and Avro encoding; `solana/` holds SPL Memo writer and reader; `lineage/` holds the directed acyclic graph of parent–child relationships between attestations and exposes a `Direction` enum (`Ancestors` / `Descendants` / `Both`) for BFS traversal. The `webapp-rethink` feature added `codec::schema::POST_V1` (a blog post IS a signed public attestation — markdown body lives in the standard `content` slot so `content_hash` commits to it, reusing the whole sign pipeline), a `blog_posts` projection table (slug PK, idempotent `CREATE TABLE IF NOT EXISTS`), and three read queries on `SqliteStore` — `list_public_artifacts(limit)`, `attestation_timeline(since)`, and the blog_post upsert/list/get — surfaced as `mnemonic_core::storage::{PublicArtifact, TimelineBucket, BlogPost}`. The public-artifacts query reuses the cross-owner public-only predicate `SEARCH_SQL_CROSS_OWNER_VIS`.

`mcp/` (submodule `mnemonic-mcp`) — MCP server binary. `src/` is split by concern: `main.rs` bootstraps Axum and McpState; `mcp.rs` is the JSON-RPC 2.0 dispatcher; `tools.rs` implements the five MCP tools; `payment.rs` implements the dual payment gate; `pricing.rs` is the dynamic pricing engine; `config.rs` reads configuration from env vars; `oauth/mod.rs` is the OAuth 2.1 + PKCE server (RFC 8414 metadata, RFC 7591 dynamic client registration, HS256 access JWT issuance, refresh-grant branch in `token_handler`, env-driven `server_origin()` via `MCP_PUBLIC_BASE_URL`, Bearer-auth middleware, `tower_governor` rate limiter); `oauth/refresh.rs` is the refresh-token storage module (SQLite-backed `refresh_tokens` table, 5-branch `BEGIN IMMEDIATE` rotation transaction, in-process LRU pair cache, hourly evictor task — added by `refresh-token-rotation`); `pending.rs` is the `PendingBundles` LRU+TTL+per-user-cap store backing the browser-mediated signing flow; `api.rs` exposes `GET /api/pending/{id}` and `POST /api/sign-callback` (capability-based auth via `correlation_id`); `cors_policy.rs` is the predicate that allows first-party + Anthropic + Cursor + OpenAI origins plus localhost loopback for dev; `lib.rs` re-exports the modules so integration tests can link against the binary; `bin/mint-test-jwt.rs` is a CLI helper used by CI smoke and Playwright e2e to mint test JWTs. The `webapp-rethink` feature added public read routes to `api.rs` (`GET /artifacts`, `GET /analytics/attestations`, `GET /blog`, `GET /blog/:slug`, `GET /blog/feed.xml` Atom 1.0 syndication) plus `GET /.well-known/agent.json` (A2A AgentCard mirroring the OAuth well-known, advertising the publish skill); `/artifacts` and the `?q=` search path return `visibility = public` rows only (Decision 6), unknown slug → 404, transient DB errors → 200 empty (the public page never 5xxes). Agent-native publishing lives in `publish.rs` (shared pipeline): the MCP tool `mnemonic_publish_post` (native path) and a Micropub-shaped `POST /blog` (OAuth2 Bearer, JSON h-entry + form-urlencoded) on the bearer-authed subrouter — anonymous publish rejected (401), per-pubkey rate limit. The pipeline reuses the core sign path verbatim (POST_V1 → validate → sign → `save_attestation(Public, Local)` → `upsert_blog_post`); V1 publishes a free `local` public write (no x402/on-chain), `author` carries the agent name, re-publishing the same title replaces the `blog_posts` row by slug PK while the prior public attestation stays in the append-only ledger. `publish.rs::fire_rebuild_hook` best-effort pings the optional `BLOG_REBUILD_HOOK` (SSRF-safe: `hook_url_allowed()` http(s)-scheme allowlist, `redirect::Policy::none`, detached task) so a publish triggers a webapp rebuild. **The mcp server renders NO HTML — all new routes are JSON/XML only (Decision 9).**

`webapp/` (submodule `mnemonic-webapp`) — TypeScript + React demo app. `src/` contains React components, Tailwind styles, and a `wasm/` subfolder that imports the compiled WASM package from `core/`. Routes (`src/App.tsx`): `/` Landing, `/install` Install (Cursor / VS Code / Claude.ai deeplink emitter + IdentityPanel for keypair generate / import / export / clear), `/chat` Chat demo, `/sign/:correlationId` Sign (browser-mediated COSE signing for `mnemonic_sign_memory`), `/oauth/consent` Consent (OAuth 2.1 challenge sign-and-redirect). The sign and consent pages call WASM exports `sign_cose_payload` / `sign_challenge` against the locally-stored keypair (`localStorage["mnemonic.identity"]`) — private keys never leave the browser. `webapp/scripts/build-wasm.sh` runs `wasm-pack build core --target web --features wasm`; the npm `build` script chains `build:wasm && tsc -b && vite build`. Playwright e2e in `webapp/e2e/` covers install deeplinks (`install.spec.ts`), OAuth flow (`oauth-flow.spec.ts`), deferred-signing (`deferred-sign-flow.spec.ts`), and the public-page smoke (`ledger.spec.ts`); helpers in `_helpers.ts` mint test JWTs via the `mint-test-jwt` binary.

The `webapp-rethink` feature added a public surface beyond the OAuth/signing pages: `/ledger` (Ledger.tsx — receipt-card list of public attestations, recall-by-meaning search, write_mode filter), `/analytics` (Analytics.tsx + bespoke TimelineChart.tsx — zero-dep custom SVG attestations-over-time chart with `animate-draw` + reduced-motion), `/blog` + `/blog/:slug` (Blog.tsx / BlogPost.tsx — markdown via react-markdown, no rehype-raw). Typed API clients live in `src/lib/{ledger,blog,seo,links}`: `ledger.ts` / `blog.ts` fetch the mcp read routes and degrade to labelled `sample:true` data on any failure (Decision 2), so pages ship and render before the backend exists; `seo.tsx` is the `<Seo>` component (React 19 head hoisting) + `safeJsonLd` JSON-LD emitter; `links.ts` builds Solana/Arweave explorer URLs (returns null for `local:` tx). Clients reach the API via `VITE_MCP_BASE` cross-origin (no hardcoded origins). **Build-time SEO prerender** (`webapp/scripts/prerender.mjs` + `src/entry-server.tsx`, Vite SSG, zero new deps, no headless browser) renders the static routes (`/`, `/ledger`, `/analytics`, `/blog`) to static HTML and, fetching `GET $VITE_MCP_BASE/blog` at build, emits one static `dist/blog/<slug>/index.html` per post (title/canonical/OG + Article JSON-LD + body) plus a dynamic `dist/sitemap.xml`; `public/robots.txt` (Allow `/`, Disallow `/sign/` `/oauth/`) points at it. The npm `build` script chains `tsc -b && vite build && node scripts/prerender.mjs`. The webapp is a **standalone static deploy** consuming the headless mcp JSON API cross-origin (Decision 9) — see deployment.md.

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
- `serde_urlencoded` — `/oauth/token` accepts both `application/json` and `application/x-www-form-urlencoded` (VS Code and Claude.ai use form-encoded; Cursor uses JSON — per `oauth/mod.rs:975-977`)

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
- Auth: OAuth 2.1 + PKCE against `mcp.mnemonik.xyz/oauth/*`. Per `mcp/src/oauth/mod.rs:975-977`: VS Code and Claude.ai POST `/oauth/token` as `application/x-www-form-urlencoded`; Cursor POSTs as `application/json`. Claude.ai also registers itself via `POST /oauth/register` (RFC 7591 dynamic client registration). Claude.ai POSTs JSON-RPC to apex `/`; Cursor and VS Code POST to `/mcp`. Both paths are wired to the same handler.

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

**Mode dispatch (`modes-user-choice` feature, T1–T4):** Write mode is a per-request user choice on `mnemonic_sign_memory` (`mode: "local" | "participate"`, optional, default `local`; absent field falls back to env-var for shipped clients). Four pieces wire this together:

- **`whoami` envelope contract.** `mnemonic_whoami` returns `supported_modes`, `default_mode`, and `participate_cost { currency, amount_cents, payment_methods }` (or `null` on local-only deploys) alongside the legacy fields. Derived once from operator config at process start (cached on `McpState::envelope`); clients use it to discover what the deploy can serve before attempting a write. Requesting an unsupported mode returns the typed `-32010 UnsupportedMode { requested, supported }` error — never a silent downgrade.
- **Single-source-of-truth resolver.** `mcp/src/tools.rs::resolve_write_mode` is a pure function that maps the optional input field to a typed `ResolvedMode { write_mode, explicit }`, with strict rejection (`-32602 InvalidParams`) on every non-canonical input (case-variant, null, whitespace, unknown). `mcp_handler` calls the resolver once before the paywall gate, and the same resolved `WriteMode` is threaded into the paywall check, `sign_memory`, and `save_attestation` — paywall + persisted column cannot drift.
- **Delivery confirmation (`tools::confirm_delivery_or_demote`).** Shared helper invoked by both the inline path (`sign_memory_inline`) and the deferred Cloud-tier path (`api::sign_callback_handler`). After the Arweave + Solana anchor it re-fetches the COSE bytes (wall-clock budget `MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS`, default 15s, exponential backoff), runs `verify_cose` plus an in-process recall existence check, and on any failure demotes the row to `WriteMode::Local` via `INSERT OR REPLACE`, skips `record_attestation_cost`, releases the reserved payment via `payment::refund_balance`, and returns `-32011 DeliveryNotConfirmed { stage, arweave_tx, solana_tx, row_demoted_to: "local", attestation_id }`. Lock discipline: two short critical sections, neither holds the SQLite mutex across an `.await`.
- **DoS guard (`RefundsBySubject`).** A `DashMap<quota_subject, SlidingWindowCounter>` in `mcp/src/payment.rs`, keyed via `derive_quota_subject(headers, payment_mode)` — `blake3(api_key).to_hex()` for Bearer-authed callers, `blake3(tx_sig).to_hex()` for x402 — *not* `owner_pubkey` (Ed25519 keys rotate freely; billable subjects don't, so the latter is the right blast-radius). Incremented in the refund branch; consulted at the *entry* of the participate path before any chain write. Exceeded → `-32011 DeliveryQuotaExceeded` short-circuit, zero chain spend. Configured by `MNEMONIC_DELIVERY_QUOTA_THRESHOLD` (default 5), `MNEMONIC_DELIVERY_QUOTA_WINDOW_SECS` (default 60), and a background evictor running every `MNEMONIC_DELIVERY_QUOTA_EVICT_SECS` (default 30) so map size tracks active spenders, not lifetime cardinality.

Rows are tagged with `write_mode` (`local` | `participate`) and `recall` spans both modes for one owner. `verify` routes by the stored column, not the env-var, with an `owner_pubkey` filter on `find_by_tx` / `find_write_mode_by_tx` so cohabiting tenants in one DB cannot probe each other's rows. See `work/modes-user-choice/user-spec.md` (canonical) and `tech-spec.md` for the full design.

**Browser-mediated `sign_memory` (Decision 12 of `mnemonic-integrations`):** when the MCP server runs in hosted mode, it does not hold the user's signing key. `tools.rs::sign_memory` builds the canonical-CBOR payload server-side, stores it in `pending::PendingBundles` keyed by a UUID `correlation_id` bound to the OAuth user's pubkey, and returns a JSON-RPC response containing `https://mnemonik.xyz/sign/{correlation_id}` plus an expiry. The webapp `Sign.tsx` page fetches the bundle via `GET /api/pending/{id}` (capability-based auth — possession of the unguessable `correlation_id` is the capability), runs `sign_cose_payload` in WASM against the locally-stored keypair, and POSTs the COSE_Sign1 envelope to `/api/sign-callback`. The server validates the envelope, persists the attestation tagged with `owner_pubkey`, and (in `full` mode) writes Arweave + Solana asynchronously. `PendingBundles` enforces a TTL, an LRU cap, and a per-user-pubkey limit so an authenticated client cannot flood the queue.

**OAuth 2.1 + PKCE flow (Decision 11):** MCP clients hit `GET /.well-known/oauth-authorization-server` for RFC 8414 metadata, then either `POST /oauth/register` (Claude.ai DCR) or use a static client_id. They redirect the user to `GET /oauth/authorize?...` which serves a bootstrap HTML page that loads `/oauth/consent`; the React Consent page asks WASM `sign_challenge` to sign the PKCE-bound challenge bytes with the local Ed25519 key and POSTs the raw signature back to `/oauth/authorize` (raw Ed25519, not COSE_Sign1 — the challenge is a single binary blob, no canonical-CBOR wrapping required). The server verifies the signature against `challenge_bytes`, issues an HS256 access JWT (3600s TTL by default, overridable via `MCP_JWT_TTL_SECS` clamped to `[60, 604800]`) PLUS a 32-random-byte opaque refresh token, and redirects to the client's callback. The token endpoint accepts both `application/json` (Cursor) and `application/x-www-form-urlencoded` (VS Code / Claude.ai) — see `oauth/mod.rs:975-977`. Discovery advertises both `authorization_code` and `refresh_token` grant types in `grant_types_supported`. The advertised resource origin is driven by `MCP_PUBLIC_BASE_URL` so dev tunnels and third-party operators can pass clients' RFC 8707 protected-resource origin check.

**Refresh-token rotation (`refresh-token-rotation` feature):** clients that hold a refresh token call `POST /oauth/token` with `grant_type=refresh_token` to silently obtain a fresh `(access, refresh)` pair before the access JWT expires (Stripe MCP precedent — Anthropic-managed connectors do this automatically; VS Code 1.93+ does it natively). The server stores only `blake3(MCP_REFRESH_SALT || plaintext)` at rest in the `refresh_tokens` table — plaintext is never persisted. Rotation runs in a single `BEGIN IMMEDIATE` SQLite transaction with five outcome branches: A=happy-path rotation, B=reuse-window cache hit (idempotent retry, same `(access, refresh)` returned), B'=reuse-window cache miss (fail-closed — Decision 5), C=replay outside reuse window (family-revoke in the same transaction — Decision 8), D=expired, E=unknown. The in-process `ReuseCache` LRU publishes the new pair BEFORE COMMIT (Decision 5 — CWE-362 race fix); an hourly background evictor reclaims `WHERE expires_at < now()` rows. Per `refresh-token-rotation` Decision 6, `OAuthState` opens a SECOND `rusqlite::Connection` on the same SQLite file as `McpState.store` (NOT a clone, NOT Arc-shared — SQLite's WAL writer-lock serialises file-level writes; two physical Connections per `OAuthState::new` are well-defined). T12 verified live silent rotation on prod via `journalctl -u mnemonic-mcp` — look for `refresh_rotate success outcome="rotated" branch="A"` INFO lines and `family_revoke` WARN lines; D14 logging policy whitelist (only `outcome`, `branch`, `family_id`, `sub`, `remote_addr`, `request_id` — never plaintext / token_hash / salt / full access JWT).

---

## Data Model

**Database:** SQLite (`~/.mnemonic/attestations.db`)

**attestations** — one row per memory item. Key fields: `attestation_id` UUID PK, `content` text, `content_hash` SHA-256 hex, `tags` JSON array, `solana_tx`, `arweave_tx`, `signer_pubkey`, `created_at`, `owner_pubkey` (added by `migrate_owner_pubkey_columns` — OAuth-resolved tenant scope; legacy rows are NULL and won't match any caller, enforcing tenant isolation per Decision 13).

**memory_embeddings** — full-precision vectors for cosine search. Key fields: `attestation_id` FK, `embedding` BLOB (f32 array), `dim`, `provider`.

**api_keys** — OAuth/Bearer credentials. Key fields: `api_key` PK, `owner_pubkey` (deposit account, legacy), `oauth_pubkey` (links the row to the OAuth user pubkey from `sub` — added by the same migration), `balance_micro_usdc`, `created_at`.

**attestation_costs** — P&L tracking, full mode only. Key fields: `attestation_id` FK, `irys_lamports`, `sol_tx_fee_lamports`, `sol_price_usdc`, `charge_micro_usdc`.

**blog_posts** — slug-indexed projection over public POST_V1 attestations (added by `webapp-rethink`), so `GET /blog` and `/blog/:slug` are cheap ordered lookups. Key fields: `slug` PK, `title`, `body_markdown`, `tags`, `author` (carries the publishing agent name), `published_at`, plus `attestation_id` / `content_hash` linking back to the immutable ledger row. Re-publishing the same title upserts by slug. No `summary` / `reading_minutes` columns — the webapp derives those client-side (first prose paragraph / `ceil(words/200)`) in `blog.ts` so both the SPA and the prerender get real meta descriptions.

Schema is applied on first DB connection via `CREATE TABLE IF NOT EXISTS`; the `owner_pubkey` / `oauth_pubkey` columns are added idempotently by `migrate_owner_pubkey_columns` (SQLite has no `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, so the helper inspects `pragma_table_info` first). No general migration tooling for MVP. Private keys are never stored in SQLite — loaded from a keypair file at runtime (server) or `localStorage["mnemonic.identity"]` (browser).

---

## Further reading

- Competitive positioning vs decentralized RAG, zkTAM, V3DB and related directions: [docs/competitive-landscape/](../../../../docs/competitive-landscape/).
- Condensed TurboQuant compression principles (knowledge-DB ref for `compress/` design decisions): [docs/research/condensed-principles.md](../../../../docs/research/condensed-principles.md).
- Foundational paper that motivated the protocol: [docs/research/paper.pdf](../../../../docs/research/paper.pdf).
