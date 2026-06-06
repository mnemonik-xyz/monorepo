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
triggers a family-wide revocation in the same transaction (OAuth 2.1 §6.1);
**inside** the window the same plaintext returns the same already-issued
(access, refresh) pair so legitimate network retries don't kill sessions.

The implementation lives entirely in `mcp/` (architectural rule: OAuth state
stays in `mcp/`, never `core/`). One new module `mcp/src/oauth/refresh.rs`
holds the SQLite-backed store + migration + rotation transaction; the existing
`token_handler` gains a post-parse branch on `grant_type`; `OAuthState` widens
to carry the shared `rusqlite::Connection` handle and an LRU pair cache.
Access-token format and middleware are unchanged. Stdio transport is
untouched. Discovery metadata adds `"refresh_token"` to `grant_types_supported`.

A small env-plumbing change (`MCP_JWT_TTL_SECS` via `OnceLock<u64>`) makes the
JWT TTL overridable for the pre-ship R1 empirical gate (dev deploy with
TTL=60s + Cursor/Claude.ai parallel observation). All migrations are
idempotent `CREATE TABLE IF NOT EXISTS` (rolling-deploy safe). Rollback is
revert-the-tag with no data migration to undo.

## Architecture

### What we're building/modifying

- **`mcp/src/oauth/refresh.rs` (NEW)** — opaque-token store, BEGIN IMMEDIATE
  rotation transaction (with family-revoke inside the same transaction),
  blake3-salted hashing, in-memory LRU **pair cache** holding the full
  `(access_token, refresh_token)` strings for reuse-interval idempotency,
  migration helper, hourly background evictor task.
- **`mcp/src/oauth/mod.rs` (EDIT)** — `OAuthState` widened with the shared
  `Connection`, mandatory salt, reuse-interval and evictor-tick
  `Duration`s, and the pair-cache handle. `TokenRequest` widened so all
  fields are optional and `grant_type` + `refresh_token` are explicit.
  `token_handler` post-parse dispatches on `grant_type` to either the
  existing `authorization_code` path or the new refresh path; unknown
  `grant_type` values return `400 unsupported_grant_type` per RFC 6749
  §5.2. Discovery metadata appends `"refresh_token"` to
  `grant_types_supported`. `JWT_TTL_SECS` const replaced with
  `jwt_ttl_secs()` reading `OnceLock<u64>` seeded from
  `MCP_JWT_TTL_SECS` env var.
- **`mcp/src/main.rs` (EDIT)** — `run_http` validates `MCP_REFRESH_SALT`
  presence + minimum length, seeds the `OnceLock<u64>`, opens the shared
  `Connection`, threads `Arc<Mutex<Connection>>` to `OAuthState::new`,
  calls `refresh::migrate_refresh_tokens`, spawns
  `refresh::start_evictor`.
- **`mcp/tests/_helpers/mod.rs` + `mcp/src/test_support.rs` (EDIT)** —
  `TestServerBuilder::with_oauth_token(bool)` mounts `/oauth/token` in
  the same tower stack as `/mcp`. All fixture helpers
  (`bootstrap_oauth`, `rotate`, `insert_expired_refresh`, fast
  `reuse_interval` setter) live alongside existing `mint_jwt`.
- **`mcp/tests/oauth_refresh_e2e.rs` (NEW)** — 13 integration tests, one
  per AC1–AC13.

### How it works

1. **First login** — client posts `grant_type=authorization_code`.
   Existing PKCE+code flow validates. After the access JWT is minted,
   `refresh::mint_for_authorization_code` generates 32 random bytes,
   base64url-encodes them, computes `blake3(salt + plaintext)`, and
   inserts a `refresh_tokens` row with a fresh `family_id =
   Uuid::new_v4()` and `expires_at = SystemTime::now() + 1y`. Response is
   the 4-field JSON (`access_token`, `token_type:"Bearer"`,
   `expires_in`, `refresh_token`). Both content-types share the path via
   the existing dispatch (`oauth/mod.rs:990-1017`).
2. **Routine use within 1h** — client uses `access_token` as Bearer on
   `/mcp`; bearer middleware unchanged.
3. **Access expiry** — bearer middleware returns `-32001 unauthorized`.
   Client POSTs `grant_type=refresh_token` + the saved plaintext.
4. **Rotation (atomic).** `refresh::rotate(state, plaintext)` runs ONE
   `BEGIN IMMEDIATE` transaction that contains every branch below
   (including family-revoke). Concurrent rotations on sibling rows of
   the same family wait on the SQLite writer lock, so a Branch-C
   detection cannot race with a Branch-A on a sibling.
   - **Branch A** — found, `revoked = 0`, `expires_at > now`: legitimate
     rotation. Compute new plaintext, INSERT new row with same
     `family_id` + `rotated_to` FK pointing back to old; UPDATE old
     (`revoked = 1, rotated_at = now, rotated_to = <new_hash>`); roll
     `expires_at = now + 1y` on the new row. **Insert
     `(access_jwt_string, new_plaintext)` into the LRU pair-cache keyed
     by `old_token_hash` BEFORE `COMMIT`** while Writer-1 still holds
     the SQLite writer lock acquired by `BEGIN IMMEDIATE`. Concretely:
     (i) Writer-1 acquires the `Arc<Mutex<Connection>>` Mutex (project
     existing pattern), (ii) BEGIN IMMEDIATE on the connection, (iii)
     execute UPDATE/INSERT, (iv) `reuse_cache.put(old_hash, pair)` (own
     internal mutex; releases immediately after the put), (v) COMMIT,
     (vi) release the `Connection` Mutex. `LruCache::put` is infallible
     (lru crate API; only side effect is evicting the oldest entry if
     cap reached). The publish ordering is safe by construction: every
     other writer/reader on this row enters via `BEGIN IMMEDIATE` and
     therefore blocks on Writer-1's `Connection` Mutex — and by
     extension on Writer-1's SQLite writer-lock release at step (vi);
     by that point the cache entry was published at step (iv). A
     `debug_hook_cache_put_after_commit` test (D5 unit tests) flips
     the order in a debug build and asserts the race materializes,
     pinning the invariant. Concurrent rotators on **sibling rows of the same
     `family_id`** are blocked by the same writer lock through the
     family-revoke walk in Branch C — see Decision 8. COMMIT.
   - **Branch B** — found, `revoked = 1`, `rotated_at + reuse_interval >
     now`, cache HIT on `old_token_hash`: legitimate network retry
     within reuse window. Return the cached `(access_jwt_string,
     new_plaintext)` pair byte-for-byte identical to what Branch A
     emitted. No DB writes.
   - **Branch B'** — found, `revoked = 1`, `rotated_at + reuse_interval
     > now`, cache MISS (server restart, LRU eviction, or unusually
     slow retry): return `400 invalid_grant` **without** family-revoke.
     This is defensive — a cache miss inside the window could be a
     genuine retry whose Branch A pair is no longer reconstructable, OR
     the start of an attack. Failing closed (no new tokens) without
     family-revoke preserves the other family members for the
     legitimate first caller. Logged at WARN with `family_id` + `sub`
     for forensic follow-up.
   - **Branch C** — found, `revoked = 1`, outside reuse window:
     potential replay. **In the same transaction**, walk `family_id`
     and UPDATE every row in the family to `revoked = 1`
     (`family_revoke`). Drop matching entries from the pair-cache.
     Return `400 invalid_grant`. Logged at WARN with `family_id` +
     `sub`.
   - **Branch D** — found, `revoked = 0`, `expires_at <= now`: expired
     refresh, not an attack. Return `400 invalid_grant` without
     family-revoke. Logged at INFO.
   - **Branch E** — not found: return `400 invalid_grant`. Logged at
     INFO.
5. **Cache eviction.** Pair-cache LRU evicts on capacity (256 entries)
   or TTL (`reuse_interval`). After `reuse_interval` elapses, even a
   cache hit is irrelevant because Branch A/B/C dispatch keys on
   `rotated_at + reuse_interval > now` against the DB row, not the
   cache.
6. **Background DB eviction.** `refresh::start_evictor`, spawned from
   `main.rs` next to `_confirmation_evictor`, sleeps `evictor_tick`
   between sweeps (`tokio::time::sleep(tick).await` per the
   `confirmation_token::start_evictor:259-267` pattern) and runs
   `DELETE FROM refresh_tokens WHERE expires_at + grace < now`.
7. **Discovery.** `/.well-known/oauth-authorization-server`
   `grant_types_supported` now contains `"authorization_code"` AND
   `"refresh_token"`.
8. **Token response headers.** Every `/oauth/token` response (success
   and error) emits `Cache-Control: no-store` + `Pragma: no-cache`
   per RFC 6749 §5.1 so caches don't retain credentials.

### Shared resources

| Resource | Owner (creates) | Consumers | Instance count |
|----------|----------------|-----------|----------------|
| `Arc<Mutex<rusqlite::Connection>>` (shared SQLite handle) | `main.rs::run_http` | `McpState.store`, `OAuthState.refresh_store`, refresh evictor task | 1 (singleton — same file as attestations) |
| `OnceLock<u64>` JWT TTL | `main.rs::run_http` (seeds from `MCP_JWT_TTL_SECS` env, clamped to [60, 604800], default 3600) | `oauth/mod.rs::jwt_ttl_secs()` callers (6 read sites: 4 in `oauth/mod.rs:391, 1075, 1113, 1124` + 2 in `escrow.rs:511, 797`; the const declaration at `oauth/mod.rs:58` and the `use` import at `escrow.rs:59` are not read sites) | 1 (process-global) |
| Pair-cache (`refresh::ReuseCache`) holding `(String, String) = (access_jwt, refresh_plaintext)` | `OAuthState::new` | `refresh::rotate` Branch A (put) + Branch B (get) | 1 per server, cap configurable via `OAuthState.reuse_cache_cap` (default 256 entries), TTL = `reuse_interval` |
| Refresh-token salt (`Vec<u8>`, ≥32 bytes) | `OAuthState::new` (reads `MCP_REFRESH_SALT` env; **mandatory in hosted mode** — server aborts boot if absent or shorter than 32 bytes) | `refresh::hash_token` | 1 per deploy |

## Decisions

### Decision 1: Standard OAuth 2.1 refresh-token rotation, Stripe-precedent timing (1h access + 1y rolling refresh)
**Decision:** Implement OAuth 2.1 `grant_type=refresh_token`. Access TTL stays
1h. Refresh TTL is 1 year, rolling on every use.
**Rationale:** Stripe's hosted MCP server runs the same OAuth model and the
same MCP-host clients (Cursor, VS Code, Claude.ai) silently rotate without
UX impact.
**Alternatives considered:**
- Bump JWT TTL to 24h/7d — postpones but doesn't fix; widens revocation blast radius.
- Per-request signing — proper long-term, breaking-change for every client; parked at `work/stateless-auth-rearch/`.
**User-spec anchor:** Что делаем + Зачем; AC1–AC9 baseline.

### Decision 2: Opaque refresh tokens, blake3(salt+plaintext) at rest, salt is a mandatory deploy secret
**Decision:** Refresh tokens are 32 random bytes, base64url for the wire.
Stored as `blake3(salt + plaintext)`. Plaintext leaves the server once.
`MCP_REFRESH_SALT` is a **mandatory** env var on hosted deploys —
server aborts boot with a clear error if absent or under 32 bytes
**after base64url decoding** (this guards against operators setting
a 32-character ASCII string with low entropy — `~5` bytes of effective
entropy — which would pass a raw-byte length check). `.env.example`
ships a one-liner that mandates `openssl rand -base64 32` as the
generator. **No fallback derived from `MCP_JWT_SECRET`.**
**Rationale:** Stripe/Auth0 standard for opaque tokens. Blake3 matches the
`payment.rs:737-744` precedent — one hashing primitive across the auth
surface. A deterministic salt fallback would couple two secrets: leak of
`MCP_JWT_SECRET` would also expose the salt and enable precomputed rainbow
tables against stored hashes. Mandatory random salt closes the hole.
**Alternatives considered:**
- JWT-as-refresh-token — revocation still needs a blacklist; gains nothing.
- SHA-256 — splits the codebase across two hash primitives.
- Generate-and-persist salt at first boot — works, but operationally
  brittle (where does it live? what if filesystem is read-only?).
  Mandatory env is simpler.
**User-spec anchor:** D1, D3.

### Decision 3: Per-grant UUID `family_id` (multi-device isolation)
**Decision:** Each `authorization_code` exchange mints
`family_id = Uuid::new_v4()`. A user logged in from two browsers has two
independent families.
**Rationale:** Stripe/Auth0 standard. Sub-bound `family_id = sub` would
brick all sessions on a single leak.
**User-spec anchor:** D13.1.

### Decision 4: 5-second reuse-interval (Auth0 default) for retry-vs-replay distinction
**Decision:** `reuse_interval = 5s`. Inside window the revoked refresh
returns the cached pair; outside, it triggers family-revoke.
**Rationale:** Auth0 default. 5s covers every realistic network retry
while keeping the replay-attack window short. Okta's 30s default was too
wide.
**User-spec anchor:** D13, AC3, AC4, AC12.

### Decision 5: In-memory LRU **pair** cache for reuse-interval idempotency `[TECHNICAL]`
**Decision:** `refresh::ReuseCache` keyed by the **old** token hash, stores
the **complete** `(access_jwt_string, new_refresh_plaintext)` pair for
`reuse_interval`. Cap 256 entries. **Branch A puts the cache entry
BEFORE the SQL `COMMIT`**; readers in Branch B that see `revoked=1` after
`COMMIT` are therefore guaranteed to find the cache entry. Branch B'
(cache miss inside window) returns `400 invalid_grant` without
family-revoke as a defensive fail-closed.
**Rationale:** `[TECHNICAL]` JWT minting is non-deterministic (`jti =
Uuid::new_v4()`, `iat = now`) so the access JWT cannot be reconstructed.
Caching only the refresh plaintext (early-draft proposal) would make AC12
fail — the two `tokio::join!` responses would carry different
access_tokens. The cache must hold the complete pair. The put-before-COMMIT
ordering closes the CWE-362 race that would have allowed a retry to see
`revoked=1` before the cache was populated, falling through to
family-revoke.
**Alternatives considered:**
- Persist plaintext in DB — defeats hash-at-rest if DB leaks.
- Cache only access_token, re-mint refresh — same idempotency hole.
- Encrypt-at-rest plaintext with deploy key — extra surface.
- Cache-after-COMMIT — re-introduces the CWE-362 race because Writer 2
  could observe `revoked=1` from SQL before Writer 1 publishes to cache.
- Persist cache pair in SQLite (write the pair into the new row) —
  defeats hash-at-rest if the DB leaks.
- Per-token mutex outside SQLite — duplicates what `BEGIN IMMEDIATE`
  already gives us via the writer lock.
**User-spec anchor:** `[TECHNICAL]` — required for AC12 under D2 hash-at-rest.

### Decision 6: `OAuthState` holds `Arc<Mutex<rusqlite::Connection>>` (not `SqliteStore`)
**Decision:** `OAuthState::new` accepts `Arc<Mutex<Connection>>` directly;
shared with `McpState.store`.
**Rationale:** Matches `escrow.rs:113` and `oauth/google.rs:340` precedent.
Avoids a new wrapper type.
**User-spec anchor:** D10.

### Decision 7: Access-token format unchanged (JWT HS256, 1h TTL) — accepts no-global-logout trade-off
**Decision:** Access tokens remain HS256 JWTs. `Claims` and
`bearer_auth_middleware` untouched.
**Rationale:** Constrains scope. Moving access tokens to opaque
introspection would be a much larger change without addressing the bug.
The no-global-logout limitation is R7 and accepted for V1.
**Alternatives considered:**
- Opaque access tokens — large rewrite.
- Shorter access TTL — marginal security gain; defer until R1 verification shows we need it.
**User-spec anchor:** D7, R7.

### Decision 8: One `BEGIN IMMEDIATE` transaction covers detection AND family-revoke
**Decision:** Rotation runs in a **single** `BEGIN IMMEDIATE` transaction
that contains the SELECT, the branch decision, AND the family-revoke
UPDATE (Branch C). No second transaction for the revoke step.
**Rationale:** `payment.rs:478-505` canonical pattern. Holding the writer
lock through family-revoke serializes any concurrent rotation on a
sibling row in the same family — a Branch C cannot race with a
concurrent Branch A on a sibling. Releasing the lock between detection
and revoke would allow the race.
**Alternatives considered:**
- Two transactions (detect → COMMIT → revoke) — race on siblings.
- Application-level family lock outside SQLite — duplicates what
  `BEGIN IMMEDIATE` already gives us.
- `spawn_blocking` — no upside; rotation is fast, synchronous.
**User-spec anchor:** D11, AC4, AC12.

### Decision 9: Hourly background evictor only
**Decision:** `refresh::start_evictor` sleeps `evictor_tick` (default 1h)
between `DELETE FROM refresh_tokens WHERE expires_at + grace < now`. Uses
`tokio::time::sleep(tick).await` in a loop — per the actual pattern in
`confirmation_token.rs:259-267` (NOT `tokio::time::interval`).
**Rationale:** 1y TTL means hourly is more than enough; opportunistic
in-transaction cleanup adds complexity without observable benefit.
**User-spec anchor:** D12.

### Decision 10: Dual content-type parity via existing dispatch
**Decision:** `grant_type=refresh_token` flows through the same
content-type dispatch as `authorization_code` (`oauth/mod.rs:990-1017`).
`TokenRequest` widens all fields to `Option<String>`; post-parse
validation gates each branch.
**Rationale:** Per `oauth/mod.rs:975-977`: VS Code + Claude.ai send form;
Cursor sends JSON. The existing dispatch handles both.
**Alternatives considered:**
- Two separate handlers — would duplicate the dispatch.
- Pre-peek `grant_type` — would force two-pass parsing.
**User-spec anchor:** D14, AC10.

### Decision 11: RFC 6749 §5.2 error codes for the new branch
**Decision:** Three distinct error codes on `/oauth/token`:
- `invalid_request` — missing or empty required field (e.g.,
  `grant_type=refresh_token` with no `refresh_token` field).
- `invalid_grant` — token unknown / expired / revoked / outside reuse
  window.
- `unsupported_grant_type` — `grant_type` not in
  `{"authorization_code", "refresh_token"}`. **Closes a fallthrough
  vector where unknown grant types would otherwise silently re-enter
  the authorization_code path.**

All emitted via the existing `oauth_error` builder
(`oauth/mod.rs:1158`).
**Rationale:** RFC 6749 §5.2 explicit codes; clients can distinguish
client-side vs server-side rejection without parsing prose.
**User-spec anchor:** AC13.

### Decision 12: `JWT_TTL_SECS` via `OnceLock<u64>` + `MCP_JWT_TTL_SECS` env override
**Decision:** Replace `pub const JWT_TTL_SECS: u64 = 3600` with
`pub fn jwt_ttl_secs() -> u64` reading `std::sync::OnceLock<u64>` seeded
once in `run_http` from env (fallback 3600). Seed clamps the value to
`[60, 604800]` (1 minute to 7 days) — outside-range values clamp + log
WARN. **Parse failures (e.g. `MCP_JWT_TTL_SECS=notanumber` or empty)
log WARN and fall back to 3600** — explicit non-silent behaviour so
a deploy-typo on Task 10 doesn't produce a 1h gate mistaken for a 60s
one. All 6 production read sites switch to the function.
**Rationale:** R1 verification (Option B) requires deploying to
`mcp.dev.mnemonik.xyz` with `MCP_JWT_TTL_SECS=60` for 2-minute
observation. OnceLock keeps the change surgical — every reader uses a
function call instead of a constant, no `OAuthState` field-threading is
needed at any reader site, and the single-source pattern matches the
`confirmation_token::DEFAULT_TTL` style. `OAuthState` IS in scope at
`escrow.rs:797` (`mint_extension_jwt` accepts `oauth: &OAuthState`), so
field-threading was technically feasible — the OnceLock was picked for
the additional reason that it gives a single seed point at startup that
the operator controls via env, matching how every other deploy knob is
exposed. **Note on R1 jitter:** combining the 60s clamped TTL with the
5s `reuse_interval` is intentional — Cursor and Claude.ai are expected
to refresh many times during the 2-minute observation window. False
positives (a sluggish client misses the 5s window after expiry) are
acceptable as "client did not silently refresh" signals for the gate.
**Alternatives considered:**
- Field on `OAuthState` — works, but seven call-site signature changes
  and forced state-threading for what is conceptually a process-wide
  constant.
- Dev-only patch on the const — fragile.
**User-spec anchor:** R1 Option B prerequisite.

### Decision 13: `reuse_interval` and `evictor_tick` as `OAuthState` fields (test-overridable) `[TECHNICAL]`
**Decision:** Both timing constants are `Duration` fields on `OAuthState`,
defaulting to `Duration::from_secs(5)` and `Duration::from_secs(3600)`.
`TestServerBuilder` exposes setters so integration tests can configure
`reuse_interval = Duration::from_millis(100)` and `evictor_tick =
Duration::from_millis(50)`.
**Rationale:** `[TECHNICAL]` Without this, tests for AC3/AC4/AC12 and the
evictor would need real `tokio::time::sleep(Duration::from_secs(6))` —
flaky and slow. Matches `ConfirmationLedger::with_config`
(`confirmation_token.rs:96-105`).
**Alternatives considered:**
- `tokio::time::pause` + `advance` — adds a `tokio_test` dependency the
  rest of the suite avoids.
- `cfg(test)` constants — would diverge prod and test code paths.

### Decision 14: Logging policy — never log plaintext, never log token_hash, log forensic fields per branch
**Decision:** All `tracing` calls in `refresh.rs` and the refresh-branch
of `token_handler` log `outcome` plus a branch-appropriate subset of
forensic fields:
- Branches A, B, C, D (token resolved to a row): `family_id` + `sub`
  + `outcome` + `remote_addr` + `request_id`.
- Branch B' (cache miss inside window) + Branch E (unknown token):
  `outcome` + `remote_addr` + `request_id` + length-prefix of presented
  refresh (first 8 chars of plaintext SHA256 stem — collision-resistant
  but does NOT match the at-rest hash because the stored hash uses the
  refresh-salt blake3 keyed-mode; useful only for correlating
  log-vs-client-side debugging).
- All branches NEVER log: `token_hash`, full plaintext, JWT
  `access_token`, salt bytes.
Branches C, D, E log at WARN (C) / INFO (D, E) so log-volume alarms
can distinguish potential abuse (C) from operational expiry (D, E).
**Rationale:** `token_hash` is the credential-at-rest under D2 — logging
it is equivalent to logging the credential. Branch E with no
`family_id`/`sub` (token unknown) is the credential-stuffing detection
surface; without `remote_addr` + `request_id` it would be a blind
spot (CWE-778). The optional SHA256-stem is a one-way digest distinct
from the at-rest hash so it cannot be cross-correlated to confirm
hash-at-rest values.
**Alternatives considered:**
- Log `token_hash` for forensics — equivalent to logging the
  credential.
- Log only `outcome` (no forensic fields) — Branch E becomes blind to
  credential-stuffing.
- Log full plaintext on Branch E only — round-trip leaking; rejected
  even on internal logs.
**User-spec anchor:** D14 (operational), security audit findings M11 +
m4 (CWE-778).

### Decision 15: `Cache-Control: no-store` + `Pragma: no-cache` on every `/oauth/token` response
**Decision:** Every response from `token_handler` — success **and**
error alike — sets `Cache-Control: no-store` and `Pragma: no-cache`
headers.
**Rationale:** RFC 6749 §5.1 explicit requirement. Prevents intermediate
caches (CDNs, browser caches, proxies) from retaining tokens. Error
responses included because RFC 6749 §5.2 error responses may still
disclose grant state (e.g. `unsupported_grant_type` reveals what
grants the server accepts) that an intermediate cache should not
retain across users.
**Alternatives considered:**
- Headers only on success responses — round-1 security finding noted
  errors can carry sensitive state too.
- `Vary: Authorization` instead — does not prevent storage, only
  varies the cache key; misses the intent.
**User-spec anchor:** `[TECHNICAL]` security audit finding M14.

### Decision 16: Refresh-token field max length 4 KiB on the wire
**Decision:** The refresh-grant path rejects `refresh_token` values
longer than 4 KiB with `400 invalid_request` BEFORE any hashing or DB
work. Legitimate refresh tokens are 43 bytes base64url-encoded; 4 KiB is
~100× the legitimate length and well below the 1 MiB body cap.
**Rationale:** Without an early length cap an attacker can POST a giant
`refresh_token` and amplify the shared `Mutex<Connection>` contention
(CWE-400). The 1 MiB body cap (`MAX_PEEK_BODY` at `oauth/mod.rs:1307`)
is too coarse — by the time it fires we've already allocated and
parsed. A field-level cap short-circuits at parse time. Per-IP rate
limiting on `/oauth/*` is already in place via `tower_governor` (see
`architecture.md:65`); the field cap closes the per-request size
vector that the rate limiter doesn't address.
**Alternatives considered:**
- Use the 1 MiB body cap alone — allocations already happened.
- Cap at 256 bytes (closer to legitimate size) — too aggressive; some
  OAuth clients embed metadata in refresh tokens up to a few hundred
  bytes; 4 KiB gives headroom without enabling abuse.
- No cap, rely on `tower_governor` — rate limiter throttles per IP;
  does not address per-request size amplification.
**User-spec anchor:** `[TECHNICAL]` security audit finding M12.

## Data Models

### `refresh_tokens` table

```sql
CREATE TABLE IF NOT EXISTS refresh_tokens (
    token_hash   TEXT    PRIMARY KEY,           -- blake3(salt + plaintext) hex
    sub          TEXT    NOT NULL,
    google_sub   TEXT,
    issued_at    INTEGER NOT NULL,              -- unix seconds (SystemTime::now())
    expires_at   INTEGER NOT NULL,              -- rolling 1y from last rotation
    revoked      INTEGER NOT NULL DEFAULT 0,
    rotated_at   INTEGER,                       -- unix seconds when revoked (for reuse-interval)
    rotated_to   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL,
    family_id    TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS refresh_tokens_family_idx
    ON refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS refresh_tokens_expires_idx
    ON refresh_tokens(expires_at);
CREATE INDEX IF NOT EXISTS refresh_tokens_sub_idx
    ON refresh_tokens(sub);
```

Migration via `mcp/src/oauth/refresh.rs::migrate_refresh_tokens`, modeled
on `escrow.rs:113-133` (`execute_batch` + `MIGRATION_SQL` const). Single
transaction; SQLite all-or-nothing rollback semantics ensure no partial
schema on failure.

### `TokenRequest` widened

```rust
#[derive(Deserialize)]
struct TokenRequest {
    grant_type:    Option<String>,
    code:          Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    client_id:     Option<String>,
    redirect_uri:  Option<String>,
}
```

Post-parse validation:
- `grant_type` absent or `"authorization_code"` → existing path
  requires `code` + `code_verifier` (else `400 invalid_request`).
- `grant_type == "refresh_token"` → require `refresh_token` non-empty
  AND length ≤ 4096 (else `400 invalid_request`).
- Any other `grant_type` → `400 unsupported_grant_type` (D11).

### `/oauth/token` success response

```json
{
  "access_token": "<JWT>",
  "token_type": "Bearer",
  "expires_in": 3600,
  "refresh_token": "<32-byte base64url>"
}
```

Headers: `Cache-Control: no-store`, `Pragma: no-cache` (D15).

### `OAuthState` additions

```rust
pub struct OAuthState {
    // ... existing fields ...
    pub refresh_store: Arc<Mutex<rusqlite::Connection>>,
    pub refresh_salt:  Vec<u8>,                 // ≥32 bytes, validated at boot
    pub reuse_interval: std::time::Duration,
    pub evictor_tick:   std::time::Duration,
    pub reuse_cache:    Arc<refresh::ReuseCache>,  // holds (String, String)
}
```

## Dependencies

### New packages
- `tracing-test = "0.2"` as `[dev-dependencies]` only — required by the
  `logging_policy_no_plaintext_no_hash_across_branches` unit test to
  capture `tracing` output during D14 verification. Not linked into
  the production binary. Alternative if `tracing-test` is undesirable
  in the dev tree: use `tracing_subscriber::fmt::layer().with_writer(
  buffer)` with an existing `tracing_subscriber` dep — chosen NOT to
  in favor of the more ergonomic `traced_test` macro.

### Using existing (from project)
- `rusqlite` — same `Connection` handle as `McpState.store`.
- `blake3` — `payment.rs:737-744` precedent.
- `lru = "=0.12.5"` — already present at `mcp/Cargo.toml:51` and used by
  5 modules including `oauth/mod.rs:46`. Direct import, no Cargo.toml
  change.
- `uuid` — already used for `Claims.jti`; reuse for `family_id`.
- `serde_urlencoded` + `serde_json` — existing dispatch handles both
  content-types.
- `tokio` — existing runtime. Evictor uses `tokio::time::sleep(tick)
  .await` in a loop, mirroring the actual code at
  `confirmation_token.rs:259-267`.
- `tracing` — log per D14.
- `std::sync::OnceLock` (stable since Rust 1.70) for the JWT TTL knob.
- `std::time::SystemTime::now()` for clock reads (project precedent).

## Testing Strategy

**Feature size:** M

### Unit tests
Located alongside implementation files; run with `cargo test -p mnemonic-mcp`.

- `refresh::tests::rotate_branch_a_writes_row_and_caches_pair_before_commit`
  — Branch A ordering: cache contains entry after rotation, even if
  caller crashes immediately after.
- `refresh::tests::reuse_within_window_returns_byte_identical_pair`
  — Branch B: same `(access_jwt, refresh)` strings.
- `refresh::tests::reuse_cache_miss_inside_window_returns_invalid_grant_without_family_revoke`
  — Branch B' fail-closed.
- `refresh::tests::replay_outside_window_revokes_full_family_in_one_tx`
  — Branch C atomicity: spawn parallel rotation on sibling row, assert
  it is blocked / fails.
- `refresh::tests::expired_refresh_rejected_without_family_revoke`
  — Branch D.
- `refresh::tests::unknown_token_returns_invalid_grant`
  — Branch E.
- `refresh::tests::cross_family_isolation`
  — Two families for same `sub`, family-revoke on one leaves the other
  intact.
- `refresh::tests::migration_is_idempotent`
  — Run `migrate_refresh_tokens` twice on the same connection; second
  call is a no-op.
- `refresh::tests::evictor_evicts_expired_rows`
  — Seed expired + live rows, run one tick (configurable tick), assert
  expired are gone and live remain.
- `refresh::tests::lru_cap_evicts_oldest`
  — Insert 300 entries, assert 256 cap holds and oldest 44 are gone.
- `refresh::tests::lru_ttl_expires_within_window`
  — Set 50ms TTL, insert, sleep 75ms, assert miss.
- `refresh::tests::family_revoke_drops_matching_cache_entries`
  — Branch C cleans pair-cache, not just DB.
- `oauth::tests::missing_refresh_token_field_returns_invalid_request`
  — AC13.
- `oauth::tests::unknown_grant_type_returns_unsupported_grant_type`
  — RFC 6749 §5.2 / D11.
- `oauth::tests::refresh_token_too_long_returns_invalid_request`
  — D16 length cap.
- `oauth::tests::salt_missing_aborts_boot`
  — Boot harness with `MCP_REFRESH_SALT` unset → boot fails.
- `oauth::tests::salt_under_32_bytes_aborts_boot`
  — D2 minimum length.
- `oauth::tests::jwt_ttl_seeded_from_env_and_clamped`
  — Out-of-range values (10s, 9999999s) clamp to [60, 604800] with a
  WARN log.
- `oauth::tests::jwt_ttl_parse_failure_logs_warn_and_uses_default`
  — `MCP_JWT_TTL_SECS=notanumber` / empty / whitespace each fall back
  to 3600 with a WARN log. Closes the silent-default vector for the R1
  gate (Task 10).
- `oauth::tests::token_response_emits_no_store_cache_headers`
  — D15 (both success and error paths).
- `oauth::tests::logging_policy_no_plaintext_no_hash_across_branches`
  — `tracing_test::traced_test` captures `refresh::rotate` output for
  Branch A, B, B', C, D, E and asserts no log line contains the
  plaintext, the at-rest `token_hash`, the salt, or the full
  `access_token`. Forensic fields (`family_id`, `sub`, `remote_addr`,
  `request_id`) are required to be present where D14 specifies.
- `oauth::tests::reuse_cache_cap_respects_oauth_state_field`
  — `ReuseCache::with_cap(10)` constructed via `OAuthState` field;
  insert 12 entries; assert 10 cap enforced (closes round-2 security
  m6: cap was hardcoded).

(Dropped from earlier draft: `token_request_deserializes_both_grants`
and `mint_and_hash_roundtrip` — they tested `serde_derive` / `blake3`
behavior, not project code. Their coverage is subsumed by the
AC-bound integration tests and the security-relevant tests above.)

### Integration tests
`mcp/tests/oauth_refresh_e2e.rs` (NEW). One function per AC1–AC13, plus
two extras: AC11 "legacy client lifecycle" (a multi-call session where
the client never reads the `refresh_token` field but still uses
`/oauth/token` for fresh codes — must keep working); AC12 "10
parallel rotations" (`tokio::join!` ten copies of the same `rt_X`).
The test helper `rotate(server, refresh) -> (String, String)` returns
**both** the new `access_token` and the new `refresh_token`; AC12
asserts ALL ten responses are byte-identical on BOTH fields (closes
round-2 test-reviewer minor — single-field assertion would silently
pass a regression where the cache stores only refresh and Branch B
re-mints the access JWT). Each test uses
`TestServerBuilder::with_oauth_token(true)` with
`reuse_interval = Duration::from_millis(100)` to keep wall-clock <2s.
AC10 form/JSON parity asserts **structural equality of the response
body across the two formats** — same field set, same lengths,
identical metadata — except for the access_token JWT itself which is
intentionally non-deterministic on **independent** rotations (D5
makes Branch B idempotent on the SAME rotation, not across distinct
ones).

Cross-cutting integration tests:
- `cross_table_mutex_no_starvation` — interleave 50 refresh rotations
  with 50 attestation writes against the shared `Connection`; assert
  both make progress without deadlock or unbounded blocking.
- `db_write_failure_during_rotation_returns_500_and_is_retry_safe`
  — fault inject a SQL error mid-Branch-A (test harness wraps the
  store with a write-failing decorator on first call); assert response
  is `500` AND a retry within reuse window succeeds with the same
  byte-identical pair.

### E2E tests
Single curl-based smoke at `mcp/tests/oauth_refresh_e2e.rs` runs a real
HTTP server end-to-end. Real Claude.ai empirical verification is the
**R1 pre-ship gate** (T11) — not part of CI.

## Agent Verification Plan

**Source:** user-spec.md "Как проверить" + the R1 pre-ship gate.

### Verification approach

Three tiers:

1. **Per-task smoke checks** (each task specifies `Verify-smoke:` or
   `Verify-user:` where applicable — see Implementation Tasks).
2. **Pre-deploy QA** (Task 9): `cargo test --workspace --no-fail-fast`,
   `cargo clippy -- -D warnings`, `cargo fmt -- --check`,
   `gitleaks detect`. All 13 AC integration tests GREEN plus the two
   cross-cutting tests above.
3. **Dev-deploy + R1 empirical gate** (Task 10): `mcp.dev.mnemonik.xyz`
   deployed from a topic branch with `MCP_JWT_TTL_SECS=60` AND a
   randomly-generated `MCP_REFRESH_SALT`. Connect Claude.ai AND Cursor
   in parallel for >2 minutes (multiple expiries). Cursor (known to
   rotate) is the control. Claude.ai is the device under test:
   - **GO**: Claude.ai keeps working silently — promote to prod (Task 11).
   - **NO-GO**: Claude.ai requires re-auth — STOP, do not promote.
     Open issue in `work/refresh-token-rotation/decisions.md` and
     **simultaneously file an Anthropic ticket AND advance
     `work/stateless-auth-rearch/` planning**. Both escalation paths
     are explicit in the Task 10 description so the verifier doesn't
     have to invent them.

### Tools required
- `curl` for the post-deploy discovery smoke.
- `bash` + `journalctl` on the VPS for prod log tailing (Task 12).
- A real Claude.ai session + Cursor session for Task 10 (`Verify-user`).

## Risks

| Risk | Mitigation |
|------|-----------|
| Claude.ai might ignore `refresh_token` (R1) | Task 10 pre-ship gate. NO-GO escalation path documented. |
| `MCP_REFRESH_SALT` rotation invalidates all live refresh tokens | Operational rule — document salt as a deploy secret with the same rotation discipline as `MCP_JWT_SECRET`. |
| Test flakiness on real-time reuse-interval | D13 — `OAuthState` fields let tests use 100ms. |
| Refresh-token leak (logs, client) | D14 logging policy (no plaintext, no hash). Plaintext returned once; HTTPS-only. Family-revoke on out-of-window replay. |
| DB write failure during rotation | SQLite all-or-nothing transaction rollback. Response is `500 internal_error`; client retry within reuse window is safe (Branch B cache hit). Fault-injection test asserts both. |
| `MCP_JWT_SECRET` rotation impact on refresh tokens | Salt is independent (D2). Rotating JWT secret invalidates only the access JWTs; clients can refresh and continue. Document explicitly in deployment.md. |
| Wire-format back-compat | AC11 + integration tests; old clients ignoring the new field continue to work. |
| Cursor/VS Code/Claude.ai content-type drift | AC10 structural-equality assertion. PK `architecture.md` corrected in round 3 commit `7a0065a`. |
| `TokenRequest` widening breaks legacy parse | All fields optional; post-parse validation gates each branch. Test `oauth::tests::missing_*_returns_invalid_request` cover both paths. |
| LRU cache lost on restart kills in-flight retries | Branch B' returns `400` without family-revoke. Acceptable — reuse window is 5s; server restart already kills active connections. |
| Refresh-token field DoS via oversized POST | D16 — 4 KiB length cap rejected at parse before hashing or DB work. |
| `MCP_JWT_TTL_SECS` operator footgun (e.g., setting 1s) | D12 — value is clamped to `[60, 604800]` at seed time; out-of-range logs WARN and applies clamp. |
| Rolling deploy with refresh-evictor double-spawn | `start_evictor` spawned once from `main.rs`; rolling deploy replaces the process. |
| Cross-table mutex contention (refresh + attestations + escrow) | `Connection` is the project's existing shared mutex pattern — no new contention surface. `cross_table_mutex_no_starvation` integration test asserts no starvation under interleaved load. |
| Unauthenticated DoS on `/oauth/token` | Existing `tower_governor` per-IP rate limiter applies to `/oauth/*` (per `architecture.md:65`); refresh-grant inherits the limiter, no new wiring. D16 caps the request body field size as a complementary per-request defence. |
| Salt entropy footgun (32-char ASCII passes 32-byte raw length check) | D2 requires base64url-decode of `MCP_REFRESH_SALT` to yield ≥32 bytes; `.env.example` ships `openssl rand -base64 32` as the recipe. Boot test `oauth::tests::salt_under_32_bytes_aborts_boot` enforces. |
| Branch E INFO log volume under credential-stuffing | `tower_governor` throttles but does not block repeated attacker attempts; Branch E logs INFO once per attempt with `remote_addr` + `request_id`. Volume scales linearly with attempt rate. Mitigation: operational rule to set tracing-layer rate-limited dedup (e.g. `tracing-subscriber` + `rate-limiter`) at the operator level — not in V1 server code. Documented in `deployment.md` as a tracing-volume tuning note. |


## User-Spec Deviations

Two new operational env vars (`MCP_REFRESH_SALT`, `MCP_JWT_TTL_SECS`) and a
new mandatory boot validation are introduced — both are **implementation
mechanisms required to honor user-spec R1, AC3, AC4, AC12 under D2 hash-at-rest
security**. They do not add any user-facing capability beyond what user-spec
requires.

- **Added: `MCP_REFRESH_SALT` mandatory env var.** User-spec D2 ("opaque
  random bytes + hash at rest") is implementable only with a real
  random salt; the round-1 draft's deterministic
  `blake3(MCP_JWT_SECRET + "refresh")` fallback was unsafe (single
  secret leak → rainbow-table feasibility). New deploys must set a
  random 32+ byte salt; server aborts boot if absent. Documented in
  `.env.example` and `deployment.md`. **[PENDING USER APPROVAL]** —
  user-spec approval was granted on the assumption that no new env vars
  were added; flagging explicitly for visibility.
- **Added: `MCP_JWT_TTL_SECS` optional env var with clamp + parse-failure
  fallback.** Required prerequisite for the user-spec R1 Option B
  empirical gate; the user-spec's R1 footnote (`user-spec.md`
  line 224-232) explicitly invited this env-plumbing to be resolved
  in tech-spec. No effect in prod (default 3600). Clamp `[60, 604800]`
  prevents operator footgun; parse failures log WARN and fall back to
  3600. Documented in `.env.example` and `deployment.md`.
  **[PENDING USER APPROVAL]** — surfacing for the record so the user
  is aware a new env var has been introduced.

## Acceptance Criteria

Технические критерии приёмки (дополняют пользовательские из user-spec AC1–AC13):

- [ ] `cargo test --workspace --no-fail-fast` зелёный.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` зелёный.
- [ ] `cargo fmt --all -- --check` зелёный.
- [ ] `gitleaks detect --no-banner` зелёный.
- [ ] Все 13 user-spec ACs (AC1–AC13) покрыты интеграционными тестами
      в `oauth_refresh_e2e.rs`.
- [ ] Discovery `/.well-known/oauth-authorization-server` после деплоя
      возвращает `grant_types_supported` содержащий `"refresh_token"`
      (AC7).
- [ ] `tracing` логи на проде содержат `refresh_rotation` (INFO) и
      `family_revoke` (WARN) события — БЕЗ plaintext и БЕЗ token_hash
      (D14).
- [ ] Каждый `/oauth/token` response (success И error) несёт
      `Cache-Control: no-store` + `Pragma: no-cache` (D15).
- [ ] Боот сервера с пустым или коротким `MCP_REFRESH_SALT` (после
      base64url decode) падает с понятной ошибкой (D2 +
      `oauth::tests::salt_*_aborts_boot`).
- [ ] `refresh_token` поле длиннее 4 KiB на refresh-grant возвращает
      `400 invalid_request` ДО хеширования / DB запроса (D16 +
      `oauth::tests::refresh_token_too_long_returns_invalid_request`).
- [ ] `/oauth/*` rate-limit активен (`tower_governor` уже стоит на
      этом scope per `architecture.md:65`); no new wiring required by
      this feature. Существующий тест поведения rate-limiter'а не
      затронут.
- [ ] V1 limitation accepted: refresh-grant **не валидирует**
      `client_id` поле против записи в `refresh_tokens`. Допустимо
      потому что server использует public-clients model
      (`token_endpoint_auth_methods_supported: ["none"]` в discovery,
      см. `oauth/mod.rs:1187`). Tightening to per-client binding —
      follow-up feature если confidential clients добавятся.
- [ ] Нет регрессий в `oauth/mod.rs:2022-2099` area +
      `mcp/tests/auth_allowlist.rs` + `mcp/tests/anonymous_recall.rs`.
- [ ] R1 pre-ship gate (Task 10) — Claude.ai продолжает работать с
      `MCP_JWT_TTL_SECS=60` параллельно с Cursor control'ом 2+ минуты.

## Implementation Tasks

### Wave 1 (parallel — disjoint files)

#### Task 1: Refresh-token storage module + migration + evictor
- **Description:** Implement `mcp/src/oauth/refresh.rs` per the
  Architecture and Decisions sections — storage CRUD, BEGIN IMMEDIATE
  rotation transaction with all branches (A, B, B', C, D, E), pair
  LRU, migration, evictor. Wires nothing yet; just the module. **Does
  NOT modify `oauth/mod.rs`** — the `pub mod refresh;` declaration is
  in Task 2 alongside the other `oauth/mod.rs` edits, so Wave 1 has
  zero file collisions.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp refresh::tests` —
  all listed unit tests under Testing Strategy → Unit tests pass.
  (Module is reachable in tests via `mnemonic_mcp::oauth::refresh::*`
  even before the `pub mod refresh;` line is added — Cargo
  auto-discovers the file under `oauth/`, and the module is reachable
  for tests via the test-binary; integration with prod code happens
  in Task 2's `pub mod refresh;` add.)
- **Files to modify:** `mcp/src/oauth/refresh.rs` (new).
- **Files to read:** `mcp/src/escrow.rs`, `mcp/src/payment.rs`,
  `mcp/src/confirmation_token.rs`,
  `work/refresh-token-rotation/code-research.md` §I.3, §I.5, §I.6.

#### Task 2: `JWT_TTL_SECS` env-plumbing via `OnceLock` + `pub mod refresh;`
- **Description:** Replace the constant with a function reading
  `OnceLock<u64>` seeded in `run_http` from `MCP_JWT_TTL_SECS` env
  (clamp + WARN on out-of-range or parse failure; fall back to 3600).
  Switch all 6 production read sites to the new function. Add the env
  var to `.env.example` and the deployment doc. **Also adds the
  `pub mod refresh;` declaration in `oauth/mod.rs`** so Task 1's
  module becomes reachable from prod code (collision-free with
  Task 1 because Task 1 only creates the new file).
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `MCP_JWT_TTL_SECS=60 cargo run -p mnemonic-mcp -- --transport http --port 3000` boots cleanly and `cargo test -p mnemonic-mcp jwt_ttl_seeded_from_env_and_clamped` passes.
- **Files to modify:** `mcp/src/oauth/mod.rs`, `mcp/src/escrow.rs`,
  `mcp/src/main.rs`, `.env.example`,
  `.claude/skills/project-knowledge/references/deployment.md`.
- **Files to read:** `work/refresh-token-rotation/code-research.md` §I.1.

### Wave 2 (depends on Wave 1)

#### Task 3: `OAuthState` + `TokenRequest` + `token_handler` refresh branch + discovery + boot validation
- **Description:** Extend `OAuthState` with the four new fields
  (`refresh_store`, `refresh_salt`, `reuse_interval`, `evictor_tick`,
  `reuse_cache`); widen `TokenRequest` so all fields are optional; add
  the post-parse dispatch in `token_handler` that calls the
  Wave-1 `refresh::rotate` and emits the `Cache-Control: no-store`
  headers; append `"refresh_token"` to the discovery metadata; thread
  the new state through all 4 `OAuthState::new` call sites; gate boot
  on `MCP_REFRESH_SALT` presence and length; spawn `refresh::start_evictor`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** server boots with a valid `MCP_REFRESH_SALT`;
  `curl localhost:3000/.well-known/oauth-authorization-server | jq
  '.grant_types_supported'` contains both grants.
- **Files to modify:** `mcp/src/oauth/mod.rs`, `mcp/src/main.rs`,
  `mcp/src/mcp.rs`, `mcp/tests/_helpers/mod.rs` (thread through only;
  test infrastructure additions are Task 4), `.env.example`,
  `.claude/skills/project-knowledge/references/deployment.md`.
- **Files to read:** `work/refresh-token-rotation/code-research.md`
  §I.2, §I.4, §I.7, `mcp/src/oauth/refresh.rs` (Task 1 output).

### Wave 3 (depends on Wave 2)

#### Task 4: `TestServerBuilder::with_oauth_token` + integration test helpers
- **Description:** Add the `with_oauth_token(bool)` flag and the
  fixture helpers (`bootstrap_oauth`, `rotate` returning `(String,
  String)`, `insert_expired_refresh_for_test`, `with_reuse_interval`,
  `with_evictor_tick`) entirely in this task. No other task touches
  `test_support.rs` afterward.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp --test _helpers_test_server_mounts_oauth_token` (smoke test added in the same task) confirms the new flag mounts the route.
- **Files to modify:** `mcp/tests/_helpers/mod.rs`,
  `mcp/src/test_support.rs`.
- **Files to read:** `work/refresh-token-rotation/code-research.md` §I.8.

### Wave 4 (depends on Wave 3)

#### Task 5: Integration test suite — `oauth_refresh_e2e.rs`
- **Description:** Implement the 13 AC tests + the two cross-cutting
  tests (cross-table mutex no-starvation, DB write failure retry-safe)
  enumerated under Testing Strategy → Integration tests. All tests use
  `with_oauth_token(true)` + fast `reuse_interval` so the suite runs
  in <2s.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-mcp --test oauth_refresh_e2e` — all 15 integration tests (AC1-AC13 + 2 cross-cutting) pass under 2 seconds.
- **Files to modify:** `mcp/tests/oauth_refresh_e2e.rs` (new).
- **Files to read:** `work/refresh-token-rotation/user-spec.md`
  AC1–AC13, `work/refresh-token-rotation/code-research.md` §I.9,
  `mcp/tests/_helpers/mod.rs` (Task 4 output).

### Audit Wave (3 auditors in parallel — reviewers: none)

#### Task 6: Code Audit
- **Description:** Full-feature code-quality audit across all files
  touched by Tasks 1–5. Verify mutex discipline (no lock across
  `.await`), shared-resources compliance (single `Connection`, single
  `OnceLock`, single `ReuseCache`), per-decision rationale alignment
  with implementation, error-handling patterns. Write report to
  `work/refresh-token-rotation/logs/working/code-audit.md`.
- **Skill:** code-reviewing
- **Reviewers:** none

#### Task 7: Security Audit
- **Description:** Full-feature security audit. OWASP Top 10 across
  all components. Specifically verify: salt is mandatory and ≥32
  bytes; no plaintext / token_hash logging; refresh-token field length
  cap enforced; Cache-Control headers on every response; unknown
  `grant_type` returns `unsupported_grant_type`; LRU put precedes
  COMMIT in Branch A; family_revoke is in-transaction. Write report to
  `work/refresh-token-rotation/logs/working/security-audit.md`.
- **Skill:** security-auditor
- **Reviewers:** none

#### Task 8: Test Audit
- **Description:** Full-feature test-quality audit. Each AC1–AC13 has
  ≥1 integration test; D5, D8, D9, R6 each have ≥1 test; no real
  `sleep(5s)` calls; no litmus-failing tests; no regression in
  `oauth/mod.rs:2022-2099`, `auth_allowlist.rs`, `anonymous_recall.rs`.
  Write report to
  `work/refresh-token-rotation/logs/working/test-audit.md`.
- **Skill:** test-master
- **Reviewers:** none

### Final Wave

#### Task 9: Pre-deploy QA
- **Description:** Run the full test suite + clippy + fmt + gitleaks on
  a clean local checkout. Verify every technical AC under
  "Acceptance Criteria" above holds locally. Produce pre-deploy-qa
  report.
- **Skill:** pre-deploy-qa
- **Reviewers:** none

#### Task 10: Dev deploy + R1 empirical gate
- **Description:** Deploy feature branch to `mcp.dev.mnemonik.xyz`
  with `MCP_JWT_TTL_SECS=60` and a freshly generated `MCP_REFRESH_SALT`.
  Connect Claude.ai AND Cursor in parallel for 2+ minutes. GO if both
  keep working silently → proceed to Task 11. NO-GO if Claude.ai
  requires re-auth → STOP, file Anthropic ticket, advance
  `work/stateless-auth-rearch/` planning. Document GO/NO-GO + observed
  POST /oauth/token traffic (when visible) in the task report.
- **Skill:** deploy-pipeline
- **Reviewers:** none
- **Verify-user:** real Claude.ai + Cursor parallel sessions against
  dev MCP; both stay functional across 2+ JWT expiries.

#### Task 11: Prod deploy (gated by Task 10)
- **Description:** Only execute on Task 10 GO. Tag `v0.2.5`, push,
  release.yml builds + ships. Deploy to `mcp.mnemonik.xyz` per the
  `deployment.md::VPS Deploy Process`. Confirm prod `MCP_JWT_TTL_SECS`
  is unset or 3600 and `MCP_REFRESH_SALT` is set to a deploy secret.
- **Skill:** deploy-pipeline
- **Reviewers:** none

#### Task 12: Post-deploy verification
- **Description:** Live verification on `mcp.mnemonik.xyz` v0.2.5:
  - `curl https://mcp.mnemonik.xyz/.well-known/oauth-authorization-server
    | jq '.grant_types_supported'` contains both grants — tool: curl.
  - Real Cursor session open >65 minutes; `mnemonic_whoami`
    responds without OAuth-page prompt — tool: Cursor (user check).
  - Real Claude.ai session open >65 minutes; `mnemonic_sign_memory`
    works after first JWT expiry — tool: Claude.ai (user check).
  - Tail prod logs for `refresh_rotation` (INFO) and `family_revoke`
    (WARN) events with sub + family_id, NO plaintext or token_hash —
    tool: bash + journalctl on VPS.
  Tools required: curl, bash, manual Cursor + Claude.ai.
- **Skill:** post-deploy-qa
- **Reviewers:** none
