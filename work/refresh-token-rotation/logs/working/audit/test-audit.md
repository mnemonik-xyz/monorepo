# Test Audit — refresh-token-rotation

## Scope

Tasks 1–5 of the `refresh-token-rotation` feature. Wave-5 checkpoint commit: `c35162c` (merge commit containing all five task branches).

Files audited:
- `mcp/src/oauth/refresh.rs` — Task 1 unit tests inside `#[cfg(test)] mod tests`
- `mcp/src/oauth/mod.rs` — Task 2 + Task 3 unit test additions
- `mcp/src/main.rs` — boot-time validation (no test additions)
- `mcp/src/test_support.rs` — Task 4 test infrastructure
- `mcp/tests/_helpers/mod.rs` — Task 4 helper functions
- `mcp/tests/oauth_refresh_e2e.rs` — Task 5 integration test suite (16 tests)
- `mcp/tests/auth_allowlist.rs` — regression scope (read-only)
- `mcp/tests/anonymous_recall.rs` — regression scope (read-only)

---

## AC Coverage

| AC# | Summary | Test function | File:line | Status |
|---|---|---|---|---|
| AC1 | `authorization_code` grant returns `access_token` + `refresh_token` | `test_authcode_grant_returns_refresh` | oauth_refresh_e2e.rs:274 | COVERED |
| AC2 | `refresh_token` grant returns new `access_token` + new `refresh_token` | `test_refresh_grant_returns_new_pair` | oauth_refresh_e2e.rs:294 | COVERED |
| AC3 | Old refresh outside reuse-interval → 400 `invalid_grant` | `test_old_refresh_outside_reuse_interval_rejected` | oauth_refresh_e2e.rs:317 | COVERED |
| AC4 | Replay outside reuse-interval → family revoke (whole family rejected) | `test_replay_outside_reuse_revokes_family` | oauth_refresh_e2e.rs:352 | COVERED |
| AC5 | Expired refresh → 400 `invalid_grant`; family NOT revoked | `test_expired_refresh_rejected_no_family_revoke` | oauth_refresh_e2e.rs:406 | COVERED |
| AC6 | Rolling TTL: new `expires_at > old_expires_at` after rotation | `test_rolling_expires_at_extended` | oauth_refresh_e2e.rs:439 | COVERED |
| AC7 | Discovery metadata advertises `refresh_token` grant | `test_discovery_advertises_refresh_grant` | oauth_refresh_e2e.rs:466 | COVERED |
| AC8 | Access-token format unchanged (JWT, 1h TTL, Bearer) | `test_access_token_format_unchanged` | oauth_refresh_e2e.rs:518 | COVERED |
| AC9 | Anonymous `mnemonic_recall` still works (regression anchor) | `test_anonymous_recall_unchanged` | oauth_refresh_e2e.rs:577 | COVERED |
| AC10 | Dual content-type parity (form-encoded + JSON) on refresh-grant | `test_refresh_grant_content_type_parity` | oauth_refresh_e2e.rs:630 | COVERED |
| AC11 | Back-compat: legacy client ignoring refresh field still works | `test_back_compat_ignores_refresh_field` | oauth_refresh_e2e.rs:707 | COVERED |
| AC12 | Concurrent rotation: all parallel requests return byte-identical pair | `test_concurrent_rotation_idempotent_within_reuse` | oauth_refresh_e2e.rs:756 | COVERED |
| AC13 | Malformed refresh-grant (`grant_type=refresh_token` without `refresh_token` field) → 400 `invalid_request` | `test_malformed_refresh_grant_invalid_request` | oauth_refresh_e2e.rs:799 | COVERED |

All 13 ACs have at least one dedicated integration test in `mcp/tests/oauth_refresh_e2e.rs`. **No BLOCKER.**

---

## Decision & Risk Coverage

| D#/R# | Property under test | Test function | File:line | Status |
|---|---|---|---|---|
| D5 | LRU cache.put precedes COMMIT in Branch A (CWE-362 race observer) | `rotate_branch_a_writes_row_and_caches_pair_before_commit` | refresh.rs:969 | COVERED |
| D8 | Single BEGIN IMMEDIATE covers replay detection AND family-revoke | `replay_outside_window_revokes_full_family_in_one_tx` | refresh.rs:1166 | COVERED |
| D9 | Hourly evictor sweeps expired rows, leaves live rows intact | `evictor_evicts_expired_rows` | refresh.rs:1422 | COVERED |
| R6 | DB write failure during rotation is retry-safe (ROLLBACK + cache idempotency) | Delegated to Task 6 code-audit (explicit Task 5 deviation) | See Task 6 decisions.md entry | COVERED (cross-audit) |

Notes:
- D5: covered by a dedicated unit test with a thread-local race observer that proves the CWE-362 window materialises when the hook is flipped. The production put-before-COMMIT ordering is also asserted in the same test. Transitive coverage also exists via AC12 (`test_concurrent_rotation_idempotent_within_reuse`).
- D8: dedicated unit test with three siblings; asserts atomic family-revoke across all rows. Cross-cutting integration test `cross_table_tokio_mutex_no_starvation` (oauth_refresh_e2e.rs:955) provides complementary concurrency coverage.
- D9: dedicated unit test with 50ms tick and a 250ms wait window. Sleep is well below 1s (INFO: see Real-time Waits section).
- R6: explicit scope-down in Task 5 (no production hook exists). Task 6 code-audit explicitly verified ROLLBACK-on-error contract and Branch A COMMIT-failure orphan-cache safety. Cross-audit chain is closed.

**No BLOCKER.**

---

## Real-time Waits

All `sleep(...)` calls in audited test files:

| Call site | Duration | Purpose | ≥1s? |
|---|---|---|---|
| `oauth_refresh_e2e.rs:326` | `Duration::from_millis(150)` (POST_REUSE_SLEEP_MS) | AC3: wait past 100ms reuse-interval cache TTL | NO |
| `oauth_refresh_e2e.rs:364` | `Duration::from_millis(1100)` (POST_FAMILY_REVOKE_SLEEP_MS) | AC4: cross wall-clock second boundary for DB-layer `as_secs()` inside-window check | YES — justified |
| `oauth_refresh_e2e.rs:452` | `Duration::from_millis(1100)` (POST_FAMILY_REVOKE_SLEEP_MS) | AC6: cross wall-clock second boundary for strict `expires_after > expires_before` assertion | YES — justified |
| `refresh.rs:1386` | `Duration::from_millis(75)` | `lru_ttl_expires_within_window`: verify entry expires after 50ms TTL | NO |
| `refresh.rs:1459` | `Duration::from_millis(250)` | `evictor_evicts_expired_rows`: ~5 tick attempts at 50ms each | NO |
| `mod.rs:4954` | `Duration::from_millis(1100)` | `logging_policy_no_plaintext_no_hash_across_branches` Branch C: cross wall-clock second for family-revoke | YES — justified |

The three 1100ms sleeps are documented workarounds for `reuse_interval.as_secs()` integer truncation at `refresh.rs:689`. With a 100ms `reuse_interval`, `as_secs()` returns 0, causing `inside_window = (now <= rotated_at)`. Landing on Branch C (family revoke) — required by AC4 and the logging-policy Branch C assertion — requires crossing a unix-second boundary. This is explicitly documented in the module-level comment of `oauth_refresh_e2e.rs` (lines 14–21) and in the `POST_FAMILY_REVOKE_SLEEP_MS` docstring (lines 79–91). All three 1100ms sleeps are inline-justified.

**Pass — the ≥1s sleeps are justified and documented. No finding.**

---

## Litmus Tests

No litmus-failing tests identified.

The early-draft pair `token_request_deserializes_both_grants` and `mint_and_hash_roundtrip` did not re-appear (confirmed by grepping for these function names: zero hits).

All unit tests in `refresh.rs` and `mod.rs` assert post-parse dispatch behaviour, DB state, or cryptographic-property invariants — not mere deserialization success or library round-trips. Specifically:
- `migration_is_idempotent` (refresh.rs:938): asserts row count post-re-run, not just no-panic.
- `unknown_token_returns_invalid_grant` (refresh.rs:952): asserts the `RotateOutcome::InvalidGrant` enum variant on a never-issued token.
- `lru_cap_evicts_oldest` (refresh.rs:1351): asserts the oldest 44 keys are evicted, the newest 256 retained — this tests the LRU eviction ordering, which is a property of the `ReuseCache` wrapper, not a direct test of `lru::LruCache` internals. The test is borderline (testing the wrapper's cap behaviour), but the cap/eviction contract is load-bearing for D5 security invariants (an unbounded cache would be a DoS vector). INFO-tagged rather than LITMUS finding.

**Pass.**

---

## Regression Check

### `mcp/tests/auth_allowlist.rs`

`git diff main...c35162c -- mcp/tests/auth_allowlist.rs` produces no output — the file was unchanged by the refresh-token-rotation wave. Current HEAD also shows no diff on this file relative to `c35162c`.

Test function names present (at wave-5 checkpoint):
- `test_expired_jwt_is_401_challenge_on_initialize_recall_and_gated_tools`
- `test_tools_list_initialize_no_auth_200_sign_memory_no_auth_401`

Both present; assertion intent preserved. **No regression.**

### `mcp/tests/anonymous_recall.rs`

`git diff main...c35162c -- mcp/tests/anonymous_recall.rs` produces no output — the file was unchanged by the wave.

Test function names present:
- `anonymous_recall_returns_public_rows_only`
- `authenticated_recall_returns_both`
- `cross_owner_pool_visible`
- `foreign_public_rows_not_in_authenticated_recall`

All present; assertion intent preserved. **No regression.**

### `mcp/src/oauth/mod.rs:2022–2099`

Lines 2022–2099 cover: `oauth_error` (2026), `oauth_error_typed` (2041), and `extract_forensic_remote_addr` (2063). These are shared production helper functions, not a test block. The bearer-auth middleware tests in this file's `#[cfg(test)]` section (starting at line ~4100 approximately) include the existing JWT verify, bearer-auth allowlist, and related tests. The `with_defaults` constructor rename (from `OAuthState::new` → `OAuthState::with_defaults` for the test-only constructor) was introduced by Task 3 and is the only semantic change to existing test call-sites. No test function was deleted; no assertion intent was weakened.

Confirmed: all pre-existing test function names in `oauth/mod.rs` tests are preserved. The constructor rename changed call-site syntax but not the behaviour being asserted. **No regression.**

---

## Findings

1. **INFO — R6 cross-audit chain documentation.** R6 (DB write failure retry-safety) is explicitly delegated to Task 6 code-audit by Task 5's scope-down. Task 6 verdict is CLEAN and explicitly closes R6 via the ROLLBACK-on-error contract. The delegation is documented in `decisions.md` for both tasks. Suggested fix: none — the cross-audit chain is complete and properly documented.

2. **INFO — Same-file SQLite-writer-lock fairness not covered.** `cross_table_tokio_mutex_no_starvation` verifies tokio-Mutex fairness on TWO separate-tempfile connections (per Decision 6 production wiring), NOT same-file SQLite writer-lock fairness. A bespoke test exercising two in-process readers competing for the same WAL writer lock would strengthen D8 + D11 concurrency coverage. File: `oauth_refresh_e2e.rs:955`. Suggested fix: open as a follow-up task for a same-file SQLite-writer-lock anchor test.

3. **INFO — D5 race-observer test design rationale.** The `rotate_branch_a_writes_row_and_caches_pair_before_commit` test uses a thread-local `std::cell::Cell<bool>` hook that flips the production `cache.put → COMMIT` ordering to `COMMIT → cache.put`. The production path is asserted in the first half of the test. The race-window path is then separately exercised with a fresh token (`seed2`). The observer asserts a cache miss between COMMIT and put. This design is correct and intentional but may be confusing to maintainers. The inline comments are sufficient. Suggested fix: consider adding a one-sentence top-level doc comment to the test naming the two invariants it asserts.

4. **INFO — `_helpers_*` test naming style.** The helpers smoke test target (`mcp/tests/helpers_smoke.rs`) uses a non-standard `#[path]` import pattern (`mod _helpers;`). This is correct and functional but differs from the project convention of integration tests importing production symbols only. Not a quality finding, merely an observation for maintainers.

5. **INFO — AC10 structural-not-byte-equality nuance.** AC10 tests form-encoded vs JSON content-type parity using TWO independent `bootstrap_oauth` exchanges. Access tokens are asserted by three-segment JWT structure and non-empty check, not byte-equality across the two independent rotations — because independent rotations intentionally produce distinct JWTs (different `jti`/`iat`). This is correct design; byte-equality across independent rotations would be wrong. The test correctly uses structural equality. Noted for reviewer awareness.

---

## Verdict

**PASS**

All 13 user-spec ACs (AC1–AC13) map to a dedicated integration test in `mcp/tests/oauth_refresh_e2e.rs`. D5 (put-before-COMMIT race observer), D8 (single-tx detect+revoke), and D9 (hourly evictor) each have a dedicated unit test in `mcp/src/oauth/refresh.rs#tests`. R6 is covered by cross-audit chain (Task 6 CLEAN verdict). Regression anchors (`auth_allowlist.rs`, `anonymous_recall.rs`, `oauth/mod.rs` bearer-auth tests) preserve every test function name and assertion intent. No real-time sleep ≥1s except the three documented 1100ms wall-clock-second-boundary workarounds. No litmus-failing test; the early-draft dropped pair did not re-appear. AC12 correctly asserts byte-identity on the full `(access_token, refresh_token)` PAIR across all 10 parallel rotations.

The five INFO findings are all documentation or follow-up items; none require action before deploy. Task 9 pre-deploy QA gate is unblocked.
