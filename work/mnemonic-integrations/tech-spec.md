---
created: 2026-04-26
status: draft
branch: dev
size: L
---

# Tech Spec: mnemonic-integrations Phase 1 (Hackathon MVP)

## Solution

Ship a public `mcp.mnemonik.xyz` endpoint that any MCP-capable AI tool (Cursor, VS Code, Claude.ai Pro, Perplexity Pro) can install as a remote connector. Identity is generated client-side: the existing `mnemonic-core` Rust crate is wrapped with `#[wasm_bindgen]` to expose `generate_keypair`, `sign_challenge`, `export_keypair_json`, `import_keypair_json` to JavaScript. The webapp at `mnemonik.xyz` adds two routes: a landing page and an install-hub (with deeplink buttons + identity panel). OAuth 2.1 + PKCE is implemented as a small Axum module in `mcp/`; the user signs the OAuth challenge in-browser with their localStorage keypair, the server issues a JWT bound to the pubkey.

For the hackathon demo, the server runs in `STORAGE_MODE=local` — synthetic `local:` IDs, SQLite-only, no Arweave/Solana RPC dependency. This keeps the live demo fast and offline-tolerant. The full on-chain mode is preserved in code but unused on stage.

Smithery listing is the single discovery surface for Phase 1; Anthropic Connectors / mcp.directory / Glama are deferred. Docker GHCR publish, Turnkey MPC migration, browser extension, and additional webapp pages are explicitly in `backlog.md`.

Total scope: ~11 dev-days, 14 implementation tasks across 4 waves + audit + final.

## Architecture

### What we're building / modifying

- **`mcp/` (modified)** — upgrade HTTP transport from request-response JSON-RPC to streamable HTTP per MCP spec 2025. Add `mcp/src/oauth.rs` (new) implementing OAuth 2.1 + PKCE server (`/oauth/authorize`, `/oauth/token`, JWT issuance). Add Axum middleware that validates the `Authorization: Bearer <jwt>` header and resolves the user's pubkey before tool dispatch. `payment.rs` is **not** refactored — `PAYMENT_MODE=none` for the demo, the OAuth pubkey hook is wired as a no-op for billing.
- **`core/` (minimally modified)** — add `core/src/wasm/` module (new) containing `#[wasm_bindgen]` wrappers around the existing `identity` functions. The wrappers are gated behind `#[cfg(target_arch = "wasm32")]` and a new feature flag `wasm`. No business-logic changes in `core/`.
- **`webapp/` (extended)** — add WASM build pipeline (wasm-pack output integrated into Vite). Add two new pages: `Landing` (root `/`) and `Install` (`/install`). The Install page shows: identity panel (DID/pubkey, Generate / Import / Export buttons) + deeplink buttons for Cursor / VS Code / Claude.ai. Existing chat demo at `/chat` is preserved; landing replaces the current root.
- **`smithery.yaml` (new, repo root)** — Smithery registry manifest with `mcp.mnemonik.xyz` endpoint and one-click install URL.
- **`.github/workflows/ci.yml` (modified)** — add MCP Inspector validation step on every PR; add COSE round-trip-via-proxy test invocation.
- **`mcp/tests/` (new test)** — `roundtrip_cose_via_http_proxy.rs` exercises a mock streamable-HTTP proxy that re-serializes the response body, then verifies the original COSE_Sign1 payload still validates byte-for-byte.

### How it works

**OAuth flow:**

1. User opens `mnemonik.xyz/install`. Webapp loads WASM core; if no keypair in `localStorage`, calls `generate_keypair()` and stores the JSON.
2. User clicks "Install in Cursor". Webapp generates a Cursor deeplink with `config={base64({"url":"https://mcp.mnemonik.xyz","name":"Mnemonic"})}`. Cursor opens, prompts to add the connector.
3. Cursor initiates OAuth: redirects browser to `https://mcp.mnemonik.xyz/oauth/authorize?response_type=code&client_id=cursor&code_challenge=<S256>&...`.
4. The hosted MCP serves a small consent page (or redirects back to webapp) that asks the user to sign the auth-request hash with their localStorage keypair. Webapp calls `sign_challenge(hash)` via WASM, posts the signature to `/oauth/authorize`. Server validates signature against the embedded pubkey; if valid, issues an authorization code bound to that pubkey.
5. Cursor exchanges the code at `/oauth/token` (with `code_verifier`); server issues a JWT containing `sub=<pubkey_b58>`.
6. Subsequent tool calls from Cursor carry `Authorization: Bearer <jwt>`. The Axum middleware resolves `pubkey` and passes it down as request-scoped state into tool handlers.

**Tool call (sign_memory in `STORAGE_MODE=local`):**

`Cursor → POST /mcp { method: tools/call, params: { name: mnemonic_sign_memory, ... } }` → middleware validates JWT → dispatch to existing `tools::sign_memory` handler → embedder + compressor + COSE sign with **server-managed** identity (the existing keypair file at `MNEMONIC_KEYPAIR_PATH` — single shared signer for the whole hosted instance in Phase 1) → SQLite write with synthetic `local:<uuid>` ID → response.

**Rationale for shared signer in `local` mode:** the hackathon demo signs all attestations with the hosted server's keypair (DID = server's), not the user's per-OAuth pubkey. The OAuth pubkey is used only for **identity scope** (which user owns which row) and for billing in P1.5. Per-user signing identity is a P1.5 task that requires server-side key management or Turnkey MPC.

### Shared Resources

- **Hosted MCP server keypair** — single Ed25519 keypair file at `MNEMONIC_KEYPAIR_PATH` on the VPS. Used for COSE signing of all `sign_memory` calls in Phase 1. Generated once during deploy. Owner: deploy-pipeline task; consumers: all `tools::sign_memory` calls.
- **`SqliteStore` + DB connection** — single `attestations.db` file at `DATABASE_PATH` shared across all users (scoped by `owner_pubkey` column = OAuth user pubkey). Owner: `McpState`; consumers: tool handlers + OAuth code/token storage.
- **`OAuthState`** (new) — in-memory map of `code → (pubkey, code_challenge, expiry)` and JWT signing key. Lives inside `McpState`. Single instance.

## Decisions

### Decision 1: Streamable HTTP per MCP spec 2025
**Decision:** Upgrade `mcp/src/main.rs` HTTP path to MCP streamable HTTP — chunked response with NDJSON tool-call events, per the 2025 specification. Old SSE path (if any) is removed.
**Rationale:** Anthropic and OpenAI both require streamable HTTP for paid-tier custom connectors (research.md §3.1, line 73-77). Without this, Claude.ai Pro and Cursor can't accept `mcp.mnemonik.xyz` as a connector. Supports user-spec MUST: "`mcp.mnemonik.xyz` отвечает на `tools/list` через streamable HTTP".
**Alternatives:** Keep request-response HTTP — rejected, fails connector compliance. SSE — rejected, being phased out by spec.

### Decision 2: OAuth 2.1 + PKCE in `mcp/`, not third-party provider
**Decision:** Implement OAuth server in-house using `oauth2` and `jsonwebtoken` crates. Authorization endpoint validates user-signed challenge against embedded pubkey; token endpoint issues HS256-signed JWT containing `sub=<pubkey_b58>`. No Auth0 / Clerk / Turnkey in Phase 1.
**Rationale:** User-spec mandates "provider-agnostic OAuth 2.1 + PKCE — собственный OAuth server". Implementation is ~300 LOC in Axum; no external service dependency = lower cost and demo-time fragility. Supports user-spec MUST: "OAuth 2.1 + PKCE endpoints работают; JWT токен issued bound к user pubkey".
**Alternatives:** Third-party provider — rejected per user-spec. Anonymous Bearer tokens — rejected, fails Anthropic connector compliance for Claude.ai Pro.

### Decision 3: WASM bindgen wrappers in `core/src/wasm/`, gated by feature
**Decision:** Add `core/src/wasm/mod.rs` with `#[wasm_bindgen]` exports calling existing `identity::*` functions. Gate the entire module behind `#[cfg(target_arch = "wasm32")]` and a new `wasm` feature flag in `core/Cargo.toml`. Native builds of `mcp/` are unaffected — they don't enable the `wasm` feature.
**Rationale:** Avoid touching business logic in `core/`. Gating ensures native compilation stays identical, satisfying the architecture rule from CLAUDE.md ("`core/` has zero references to anything in `mcp/`"). Supports user-spec MUST: "WASM core ... экспортирует `generate_keypair`, `sign_challenge`, `export_keypair_json`, `import_keypair_json`".
**Alternatives:** Wrap from outside (separate `core-wasm` crate) — rejected, doubles maintenance burden and introduces a duplicate crate. Inline wrappers without feature gate — rejected, pollutes the native compilation graph with `wasm-bindgen` types.

### Decision 4: `STORAGE_MODE=local` on hosted demo, single-signer identity
**Decision:** Hosted `mcp.mnemonik.xyz` runs with `STORAGE_MODE=local`. Synthetic `local:<uuid>` attestation IDs. The hosted server's keypair signs all attestations (server is the COSE signer); the OAuth user pubkey defines **ownership scope** (`owner_pubkey` column) but not signing identity.
**Rationale:** Demo on stage must not depend on Solana RPC reachability, Arweave costs, or funded keypair operational concerns. Per-user signing keys would require server-side custody or Turnkey — both backlog. Supports user-spec MUST: "`STORAGE_MODE=local` для хакатон-демо: SQLite-only, синтетические `local:` ID".
**Alternatives:** Per-user signing in Phase 1 — rejected, requires Turnkey or server-side key management (~5+ days). Full mode — rejected, demo brittleness.

### Decision 5: Smithery as the single registry, repo-root `smithery.yaml`
**Decision:** Add `smithery.yaml` at repo root with the `mcp.mnemonik.xyz` HTTP endpoint and OAuth flow declaration. Submit to Smithery once webapp + MCP are deployed. No simultaneous submission to other registries.
**Rationale:** Smithery is the highest-leverage MCP registry per research.md §4. Other registries (Anthropic Connectors — partner-led no-portal, mcp.directory / Glama — community) are non-blocking and deferred. Supports user-spec MUST: "`smithery.yaml` в репо, листинг на smithery.ai активен".
**Alternatives:** Multi-registry submission — rejected per user-spec ("Один реестр в Phase 1").

### Decision 6: Webapp WASM integration via `wasm-pack` + Vite plugin `[TECHNICAL]`
**Decision:** Build `core/` to WASM via `wasm-pack build core --target web --out-dir webapp/src/wasm`. Vite imports the generated bindings as ES modules. A `package.json` script `build:wasm` runs before `vite build`. No webpack, no `wasm-loader`.
**Rationale:** wasm-pack with `--target web` produces ESM-compatible output that Vite consumes natively. Industry-standard for Rust+React WASM integration.
**Alternatives:** wasm-bindgen-cli + manual glue — rejected, more boilerplate. wasm-pack `--target bundler` — rejected, requires webpack.

### Decision 7: COSE round-trip-via-proxy test using mock proxy
**Decision:** Add `mcp/tests/roundtrip_cose_via_http_proxy.rs` that boots a local Axum mock proxy, configures it to deserialize-and-reserialize JSON-RPC bodies (simulating Anthropic/OpenAI proxy behavior), then verifies the original COSE_Sign1 bytes survive untouched in the response when transported as a base64-encoded string field.
**Rationale:** User-spec R1 risk — "COSE подпись invalidates через Anthropic/OpenAI MCP прокси". Without this test, we can't confidently submit to Smithery or trust live Cursor/Claude.ai integration. Mock proxy is sufficient because the threat model is JSON re-encoding, not vendor-specific quirks.
**Alternatives:** Live test against Anthropic prod proxy — rejected, requires real API key + flaky CI. No test — rejected, R1 is critical.

### Decision 8: Existing webapp `/chat` route preserved; landing replaces root `[TECHNICAL]`
**Decision:** Current root `/` (existing chat-input page that transitions into the chat demo) becomes `/chat`. New `/` is the integration landing page; `/install` is the install hub. URL routing requires adding `react-router-dom` as a new webapp dependency (current `webapp/src/App.tsx` uses internal state-based view switching, not URL routing).
**Rationale:** Landing must be the entry point for hackathon visitors — the chat demo is supplementary content, not the primary CTA. Preserves existing demo for users who arrive via direct link to `/chat`.
**Alternatives:** Replace `/chat` entirely — rejected, removes existing functionality without user-spec authorization. Leave root unchanged — rejected, no clear entry point for the integration story. Hash-based routing without `react-router-dom` — rejected, doesn't match Smithery / deeplink patterns expected by external services.

### Decision 9: Mandatory ownership filter + per-IP rate limit on `/mcp` + auth allowlist
**Decision:** `recall` SQL query in `core/src/storage/sqlite.rs::search` adds a mandatory `owner_pubkey: &str` parameter; the trait definition in `core/src/storage/traits.rs` is updated to match. SQL gains `WHERE owner_pubkey = ?` — no anonymous/unfiltered path.

The HTTP middleware uses an **explicit allowlist** for unauthenticated routes: `/oauth/authorize`, `/oauth/token`, `/health`, plus the MCP discovery methods `initialize` and `tools/list` (the latter two needed for connector-install handshake before OAuth completes — Cursor/Claude.ai POST `tools/list` first to confirm the server is reachable, then trigger OAuth). All other JSON-RPC methods including `tools/call` (sign_memory, recall, etc.) require valid Bearer JWT or return HTTP 401.

Add `tower_governor` per-IP rate limit on `/mcp`: `sign_memory ≤ 5 req/min/IP`, `recall ≤ 30 req/min/IP`, applied regardless of `PAYMENT_MODE`. Rate limits don't apply to stdio transport. **Note**: the existing `mcp/Cargo.toml` already has `governor = "0.8"` (used by `pricing.rs` for CoinGecko throttling). Task 4 must reconcile — pin `tower_governor` to the version compatible with `governor = "0.8"` (likely `tower_governor = "0.5"` or higher); pre-condition check: `cargo tree -p tower_governor` after adding the dep, must not introduce duplicate `governor` versions.

CORS on `/mcp` is narrowed to `Authorization, Content-Type` headers only and explicit origin `https://mnemonik.xyz` (no `Any` wildcard); `/oauth/*` and `/health` already use this same origin allowlist.
**Rationale:** Hackathon demo on a public Smithery-listed endpoint with `PAYMENT_MODE=none` is a DoS / data-fill / fee-burn target. Anonymous `/mcp` recall returning rows from any tenant is a privacy disaster. Without `initialize`/`tools/list` allowlist, MCP connector install fails because clients can't discover tools before completing OAuth. Supports user-spec MUST: "Backward-compat: stdio transport работает как сейчас" and addresses security-auditor critical findings 1 & 2 + round-2 critical regression.
**Alternatives:** Accept anonymous `/mcp` for demo simplicity — rejected, security boundary is non-negotiable. Require auth on `tools/list` — rejected, breaks MCP discovery flow. Auth-gate `/mcp` but skip rate limit — rejected, public endpoint without rate limit is exploit-bait.

### Decision 10: OAuth signed-challenge contents — canonical CBOR encoding
**Decision:** The challenge the user signs at `/oauth/authorize` is `blake3(canonical_cbor({server_origin, state, client_id, redirect_uri, code_challenge, code_challenge_method, nonce, exp}))` where:
- `server_origin = "https://mcp.mnemonik.xyz"` — binds challenge to this server's origin (defeats CursorJack typo-squat where attacker spoofs a malicious endpoint)
- `state` — 16-byte random from client (CSRF binding)
- `client_id`, `redirect_uri` — from authorize request
- `code_challenge`, `code_challenge_method` — must be `"S256"` (rejected if other method)
- `nonce` — 16-byte server-generated random
- `exp` — 60-second expiry timestamp

Canonical CBOR encoding (per `core/src/codec/canonical.rs`) ensures unambiguous serialization — no length-extension or delimiter-injection ambiguity. The server stores `(challenge_hash, expected_pubkey, exp)` keyed by `state`; on `/oauth/authorize` callback, validates signature, atomically removes the entry from the store (single-use, prevents replay), and rejects stale (exp passed) or unknown state.
**Rationale:** Concatenation without delimiters allows ambiguous-encoding attacks (`a || bc` vs `ab || c`). Canonical CBOR is already used in `core/codec` for COSE — reuse the same primitive. Server-origin binding closes CursorJack residual risk. S256-only enforcement at the protocol layer prevents PKCE downgrade. Atomic single-use prevents TOCTOU replay between sig-verify and code-issue. Supports security-auditor round 2 majors #2 + #3.
**Alternatives:** Length-prefix concatenation — rejected, ad-hoc and easy to mis-implement. Plain string join with `|` — rejected, ambiguous if any field contains `|`. JSON canonical — rejected, less stable than CBOR (key ordering, whitespace).

### Decision 11: JWT format — HS256, 1-hour TTL, secret in env
**Decision:** JWT uses HS256 with secret loaded from `MCP_JWT_SECRET` env var (32-byte base64). Claims: `iss=mcp.mnemonik.xyz`, `aud=mcp`, `sub=<user_pubkey_b58>`, `iat`, `exp` (1 hour TTL), `jti` (UUIDv4 for tracking). Server verifies `iss` and `aud` on every request. No refresh tokens in Phase 1 — re-auth required after expiry. Algorithm is fixed at HS256 in code (no `alg=none` or RS256 acceptance) to prevent algorithm confusion attacks.
**Rationale:** Hackathon scope — single hosted instance, single secret. Asymmetric keys (RS256/ES256) add operational burden without proportional benefit at this scale. Fixed algorithm prevents `alg=none` exploits. Supports security-auditor finding #4.
**Alternatives:** Refresh tokens — backlog. RS256 — backlog. Longer TTL — rejected, hackathon demo doesn't need long sessions.

## Data Models

**No new tables.** OAuth state is in-memory only (Phase 1 scope — restart loses pending auth codes; acceptable for demo).

**Schema migrations** — both run idempotently on first connection in `core/src/storage/sqlite.rs::ensure_schema`:

- `attestations` table: **`ALTER TABLE attestations ADD COLUMN owner_pubkey TEXT;`** — base58 OAuth user pubkey for ownership scope. The existing `signer_pubkey` column already exists (set to the hosted server's keypair per Decision 4) and is **distinct** from `owner_pubkey`. NOTE: `attestations` table does **not** currently have a `owner_pubkey` column (see `core/src/storage/sqlite.rs:13-29` schema). This migration adds it.
- `api_keys` table: **`ALTER TABLE api_keys ADD COLUMN oauth_pubkey TEXT;`** — links existing API key rows to the OAuth-issued pubkey. The existing `owner_pubkey` column on `api_keys` is unrelated to the new `attestations.owner_pubkey` (different semantic — `api_keys.owner_pubkey` is the deposit owner; `attestations.owner_pubkey` is the per-attestation tenant scope).

**Migration mechanism:** the existing `SqliteStore::open` in `core/src/storage/sqlite.rs` runs `conn.execute_batch(SCHEMA)` on first connection and has one ad-hoc helper `migrate_payment_events_unique_index`. Add a parallel helper `migrate_owner_pubkey_columns(conn: &Connection) -> Result<()>` invoked from `open` and `in_memory` constructors. The helper queries `PRAGMA table_info(attestations)` / `table_info(api_keys)`, runs the `ALTER` only if the column is absent — idempotent across deploys.

`save` (in `core/src/storage/traits.rs:33-44` + `core/src/storage/sqlite.rs:150-182`) gains a non-optional `owner_pubkey: &str` parameter. The MCP tool dispatcher passes the JWT-resolved pubkey down. CLI/stdio callers (without JWT) pass the keypair-loaded pubkey from `MNEMONIC_KEYPAIR_PATH` so existing local CLI flows continue to work.

**`search` query** (in `core/src/storage/traits.rs:50` + `core/src/storage/sqlite.rs:212`) gains a non-optional `owner_pubkey: &str` parameter. SQL filters by `WHERE owner_pubkey = ?` **always**. No "returns all rows" carve-out. For stdio transport (no JWT), the filter resolves to the local-keypair pubkey — preserving single-tenant local CLI semantics.

## Dependencies

### New packages

**`mcp/Cargo.toml`:**
- `oauth2 = "=4.4.2"` (pinned) — OAuth 2.1 client/server primitives for `code_verifier`/`code_challenge` validation
- `jsonwebtoken = "=9.3.0"` (pinned) — JWT issue + validate
- `tower_governor` — per-IP rate limiting on `/mcp` (Decision 9). **Version selection deferred to Task 4 implementation:** `mcp/Cargo.toml` already contains `governor = "0.8"` (used by `pricing.rs`). The chosen `tower_governor` version must be compatible — Task 4 author runs `cargo tree -p tower_governor -p governor` after adding the dep and pins to a version that does not introduce a duplicate `governor` major version. Prefer `tower_governor = "0.5"` or later (compatible with `governor = "0.8"`).
- Tech-spec author MUST verify each pinned version on crates.io before Task 4 starts; if a pinned version is yanked, update spec and re-run dependency audit.
- CI gains `cargo audit` step on every PR to catch newly-disclosed CVEs in pinned versions.

**`core/Cargo.toml`:**
- `wasm-bindgen = "=0.2.95"` (pinned) — Rust↔JS bridge (added under `[target.'cfg(target_arch = "wasm32")'.dependencies]`)
- `getrandom = { version = "=0.2.15", features = ["js"] }` — required for Ed25519 keypair gen in browser
- `wasm-bindgen-test = "=0.3.45"` (dev-dependency, gated to wasm32 target) — for in-browser keypair tests

**`webapp/package.json`:**
- `react-router-dom = "^6.27.0"` — **NEW dep** for URL routing (`/`, `/install`, `/chat`). Required by Decision 8 — current webapp uses internal state-based view switching, no router installed.
- `wasm-pack` is a build-time tool installed via `cargo install wasm-pack` in CI / dev setup, not an npm dep.

### Removed packages

None.

### Existing (used as-is)

`axum`, `tokio`, `tower-http` (incl. `cors` feature), `serde_json`, `tracing`, `solana-sdk` (mcp/), `mnemonic-core` (path dep), `react`, `react-dom`, `vite`, `tailwindcss` (webapp/).

## Testing Strategy

**Feature size:** L

### Unit tests
- **`mcp/src/oauth.rs`** (~12 tests, expanded from 6 per security-auditor / test-reviewer): authorize endpoint with valid signature, invalid signature (expects 401), tampered `sub` claim (expects 401), `alg=none` JWT submission (expects 401, asserting algorithm-confusion mitigation), token exchange with valid/invalid `code_verifier`, JWT issue + validate roundtrip, expired-code rejection (TTL 60s), single-use code rejection (replay → 401), concurrent-session unique-`jti` collision check, mismatched `iss`/`aud` rejection, missing-`state` CSRF rejection
- **`mcp/src/oauth.rs` rate-limit tests** (~3 tests): 6th `sign_memory` request from same IP within 60s → HTTP 429; 31st `recall` request → HTTP 429; rate limit doesn't apply when `transport=stdio`
- **`core/src/wasm/`** (gated `#[cfg(target_arch = "wasm32")]`, ~6 tests via `wasm-bindgen-test`): keypair gen produces valid Ed25519, sign_challenge round-trip with native verifier, JSON export-import preserves keypair, repeated gen produces distinct keys, malformed JSON in `import_keypair_json` returns Err (not panic), `getrandom` produces non-zero entropy after page reload
- **Streamable HTTP transport** (in `mcp/src/mcp.rs` test module, ~5 tests): chunked response encoding, error path returns valid JSON-RPC error, large response splits across chunks, partial-response on client disconnect mid-stream (no server panic), missing-Authorization-header request returns 401 (not 200 with empty body)

### Integration tests
- **OAuth full flow** (`mcp/tests/oauth_flow.rs`, 1 test): boot Axum app in test mode, simulate browser flow (POST /authorize with signed challenge containing `state`, `client_id`, `redirect_uri`, `code_challenge`, `nonce`; GET /token with code+verifier; parse JWT), assert pubkey roundtrip + JWT claims (`iss`, `aud`, `sub`, `exp`)
- **MCP tool call with OAuth** (`mcp/tests/oauth_tool_call.rs`, 1 test): obtain JWT via flow above, call `tools/list` with Bearer header, assert 5 tools returned and `tools/call sign_memory` succeeds, attestation row has `owner_pubkey = <jwt.sub>` and `local:` ID prefix
- **Recall ownership isolation** (`mcp/tests/recall_owner_isolation.rs`, 1 test, **CRITICAL** — addresses test-reviewer / security-auditor critical findings): boot Axum app, mint 2 distinct JWTs (user A + user B); user A signs 2 attestations, user B signs 1; user B's recall returns ONLY user B's row, never user A's; anonymous request to `/mcp` recall (no Bearer) returns 401 (not 200 with rows)
- **Stdio backward-compat** (`mcp/tests/stdio_backward_compat.rs`, 1 test): spawn pre-built `mnemonic-mcp --transport stdio` binary (cached from build step; not `cargo run` to avoid build-vs-run race) with `EMBED_PROVIDER=mock` and `STORAGE_MODE=local`, pipe `tools/list` then `tools/call sign_memory` then `tools/call recall` JSON-RPC, each request wrapped in `tokio::time::timeout(Duration::from_secs(5))`; assert round-trip succeeds without OAuth (single-tenant with local keypair)
- **COSE round-trip via adversarial mock proxy** (`mcp/tests/roundtrip_cose_via_http_proxy.rs`, 1 test): boot mock proxy that **mutates** JSON byte-for-byte differently from std `serde_json` (e.g., re-orders object keys alphabetically, normalizes whitespace, re-encodes numbers using `simd-json` instead of `serde_json`); send `sign_memory` through it; verify base64-encoded CBOR field survives unmutated (committing to base64-string-field encoding per Decision 7 + R1 mitigation, not relying on JSON-byte-stability)
- **MCP Inspector** (CI-only, GitHub Action step): start server with test JWT in background (`EMBED_PROVIDER=mock STORAGE_MODE=local mnemonic-mcp --transport http --port 3000 &`), `wait-for-port 3000` script with 30s timeout polling `curl -fsS http://localhost:3000/health`, then `npx @modelcontextprotocol/inspector@0.6.x --validate http://localhost:3000/mcp -H "Authorization: Bearer ${TEST_JWT}"`. Pinned npx version (currently latest 0.6.x at time of writing — bump in tandem with MCP spec releases).
- **Rate-limit wired** (`mcp/tests/rate_limit_routing.rs`, 1 test): boots the actual `mcp/src/main.rs` Axum router (not just the limiter logic) with `tower_governor` configured per Decision 9; sends 6 `sign_memory` requests from the same simulated IP within 60s, asserts the 6th returns HTTP 429. Catches `.layer()` ordering regressions where the limiter is defined but not wired.
- **CORS exact-origin** (`mcp/tests/cors.rs`, 1 test): sends two preflight `OPTIONS` requests with `Origin: https://mnemonik.xyz` (allowed) and `Origin: https://evil.example.com` (rejected); asserts `Access-Control-Allow-Origin` header on the first only and 403 on the second. Implements Risks-table mitigation requirement.
- **Authorize-allowlist coverage** (`mcp/tests/auth_allowlist.rs`, 1 test): sends `tools/list` and `initialize` JSON-RPC without Authorization header — asserts both return 200 (per Decision 9 allowlist). Sends `tools/call sign_memory` without auth — asserts 401. Catches future regressions if someone tightens the allowlist to break MCP discovery.

### Manual smoke tests (pre-demo checklist in `tasks/`)
1. Open `mnemonik.xyz` on fresh browser (no localStorage) → see landing → "Get Started" → identity panel shows new keypair → click "Download backup" → verify JSON file content
2. Click "Install in Cursor" → Cursor deeplink fires → OAuth approve flow → in fresh chat, "save this onchain: hello" → tool call succeeds → attestation row visible via webapp/devtools
3. Add `mcp.mnemonik.xyz` connector to Claude.ai Pro Settings → OAuth approve → in fresh chat, "recall hello" → returns the attestation from step 2
4. Re-open `mnemonik.xyz/install` on second device → import keypair JSON → confirm same DID → connector association works (manual)

### E2E tests
None automated — relies on manual smoke. Headless Claude Code in CI is `backlog.md`.

## Agent Verification Plan

**Source:** user-spec "Как проверить" section.

### Verification approach

After each implementation wave: `cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets -- -D warnings`. After Wave 4, full verification per the user-spec table (with subdomain corrected to `mcp.mnemonik.xyz`):

1. `curl -X POST https://mcp.mnemonik.xyz/mcp -d '{"jsonrpc":"2.0","method":"tools/list","id":1}'` → 5 tools, valid JSON-RPC
2. `bash scripts/test-oauth-flow.sh` → JWT issued, payload contains user pubkey
3. `cargo test --workspace --no-fail-fast` → green
4. `cargo clippy --workspace --all-targets -- -D warnings` → zero warnings
5. `curl -fsSL https://smithery.ai/mcp/mnemonic` → 200 OK with install-deeplink in HTML
6. `grep -rE "OAuth|http_transport|axum" core/src/` → empty (modulo `core/src/wasm/` which is allowed)
7. `git diff main -- mcp/src/payment.rs | grep -E "^-CREATE TABLE|^-ALTER TABLE"` → empty (no schema regression)
8. `npx @modelcontextprotocol/inspector --validate https://mcp.mnemonik.xyz/mcp` → all checks pass
9. `cargo test -p mnemonic-mcp roundtrip_cose_via_http_proxy` → signature valid post-passthrough
10. `for r in / /install; do curl -fI https://mnemonik.xyz$r; done` → both 200
11. `STORAGE_MODE=local POST /mcp tools/call mnemonic_sign_memory` → response `attestation_id` starts with `local:`

### Tools required

bash (cargo, curl, grep, git, npx). No MCP tools needed for verification — the AVP is fully automatable by `pre-deploy-qa` and `post-deploy-qa` skills via shell.

## Risks

| Risk | Mitigation |
|------|-----------|
| Streamable HTTP spec compliance is moving target — Anthropic/OpenAI proxies may have undocumented quirks | Implement against `modelcontextprotocol` Rust SDK reference; test with pinned `npx @modelcontextprotocol/inspector` on every PR. Live-validate with Cursor + Claude.ai before demo. |
| OAuth flow needs user-signed challenge but webapp ↔ MCP CORS could block POST | Configure `tower-http::cors::CorsLayer` allow-listed to **exact** origin `https://mnemonik.xyz` (no `Any` wildcard) on `/oauth/*` endpoints. Test in CI with a request from a different origin asserting 403/CORS-rejected. |
| WASM keypair lost when user clears browser → identity loss → demo embarrassment | Aggressive "Download backup" prompt on first generation; warning before page exit if backup not downloaded. Demo dry-run with a pre-saved backup as fallback. localStorage value is encrypted via `crypto.subtle` AES-GCM with passphrase derived from a session secret + user-entered PIN (P1.5 may upgrade to passkey). |
| XSS in webapp → localStorage keypair theft | Comprehensive CSP header on webapp: `default-src 'self'; script-src 'self'; connect-src 'self' https://mcp.mnemonik.xyz; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'`. `import_keypair_json` validates JSON shape + Ed25519 byte length (32-byte secret + 32-byte public) before storing; suspicious input rejected with user-visible error. localStorage value encrypted via `crypto.subtle` AES-GCM with passphrase-derived key (P1.5 → passkey). |
| `mcp.mnemonik.xyz` subdomain needs DNS + SSL cert before demo | Schedule DNS update in Wave 2 (Smithery task); use existing `certbot` flow per `deployment.md`. Validate DNS propagation 24h before demo. |
| COSE byte-stability through proxies fails despite mock test | **Default to base64-encoded CBOR in JSON string field** (not JSON-byte-stability). Adversarial mock proxy in CI test forces this encoding to be the only safe one. |
| `STORAGE_MODE=local` ownership scope (`owner_pubkey`) leaks across users if filter forgotten | Decision 9 makes the SQL-level filter mandatory. Test `recall_owner_isolation` enforces cross-tenant assertion in CI. SQL helper function `search_attestations(owner_pubkey, query, limit)` requires the parameter at compile time — no signature without it. |
| Smithery review rejects crypto-related listing | Position as "verifiable knowledge memory"; lead utility, blockchain framing as "plumbing". Smithery is community-driven so risk is low; if rejected, escalate to mcp.directory in P1.5. |
| Live-demo network failure on stage | Pre-recorded fallback video; local stdio MCP (existing) as backup demo without hosted-service dependency. |
| Hosted server keypair (`MNEMONIC_KEYPAIR_PATH`) compromise = all attestations forgeable retroactively | File mode `0600`, owned by `claude` user (not root or world-readable). Generated once during deploy via `sodium randombytes_buf`; not committed to git. Deployment runbook in `deployment.md` specifies generation. P1.5 migrates to per-user signing via Turnkey, eliminating this attack surface. |
| OAuth `OAuthState` in-memory map → DoS by exhausting memory with `/oauth/authorize` storms | Per-IP rate limit on `/oauth/*` (5 req/min/IP); `OAuthState` map size cap of 10000 entries with LRU eviction; entries TTL 60s. |

## User-Spec Deviations

### Deviation 1: per-attestation signer is server, not user
**User-spec says:** Sign-with-Solana via WASM keypair → user is signer. JWT issued bound to user Turnkey/localStorage pubkey, OAuth challenge signed by user.
**Tech-spec does:** OAuth challenge IS signed by the user's localStorage keypair (matches user-spec). But the COSE signature on each attestation is from the **hosted server's keypair**, not the user's. The user pubkey appears as `owner_pubkey` (ownership scope) only.
**Why:** Per-user COSE signing requires server-side custody of user keys (or Turnkey MPC) — both deferred to backlog.
**Status:** `[PENDING USER APPROVAL]` — already discussed in interview round 6 (Q-A `STORAGE_MODE=local`); spec text re-confirms.

### Deviation 2: `mcp.mnemonik.xyz` subdomain (not `mcp.mnemonic.dev`)
**User-spec says:** Original draft mentioned `mcp.mnemonic.dev`; updated to `mcp.mnemonik.xyz` after user clarification.
**Tech-spec does:** Uses `mcp.mnemonik.xyz` consistently.
**Why:** User confirmed actual domain in interview round 7.
**Status:** Aligned; user-spec already updated to match.

### Deviation 3: `oauth_pubkey` column added to `api_keys` and `owner_pubkey` to `attestations`
**User-spec says:** "`payment.rs` НЕ рефакторится: для хакатона `PAYMENT_MODE=none`".
**Tech-spec does:** Adds two `ALTER TABLE` migrations (idempotent `ADD COLUMN`). No code changes in `payment.rs`. The new `attestations.owner_pubkey` is required for ownership scope per Decision 9 (recall isolation). The new `api_keys.oauth_pubkey` links existing API key rows to OAuth-issued pubkeys for P1.5 billing wiring.
**Why:** Schema additions are forward-only and backwards-compatible — existing rows remain valid. Without `attestations.owner_pubkey`, multi-tenant recall on hosted MCP cannot be safely scoped. Decision 9 mandates this.
**Status:** `[PENDING USER APPROVAL]` — minor, but technically a deviation from "не рефакторится" if interpreted strictly.

### Deviation 4: `react-router-dom` added as new webapp dep
**User-spec says:** Webapp implementation details not specified — the user-spec says only "Webapp `mnemonik.xyz` имеет 2 страницы". Routing tech is implicit choice.
**Tech-spec does:** Adds `react-router-dom = ^6.27.0` to `webapp/package.json`. Current webapp uses internal state-based view switching with no router installed; URL routing (`/`, `/install`, `/chat`) requires a router (Decision 8). Skeptic validation flagged this as unstated.
**Why:** URL routes are necessary for Smithery / Cursor / Claude.ai deeplinks to land on `/install` directly. Hash routing would break SSR/static-host expectations.
**Status:** `[PENDING USER APPROVAL]` — minor, dependency addition needs explicit acknowledgement.

### Deviation 5: `MCP_JWT_SECRET` env var added
**User-spec says:** Env vars for hosting not specified beyond existing `STORAGE_MODE` / `EMBED_PROVIDER` / etc.
**Tech-spec does:** Adds `MCP_JWT_SECRET` (32-byte base64) to env vars. Generated once during deploy. Required by Decision 11 (HS256 JWT).
**Why:** Without a stable secret, JWT validation breaks across server restarts.
**Status:** `[PENDING USER APPROVAL]` — minor, deploy runbook addition.

### Deviation 6: Reviewer agent substitution for missing installed agents
**User-spec says:** No specific reviewer constraint.
**Tech-spec does:** Substitutes installed `security-auditor + test-reviewer` everywhere the catalog (`~/.claude/skills/tech-spec-planning/references/skills-and-reviewers.md`) lists `code-reviewer`, `infrastructure-reviewer`, or `deploy-reviewer` — those reviewer agents are NOT installed in `~/.claude/agents/` for this environment. Audit-wave skills `code-reviewing` and `test-master` are installed as skills (dispatched via the Skill tool), not as agents — acceptable per Audit Wave dispatch model.
**Why:** Without the substitution, no review agent could actually run on tasks 1-7. The chosen substitutes cover code quality (via security-auditor's broader scope) and test quality (test-reviewer).
**Status:** `[PENDING USER APPROVAL]` — operational reality, but technically deviation from catalog defaults.

## Acceptance Criteria

Technical AC supplementing user-spec MUST:

- [ ] `cargo build --workspace` (native, no wasm feature) succeeds
- [ ] `cargo build -p mnemonic-core --features wasm --target wasm32-unknown-unknown` succeeds (NB: workspace-wide wasm32 build will fail because `mcp/` pulls tokio-full/axum/solana-sdk; only `core/` is wasm32-compatible)
- [ ] `wasm-pack build core --target web --out-dir ../webapp/src/wasm --release` produces a valid ES module that imports cleanly in Vite
- [ ] `mcp/src/oauth.rs` exists; `oauth2`, `jsonwebtoken`, `tower_governor` in `mcp/Cargo.toml` (pinned versions)
- [ ] `core/src/wasm/mod.rs` exists; gated by `#[cfg(target_arch = "wasm32")]` and `wasm` feature; native build does not include wasm-bindgen
- [ ] `smithery.yaml` exists at repo root, references `mcp.mnemonik.xyz`
- [ ] CI workflow includes MCP Inspector validation step on PR + `cargo audit` on PR
- [ ] `mcp/tests/roundtrip_cose_via_http_proxy.rs`, `mcp/tests/recall_owner_isolation.rs`, `mcp/tests/stdio_backward_compat.rs`, `mcp/tests/oauth_flow.rs`, `mcp/tests/oauth_tool_call.rs`, `mcp/tests/rate_limit_routing.rs`, `mcp/tests/cors.rs`, `mcp/tests/auth_allowlist.rs` all exist and pass
- [ ] DNS A-record for `mcp.mnemonik.xyz` resolves to VPS IP; HTTPS cert valid
- [ ] Webapp routes `/`, `/install`, `/chat` all return 200; CSP header `default-src 'self'; ...` sent on each
- [ ] Existing 5 MCP tools (`whoami`, `sign_memory`, `verify`, `prove_identity`, `recall`) signatures unchanged
- [ ] `grep -rE "OAuth|http_transport|axum" core/src/ | grep -v "core/src/wasm"` is empty (`core/` business logic untouched)
- [ ] No regressions in existing stdio MCP behavior — round-trip `sign_memory → recall` via stdio still works locally (asserted by `stdio_backward_compat` test)
- [ ] `MCP_JWT_SECRET` env var documented in `deployment.md` and `.env.example`
- [ ] Anonymous `curl https://mcp.mnemonik.xyz/mcp -d '{"method":"tools/call",...}'` returns HTTP 401 (not 200 with rows from any tenant)
- [ ] Per-IP rate limit returns 429 after threshold (5 sign_memory/min, 30 recall/min)

## Implementation Tasks

### Wave 1: Foundation (parallel)

#### Task 1: Streamable HTTP transport upgrade
- **Description:** Upgrade `mcp/src/main.rs` and `mcp/src/mcp.rs` HTTP path to MCP streamable HTTP per spec 2025 (chunked response, NDJSON event framing). Add Axum middleware scaffolding (no-op now) where OAuth Bearer validation will plug in (Task 4). Stdio transport unchanged.
- **Skill:** code-writing
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp -- transport && curl -N -X POST http://localhost:3000/mcp -d '{"jsonrpc":"2.0","method":"tools/list","id":1}'` returns chunked NDJSON
- **Files to modify:** `mcp/src/main.rs`, `mcp/src/mcp.rs`, `mcp/Cargo.toml`
- **Files to read:** `mcp/src/main.rs`, `mcp/src/mcp.rs`, `work/mnemonic-integrations/code-research.md` §1

#### Task 2: WASM bindgen wrappers in core
- **Description:** Add `core/src/wasm/mod.rs` with `#[wasm_bindgen]` exports `generate_keypair`, `sign_challenge`, `export_keypair_json`, `import_keypair_json`. These are NEW helper wrappers — `core/src/identity/mod.rs` currently exports `load_or_create_keypair`, `pubkey_base58`, `did_sol`, `did_key`, `sign_bytes`, `verify_signature`. The new helpers compose the existing primitives: `generate_keypair = solana_sdk::Keypair::new()`, `sign_challenge = sign_bytes`, `export_keypair_json` / `import_keypair_json` are JSON serializers around the keypair byte array. Gate the entire `wasm` module behind `#[cfg(target_arch = "wasm32")]` AND a `wasm` feature flag in `core/Cargo.toml`. Add `wasm-bindgen-test`-driven unit tests per Testing Strategy.
- **Skill:** code-writing
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-core --features wasm --target wasm32-unknown-unknown && wasm-pack test --headless --chrome core --features wasm`
- **Files to modify:** `core/src/wasm/mod.rs` (new), `core/src/lib.rs` (mod gate), `core/Cargo.toml` (wasm-bindgen, getrandom features, `wasm` feature flag)
- **Files to read:** `core/src/identity/mod.rs`, `work/mnemonic-integrations/code-research.md` §3

#### Task 3: Webapp WASM build pipeline
- **Description:** Add `webapp/scripts/build-wasm.sh` invoking `wasm-pack build core --target web --out-dir webapp/src/wasm --release`. Wire into `webapp/package.json` as `build:wasm` and as a pre-step for `build`. Update `webapp/.gitignore` to exclude generated `webapp/src/wasm/`. Verify Vite imports the `.js` ES module without configuration tweaks.
- **Skill:** infrastructure-setup
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `cd webapp && npm run build:wasm && npm run build` produces `dist/` with WASM assets
- **Files to modify:** `webapp/scripts/build-wasm.sh` (new), `webapp/package.json`, `webapp/.gitignore`, `webapp/vite.config.ts` (only if needed for WASM mime type)
- **Files to read:** `webapp/package.json`, `webapp/vite.config.ts`

### Wave 2: OAuth + Smithery (parallel)

#### Task 4: OAuth 2.1 + PKCE server module
- **Description:** Implement `mcp/src/oauth.rs` per Decisions 9, 10, 11 — authorize and token endpoints, signed-challenge validation, HS256 JWT issuance, Bearer middleware with mandatory auth on `/mcp` (anonymous → 401), per-IP rate limiting via `tower_governor`. Run idempotent schema migrations on first connection (see Data Models section). Update `recall` SQL to require `owner_pubkey` parameter. Tighten CORS to exact origin `https://mnemonik.xyz`.
- **Skill:** code-writing
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp -- oauth && bash scripts/test-oauth-flow.sh` returns valid JWT
- **Files to modify:** `mcp/src/oauth.rs` (new), `mcp/src/mcp.rs` (route registration + middleware), `mcp/src/main.rs` (state init), `mcp/src/tools.rs` (recall filter by `owner_pubkey`), `mcp/Cargo.toml`, `core/src/storage/sqlite.rs` (migration runner)
- **Files to read:** `mcp/src/payment.rs`, `mcp/src/tools.rs`, `core/src/storage/sqlite.rs`, `work/mnemonic-integrations/code-research.md` §2, §6

#### Task 5: Smithery listing + DNS subdomain + nginx
- **Description:** Create `smithery.yaml` at repo root with `mcp.mnemonik.xyz` endpoint and OAuth flow declaration. Coordinate DNS A-record for `mcp.mnemonik.xyz` → VPS. Update nginx config (`/etc/nginx/sites-available/mnemonic` per `deployment.md`) to add subdomain server-block proxying to `localhost:3000`. Run `certbot --nginx -d mcp.mnemonik.xyz`. Submit listing to smithery.ai.
- **Skill:** infrastructure-setup
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `dig +short mcp.mnemonik.xyz` returns VPS IP; `curl -fI https://mcp.mnemonik.xyz/health` returns 200
- **Verify-user:** Visit `smithery.ai/mcp/mnemonic` — listing visible with install-deeplink
- **Files to modify:** `smithery.yaml` (new), `deployment.md` (subdomain section), nginx config on VPS (out-of-tree)
- **Files to read:** `.claude/skills/project-knowledge/references/deployment.md`, `work/mnemonic-integrations/code-research.md` §7

### Wave 3: UI + tests (parallel)

#### Task 6: Webapp landing + install-hub + identity panel
- **Description:** Add `webapp/src/pages/Landing.tsx` (route `/`) — protocol pitch + "Get started" CTA leading to `/install`. Add `webapp/src/pages/Install.tsx` (route `/install`) — identity panel (Generate / Import / Export keypair via WASM core) + deeplink buttons for Cursor / VS Code / Claude.ai. Move existing chat demo from `/` to `/chat`. Use existing Tailwind tokens from `ux-guidelines.md`.
- **Skill:** code-writing
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `cd webapp && npm run dev` — open localhost:5173/, /install, /chat — all render without console errors
- **Verify-user:** On `/install`, click "Generate keypair" → DID/pubkey appears → click "Download backup" → JSON file with valid Ed25519 keypair
- **Files to modify:** `webapp/src/App.tsx` (router), `webapp/src/pages/Landing.tsx` (new), `webapp/src/pages/Install.tsx` (new), `webapp/src/components/IdentityPanel.tsx` (new), `webapp/src/components/InstallButtons.tsx` (new)
- **Files to read:** `webapp/src/App.tsx`, `.claude/skills/project-knowledge/references/ux-guidelines.md`, `webapp/src/wasm/` (generated by Task 3)

#### Task 7: Integration tests + MCP Inspector CI
- **Description:** Add 7 integration tests per Testing Strategy: `roundtrip_cose_via_http_proxy.rs`, `recall_owner_isolation.rs`, `stdio_backward_compat.rs`, `rate_limit_routing.rs`, `cors.rs`, `auth_allowlist.rs`, plus expanded `oauth_flow.rs`. Update `.github/workflows/ci.yml` to add: MCP Inspector validation step (with `wait-for-port` readiness probe + pre-built binary + test JWT), `cargo audit` step, schema validation step for `smithery.yaml`.
- **Skill:** code-writing
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp roundtrip_cose_via_http_proxy` passes; CI run shows MCP Inspector step green
- **Files to modify:** `mcp/tests/roundtrip_cose_via_http_proxy.rs` (new), `.github/workflows/ci.yml`
- **Files to read:** `core/src/codec/sign.rs`, `core/tests/integration_cbor.rs`, `.github/workflows/ci.yml`, `work/mnemonic-integrations/code-research.md` §8

#### Task 8: Pre-demo manual smoke checklist
- **Description:** Author `work/mnemonic-integrations/tasks/smoke-checklist.md` — exhaustive manual flow on Cursor + Claude.ai Pro covering: fresh-browser onboarding, keypair gen, install deeplink, OAuth approve, sign_memory, switch to second browser/tool, recall. Each step has expected result and rollback note. Document used during pre-release smoke and live demo dry-run.
- **Skill:** documentation-writing
- **Reviewers:** none (manual review via `Verify-user`)
- **Verify-user:** A team member who didn't write the spec executes the checklist on a fresh laptop end-to-end without ambiguity
- **Files to modify:** `work/mnemonic-integrations/tasks/smoke-checklist.md` (new)
- **Files to read:** `work/mnemonic-integrations/user-spec.md` (Сценарии), `work/mnemonic-integrations/code-research.md`

### Audit Wave (parallel, reviewers: none)

#### Task 9: Code Audit
- **Description:** Holistic code-quality audit of all feature code: streamable HTTP transport, OAuth module, WASM bindgen wrappers, webapp pages, CI changes. Verify architectural rules from CLAUDE.md (core/ untouched in business logic, payment.rs schema-only changes, no cross-import violations).
- **Skill:** code-reviewing
- **Reviewers:** none
- **Files to read:** `mcp/src/oauth.rs`, `mcp/src/main.rs`, `mcp/src/mcp.rs`, `mcp/src/tools.rs`, `core/src/wasm/`, `core/Cargo.toml`, `webapp/src/pages/`, `webapp/src/components/`, `smithery.yaml`, `.github/workflows/ci.yml`
- **Files to modify:** N/A (analysis only — report to `decisions.md`)

#### Task 10: Security Audit
- **Description:** OWASP Top 10 review focused on auth and key handling: OAuth flow PKCE correctness, JWT signing-key storage, code/token TTL and revocation, CORS configuration, Bearer-token middleware bypass, localStorage keypair exposure (XSS surface), CSRF on `/oauth/authorize`, secret leakage in logs.
- **Skill:** security-auditor
- **Reviewers:** none
- **Files to read:** `mcp/src/oauth.rs`, `mcp/src/mcp.rs`, `webapp/src/components/IdentityPanel.tsx`, `webapp/src/wasm/`, `mcp/Cargo.toml`, `nginx` server-block (if accessible)
- **Files to modify:** N/A (analysis only — report to `decisions.md`)

#### Task 11: Test Audit
- **Description:** Test quality and pyramid balance audit. Verify: OAuth flow has unit + integration coverage, WASM wrappers have wasm-bindgen-test coverage, COSE-via-proxy test exercises realistic mutation, MCP Inspector step actually fails the build on schema regression, manual smoke checklist is unambiguous.
- **Skill:** test-master
- **Reviewers:** none
- **Files to read:** `mcp/src/oauth.rs` (test module), `mcp/tests/`, `core/src/wasm/` (test module), `webapp/src/pages/` (component tests if any), `work/mnemonic-integrations/tasks/smoke-checklist.md`
- **Files to modify:** N/A (analysis only — report to `decisions.md`)

### Final Wave

#### Task 12: Pre-deploy QA
- **Description:** Run full test suite (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cd webapp && npm run build`). Walk through every user-spec MUST and tech-spec AC line — confirm pass or document failures. Execute the Wave-3 manual smoke checklist on a clean laptop.
- **Skill:** pre-deploy-qa
- **Reviewers:** none
- **Files to modify:** N/A (testing only — produces report in `decisions.md`)
- **Files to read:** `work/mnemonic-integrations/user-spec.md`, `work/mnemonic-integrations/tech-spec.md`, `work/mnemonic-integrations/tasks/smoke-checklist.md`, all modified source files

#### Task 13: Deploy
- **Description:** Deploy hosted MCP to `mcp.mnemonik.xyz` subdomain on existing VPS (per `deployment.md` flow — `cargo build --release`, restart `mnemonic-mcp.service`, verify systemd status). Deploy webapp to Cloudflare Pages (or VPS nginx, per existing flow). Verify both endpoints return 200 over HTTPS. Submit Smithery listing.
- **Skill:** deploy-pipeline
- **Reviewers:** security-auditor, test-reviewer
- **Verify-smoke:** `curl -fI https://mcp.mnemonik.xyz/health && curl -fI https://mnemonik.xyz/install`
- **Files to modify:** VPS nginx config (out-of-tree), GitHub Actions workflow if Cloudflare Pages deploy is added
- **Files to read:** `.claude/skills/project-knowledge/references/deployment.md`

#### Task 14: Post-deploy QA
- **Description:** On the live `mcp.mnemonik.xyz` endpoint, run `npx @modelcontextprotocol/inspector --validate https://mcp.mnemonik.xyz/mcp`. Trigger full OAuth flow via real Cursor connector install on a clean Cursor profile; verify `sign_memory → recall` round-trip. Verify Smithery listing is live and the install-deeplink works. Verify anonymous `curl` to `/mcp` returns 401. Mark all user-spec success metrics measurable (install counter wired up).
- **Skill:** post-deploy-qa
- **Reviewers:** none
- **Files to modify:** N/A (live verification only — produces report in `decisions.md`)
- **Files to read:** `work/mnemonic-integrations/user-spec.md` (success metrics + verification table), `work/mnemonic-integrations/tasks/smoke-checklist.md`
