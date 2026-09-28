# Security Audit — refresh-token-rotation

**Auditor:** security-auditor (Task 7, Wave 5)
**Date:** 2026-06-07
**Scope:** All files modified by Tasks 1–5 (final state post-round-2 fixes).
**Branch SHA:** c35162c (wave merge commit; all five tasks included)

---

## Executive Summary

**Verdict: PASS (CLEAN — ship-ready)**

| Severity | Count |
|---|---|
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Minor | 2 |
| Informational | 4 |

No exploitable conditions exist in the new surface. Every decision-specific invariant (D2, D5, D8, D11, D14, D15, D16, tower_governor, R7) is verified by file:line citation plus existing test-pinning evidence. OWASP A01–A10 walk closes with no unmitigated threats. The two MINOR findings are pre-existing design trade-offs documented in Task 3/4 decisions; they do not block deploy.

---

## Decision-Specific Invariants

### D2 — Salt mandatory and ≥32 bytes after base64url decode

**Verdict: CLOSED**

- `validate_refresh_salt` at `mcp/src/oauth/mod.rs:1465–1485` decodes with `base64::engine::general_purpose::STANDARD` (standard-padded, matches `openssl rand -base64 32`), then checks `decoded.len() < 32`. The 32-character-ASCII footgun is closed: 32 ASCII chars base64-decode to ~24 raw bytes, which fails the ≥32 check.
- Boot abort at `mcp/src/main.rs:1276–1277`: `validate_refresh_salt(std::env::var("MCP_REFRESH_SALT").ok().as_deref())?` — the `?` propagates to `run_http`'s `Result<()>` return, terminating the boot sequence with an error message that names `MCP_REFRESH_SALT`.
- No fallback from `MCP_JWT_SECRET` exists anywhere in the codebase (confirmed by grep: zero hits on any fallback path).
- Tests: `salt_missing_aborts_boot` (mod.rs:4736) and `salt_under_32_bytes_aborts_boot` (mod.rs:4754) pin both failure modes.

### D14 — No plaintext, token_hash, salt, or full access_token in tracing logs

**Verdict: CLOSED**

Grep output from `grep -n "tracing::" mcp/src/oauth/refresh.rs mcp/src/oauth/mod.rs`:

| Line | Macro | Branch | Fields logged | Verdict |
|---|---|---|---|---|
| refresh.rs:429 | info! | evictor | evicted=n | CLEAN — no credentials |
| refresh.rs:435 | warn! | evictor error | error=%e | CLEAN |
| refresh.rs:489–498 | info! | preflight (empty) | outcome, branch, remote_addr, request_id, stem="empty" | CLEAN — "empty" literal, no token data |
| refresh.rs:644–657 | info! | Branch E | outcome, branch, remote_addr, request_id, stem=sha256_stem(plaintext) | CLEAN — SHA256 stem is a one-way digest distinct from the blake3 at-rest hash |
| refresh.rs:664–672 | info! | Branch D | outcome, branch, family_id, sub, remote_addr, request_id | CLEAN |
| refresh.rs:694–703 | debug! | Branch B | outcome, branch, family_id, sub, remote_addr, request_id | CLEAN |
| refresh.rs:714–723 | warn! | Branch B' | outcome, branch, family_id, sub, remote_addr, request_id, stem=sha256_stem(plaintext) | CLEAN — stem is one-way, not the at-rest hash |
| refresh.rs:736–744 | warn! | Branch C | outcome, branch, family_id, sub, remote_addr, request_id | CLEAN |
| refresh.rs:804–812 | info! | Branch A | outcome, branch, family_id, sub, remote_addr, request_id | CLEAN |

None of the 9 call sites log the plaintext token, the blake3 `token_hash`, the salt bytes, or a full JWT access token. Branch B' and E log `sha256_stem(plaintext)` — the first 8 hex chars of SHA256(plaintext) — which is a one-way digest computationally distinct from the at-rest `blake3(salt||plaintext)` hash (different algorithm, no salt). A log observer cannot derive the at-rest hash from the log stem.

Branches C, D, and E use WARN/INFO levels as documented (CWE-778 mitigation confirmed).

DEVIATION accepted by prior security-auditor-1 (Task 1 round 2): Branch B' additionally logs `family_id` and `sub` (beyond the D14 literal field list of `outcome+addr+request_id+stem`). This is forensically valuable and does not leak credential material. Tech-spec D14 wording should be updated in a follow-up.

### D16 — 4 KiB refresh_token length cap before hashing

**Verdict: CLOSED**

- Check at `mcp/src/oauth/mod.rs:1768`: `if plaintext.len() > REFRESH_TOKEN_MAX_LEN` returns `400 invalid_request` with `oauth_error_typed`.
- This gate fires BEFORE any blake3 hashing, before `Arc<Mutex<Connection>>` acquisition, and before any SQL. The code flow is: parse body → extract `refresh_token` field → length check → (only then) spawn_blocking for rotate.
- Test: `refresh_token_too_long_returns_invalid_request` (mod.rs:4654) asserts the DB row count is still 0 after rejection, proving no DB work occurred.
- Integration test: `test_oversized_refresh_token_rejected_as_invalid_request` (oauth_refresh_e2e.rs:860).

### D15 — Cache-Control: no-store + Pragma: no-cache on every /oauth/token response

**Verdict: CLOSED**

- `apply_no_store_headers` at `mcp/src/oauth/mod.rs:1931–1943` inserts both headers via `h.insert(CACHE_CONTROL, "no-store")` and `h.insert(PRAGMA, "no-cache")`.
- `token_handler` at mod.rs:1519–1526 wraps every exit of `token_handler_inner` unconditionally: `apply_no_store_headers(resp)`. The wrapper is the sole path that returns from `token_handler` — no early-return or error-shortcut escapes it.
- All error paths (`invalid_request`, `invalid_grant`, `unsupported_grant_type`, `500 internal_error`) return from `token_handler_inner` and are wrapped by `token_handler` before reaching the client.
- Tests: `token_response_emits_no_store_cache_headers` (mod.rs:4686) iterates 4 error paths + 1 success path. `assert_no_store_headers` in `oauth_refresh_e2e.rs` anchors it across integration tests.

### D11 — Unknown grant_type returns 400 unsupported_grant_type

**Verdict: CLOSED**

- Dispatch in `token_handler_inner` at mod.rs:1579–1594:
  ```
  match req.grant_type.as_str() {
      "authorization_code" => ...,
      "refresh_token" => ...,
      _ => oauth_error_typed(BAD_REQUEST, "unsupported_grant_type", "..."),
  }
  ```
- The `_` arm catches any value not in `{"authorization_code", "refresh_token"}`, including `"client_credentials"`, `"implicit"`, and empty strings that somehow reach this point.
- The absent-grant-type case: `serde(default = "default_grant_type")` on `TokenRequest.grant_type` (mod.rs:1416) defaults to `"authorization_code"`. A request with no `grant_type` field AND with valid `code`+`code_verifier` will follow the authorization_code path. A request with no `grant_type` AND no `code` will return `400 invalid_request` from the authorization_code handler's non-empty validation. No silent success from an absent grant_type.
- User-supplied `grant_type` value is NOT echoed to the response body (SA3-R1-M1 fix at mod.rs:1587–1594).
- Tests: `unknown_grant_type_returns_unsupported_grant_type` (mod.rs:4632) and `test_unsupported_grant_type_rejected` (oauth_refresh_e2e.rs:898).

### D5 — LRU cache.put precedes COMMIT in Branch A

**Verdict: CLOSED**

- Ordering in `rotate` (refresh.rs:539–572):
  1. `rotate_inner` performs BEGIN IMMEDIATE, SELECT, INSERT, UPDATE (writes complete inside transaction).
  2. Branch A returns `RotateInnerOk { pending_publish: Some(p), did_writes: true }`.
  3. Outer `rotate`: if `publish_before_commit` (= `!debug_hook_cache_put_after_commit()`, always `true` in production/release builds) → `cache.put(...)` at refresh.rs:541–549.
  4. Then `conn.execute("COMMIT", [])` at refresh.rs:556.
  5. Only after COMMIT does the function return.

  The production path is unambiguously: writes → cache.put → COMMIT.

- The `debug_hook_cache_put_after_commit` hook exists at refresh.rs:859–875. It is `#[cfg(test)]`-gated — always returns `false` in non-test builds. The thread-local `DEBUG_HOOK_CACHE_PUT_AFTER_COMMIT` cell at refresh.rs:860 and the `BRANCH_A_RACE_OBSERVER` at refresh.rs:890 are both test-only.

- Test: `rotate_branch_a_writes_row_and_caches_pair_before_commit` (refresh.rs:969) explicitly flips the hook, installs a race observer that fires between COMMIT and put, and asserts both that the observer fires AND that at observer-time the cache has a miss. This pins the CWE-362 race window. Production path is also asserted (cache HIT immediately after rotate returns).

### D8 — family_revoke runs inside the same BEGIN IMMEDIATE transaction as replay detection

**Verdict: CLOSED**

- In `rotate_inner` (refresh.rs:602–751): the function begins after `conn.execute("BEGIN IMMEDIATE", [])` in the outer `rotate` (refresh.rs:508). All branch logic including the Branch C path at lines 733–750 runs inside the same transaction scope.
- Branch C sequence (refresh.rs:733–750): `family_revoke(conn, &row.family_id)?` at line 734, followed by `cache.remove_by_family(&row.family_id)` at line 735, then returns `RotateInnerOk { did_writes: true, ... }`. The outer `rotate` then does `COMMIT` at refresh.rs:556.
- There is NO intermediate COMMIT between the `revoked == true` detection (SELECT result at line 661) and the `family_revoke` UPDATE (line 734). The detection and revoke are a single atomic unit under the writer lock.
- Test: `replay_outside_window_revokes_full_family_in_one_tx` (refresh.rs:1166) uses three siblings; confirms all 3 are revoked after a single rotate call AND that a subsequent spawn_blocking on a sibling sees `revoked=1`.
- Cache entries for the revoked family are also dropped (Branch C calls `cache.remove_by_family`) — confirmed by assertion in the same test at lines 1249–1260.

### tower_governor per-IP rate limiter still active on /oauth/*

**Verdict: CLOSED**

- `oauth_governor_conf` is built at `mcp/src/main.rs:1333–1340` (1 req/s refill, 5 burst in production; `OAUTH_RATELIMIT_DISABLE=1` relaxes to 100/s × 1000 burst for test runs only).
- `/oauth/token` is part of `oauth_routes` (main.rs:1484–1498), which has `.layer(GovernorLayer { config: oauth_governor_conf.clone() })` applied at lines 1495–1497.
- `oauth_routes` is merged into the main router at main.rs:1712: `.merge(oauth_routes)`.
- No bypass route exists. The only widening is via the `OAUTH_RATELIMIT_DISABLE=1` env var (main.rs:1327), which emits a WARN log and is explicitly for test/dev use. A deploy with this env var set in production is a misconfiguration the WARN log makes visible.
- The `tower_governor` import confirms the crate is in the production dependency tree (not a dev-only dep).

### R7 — No-global-logout trade-off documented, not silently introduced

**Verdict: CLOSED**

- Decision D7 (mod.rs comments and decisions.md) explicitly documents that access-token JWTs cannot be revoked once issued (JWT format unchanged — HS256, 1h TTL).
- No `/oauth/revoke` endpoint exists anywhere in the codebase (grep confirms zero hits for `"revoke"` as a route path).
- No code path falsely advertises a logout/revoke capability. The family-revoke in Branch C revokes REFRESH tokens only; the associated access tokens issued before the revoke remain valid until their 1h JWT `exp`. This trade-off is called out explicitly in D16 + R7 in user-spec.md.
- Tech-spec and user-spec both name D7 / R7 explicitly.

---

## OWASP Top 10 Walk

### A01 — Broken Access Control
**CLOSED**

- `rotate` does not accept a `sub` parameter from the caller — `sub` is read from the DB row identified by the presented `token_hash`. One `sub` cannot rotate another `sub`'s token.
- `family_id` isolation: each `authorization_code` exchange mints a fresh UUID family (D13.1). `family_revoke` at refresh.rs:386–396 filters by `WHERE family_id = ?1` — it cannot cross family boundaries regardless of caller-supplied input.
- Client_id is not validated against the refresh row (V1 limitation). This is acceptable under the public-clients model (`token_endpoint_auth_methods_supported: ["none"]`) and is documented in the tech-spec.

### A02 — Cryptographic Failures
**CLOSED**

- Plaintext is 32 random bytes from `rand::thread_rng().fill_bytes(&mut raw)` (refresh.rs:226). The `rand` 0.8 crate on this platform (Linux x86_64) uses the OS CSPRNG (`getrandom` / `urandom`). Not `rand::random()` (which uses a seeded PRNG) — the explicit `fill_bytes` call on `thread_rng` is the project-wide pattern.
- At-rest hash: `blake3(salt || plaintext_bytes)` at refresh.rs:214–219. Salt is fed as a streaming update before plaintext bytes — no intermediate heap allocation of the concatenation.
- Salt entropy: validated at ≥32 decoded bytes (D2). A valid 32-byte salt from `openssl rand -base64 32` has 256 bits of entropy.
- JWT HS256 handling: unchanged from pre-feature baseline.
- No plaintext or hash-at-rest leaks in logs (D14, verified above) or response bodies (response body contains only `access_token` JWT and `refresh_token` plaintext — both intentionally surfaced).

### A03 — Injection
**CLOSED**

- Every SQL in `refresh.rs` uses `rusqlite` positional `params![...]` binding. Grep confirms zero `format!` calls in `refresh.rs` outside `#[cfg(test)]`.
- `family_id` values flow from `Uuid::new_v4().to_string()` (UUID, fixed-format, no user input) or from DB row reads (already stored in parameterized form).
- `Cache-Control` and `Pragma` headers are `HeaderValue::from_static` literals — no string interpolation.
- The `oauth_error_typed` builder (mod.rs:2041–2050) uses fixed operator-authored strings for both `error` and `error_description`; no user-supplied value is echoed.

### A04 — Insecure Design
**CLOSED**

- D5 (put-before-COMMIT) closes CWE-362. D8 (single BEGIN IMMEDIATE for detect + revoke) closes the sibling-family race. D13 / reuse-interval closes the retry-vs-replay race. Branch B' fail-closed is intentional for defense-in-depth.

### A05 — Security Misconfiguration
**CLOSED**

- Boot aborts on absent/short salt (D2 — confirmed).
- `MCP_JWT_TTL_SECS` clamped `[60, 604800]` with WARN on any deviation (D12 — confirmed in `compute_jwt_ttl_from_env_str`).
- No debug endpoints exposed. The `OAUTH_RATELIMIT_DISABLE` knob emits a production WARN and is not a debug endpoint.
- No permissive CORS added to `/oauth/token` — the token endpoint sits inside `oauth_routes` which uses the existing predicate-based CORS policy, not a wildcard.

### A06 — Vulnerable / Outdated Components
**CLOSED**

- New production dependencies introduced by Tasks 1–5: `lru 0.12`, `sha2`, `uuid` (already present), `rand` (already present), `base64` (already present), `blake3` (already present). `tracing-test` is `[dev-dependencies]` only.
- No new production dep with known CVEs at audit time. `lru 0.12.5` is a pure data-structure crate with no unsafe code surface relevant to this feature.

### A07 — Identification & Authentication Failures
**CLOSED**

- Refresh tokens are opaque (D1, D2). Family-revoke on out-of-window replay (Branch C, D8). Rotation is single-use within window (D4, D5). Access-token format unchanged (D7).
- No fallback allows a revoked refresh to succeed: Branch B requires a cache HIT; if the cache is empty (server restart / LRU eviction) Branch B' fails closed.

### A08 — Software & Data Integrity Failures
**CLOSED**

- Migration is idempotent (`CREATE TABLE IF NOT EXISTS` — test: `migration_is_idempotent` at refresh.rs:938). Rolling deploy is safe. Rollback = revert-the-tag; no data migration to undo (the `refresh_tokens` table is additive, not replacing any existing table).
- DB write failure during rotation: `rotate` propagates `rusqlite::Error` via `anyhow::Result`; `ROLLBACK` is explicit on every error path (refresh.rs:529, refresh.rs:558–561). The `spawn_blocking` `Err` path (task panic) also maps to 500 at mod.rs:1826–1838.

### A09 — Security Logging & Monitoring Failures
**CLOSED**

- Branch E logs `remote_addr` + `request_id` + SHA256 stem — credential-stuffing detection surface.
- Branches C/D/E use WARN/INFO so log-volume alarms can distinguish abuse from operational expiry.
- The SHA256 stem is a one-way digest distinct from the at-rest hash (different algorithm, no salt) — a log observer cannot derive the stored hash.

### A10 — Server-Side Request Forgery
**CLOSED / NOT APPLICABLE**

- No new external HTTP call introduced in the refresh path. `spawn_blocking` calls only into `refresh::rotate` (rusqlite operations) and `issue_jwt_with_google_sub` (in-process JWT minting). Confirmed by grep over the entire refresh/token handler path.

---

## Specific Grep Results

### `grep -rn "tracing::" mcp/src/oauth/refresh.rs mcp/src/oauth/mod.rs`

See D14 table above. All 9 call sites in `refresh.rs` are documented. Additional call sites in `mod.rs` cover: evictor-sweep (info/warn), seed_jwt_ttl (info/debug), seed_server_origin (info/debug), token_handler_refresh (info/warn for internal errors), handler-side Branch A (info), Branch B (debug), InvalidGrant (info). None log credential material.

### `grep -rn "format!\|format_args!" mcp/src/oauth/refresh.rs`

Only inside `#[cfg(test)] mod tests` (lines 1356–1418) — test-only fixture formatting, never SQL or log messages. Zero production hits.

### `grep -rn "unwrap()\|expect(" mcp/src/oauth/refresh.rs`

All inside `#[cfg(test)] mod tests` starting at line 904. Zero production hits outside test code.

### `grep -rn "MCP_REFRESH_SALT\|MCP_JWT_TTL_SECS" mcp/src/ .env.example .claude/skills/project-knowledge/references/deployment.md`

- `MCP_REFRESH_SALT`: read at exactly one site (`main.rs:1289`, `std::env::var("MCP_REFRESH_SALT")`). Documented in `.env.example:75` and `deployment.md:66`. No second read site anywhere in the codebase.
- `MCP_JWT_TTL_SECS`: read at exactly one site (`mod.rs:103`, `std::env::var("MCP_JWT_TTL_SECS")`). Documented in `.env.example:67` and `deployment.md:65`.

### `grep -rn "tower_governor" mcp/src/`

`main.rs:1252` (import), `main.rs:1315–1340` (two GovernorConfig builds), `main.rs:1371–1373` (/mcp), `main.rs:1495–1497` (/oauth/*), `main.rs:1552–1554`, `main.rs:1572–1574`, `main.rs:1584–1586`, `main.rs:1589–1591` (google routes), `main.rs:1668–1680` (approval router). `/oauth/token` is inside `oauth_routes` at lines 1484–1498, which has GovernorLayer applied before `.with_state`. Confirmed active on `/oauth/token`.

---

## Admin / Backdoor Confirmation

No admin-override, god-key, or hardcoded-pubkey backdoor exists in any of the new code (Tasks 1–5 files). Confirmed by full read of `refresh.rs`, token-handler and related `mod.rs` sections, `main.rs` boot sequence, `test_support.rs`, and `_helpers/mod.rs`.

---

## Findings

### MINOR-1 — `OAuthState::with_defaults` not `#[cfg(test)]`-gated

**Severity:** Minor
**File:line:** `mcp/src/oauth/mod.rs:584` (`with_defaults` constructor)
**Description:** The test-only constructor is publicly accessible in production builds because gating it on `cfg(test)` breaks all 14 integration-test crates (they import via the library facade, which is not compiled under `cfg(test)`). Hardening option: gate on `feature = "test-support"`. Pre-existing Task 3 design decision; not a new finding.
**Threat-model:** An operator could theoretically call `with_defaults` (which uses a fixed `[0xAB; 32]` salt) in production code they write on top of this library. In practice, the production binary always uses `OAuthState::new` (main.rs:1282); the only call to `with_defaults` is in integration test helpers. Not exploitable from outside the process.
**Action:** Deferred to post-V1 hardening. Does not block ship.

### MINOR-2 — `refresh_salt` and `refresh_store` are `pub` fields on `OAuthState`

**Severity:** Minor
**File:line:** `mcp/src/oauth/mod.rs` (OAuthState struct definition)
**Description:** The two fields are `pub` to allow Task 4's `_helpers/mod.rs` to read them for test helper functions (`insert_expired_refresh_for_test`, `family_has_unrevoked_rows`). In a well-encapsulated design they would be private with accessor methods. Pre-existing Task 3/4 design trade-off.
**Threat-model:** No external process can read struct fields at runtime in Rust. The `pub` visibility is a library API surface concern, not an attacker-exploitable vector. Any code in the same binary with access to an `OAuthState` instance could read these fields — but all such code is operator-controlled.
**Action:** Deferred to post-V1 hardening. Does not block ship.

---

## Informational Items

### INFO-1 — Branch B' field-set deviation from D14 literal spec

Branch B' logs `family_id` and `sub` in addition to the D14-specified `outcome+remote_addr+request_id+stem`. This was accepted by security-auditor-1 in Task 1 round 2 as forensically valuable and not a security regression. Tech-spec D14 wording should be updated in a follow-up.

### INFO-2 — In-memory `ReuseCache` is lost on server restart

A restart within the 5-second reuse-interval window will cause in-window Branch B retries to land on Branch B' (fail-closed) instead of Branch B (idempotent). This is the documented trade-off: fail-closed is safe. No remediation required for V1.

### INFO-3 — No per-`client_id` validation on the refresh row

Per V1 public-clients model. `client_id` is not validated against the `refresh_tokens` row on rotation. A bearer of a valid refresh token can rotate regardless of which `client_id` was used at authorization time. Acceptable under `token_endpoint_auth_methods_supported: ["none"]`.

### INFO-4 — `OAUTH_RATELIMIT_DISABLE=1` knob

Exists for e2e test convenience. Emits a production WARN. If accidentally set in production, rate limiting is effectively disabled on `/oauth/*`. Mitigation: the WARN log is observable by any log monitoring stack; a deploy that sets this env var will fire the WARN on every boot.
