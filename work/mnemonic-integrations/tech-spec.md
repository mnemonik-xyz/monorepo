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

### Decision 6: Webapp WASM integration via `wasm-pack` + Vite plugin
**Decision:** Build `core/` to WASM via `wasm-pack build core --target web --out-dir webapp/src/wasm`. Vite imports the generated bindings as ES modules. A `package.json` script `build:wasm` runs before `vite build`. No webpack, no `wasm-loader`.
**Rationale:** wasm-pack with `--target web` produces ESM-compatible output that Vite consumes natively. Industry-standard for Rust+React WASM integration.
**Alternatives:** wasm-bindgen-cli + manual glue — rejected, more boilerplate. wasm-pack `--target bundler` — rejected, requires webpack.

### Decision 7: COSE round-trip-via-proxy test using mock proxy
**Decision:** Add `mcp/tests/roundtrip_cose_via_http_proxy.rs` that boots a local Axum mock proxy, configures it to deserialize-and-reserialize JSON-RPC bodies (simulating Anthropic/OpenAI proxy behavior), then verifies the original COSE_Sign1 bytes survive untouched in the response when transported as a base64-encoded string field.
**Rationale:** User-spec R1 risk — "COSE подпись invalidates через Anthropic/OpenAI MCP прокси". Without this test, we can't confidently submit to Smithery or trust live Cursor/Claude.ai integration. Mock proxy is sufficient because the threat model is JSON re-encoding, not vendor-specific quirks.
**Alternatives:** Live test against Anthropic prod proxy — rejected, requires real API key + flaky CI. No test — rejected, R1 is critical.

### Decision 8: Existing webapp `/chat` route preserved; landing replaces root
**Decision:** Current root `/` (Qwen2.5 chat demo) becomes `/chat`. New `/` is the integration landing page; `/install` is the install hub.
**Rationale:** Landing must be the entry point for hackathon visitors — the chat demo is supplementary content, not the primary CTA. Preserves existing demo for users who arrive via direct link to `/chat`.
**Alternatives:** Replace `/chat` entirely — rejected, removes existing functionality without user-spec authorization. Leave root unchanged — rejected, no clear entry point for the integration story.

## Data Models

**No new tables.** OAuth state is in-memory only (Phase 1 scope — restart loses pending auth codes; acceptable for demo).

**One new column on existing `api_keys` table** — `oauth_pubkey TEXT` (nullable, base58-encoded). Rows are created on first OAuth token issue and looked up on Bearer token validation. Backwards-compatible: `ALTER TABLE api_keys ADD COLUMN oauth_pubkey TEXT;` runs idempotently on first connection.

The existing `attestations` table gains no schema change. The `signer_pubkey` column already exists and is set to the hosted server's keypair (Decision 4). A new column `owner_pubkey TEXT` is added (nullable, base58 OAuth user pubkey) for ownership scope. `ALTER TABLE attestations ADD COLUMN owner_pubkey TEXT;` runs idempotently.

`recall` filters by `owner_pubkey = <jwt.sub>` (or returns all rows if `JWT.sub` is unset, for backward-compat with existing CLI-based calls in the same DB).

## Dependencies

### New packages

**`mcp/Cargo.toml`:**
- `oauth2 = "4.4"` — OAuth 2.1 client/server primitives for `code_verifier`/`code_challenge` validation
- `jsonwebtoken = "9.3"` — JWT issue + validate

**`core/Cargo.toml`:**
- `wasm-bindgen = "0.2"` — Rust↔JS bridge (added under `[target.'cfg(target_arch = "wasm32")'.dependencies]`)
- `getrandom = { version = "0.2", features = ["js"] }` — required for Ed25519 keypair gen in browser

**`webapp/package.json`:**
- No new npm deps; `wasm-pack` is a build-time tool installed via `cargo install wasm-pack` in CI / dev setup.

### Removed packages

None.

### Existing (used as-is)

`axum`, `tokio`, `tower-http`, `serde_json`, `tracing`, `solana-sdk` (mcp/), `mnemonic-core` (path dep), `react`, `vite`, `tailwindcss`, `react-router` (webapp/).

## Testing Strategy

**Feature size:** L

### Unit tests
- **`mcp/src/oauth.rs`** (~6 tests): authorize endpoint with valid/invalid signature, token exchange with valid/invalid `code_verifier`, JWT issue + validate roundtrip, expired-code rejection
- **`core/src/wasm/`** (gated `#[cfg(target_arch = "wasm32")]`, ~4 tests via `wasm-bindgen-test`): keypair gen produces valid Ed25519, sign_challenge round-trip with native verifier, JSON export-import preserves keypair, repeated gen produces distinct keys
- **Streamable HTTP transport** (in `mcp/src/mcp.rs` test module, ~3 tests): chunked response encoding, error path returns valid JSON-RPC error, large response splits across chunks

### Integration tests
- **OAuth full flow** (`mcp/tests/oauth_flow.rs`, 1 test): boot Axum app in test mode, simulate browser flow (POST /authorize with signed challenge, GET /token with code+verifier, parse JWT), assert pubkey roundtrip
- **MCP tool call with OAuth** (`mcp/tests/oauth_tool_call.rs`, 1 test): obtain JWT via flow above, call `tools/list` with Bearer header, assert 5 tools returned and `tools/call sign_memory` succeeds, attestation row has `owner_pubkey = <jwt.sub>` and `local:` ID prefix
- **COSE round-trip via mock proxy** (`mcp/tests/roundtrip_cose_via_http_proxy.rs`, 1 test): boot mock proxy that re-serializes JSON, send `sign_memory` through it, re-fetch the bundle, verify COSE_Sign1 still validates byte-for-byte
- **MCP Inspector** (CI-only, GitHub Action step): `npx @modelcontextprotocol/inspector --validate http://localhost:3000/mcp` after spinning up the server with a test JWT

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
| Streamable HTTP spec compliance is moving target — Anthropic/OpenAI proxies may have undocumented quirks | Implement against `modelcontextprotocol` Rust SDK reference; test with `npx @modelcontextprotocol/inspector` on every PR. Live-validate with Cursor + Claude.ai before demo. |
| OAuth flow needs user-signed challenge but webapp ↔ MCP CORS could block POST | Configure `tower-http::cors::CorsLayer` to allow `mnemonik.xyz` origin on `/oauth/*` endpoints. Test in CI. |
| WASM keypair lost when user clears browser → identity loss → demo embarrassment | Aggressive "Download backup" prompt on first generation; warning before page exit if backup not downloaded. Demo dry-run with a pre-saved backup as fallback. |
| `mcp.mnemonik.xyz` subdomain needs DNS + SSL cert before demo | Schedule DNS update in Wave 2 (Smithery task); use existing `certbot` flow per `deployment.md`. Validate DNS propagation 24h before demo. |
| COSE byte-stability through proxies fails despite mock test | Fallback: encode bundle as `base64(cbor_bytes)` in a JSON string field rather than relying on JSON-RPC payload byte-stability. Test both encodings in CI. |
| `STORAGE_MODE=local` ownership scope (`owner_pubkey`) leaks across users if filter forgotten | Add a SQL-level guard: every `recall` query MUST include `owner_pubkey = ?` clause; add a clippy-style lint or unit test that asserts this on every code path. |
| Smithery review rejects crypto-related listing | Position as "verifiable knowledge memory"; lead utility, blockchain framing as "plumbing". Smithery is community-driven so risk is low; if rejected, escalate to mcp.directory in P1.5. |
| Live-demo network failure on stage | Pre-recorded fallback video; local docker self-host (existing) as backup demo without hosted-service dependency. |

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
**Tech-spec does:** Adds two `ALTER TABLE` migrations (idempotent `ADD COLUMN`). No code changes in `payment.rs`. New columns are nullable and unused when `PAYMENT_MODE=none`.
**Why:** Schema additions are forward-only and backwards-compatible — existing rows remain valid. The hook is needed for ownership scope in `STORAGE_MODE=local` (Decision 4).
**Status:** `[PENDING USER APPROVAL]` — minor, but technically a deviation from "не рефакторится" if interpreted strictly.

## Acceptance Criteria

Technical AC supplementing user-spec MUST:

- [ ] `cargo build --workspace` and `cargo build --workspace --features wasm --target wasm32-unknown-unknown` both succeed
- [ ] `wasm-pack build core --target web --out-dir webapp/src/wasm --release` produces a valid ES module that imports cleanly in Vite
- [ ] `mcp/src/oauth.rs` exists; `oauth2`, `jsonwebtoken` in `mcp/Cargo.toml`
- [ ] `core/src/wasm/mod.rs` exists; gated by `#[cfg(target_arch = "wasm32")]`; native build does not include wasm-bindgen
- [ ] `smithery.yaml` exists at repo root, references `mcp.mnemonik.xyz`
- [ ] CI workflow includes MCP Inspector validation step on PR
- [ ] `mcp/tests/roundtrip_cose_via_http_proxy.rs` exists and passes
- [ ] DNS A-record for `mcp.mnemonik.xyz` resolves to VPS IP; HTTPS cert valid
- [ ] Webapp routes `/`, `/install`, `/chat` all return 200
- [ ] Existing 5 MCP tools (`whoami`, `sign_memory`, `verify`, `prove_identity`, `recall`) signatures unchanged
- [ ] `grep -rE "OAuth|http_transport|axum" core/src/ | grep -v "core/src/wasm"` is empty (`core/` business logic untouched)
- [ ] No regressions in existing stdio MCP behavior — round-trip `sign_memory → recall` via stdio still works locally

## Implementation Tasks

### Wave 1: Foundation (parallel)

#### Task 1: Streamable HTTP transport upgrade
- **Description:** Upgrade `mcp/src/main.rs` and `mcp/src/mcp.rs` HTTP path to MCP streamable HTTP per spec 2025 (chunked response, NDJSON event framing). Add Axum middleware scaffolding (no-op now) where OAuth Bearer validation will plug in (Task 4). Stdio transport unchanged.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp -- transport && curl -N -X POST http://localhost:3000/mcp -d '{"jsonrpc":"2.0","method":"tools/list","id":1}'` returns chunked NDJSON
- **Files to modify:** `mcp/src/main.rs`, `mcp/src/mcp.rs`, `mcp/Cargo.toml`
- **Files to read:** `mcp/src/main.rs`, `mcp/src/mcp.rs`, `work/mnemonic-integrations/code-research.md` §1

#### Task 2: WASM bindgen wrappers in core
- **Description:** Add `core/src/wasm/mod.rs` with `#[wasm_bindgen]` exports — `generate_keypair`, `sign_challenge`, `export_keypair_json`, `import_keypair_json` — calling existing `core/src/identity/` functions. Gate behind `#[cfg(target_arch = "wasm32")]` and a new `wasm` feature in `core/Cargo.toml`. Add `wasm-bindgen-test`-driven unit tests.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-core --features wasm --target wasm32-unknown-unknown && wasm-pack test --headless --chrome core --features wasm`
- **Files to modify:** `core/src/wasm/mod.rs` (new), `core/src/lib.rs` (mod gate), `core/Cargo.toml` (wasm-bindgen, getrandom features, `wasm` feature flag)
- **Files to read:** `core/src/identity/mod.rs`, `work/mnemonic-integrations/code-research.md` §3

#### Task 3: Webapp WASM build pipeline
- **Description:** Add `webapp/scripts/build-wasm.sh` invoking `wasm-pack build core --target web --out-dir webapp/src/wasm --release`. Wire into `webapp/package.json` as `build:wasm` and as a pre-step for `build`. Update `webapp/.gitignore` to exclude generated `webapp/src/wasm/`. Verify Vite imports the `.js` ES module without configuration tweaks.
- **Skill:** infrastructure-setup
- **Reviewers:** code-reviewer, security-auditor, infrastructure-reviewer
- **Verify-smoke:** `cd webapp && npm run build:wasm && npm run build` produces `dist/` with WASM assets
- **Files to modify:** `webapp/scripts/build-wasm.sh` (new), `webapp/package.json`, `webapp/.gitignore`, `webapp/vite.config.ts` (only if needed for WASM mime type)
- **Files to read:** `webapp/package.json`, `webapp/vite.config.ts`

### Wave 2: OAuth + Smithery (parallel)

#### Task 4: OAuth 2.1 + PKCE server module
- **Description:** Implement `mcp/src/oauth.rs` with `/oauth/authorize` (validates user-signed challenge against pubkey, issues code) and `/oauth/token` (validates `code_verifier`, issues HS256 JWT with `sub=<pubkey_b58>`). Wire as Axum routes. Add Bearer-token validation middleware that resolves pubkey for downstream tool dispatch. Apply migrations: `ALTER TABLE api_keys ADD COLUMN oauth_pubkey TEXT;` and `ALTER TABLE attestations ADD COLUMN owner_pubkey TEXT;` (idempotent). `recall` filters by `owner_pubkey = jwt.sub`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp -- oauth && bash scripts/test-oauth-flow.sh` returns valid JWT
- **Files to modify:** `mcp/src/oauth.rs` (new), `mcp/src/mcp.rs` (route registration + middleware), `mcp/src/main.rs` (state init), `mcp/src/tools.rs` (recall filter by `owner_pubkey`), `mcp/Cargo.toml`, `core/src/storage/sqlite.rs` (migration runner)
- **Files to read:** `mcp/src/payment.rs`, `mcp/src/tools.rs`, `core/src/storage/sqlite.rs`, `work/mnemonic-integrations/code-research.md` §2, §6

#### Task 5: Smithery listing + DNS subdomain + nginx
- **Description:** Create `smithery.yaml` at repo root with `mcp.mnemonik.xyz` endpoint and OAuth flow declaration. Coordinate DNS A-record for `mcp.mnemonik.xyz` → VPS. Update nginx config (`/etc/nginx/sites-available/mnemonic` per `deployment.md`) to add subdomain server-block proxying to `localhost:3000`. Run `certbot --nginx -d mcp.mnemonik.xyz`. Submit listing to smithery.ai.
- **Skill:** infrastructure-setup
- **Reviewers:** code-reviewer, security-auditor, infrastructure-reviewer
- **Verify-smoke:** `dig +short mcp.mnemonik.xyz` returns VPS IP; `curl -fI https://mcp.mnemonik.xyz/health` returns 200
- **Verify-user:** Visit `smithery.ai/mcp/mnemonic` — listing visible with install-deeplink
- **Files to modify:** `smithery.yaml` (new), `deployment.md` (subdomain section), nginx config on VPS (out-of-tree)
- **Files to read:** `.claude/skills/project-knowledge/references/deployment.md`, `work/mnemonic-integrations/code-research.md` §7

### Wave 3: UI + tests (parallel)

#### Task 6: Webapp landing + install-hub + identity panel
- **Description:** Add `webapp/src/pages/Landing.tsx` (route `/`) — protocol pitch + "Get started" CTA leading to `/install`. Add `webapp/src/pages/Install.tsx` (route `/install`) — identity panel (Generate / Import / Export keypair via WASM core) + deeplink buttons for Cursor / VS Code / Claude.ai. Move existing chat demo from `/` to `/chat`. Use existing Tailwind tokens from `ux-guidelines.md`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cd webapp && npm run dev` — open localhost:5173/, /install, /chat — all render without console errors
- **Verify-user:** On `/install`, click "Generate keypair" → DID/pubkey appears → click "Download backup" → JSON file with valid Ed25519 keypair
- **Files to modify:** `webapp/src/App.tsx` (router), `webapp/src/pages/Landing.tsx` (new), `webapp/src/pages/Install.tsx` (new), `webapp/src/components/IdentityPanel.tsx` (new), `webapp/src/components/InstallButtons.tsx` (new)
- **Files to read:** `webapp/src/App.tsx`, `.claude/skills/project-knowledge/references/ux-guidelines.md`, `webapp/src/wasm/` (generated by Task 3)

#### Task 7: COSE round-trip-via-proxy test + MCP Inspector CI step
- **Description:** Add `mcp/tests/roundtrip_cose_via_http_proxy.rs` — boot a local Axum mock proxy that re-serializes JSON-RPC bodies, send `sign_memory` through it, verify COSE_Sign1 bytes survive. Update `.github/workflows/ci.yml` to add MCP Inspector validation step running against `cargo run -p mnemonic-mcp` started in background.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp roundtrip_cose_via_http_proxy` passes; CI run shows MCP Inspector step green
- **Files to modify:** `mcp/tests/roundtrip_cose_via_http_proxy.rs` (new), `.github/workflows/ci.yml`
- **Files to read:** `core/src/codec/sign.rs`, `core/tests/integration_cbor.rs`, `.github/workflows/ci.yml`, `work/mnemonic-integrations/code-research.md` §8

#### Task 8: Pre-demo manual smoke checklist
- **Description:** Author `work/mnemonic-integrations/tasks/smoke-checklist.md` — exhaustive manual flow on Cursor + Claude.ai Pro covering: fresh-browser onboarding, keypair gen, install deeplink, OAuth approve, sign_memory, switch to second browser/tool, recall. Each step has expected result and rollback note. Document used during pre-release smoke and live demo dry-run.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-user:** A team member who didn't write the spec executes the checklist on a fresh laptop end-to-end without ambiguity
- **Files to modify:** `work/mnemonic-integrations/tasks/smoke-checklist.md` (new)
- **Files to read:** `work/mnemonic-integrations/user-spec.md` (Сценарии), `work/mnemonic-integrations/research.md`

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
- **Files to read:** `work/mnemonic-integrations/user-spec.md`, `work/mnemonic-integrations/tech-spec.md`, `work/mnemonic-integrations/tasks/smoke-checklist.md`, all modified source files

#### Task 13: Deploy
- **Description:** Deploy hosted MCP to `mcp.mnemonik.xyz` subdomain on existing VPS (per `deployment.md` flow — `cargo build --release`, restart `mnemonic-mcp.service`, verify systemd status). Deploy webapp to Cloudflare Pages (or VPS nginx, per existing flow). Verify both endpoints return 200 over HTTPS. Submit Smithery listing.
- **Skill:** deploy-pipeline
- **Reviewers:** code-reviewer, security-auditor, deploy-reviewer
- **Verify-smoke:** `curl -fI https://mcp.mnemonik.xyz/health && curl -fI https://mnemonik.xyz/install`
- **Files to modify:** VPS nginx config (out-of-tree), GitHub Actions workflow if Cloudflare Pages deploy is added
- **Files to read:** `.claude/skills/project-knowledge/references/deployment.md`

#### Task 14: Post-deploy QA
- **Description:** On the live `mcp.mnemonik.xyz` endpoint, run `npx @modelcontextprotocol/inspector --validate https://mcp.mnemonik.xyz/mcp`. Trigger full OAuth flow via real Cursor connector install on a clean Cursor profile; verify `sign_memory → recall` round-trip. Verify Smithery listing is live and the install-deeplink works. Mark all user-spec success metrics measurable (install counter wired up).
- **Skill:** post-deploy-qa
- **Reviewers:** none
- **Files to read:** `work/mnemonic-integrations/user-spec.md` (success metrics + verification table), `work/mnemonic-integrations/tasks/smoke-checklist.md`
