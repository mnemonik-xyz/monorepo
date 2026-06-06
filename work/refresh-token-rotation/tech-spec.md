---
created: 2026-06-06
status: draft
branch: dev
size: M
---

# Tech Spec: refresh-token-rotation

## Solution

Add an OAuth 2.1 refresh-token rotation surface to the existing `/oauth/token`
endpoint so HTTP MCP-host sessions survive past the 1h JWT access-token TTL.
Mechanics mirror Stripe MCP's proven model: every successful
`authorization_code` exchange now also issues an opaque 32-byte refresh token
with a 1-year rolling TTL. On access-token expiry the host POSTs
`grant_type=refresh_token`; the server atomically revokes the presented token,
issues a new (access, refresh) pair, and rolls the refresh `expires_at` forward
one year. Replay of a revoked token **outside** a 5-second reuse-interval
triggers a family-wide revocation (OAuth 2.1 §6.1); **inside** the window the
same plaintext returns the same already-issued descendant pair so legitimate
network retries don't kill sessions.

The implementation lives entirely in `mcp/` (architectural rule: OAuth state
stays in `mcp/`, never `core/`). One new module `mcp/src/oauth/refresh.rs`
holds the SQLite-backed store + migration + rotation transaction; the existing
`token_handler` gains a post-parse branch on `grant_type`; `OAuthState` widens
to carry the shared `rusqlite::Connection` handle and an LRU plaintext cache.
Access-token format and middleware are unchanged. Stdio transport is
untouched. Discovery metadata adds `"refresh_token"` to `grant_types_supported`.

A small env-plumbing change (`MCP_JWT_TTL_SECS` via `OnceLock<u64>`) makes the
JWT TTL overridable for the pre-ship R1 empirical gate (dev deploy with
TTL=60s + Cursor/Claude.ai parallel observation). All migrations are
idempotent `CREATE TABLE IF NOT EXISTS` (rolling-deploy safe). Rollback is
revert-the-tag with no data migration to undo.

## Architecture

### What we're building/modifying

- **`mcp/src/oauth/refresh.rs` (NEW)** — opaque-token CRUD, blake3-salted
  hashing, BEGIN IMMEDIATE rotation transaction, family-revoke walk,
  plaintext LRU cache for reuse-interval idempotency, migration helper,
  hourly background evictor task. Module-public functions:
  `mint_for_authorization_code`, `rotate`, `family_revoke`, `evict_expired`,
  `migrate_refresh_tokens`, `start_evictor`.
- **`mcp/src/oauth/mod.rs` (EDIT)** —
  - `OAuthState` widened with `Arc<Mutex<rusqlite::Connection>>`, refresh
    `salt: Vec<u8>`, reuse-interval config (`reuse_interval`,
    `evictor_tick`), plaintext-cache handle.
  - `TokenRequest` struct widened: `code`/`code_verifier` become
    `Option<String>`, new `grant_type: Option<String>` (defaults to
    `"authorization_code"`), new `refresh_token: Option<String>`.
  - `token_handler` post-parse dispatches on `grant_type`; new branch
    calls `refresh::rotate` and returns the new pair.
  - Discovery metadata: one-line `+ "refresh_token"` in
    `grant_types_supported`.
  - `JWT_TTL_SECS` const replaced with `jwt_ttl_secs()` reading
    `OnceLock<u64>` seeded from `MCP_JWT_TTL_SECS` env. All 7 production
    read sites (4 in this file + 3 in `escrow.rs`) call the new function.
- **`mcp/src/main.rs` (EDIT)** — `run_http` seeds the `OnceLock`, opens the
  shared `Connection`, passes `Arc<Mutex<Connection>>` to `OAuthState::new`,
  calls `refresh::migrate_refresh_tokens` alongside
  `migrate_key_escrow_blobs` + `migrate_google_identity_links`, spawns
  `refresh::start_evictor`.
- **`mcp/tests/_helpers/mod.rs` (EDIT)** — `TestServerBuilder` gains a
  `with_oauth_token(bool)` field; `build()` conditionally merges a
  `/oauth/token`-only sub-router with `State<Arc<OAuthState>>` so
  integration tests reach the new branch without per-test mini-routers.
- **`mcp/tests/oauth_refresh_e2e.rs` (NEW)** — 13 integration tests, one
  per AC1–AC13.
- **`mcp/src/test_support.rs` (EDIT)** — short helpers used by the new
  integration tests (no new public surface beyond minor additions
  alongside existing `mint_jwt`).

### How it works

1. **First login.** Client posts `grant_type=authorization_code`. Existing
   PKCE+code flow validates. After the access JWT is minted, `refresh::
   mint_for_authorization_code(state, sub, google_sub)` generates 32 random
   bytes, base64url-encodes them, computes `blake3(salt + plaintext)`, and
   inserts a `refresh_tokens` row with a fresh `family_id = Uuid::new_v4()`
   and `expires_at = now + 1y`. Response is the 4-field JSON:
   `{access_token, token_type:"Bearer", expires_in, refresh_token}`. Both
   content-types (form-encoded and JSON) share the path; the existing
   dispatch (`oauth/mod.rs:990-1017`) already provides parity.
2. **Routine use within 1h.** Client uses `access_token` as Bearer on
   `/mcp` — unchanged.
3. **Access expiry.** Bearer middleware returns `-32001 unauthorized:
   invalid JWT: ExpiredSignature` and `WWW-Authenticate: Bearer
   error="invalid_token"`. Client posts `grant_type=refresh_token` with
   the saved refresh.
4. **Rotation (atomic).** `refresh::rotate(state, plaintext)`:
   - Hashes the plaintext (`blake3(salt + plaintext)`).
   - Opens `BEGIN IMMEDIATE`.
   - `SELECT * FROM refresh_tokens WHERE token_hash = ?1`.
   - **Branch A — found, `revoked = 0`, `expires_at > now`:** legitimate
     rotation. Mints new plaintext + new row with same `family_id`,
     `rotated_to`-FK from old, refreshes `expires_at = now + 1y`. Caches
     plaintext in the in-memory LRU keyed by `old_token_hash` with TTL =
     `reuse_interval`. UPDATE old (`revoked = 1, rotated_at = now,
     rotated_to = <new_hash>`). COMMIT.
   - **Branch B — found, `revoked = 1`, `rotated_at + reuse_interval >
     now`:** legitimate retry within reuse window. Look up cached plaintext
     by `old_token_hash` in LRU — return that exact descendant pair (same
     `access_token`+`refresh_token` already issued in Branch A).
   - **Branch C — found, `revoked = 1`, outside reuse window:**
     potential replay. Walk `family_id` and UPDATE every row in the family
     to `revoked = 1` (`family_revoke`). Return `400 invalid_grant`.
   - **Branch D — found, `revoked = 0`, `expires_at <= now`:** expired
     refresh, not an attack. Return `400 invalid_grant` without
     family-revoke.
   - **Branch E — not found:** return `400 invalid_grant`.
5. **Background eviction.** `refresh::start_evictor`, spawned from
   `main.rs` alongside `_confirmation_evictor`, ticks every 3600s and runs
   `DELETE FROM refresh_tokens WHERE expires_at + grace < now`. Plaintext
   LRU evicts on TTL (`reuse_interval`).
6. **Discovery.** `/.well-known/oauth-authorization-server`
   `grant_types_supported` now contains `"authorization_code"` AND
   `"refresh_token"`. Clients automatically detect refresh support.

### Shared resources

| Resource | Owner (creates) | Consumers | Instance count |
|----------|----------------|-----------|----------------|
| `Arc<Mutex<rusqlite::Connection>>` (shared SQLite handle) | `main.rs::run_http` | `McpState.store`, `OAuthState.refresh_store`, refresh evictor task | 1 (singleton — same file as attestations) |
| `OnceLock<u64>` JWT TTL | `main.rs::run_http` (seeds from `MCP_JWT_TTL_SECS` env or 3600 default) | `oauth/mod.rs::jwt_ttl_secs()` callers (7 prod sites in `oauth/mod.rs` + `escrow.rs`) | 1 (process-global) |
| Plaintext LRU cache (`refresh::ReuseCache`) | `OAuthState::new` | `refresh::rotate` | 1 per server, cap 256 entries, TTL = `reuse_interval` |
| Refresh-token salt (`Vec<u8>`, 32 bytes) | `OAuthState::new` (reads `MCP_REFRESH_SALT` env; fallback `blake3(MCP_JWT_SECRET + "refresh")` for backward compat) | `refresh::hash_token` | 1 per deploy |

## Decisions

### Decision 1: Standard OAuth 2.1 refresh-token rotation, Stripe-precedent timing (1h access + 1y rolling refresh)
**Decision:** Implement OAuth 2.1 `grant_type=refresh_token`. Access TTL stays
1h (unchanged). Refresh TTL is 1 year, rolling on every use.
**Rationale:** Stripe's hosted MCP server runs the same OAuth model and
the same MCP-host clients (Cursor, VS Code, Claude.ai) silently rotate
without UX impact. This addresses the user-spec's central problem
(`Зачем`: "сессия в HTTP-MCP-хосте умирает через час").
**Alternatives considered:**
- Bump JWT TTL to 24h/7d — postpones but doesn't fix; widens revocation
  blast radius (D7 below).
- Per-request signing — proper long-term, but breaking-change for every
  client. Parked at `work/stateless-auth-rearch/`.
**User-spec anchor:** Что делаем + Зачем sections; AC1–AC9 baseline.

### Decision 2: Opaque refresh tokens, blake3(salt+plaintext) at rest
**Decision:** Refresh tokens are 32 random bytes, base64url-encoded for the
wire. Stored as `blake3(salt + plaintext)`. Plaintext leaves the server
once (in the response).
**Rationale:** Stripe and Auth0 both use opaque tokens; revocation is
trivial (state flag in DB) and metadata leakage is minimal. Blake3 matches
the existing precedent in `mcp/src/payment.rs:737-744` (`hash_api_key`) so
operators have one hashing primitive across the auth surface.
**Alternatives considered:**
- JWT-as-refresh-token — revocation requires a blacklist anyway, gains
  nothing.
- SHA-256 — would split the codebase across two hash primitives.
**User-spec anchor:** D1.

### Decision 3: Per-grant UUID `family_id` (multi-device isolation)
**Decision:** Each `authorization_code` exchange mints a fresh
`family_id = Uuid::new_v4()`. A user logged in from two browsers has two
independent families; compromise of one device's refresh family does not
revoke the other.
**Rationale:** Stripe and Auth0 standard. Sub-bound `family_id = sub`
(rejected alternative) would brick all sessions on a single leak —
unacceptable for a memory protocol where users may keep many concurrent
clients.
**User-spec anchor:** D13.1.

### Decision 4: 5-second reuse-interval (Auth0 default) for retry-vs-replay distinction
**Decision:** `reuse_interval = 5s`. Inside the window, presenting the
revoked refresh returns the same descendant pair (idempotent retry path).
Outside the window, presenting the revoked refresh is treated as a
potential replay attack and revokes the entire `family_id`.
**Rationale:** 5s covers every realistic network retry while keeping the
replay-attack window short. Auth0 ships 5s by default. Okta's 30s default
was rejected as too wide.
**User-spec anchor:** D13, AC3, AC4, AC12.

### Decision 5: In-memory LRU plaintext cache for reuse-interval idempotency `[TECHNICAL]`
**Decision:** New `refresh::ReuseCache` keyed by the **old** token's hash,
stores the **new** plaintext for `reuse_interval`. Cap 256 entries. On
Branch-B (legitimate retry within window), the rotation function returns
the cached plaintext without consulting the DB further.
**Rationale:** `[TECHNICAL]` The plaintext of the new refresh is destroyed
right after the response is sent (we only persist `blake3(salt+plaintext)`).
Without a cache, an honest network retry inside the reuse window would
either: (a) return a fresh independent rotation (violates idempotency —
caller now holds TWO valid refresh tokens), or (b) return 400 (kills
sessions on plausible retries). The LRU is the smallest correct
mechanism. Pattern parallels `confirmation_token::ConfirmationLedger`
in-memory cache discipline. **Required to honor AC12 (Branch-B semantic)
under the D2 hash-at-rest constraint. No user-spec text changes.**
**Alternatives considered:**
- Store plaintext alongside hash in DB — defeats the hash-at-rest goal
  if the DB leaks.
- Encrypt plaintext-at-rest with deploy-local key — extra surface, marginal
  win vs. an in-memory cache that dies with the process.

### Decision 6: `OAuthState` holds `Arc<Mutex<rusqlite::Connection>>` (not `SqliteStore`)
**Decision:** `OAuthState::new(...)` widens to accept
`Arc<Mutex<rusqlite::Connection>>` directly (not the `SqliteStore` wrapper
type). The same `Connection` handle is shared with `McpState.store` —
single DB file, single backup, single migration.
**Rationale:** Matches existing precedent — `escrow.rs:113` and
`oauth/google.rs:340` both already take `&Connection` directly for their
migrations. Avoids a new wrapper type and threading complications.
**Alternatives considered:**
- Separate `refresh_tokens.db` file — operationally worse (extra backup
  job, extra schema config) for no observable win.
- Inject `SqliteStore` and unwrap — adds a dependency on
  `core::storage::sqlite` from `mcp/src/oauth/` which is currently clean.
**User-spec anchor:** D10.

### Decision 7: Access-token format unchanged (JWT HS256, 1h TTL) — accepts no-global-logout trade-off
**Decision:** Access tokens remain HS256 JWTs with 1h TTL. Existing
`Claims` struct untouched. Middleware path
(`bearer_auth_middleware` at `oauth/mod.rs:1382-1529`) untouched.
**Rationale:** Constrains scope. Moving access tokens to opaque
introspection-style would be a much larger change (new state in DB on
every call) without addressing the bug. The known limitation —
"log out everywhere" requires HMAC-secret rotation, not a revoke-list —
is documented as R7 and accepted for V1.
**Alternatives considered:**
- Opaque access tokens with introspection endpoint — large rewrite, not
  required to fix the bug.
- Shorter access TTL (e.g. 5 min) — more rotations, slightly more load
  on `/oauth/token`, marginal security gain; defer until R1 verification
  shows we need it.
**User-spec anchor:** D7, R7.

### Decision 8: `BEGIN IMMEDIATE` for atomic rotation, no `tokio::spawn_blocking`
**Decision:** Rotation runs in a single `BEGIN IMMEDIATE` transaction
(SELECT-with-write-lock, branch, UPDATE/INSERT, COMMIT). Connection
operations are synchronous; we hold the `Mutex` only for the duration of
the transaction. No `tokio::spawn_blocking`.
**Rationale:** `payment.rs:478-505` is the canonical project pattern for
atomic check-then-write on SQLite. `grep` over `mcp/src/` shows zero
`spawn_blocking` use — every SQLite touch site holds the lock for short,
in-process work. The hard rule from `CLAUDE.md` is "never hold the SQLite
mutex across `.await`", which a synchronous transaction respects by
construction.
**Alternatives considered:**
- Optimistic CAS without `BEGIN IMMEDIATE` — race-prone.
- `spawn_blocking` — would introduce a new pattern with no upside;
  rotation is fast (no I/O outside SQLite).
**User-spec anchor:** D11, AC12.

### Decision 9: Hourly background evictor only (no opportunistic in-transaction cleanup)
**Decision:** `refresh::start_evictor` ticks every 3600s and runs
`DELETE FROM refresh_tokens WHERE expires_at + grace < now`. No
opportunistic in-transaction `DELETE` on each rotation.
**Rationale:** With 1y TTL, hourly is more than enough — table never
balloons unless the deploy has astronomical user count, and even then
hourly + indexed `expires_at` is cheap. Opportunistic cleanup in the
rotation transaction was an early-round-1 idea, dropped in round 2
(over-engineering — adds complexity without observable benefit at this
scale).
**User-spec anchor:** D12.

### Decision 10: Dual content-type parity via existing dispatch (no new parsing branches)
**Decision:** `grant_type=refresh_token` flows through the same
content-type dispatch as `authorization_code` (`oauth/mod.rs:990-1017`).
`TokenRequest` widens to make `code` / `code_verifier` `Option<String>`,
adds `grant_type: Option<String>` (defaults to `"authorization_code"`)
and `refresh_token: Option<String>`. Post-parse dispatch on
`grant_type` selects the handler.
**Rationale:** Per `oauth/mod.rs:975-977`: VS Code + Claude.ai send
`application/x-www-form-urlencoded`; Cursor sends `application/json`.
The existing dispatch handles both; reusing it gives free parity. The
refresh-branch needs `refresh_token` field only — `code`/`code_verifier`
must therefore be optional.
**Alternatives considered:**
- Two separate handlers / routes — would require duplicating the
  content-type dispatch.
- Separate refresh-only struct — would force the handler to pre-peek
  `grant_type` before deserialize.
**User-spec anchor:** D14, AC10.

### Decision 11: Post-parse `400 invalid_request` for missing `refresh_token` field
**Decision:** When `grant_type=refresh_token` arrives with the
`refresh_token` field absent/empty, return `400 invalid_request` via
the existing `oauth_error` builder (`oauth/mod.rs:1158`).
**Rationale:** RFC 6749 §5.2 explicitly: missing or unsupported parameter
→ `invalid_request`. Validator caught this is wire-distinct from
`invalid_grant` (which means "this grant was rejected"); clients can
distinguish.
**User-spec anchor:** AC13.

### Decision 12: `JWT_TTL_SECS` via `OnceLock<u64>` + `MCP_JWT_TTL_SECS` env override (R1 prereq)
**Decision:** Replace `pub const JWT_TTL_SECS: u64 = 3600` with a function
`jwt_ttl_secs()` reading from `std::sync::OnceLock<u64>`, seeded once in
`run_http` from the env var `MCP_JWT_TTL_SECS` (fallback 3600). All 7
production read sites — 4 in `oauth/mod.rs` (`:391, :1075, :1113, :1124`)
and 3 in `escrow.rs` (`:59, :511, :797`) — switch to the function.
**Rationale:** R1 verification (Option B in user-spec) requires deploying
to `mcp.dev.mnemonik.xyz` with TTL=60s so we can observe Cursor/Claude.ai
silent rotation in 2 minutes. Today the const is hard-baked; a one-shot
patch-per-deploy is ugly and fragile. OnceLock is cleaner than
field-threading through `escrow.rs` (which has no `OAuthState` in scope
at `escrow.rs:797`).
**Alternatives considered:**
- Field on `OAuthState` — invasive change in `escrow.rs:797` (mints an
  `aud=extension` JWT without an OAuthState handle).
- Dev-only patch on the const — fragile; easy to forget on deploy.
**User-spec anchor:** R1 Option B prerequisite footnote.

### Decision 13: `reuse_interval` and `evictor_tick` as `OAuthState` fields (test-overridable) `[TECHNICAL]`
**Decision:** Both timing constants live as fields on `OAuthState`,
defaulting to `Duration::from_secs(5)` and `Duration::from_secs(3600)`.
`TestServerBuilder` exposes a setter so integration tests can configure
`reuse_interval = Duration::from_millis(100)` and run fast.
**Rationale:** `[TECHNICAL]` Without this, every integration test that
exercises reuse-interval semantics (AC3, AC4, AC12) would need real
`tokio::time::sleep(Duration::from_secs(6))` to test outside-window
behavior — flaky and slow on CI. Pattern matches
`ConfirmationLedger::with_config` precedent (`confirmation_token.rs:
96-105`). **Required for AC3/AC4/AC12 verification under reasonable CI
time budget. No user-spec changes.**
**Alternatives considered:**
- `tokio::time::pause` + `advance` — works but requires test plumbing
  and a strong `tokio_test` dependency that the rest of the suite
  doesn't use.
- Compile-time `cfg(test)` constants — would make tests not exercise
  the prod code path.

## Data Models

### `refresh_tokens` table

```sql
CREATE TABLE IF NOT EXISTS refresh_tokens (
    token_hash   TEXT    PRIMARY KEY,           -- blake3(salt + plaintext) hex
    sub          TEXT    NOT NULL,              -- user identity (matches Claims.sub)
    google_sub   TEXT,                          -- optional, mirrors Claims.google_sub
    issued_at    INTEGER NOT NULL,              -- unix seconds
    expires_at   INTEGER NOT NULL,              -- rolling 1y from last rotation
    revoked      INTEGER NOT NULL DEFAULT 0,    -- 0 = active, 1 = revoked
    rotated_at   INTEGER,                       -- unix seconds when revoked (for reuse-interval)
    rotated_to   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL,
    family_id    TEXT    NOT NULL               -- UUID, shared across rotation chain
);

CREATE INDEX IF NOT EXISTS refresh_tokens_family_idx
    ON refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS refresh_tokens_expires_idx
    ON refresh_tokens(expires_at);
CREATE INDEX IF NOT EXISTS refresh_tokens_sub_idx
    ON refresh_tokens(sub);
```

Migration via `mcp/src/oauth/refresh.rs::migrate_refresh_tokens(conn:
&Connection) -> Result<()>`, modeled on `escrow.rs:113-133`
(`execute_batch` + `MIGRATION_SQL` const).

### `TokenRequest` widened

```rust
#[derive(Deserialize)]
struct TokenRequest {
    grant_type: Option<String>,    // NEW; defaults to "authorization_code"
    code: Option<String>,          // CHANGED: was String
    code_verifier: Option<String>, // CHANGED: was String
    refresh_token: Option<String>, // NEW
    client_id: Option<String>,     // unchanged
    redirect_uri: Option<String>,  // unchanged
}
```

### `/oauth/token` success response (unchanged shape, new field)

```json
{
  "access_token": "<JWT>",
  "token_type": "Bearer",
  "expires_in": 3600,
  "refresh_token": "<32-byte base64url>"
}
```

`refresh_token` returned for BOTH `grant_type` paths going forward.
Old clients ignoring the field continue to work (AC11).

### `OAuthState` additions

```rust
pub struct OAuthState {
    // ... existing fields ...
    pub refresh_store: Arc<Mutex<rusqlite::Connection>>,
    pub refresh_salt: Vec<u8>,
    pub reuse_interval: std::time::Duration,
    pub evictor_tick: std::time::Duration,
    pub reuse_cache: Arc<refresh::ReuseCache>,
}
```

## Dependencies

### New packages
- `lru` (`^0.12`) — small in-memory LRU for `ReuseCache`. Direct
  `mcp/Cargo.toml` add: `lru = { version = "0.12", default-features = false }`.

### Using existing (from project)
- `rusqlite` — same `Connection` handle as `McpState.store`.
- `blake3` — already used by `payment.rs:737-744` for API-key hashing.
- `uuid` — already used for `Claims.jti`; reuse for `family_id`.
- `serde_urlencoded` + `serde_json` — existing dispatch handles both
  content-types.
- `tokio` — existing runtime; evictor uses `tokio::time::interval` per
  the `confirmation_token::start_evictor` pattern (`confirmation_token.rs:
  259-267`).
- `tracing` — log every rotation, family-revoke, and evictor tick. No
  Prometheus in V1 (deferred per user-spec).
- `std::sync::OnceLock` (stable since 1.70) for `JWT_TTL_SECS` — avoids
  a new direct dep.

## Testing Strategy

**Feature size:** M

### Unit tests
Located alongside the implementation files; run with `cargo test
-p mnemonic-mcp`.

- `refresh::tests::mint_and_hash_roundtrip` — generated plaintext hashes
  deterministically; same salt + plaintext → same hash.
- `refresh::tests::expired_refresh_rejected_without_family_revoke` —
  Branch D: family stays intact.
- `refresh::tests::reuse_within_window_returns_cached_pair` — Branch B
  cache hit returns identical (access, refresh).
- `refresh::tests::reuse_outside_window_revokes_family` — Branch C walks
  `family_id`, every row in family ends `revoked=1`.
- `refresh::tests::rotate_atomic_under_begin_immediate` — verifies the
  transaction holds the lock through SELECT→UPDATE→INSERT.
- `oauth::tests::token_request_deserializes_both_grants` — both content
  types, both `grant_type` values.
- `oauth::tests::missing_refresh_token_returns_invalid_request` — AC13
  wire response.

### Integration tests
`mcp/tests/oauth_refresh_e2e.rs` (NEW). Uses `TestServerBuilder::
with_oauth_token(true)` to mount `/oauth/token`. One test per AC1–AC13;
sketches in `code-research.md §I.9`. Critical fixture helpers:
`bootstrap_oauth(server) -> (code, verifier, redirect_uri)`,
`rotate(server, refresh) -> response`,
`insert_expired_refresh_for_test(state, sub) -> plaintext`.

### E2E tests
Single curl-based smoke at `mcp/tests/oauth_refresh_e2e.rs` runs a real
HTTP server end-to-end (not full Claude.ai). Real Claude.ai empirical
verification is the **R1 pre-ship gate** (see Agent Verification Plan
below) — not part of CI.

## Agent Verification Plan

**Source:** user-spec.md "Как проверить" section + the new pre-ship R1 gate.

### Verification approach

Three tiers:

1. **Per-task smoke checks** (each task specifies `Verify-smoke:` /
   `Verify-user:` where applicable — see Implementation Tasks).

2. **Pre-deploy QA** (T10): `cargo test --workspace` green, the 13 AC
   integration tests in `oauth_refresh_e2e.rs` green, no clippy warnings.

3. **Dev-deploy + R1 empirical gate** (T11): `mcp.dev.mnemonik.xyz`
   deployed from a topic branch with `MCP_JWT_TTL_SECS=60`. Connect
   Claude.ai (the web app — confirms `*.claude.ai` CORS path) AND Cursor
   in parallel. Both clients stay open for >2 minutes (multiple expiries).
   - Cursor (known to rotate) is the **control** — must keep working.
   - Claude.ai is the **device under test** — if it keeps working
     silently, R1 holds; if it requires re-auth, refresh tokens don't
     help Claude.ai → escalate (Anthropic ticket / pivot to
     `stateless-auth-rearch`).
   - Verifier: human user (the task author) observes both browsers.

4. **Prod deploy + post-deploy verification** (T12+T13): see Final Wave.

### Tools required

- `curl` for the post-deploy smoke at `mcp.mnemonik.xyz/.well-known/
  oauth-authorization-server` (verifies `grant_types_supported` includes
  `refresh_token`).
- `bash` for the test runner and the `MCP_JWT_TTL_SECS` env setup on
  dev.
- A real Claude.ai session + a real Cursor session for T11 (Verify-user).

## Risks

| Risk | Mitigation |
|------|-----------|
| Claude.ai might ignore `refresh_token` (R1) | T11 pre-ship gate. If Claude.ai fails, do not promote to prod; escalate (Anthropic) or pivot to `stateless-auth-rearch`. |
| Salt rotation invalidates all live refresh tokens | Treat `MCP_REFRESH_SALT` as a deploy secret. Document operational rule: rotating salt forces all users through re-OAuth (acceptable but rare). |
| Test flakiness on real-time reuse-interval | D13 — fields on `OAuthState` let tests use 100ms. CI tests never `sleep(5s)`. |
| Refresh-token leak (logs, client) | Plaintext returned once; never logged; HTTPS-only on prod; replay-detect revokes the family (D4). |
| DB write failure during rotation | `500 internal_error`; client retry path is safe (idempotent within reuse window). |
| Wire-format back-compat | AC11 + integration test asserts old clients ignoring the new field continue to work. |
| Cursor/VS Code/Claude.ai content-type drift | AC10 + step 5b explicitly exercises both formats on the refresh branch. PK `architecture.md` corrected in round 3 commit `7a0065a` to match `oauth/mod.rs:975-977`. |
| `TokenRequest` widening breaks legacy parse | All four fields newly optional (`code`, `code_verifier`, `grant_type`, `refresh_token`); existing `authorization_code` callers must still send `code` + `code_verifier` (post-parse validation gates this). |
| LRU cache lost on restart kills in-flight retries | Acceptable — reuse window is 5s; server restart already kills active connections. Document but don't mitigate. |
| Rolling deploy with refresh-evictor double-spawn | `start_evictor` is spawned exactly once from `main.rs`; rolling deploy replaces the process, evictor restarts cleanly. |

## User-Spec Deviations

None.

The two tech-spec footnotes from the user-spec round-3 adequacy review
(`JWT_TTL_SECS` env-plumbing prerequisite; `TokenRequest` struct
widening for AC13) are honored in Decision 12 and Decision 11
respectively. They are **implementation details** the user-spec
explicitly invited the tech-spec to resolve; they neither contradict
nor extend any user-spec requirement.

Two new technical decisions (Decision 5 plaintext LRU cache, Decision
13 test-overridable timing fields) are marked `[TECHNICAL]`. They do
not change any user-spec requirement; they are mechanisms required to
honor existing ACs (AC12, AC3, AC4) without breaking other invariants
(D2 hash-at-rest, CI time budget).

## Acceptance Criteria

Технические критерии приёмки (дополняют пользовательские из user-spec
AC1–AC13):

- [ ] `cargo test --workspace --no-fail-fast` зелёный.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` зелёный.
- [ ] `cargo fmt --all -- --check` зелёный.
- [ ] `gitleaks detect --no-banner` зелёный (на working tree и в полной
      истории по CI policy).
- [ ] Все 13 user-spec ACs (AC1–AC13) покрыты интеграционными тестами
      в `mcp/tests/oauth_refresh_e2e.rs` — каждый AC соответствует
      одной из 13 test functions.
- [ ] Tool-count assertion at `mcp/src/mcp.rs:1965` — N/A для этой фичи
      (новых MCP-тулов нет). Если случайно тронули — не меняем.
- [ ] Миграция `refresh_tokens` идемпотентна (повторный запуск не падает,
      проверено в `refresh::tests::migration_is_idempotent`).
- [ ] Discovery metadata `/.well-known/oauth-authorization-server` после
      деплоя возвращает `grant_types_supported` содержащий
      `"refresh_token"` (AC7).
- [ ] `tracing` логи в проде содержат: успешные ротации (sub, family_id,
      без plaintext), family-revoke события, hourly evictor tick.
- [ ] Нет регрессий в существующих oauth тестах (`oauth/mod.rs:2022-2099`
      area + `mcp/tests/auth_allowlist.rs` + `mcp/tests/anonymous_recall.rs`).
- [ ] R1 pre-ship gate (T11) — Claude.ai продолжает работать с
      `MCP_JWT_TTL_SECS=60` на `mcp.dev.mnemonik.xyz` параллельно с
      Cursor control'ом в течение 2+ минут.

## Implementation Tasks

### Wave 1 (независимые — могут идти параллельно)

#### Task 1: Refresh-token storage module + migration + evictor
- **Description:** Create `mcp/src/oauth/refresh.rs` with: opaque-token
  type, `migrate_refresh_tokens(conn)` (idempotent `CREATE TABLE IF
  NOT EXISTS` + 3 indexes via `execute_batch` per the escrow.rs:113-133
  pattern), `mint_for_authorization_code`, `rotate` with the 5-branch
  BEGIN IMMEDIATE transaction described in Architecture §How-it-works,
  `family_revoke`, `evict_expired`, `start_evictor` (hourly tick),
  `ReuseCache` LRU (256-entry, configurable TTL). Use blake3 for
  hashing with the salt from `OAuthState.refresh_salt`. No
  `spawn_blocking` — synchronous SQLite under short-held Mutex.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Files to modify:** `mcp/src/oauth/refresh.rs` (new), `mcp/Cargo.toml`
  (add `lru = "0.12"`).
- **Files to read:** `mcp/src/escrow.rs` (migration pattern, lines
  113-133), `mcp/src/payment.rs` (blake3 hashing precedent, lines
  478-505 atomic UPDATE+INSERT, 737-744 hash_api_key),
  `mcp/src/confirmation_token.rs` (evictor pattern + ledger config,
  lines 96-105 + 259-267), `mcp/src/oauth/mod.rs` (current OAuth surface
  for context).

#### Task 2: `JWT_TTL_SECS` env-plumbing via `OnceLock` (R1 prerequisite)
- **Description:** Replace `pub const JWT_TTL_SECS: u64` at
  `oauth/mod.rs:58` with a function `pub fn jwt_ttl_secs() -> u64`
  reading from a `std::sync::OnceLock<u64>`. Seed the OnceLock in
  `mcp/src/main.rs::run_http` from env var `MCP_JWT_TTL_SECS` (parse
  u64; if absent or unparseable, default 3600). Update all 7 production
  read sites — 4 in `oauth/mod.rs` (`:391, :1075, :1113, :1124`) and 3
  in `escrow.rs` (`:59, :511, :797`). Add `.env.example` entry +
  document in `deployment.md`. No behavior change for prod (default
  preserved).
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `MCP_JWT_TTL_SECS=60 cargo run -p mnemonic-mcp -- --transport http --port 3000` boots cleanly; an integration test for `jwt_ttl_secs()` returning 60.
- **Files to modify:** `mcp/src/oauth/mod.rs`, `mcp/src/escrow.rs`,
  `mcp/src/main.rs`, `.env.example`,
  `.claude/skills/project-knowledge/references/deployment.md`.
- **Files to read:** `mcp/src/oauth/mod.rs:54-58` (existing const block),
  `mcp/src/escrow.rs:59` (extension-aud JWT mint site),
  `mcp/src/main.rs::run_http` (where the OnceLock is seeded).

### Wave 2 (after Wave 1 — extend OAuthState + token_handler)

#### Task 3: `OAuthState` extension — DB handle + salt + reuse-interval fields
- **Description:** Extend `OAuthState` struct (`oauth/mod.rs:156-160`)
  with `refresh_store: Arc<Mutex<rusqlite::Connection>>`, `refresh_salt:
  Vec<u8>`, `reuse_interval: Duration`, `evictor_tick: Duration`,
  `reuse_cache: Arc<refresh::ReuseCache>`. Widen `OAuthState::new`
  signature; thread the new args through all 4 call sites:
  `main.rs:791`, `_helpers/mod.rs:111`, `oauth/mod.rs:1593` (`fresh_state`
  test helper), `mcp.rs:1838`. Salt sourced from `MCP_REFRESH_SALT` env
  var (32+ bytes); fallback to `blake3(MCP_JWT_SECRET + "refresh")` so
  existing deploys don't fail on first boot. Defaults: `reuse_interval =
  5s`, `evictor_tick = 1h`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Files to modify:** `mcp/src/oauth/mod.rs`, `mcp/src/main.rs`,
  `mcp/tests/_helpers/mod.rs`, `mcp/src/mcp.rs`, `.env.example`,
  `.claude/skills/project-knowledge/references/deployment.md`.
- **Files to read:** `mcp/src/oauth/mod.rs:156-160` (current OAuthState),
  `mcp/src/oauth/refresh.rs` (Task 1 output — to wire the cache),
  `mcp/src/confirmation_token.rs:96-105` (with_config precedent).

#### Task 4: `TokenRequest` widening + `token_handler` refresh-grant branch + discovery metadata
- **Description:** Three coordinated changes in `oauth/mod.rs`: (1)
  widen `TokenRequest` (lines 946-957) — make `code` and `code_verifier`
  `Option<String>`, add `grant_type: Option<String>` and `refresh_token:
  Option<String>`. (2) In `token_handler` (lines 982-1078), after
  content-type dispatch + deserialize, branch on `grant_type` — default
  `"authorization_code"` runs the existing path unchanged; `"refresh_token"`
  validates the `refresh_token` field (absent/empty → `400 invalid_request`
  via existing `oauth_error` builder at line 1158), then calls
  `refresh::rotate`, then returns the new 4-field response. (3) Update
  discovery metadata at `oauth/mod.rs:1185` to add `"refresh_token"` to
  `grant_types_supported`. Existing `authorization_code` exchange also
  now issues a refresh token via `refresh::mint_for_authorization_code`
  before returning.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** with the dev server running from Task 2 smoke,
  `curl http://localhost:3000/.well-known/oauth-authorization-server | jq '.grant_types_supported'` returns an array containing both
  `"authorization_code"` AND `"refresh_token"`.
- **Files to modify:** `mcp/src/oauth/mod.rs`.
- **Files to read:** `mcp/src/oauth/mod.rs:946-1191` (current handler +
  discovery), `mcp/src/oauth/refresh.rs` (Task 1 output — `mint` + `rotate`
  signatures).

### Wave 3 (after Wave 2 — testing infrastructure + tests)

#### Task 5: `TestServerBuilder::with_oauth_token` extension
- **Description:** Extend `TestServerBuilder` (`mcp/tests/_helpers/
  mod.rs:104-126`) with `with_oauth_token(bool)` field (default `false`
  to preserve existing tests). When `true`, `build()` merges a
  `/oauth/token`-only sub-router with `State<Arc<OAuthState>>` into the
  same tower stack that mounts `/mcp` — no per-test mini-routers, no
  drift between tested and prod router configuration. Also expose
  helpers: setter for `reuse_interval`, fixture helper
  `bootstrap_oauth(&server)` that mints a `code` + `code_verifier` via
  the existing authorize flow and returns them for use by integration
  tests.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Files to modify:** `mcp/tests/_helpers/mod.rs`,
  `mcp/src/test_support.rs` (minor helpers if needed).
- **Files to read:** `mcp/tests/_helpers/mod.rs:104-296` (current
  TestServerBuilder + how `/mcp` is mounted), `mcp/src/oauth/mod.rs`
  (the `/oauth/token` route registration).

#### Task 6: Integration test suite — `oauth_refresh_e2e.rs`
- **Description:** New test file `mcp/tests/oauth_refresh_e2e.rs` with
  13 test functions, one per AC1–AC13. Each test uses
  `TestServerBuilder::with_oauth_token(true)` + `reuse_interval =
  Duration::from_millis(100)` so the suite runs in <2s. Specific
  function names and 3-5-line bodies sketched in `code-research.md §I.9`.
  AC10 form/JSON parity test runs two independent
  `bootstrap_oauth` → `rotate` cycles (the form path consumes the
  token, so a second token is needed for the JSON path). AC11
  back-compat test confirms an old-shape client (no `grant_type`,
  ignores `refresh_token`) keeps working. AC12 uses `tokio::join!` on
  two `rotate(server, rt_X)` calls with identical input and asserts
  both responses are equal (BEGIN IMMEDIATE serializes; the loser hits
  the reuse-cache path).
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Files to modify:** `mcp/tests/oauth_refresh_e2e.rs` (new),
  `mcp/src/test_support.rs` (minor helpers if needed).
- **Files to read:** `mcp/tests/_helpers/mod.rs` (TestServer harness),
  `mcp/tests/auth_allowlist.rs:64-136` (test pattern reference),
  `mcp/tests/anonymous_recall.rs` (regression-anchor reference),
  `mcp/src/oauth/refresh.rs` (signatures), `mcp/src/oauth/mod.rs`
  (handler signatures), `work/refresh-token-rotation/user-spec.md`
  AC1–AC13.

### Audit Wave (3 auditors in parallel — reviewers: none)

#### Task 7: Code Audit
- **Description:** Full-feature code-quality audit. Read all source
  files created/modified in this feature (per Files-to-modify across
  Tasks 1–6). Review holistically for: rusqlite mutex discipline (no
  lock across `.await`), Architecture/Shared-resources compliance
  (single `Connection`, single `OnceLock`, single `ReuseCache`),
  per-decision rationale, naming consistency with project conventions,
  error-handling patterns. Write report to
  `work/refresh-token-rotation/logs/working/code-audit.md`.
- **Skill:** code-reviewing
- **Reviewers:** none

#### Task 8: Security Audit
- **Description:** Full-feature security audit. Read all source files
  modified in this feature. OWASP Top 10 across all components: focus
  on auth flows (refresh-grant branch), input validation
  (`grant_type`/`refresh_token` deserialize), insecure storage (salt +
  blake3-at-rest), replay/race conditions (reuse-interval semantics,
  BEGIN IMMEDIATE atomicity, family-revoke triggering rules), logging
  (no plaintext refresh-tokens in any tracing call), CWE-352/CSRF
  vectors on `/oauth/token`. Verify R7 trade-off (no global logout) is
  documented and accepted, not silently introduced. Write report to
  `work/refresh-token-rotation/logs/working/security-audit.md`.
- **Skill:** security-auditor
- **Reviewers:** none

#### Task 9: Test Audit
- **Description:** Full-feature test-quality audit. Read all unit and
  integration tests created in this feature, plus impacted existing
  tests. Verify: each AC1–AC13 has at least one integration test;
  test pyramid balance (unit / integration / no E2E for M); meaningful
  assertions (not just status codes); negative-path coverage; flakiness
  resistance (no real `sleep(5s)` calls — confirmed via D13). Confirm
  no regression in existing `oauth/mod.rs:2022-2099`,
  `mcp/tests/auth_allowlist.rs`, `mcp/tests/anonymous_recall.rs`. Write
  report to `work/refresh-token-rotation/logs/working/test-audit.md`.
- **Skill:** test-master
- **Reviewers:** none

### Final Wave

#### Task 10: Pre-deploy QA
- **Description:** Acceptance testing on a clean local checkout: run
  `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and
  `gitleaks detect`. Verify all 13 integration tests in
  `oauth_refresh_e2e.rs` are GREEN. Verify the technical AC list above
  (`### Acceptance Criteria` section) holds locally. Produce
  pre-deploy-qa report.
- **Skill:** pre-deploy-qa
- **Reviewers:** none

#### Task 11: Dev deploy + R1 empirical gate
- **Description:** Deploy the feature branch to `mcp.dev.mnemonik.xyz`
  with `MCP_JWT_TTL_SECS=60`. Connect a real Claude.ai session AND a
  real Cursor session in parallel against the dev endpoint. Observe
  both for 2+ minutes (multiple JWT expiries pass). Cursor is the
  control (must keep working). Claude.ai is the device under test —
  if Claude.ai keeps working without an OAuth-page prompt, R1 holds and
  prod ship is GO; if Claude.ai requires re-auth, R1 fails — STOP, do
  not promote to prod, file Anthropic ticket and pivot to
  `work/stateless-auth-rearch/`. Document observed `POST /oauth/token`
  traffic (if visible) in the task report.
- **Skill:** deploy-pipeline
- **Reviewers:** none
- **Verify-user:** open Claude.ai (real, against dev MCP) and Cursor in
  parallel; both stay functional with `mnemonic_sign_memory` across 2+
  JWT expiries.

#### Task 12: Prod deploy (gated by Task 11)
- **Description:** Only execute if Task 11 reports R1 GO. Tag
  `v0.2.5`, push tag, let `release.yml` cross-compile the binary, build
  the Docker image, attach to GitHub Release. Deploy to
  `mcp.mnemonik.xyz` per the procedure in `deployment.md::VPS Deploy
  Process`. Confirm `MCP_JWT_TTL_SECS` is **NOT** set in prod env (or is
  set to 3600 explicitly) so prod uses the standard 1h TTL.
- **Skill:** deploy-pipeline
- **Reviewers:** none

#### Task 13: Post-deploy verification
- **Description:** Live-environment verification on
  `mcp.mnemonik.xyz` v0.2.5. Concrete checks:
  - `curl https://mcp.mnemonik.xyz/.well-known/oauth-authorization-server
    | jq '.grant_types_supported'` returns an array containing both
    `"authorization_code"` and `"refresh_token"` — tool: curl.
  - Connect a real Cursor session, leave open >65 minutes, verify
    `mnemonic_whoami` still responds without an OAuth-page prompt —
    tool: Cursor (manual user check).
  - Connect a real Claude.ai session, leave open >65 minutes, verify
    `mnemonic_sign_memory` still works after the first JWT expiry —
    tool: Claude.ai (manual user check).
  - Tail prod `tracing` logs for `refresh_rotation` events and
    `family_revoke` events; both should appear (the former routinely,
    the latter if any client misbehaves) — tool: bash + journalctl
    on VPS.
  Tools required: curl, bash, manual user verification through Cursor
  + Claude.ai.
- **Skill:** post-deploy-qa
- **Reviewers:** none
