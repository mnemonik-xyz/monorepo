# Decisions — refresh-token-rotation

Append-only log of decisions and audit findings.

## Origin

This feature is the result of three scope-pivots during user-spec planning on
2026-06-06. Originally opened as `local-mode-survives-token-expiry`, then
narrowed to `session-reauth-recovery`, then renamed to
`refresh-token-rotation` after PKCE-constraint discovery and Stripe-MCP
precedent research.

## 2026-06-06 — Pivot 1: drop whitelist local+private

**Trigger:** User asked "Why we need any auth for local mode at all?"

**Finding (code-research §A):** On HTTP transport, `mode:"local"` writes
without JWT fall back to `owner_pubkey = server keypair` — producing rows
neither anonymous recall nor authenticated recall can return. Dead writes.

**Decision:** Drop AC1 (whitelist local+private). It silently makes the bug
worse, not better. Mode model cleanup reframed as separate feature
`work/binary-mode-cleanup/` (originally
`work/hosted-mode-rename-and-pricing/`, renamed after a further pivot
to binary-mode model later 2026-06-06).

## 2026-06-06 — Pivot 2: drop request_reauth

**Trigger:** Adequacy validator critical finding:
PKCE-binding makes server-side authorize URL synthesis impossible for the
Solana-wallet OAuth flow. PKCE state lives in the MCP host's process; the
server has no access to `code_verifier` or `code_challenge`.

**User challenge:** "User should sign, not server" — pointed out that JWT
represents server vouching, not user cryptographic identity. Deep
architectural point parked at `work/stateless-auth-rearch/`.

## 2026-06-06 — Pivot 3: refresh-token rotation (Stripe precedent)

**Trigger:** Researched how Stripe MCP handles auth. Findings:
- Local stdio mode: long-lived Restricted API Keys.
- Remote HTTP mode (`https://mcp.stripe.com`): OAuth 2.1 with 1h access +
  1y rolling refresh.

**Insight:** Stripe MCP works in Claude Desktop / Cursor / VS Code because
clients silently rotate refresh tokens. Standard OAuth 2.1.

**Insight on whitepaper invariant:** Cryptographic identity for memory bytes
is ALREADY user-owned (Ed25519 keypair signs COSE_Sign1 in browser-mediated
signing). JWT is session-routing only. Refresh tokens do NOT violate "user
owns identity".

**Decision A:** Implement standard OAuth 2.1 refresh-token rotation in
`/oauth/token`. Stripe-precedent: 1h access + 1y rolling refresh.

**Decision B:** Per-request signing parked at `work/stateless-auth-rearch/`
as long-term direction.

**Decision C:** Renamed `work/session-reauth-recovery/` →
`work/refresh-token-rotation/`.

## 2026-06-06 — Validation round 1 — applied fixes

Validators (completeness, quality, adequacy) returned 20 findings (3
critical, 8 major, 9 minor) on first pass of the rewritten spec.

**Critical findings & responses:**

- **C-completeness/quality (concurrent rotation atomicity)**: ROLL UP — see
  D11 + D13.
- **C-adequacy (R1 pre-ship empirical verification)**: ACCEPTED — R1
  mitigation rewritten to require one of two cheap pre-ship verifications
  (Option A: HTTP trace Claude Desktop ↔ Stripe MCP; Option B: dev deploy
  with JWT_TTL=60s). Pre-ship gate, not "smoke after ship".
- **C-adequacy (concurrent refresh + AC4 catastrophic)**: ACCEPTED —
  introduced D13 reuse-interval pattern (Auth0/Okta standard). Within 30s
  of rotation, the same old refresh returns the existing descendant pair
  idempotently. Only outside the window does presentation trigger
  family-revoke. Closes the kill-the-session race.

**Major findings rolled into decisions/spec:**

- M-completeness/M1 (dual content-type) → D14 + AC10.
- M-completeness/M2 (OAuthState DB handle) → D10.
- M-completeness/M3 (CLI cache out of scope) → D15 + Ограничения.
- M-completeness/m4 + M-quality (concurrent rotation) → D11 + D13.
- M-adequacy (1y TTL justification) → D2 expanded with rationale.
- M-adequacy (back-compat with old clients) → AC11.
- M-adequacy (hybrid 4h+refresh alternative) → considered, rejected;
  reuse-interval (D13) already solves concurrency, so trimming access
  TTL gives diminishing return. 1h kept.
- M-adequacy (AC4 UX position) → resolved by D13 reuse-interval —
  Auth0-pattern is the explicit position chosen.

**Minor findings rolled in:**

- m-completeness/m5 (eviction tick) → D12.
- m-completeness/m6 (TestServer mount) → Тестирование section + D-no-num
  spec text.
- m-quality (AC6 verification step) → Как проверить шаг 5 now explicitly
  checks `expires_at > expires_at_before`.
- m-quality (oauth/mod.rs:58 citation in Зачем) → reworded to "JWT access
  lives 1 hour" without file:line.
- m-quality (2 missing error scenarios — invalid_request + DB write fail)
  → AC13 + R6.
- m-adequacy (sha256 vs blake3 contradiction) → D1 fixed to blake3 with
  payment.rs:737-744 precedent.
- m-adequacy (no observability ask) → Ограничения now explicit:
  V1 = tracing logs only; Prometheus follow-up.
- m-adequacy (D7 forfeits "log out everywhere") → R7 + D16 explicit.

## Spec-time technical decisions (consolidated)

- **D1 (Refresh-token format)** — opaque 32-byte random, base64url for
  transport, blake3(salt+plaintext) stored. Plaintext returned once.
- **D2 (TTL)** — access 1h (unchanged), refresh 1y rolling. Stripe
  precedent + memory-protocol risk profile justification.
- **D3 (Rotation discipline)** — OAuth 2.1 §6.1 rolling rotation.
- **D4 (Family revoke on replay)** — only OUTSIDE reuse-interval (D13).
  Inside reuse-interval (5s) reused old token is treated as legitimate
  network-retry and returns the same descendant pair. Outside, it's
  treated as potential compromise and revokes the whole family by
  `family_id`.
- **D5 (Storage location)** — refresh_tokens table in mcp/ (per CLAUDE.md).
- **D6 (No /oauth/revoke v1)** — no UX scenario requires it.
- **D7 (Access-token format unchanged)** — JWT HS256 1h. R7+D16 acknowledge
  the "no global logout" trade-off.
- **D8 (Discovery metadata)** — add "refresh_token" to grant_types_supported.
- **D9 (No Token Binding)** — HTTPS-only.
- **D10 (OAuthState gets DB handle)** — Arc<Mutex<Connection>> on shared
  store. Single DB file. Migration in mcp/src/oauth/refresh.rs by escrow.rs
  pattern.
- **D11 (Atomic rotation)** — single BEGIN IMMEDIATE transaction:
  SELECT-lock → check → UPDATE old → INSERT new. Pattern from payment.rs.
- **D12 (Eviction)** — hourly background sweep only. Opportunistic
  in-transaction cleanup dropped in round 2 (overengineering for 1y TTL).
- **D13 (Reuse-interval — Auth0 pattern, 5s)** — 5-second window after
  rotation (Auth0 default; Okta default is 30s, deliberately not picked
  because longer window widens potential replay-attack timing). Same
  old refresh within window returns existing descendant pair
  idempotently. Only outside window triggers family-revoke. Solves
  network-retry race without compromising replay-detect.
- **D13.1 (`family_id` semantics)** — per-grant UUID. Each
  `authorization_code` exchange mints a fresh `family_id`. Multi-device
  users have multiple independent families; compromise of one doesn't
  revoke the others. Stripe/Auth0 standard. Alternative (sub-bound) was
  rejected as paranoid — single leak would kill all sessions.
- **D14 (Dual content-type parity)** — refresh-grant parses both
  `application/x-www-form-urlencoded` (today: VS Code, Claude.ai) and
  `application/json` (today: Cursor) via existing token_handler dispatch
  at `oauth/mod.rs:982-1078`. Round 2 corrected the inverted client
  attribution that was present in the round-1 draft.
  **Implementation hook (AC13)**: current `TokenRequest` struct
  (`oauth/mod.rs:946-957`) has no `grant_type` or `refresh_token`
  fields. Tech-spec needs to widen the struct AND add post-parse
  validation: if `grant_type=refresh_token` but `refresh_token` field
  is absent/empty → return `400 invalid_request`. The branch decision
  inside `token_handler` keys off `grant_type` after parse, not before
  (no new content-type dispatch).
- **D15 (CLI cache out of scope)** — `~/.mnemonic/token.json` not extended
  in V1. Open `work/cli-refresh-token-support/` follow-up if requested.
- **D16 (No global logout v1)** — accept JWT access trade-off; document.
- **D17 (Not doing per-request signing now)** — deferred to
  `work/stateless-auth-rearch/`.

## Code-research dossier

See `code-research.md`:
- §A-§G: prior-pivot research (request_reauth allowlist, request_public_write
  template, recall behaviour, token model). Some sections informational
  rather than load-bearing now.
- §H: refresh-token-specific touchpoints — token_handler shape,
  storage placement (mcp/src/oauth/refresh.rs by escrow precedent), blake3
  hashing precedent, discovery metadata, eviction precedent, Claims schema,
  testing harness gaps.

## Renamed history

- `work/local-mode-survives-token-expiry/` → `work/session-reauth-recovery/`
  → `work/refresh-token-rotation/` (current).
- `work/binary-mode-cleanup/` (sibling parking lot, originally
  `work/hosted-mode-rename-and-pricing/`, renamed 2026-06-06 after
  binary-mode pivot).
- `work/chrome-extension-client-side-storage/` (sibling parking lot,
  opened 2026-06-06 as part of binary-mode pivot).
- `work/stateless-auth-rearch/` (long-term architectural option, opened
  2026-06-06).

## 2026-06-06 — Validation round 2 — applied fixes

Validators ran again on round-1-fixed draft, this round with targeted
focus on user modes / client surfaces / where content is stored (per
user's explicit ask). 13 findings (1 critical, 8 major, 4 minor).

**Critical & majors closed:**

- **Content-type inversion**: spec said "form for Cursor/VS Code, JSON
  for Claude Desktop"; code (`oauth/mod.rs:975-977`) is the opposite —
  VS Code + Claude.ai send form, Cursor sends JSON. PK
  `architecture.md` also has the bug; spec inherited it. Plus
  "Claude Desktop" was the wrong name throughout (Claude.ai is the
  correct product). Renamed and re-attributed in user-spec.md + D14.
- **Claude Desktop CORS evidence absent**: CORS allowlist covers
  `*.claude.ai`, not Claude Desktop's Electron origin. User's bug
  report from 2026-06-06 most plausibly originated in Claude.ai web
  (potentially loaded via desktop wrapper that shares the
  `https://claude.ai` origin). Target users renamed to "Claude.ai
  (web or desktop wrapper)" throughout.
- **`family_id` semantics undefined**: load-bearing for AC4. Picked
  per-grant UUID (D13.1) — multi-device isolation, Stripe/Auth0 norm.
- **AC4/D4 retry-vs-replay distinction implicit**: spec now explicit
  that reuse-interval IS the mechanism distinguishing legitimate
  retry-after-network-fail from replay-attack.
- **AC12 BEGIN IMMEDIATE serializes — `tokio::join!` test degenerate**:
  rewrote AC12 honestly — test exercises sequential-after-lock +
  reuse-interval lookup, which is correctness boundary that matters.
- **D13 30s mis-attributed**: 30s is Okta default, not Auth0. Auth0
  default is closer to 5s. Switched to 5s (D13) — tighter security
  window, sufficient for network retry.
- **AC10 verification gap**: added step 5b (JSON refresh-grant) to
  Агент проверяет table.

**Sibling drift corrections:**

- `binary-mode-cleanup/README.md`: Arweave description was
  "compressed embedding"; actually COSE_Sign1 envelope wrapping
  canonical CBOR. Reference to `patterns.md::Storage modes` was wrong
  section name; corrected. Updated separately.
- `chrome-extension-client-side-storage/README.md`: claim of
  in-extension fastembed via WASM was incorrect — `core/src/lib.rs:
  10-13` gates `embed` behind `#[cfg(not(target_arch = "wasm32"))]`.
  Architecture (`architecture.md:14`) envisions `ort-web` or
  `transformers.js` for browser. Updated the stub to be honest: WASM
  embedder is a sub-task of the migration, not free.
- `stateless-auth-rearch/README.md`: Option A wording ambiguous about
  whether stdio-proxy is side-grade or replacement; clarified as
  "full replacement of hosted OAuth path".

**Minors closed:**

- D12 dropped opportunistic in-transaction cleanup (overengineering).
- R1 Option A (TLS interception of Claude.ai ↔ Stripe MCP) demoted —
  needs custom CA in Electron trust store + possible cert pinning,
  high infra cost. Promoted Option B (`JWT_TTL=60s` dev deploy) as
  primary. Added Option C (read MCP SDK source).
- 13 ACs > M threshold (size_check minor): kept as-is; all tightly
  scoped to one endpoint. Acknowledged in interview.yml.

## Task 1: Refresh-token storage module + migration + evictor

**Status:** Done
**Commits:** `fd73cbe` (initial), `5d51c77` (round-1 fixes), `d324868` (round-2 cleanup)
**Agent:** task-1-builder
**Summary:** New `mcp/src/oauth/refresh.rs` with `refresh_tokens` schema + migration, `mint_for_authorization_code`, `family_revoke`, `evict_expired`/`start_evictor`, atomic 6-branch `rotate` under one `BEGIN IMMEDIATE` (D8), and an in-process `ReuseCache` (lru 0.12 + manual TTL). Key invariants: `cache.put` BEFORE `COMMIT` in Branch A (D5, closes CWE-362); `migrate_refresh_tokens` calls `apply_oauth_connection_pragmas` so `foreign_keys=ON` + WAL + `busy_timeout=5000` are enforced on the OAuthState-owned connection per D6; explicit `ROLLBACK` on COMMIT failure (matches `payment.rs:478-505`); `D14` logging surface has zero `token_hash`/plaintext/JWT/salt leakage. Test hook restructure: production cache-put ordering is owned by outer `rotate`; the D5 unit test installs a thread-local race observer that fires between COMMIT and put and asserts the CWE-362 window materializes.

**Deviations:** Branch B' WARN log includes `family_id + sub` (the row IS resolved at B'), going beyond D14's `outcome+remote_addr+request_id+stem` literal field list — accepted by security-auditor-1 + code-reviewer-1 as forensically valuable, not a security regression. Tech-spec D14 wording should be updated to reflect this in a follow-up. Public `OAUTH_CONN_PRAGMAS` const + `apply_oauth_connection_pragmas` helper added so Task 3's `OAuthState::new` can call the helper independently — minor additive surface beyond what the task spec literally listed.

**Reviews:**

*Round 1:*
- code-reviewer-1: REQUEST_CHANGES — 1 major (D5 debug hook skipped put rather than moving it post-COMMIT) + 3 minor → `logs/working/task-1/code-reviewer-1-round1.json`
- security-auditor-1: CONDITIONAL PASS — 2 medium (PRAGMA setup; COMMIT-failure ROLLBACK) + 2 minor → `logs/working/task-1/security-auditor-1-round1.json`
- test-reviewer-1: APPROVE_WITH_MINOR_NOTES — 3 minor (Branch B 1-second-boundary flakiness; Branch C cache-cleanup unverified; evictor timing buffer) → `logs/working/task-1/test-reviewer-1-round1.json`

*Round 2 (after fixes):*
- code-reviewer-1: APPROVED — F1/F2/F3/F4 all resolved; R1 minor (dead `observed_revoked` flag) and R2 informational (B' field deviation) noted → `logs/working/task-1/code-reviewer-1-round2.json`
- security-auditor-1: PASS — M1/M2/m2 fixed; m1 accepted; D5 ordering re-verified under restructure → `logs/working/task-1/security-auditor-1-round2.json`
- test-reviewer-1: APPROVED — M1/M2/M3 resolved; D5 race-observer test now meaningful → `logs/working/task-1/test-reviewer-1-round2.json`

Round-2 R1 cleanup (dead test flag) applied in `d324868`.

**Verification:**
- `cargo test -p mnemonic-mcp --features test-support --lib refresh::tests` → 12/12 pass
- `cargo build --workspace` → OK
- `cargo clippy -p mnemonic-mcp --all-targets --features test-support -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean

## Task 2: JWT_TTL_SECS env-plumbing via OnceLock

**Status:** Done
**Commits:** `dcfdd45` (initial), `b84a031` (round-1 fix: TR2-R1-1 / CR2-R1-m1), `b092178` (round-1 fix: CR2-R1-M1 + CR2-R1-m2), `719b5c8` (round-1 fix: SA2-R1-M1 + bounded WARN echo)
**Agent:** task-2-builder
**Summary:** Replaced the hard-coded `pub const JWT_TTL_SECS: u64 = 3600` in `mcp/src/oauth/mod.rs` with a `static JWT_TTL: OnceLock<u64>` seeded by `seed_jwt_ttl_from_env()` (called once in `main::run_http` immediately after `load_jwt_secret()?`). A pure helper `compute_jwt_ttl_from_env_str(Option<&str>) -> u64` enforces the `[60, 604_800]` clamp (Decision 12) and emits WARN on parse-fail / out-of-range / empty / whitespace — never silent, so a deploy typo on the Task 10 R1 gate is loud. All 6 production read sites (4 in `oauth/mod.rs`, 2 in `escrow.rs`) call `jwt_ttl_secs()`. Two `#[tracing_test::traced_test]` unit tests exercise the pure helper directly so they never touch the process-global `OnceLock` (test-isolation seam from the spec). Docs land in `.env.example` (next to `MCP_JWT_SECRET`) and `references/deployment.md` (env-var table + hosted-mode deploy block).

**Deviations:** HTTP-boot smoke (the `MCP_JWT_TTL_SECS=60 cargo run ...` AC line) could not be executed in this sandbox — the binary's `identity::ensure` requires OS keychain access the harness does not grant, and process stdio is suppressed when backgrounded. The behavior contract is fully exercised by the two `#[traced_test]` unit tests + the unchanged `oauth_loopback` and `auth_allowlist` integration tests. All three reviewers were notified and none flagged the deviation as blocking. After round-1 review fixes, the seed function moved from `JWT_TTL.set` to `get_or_init` with a captured `Option<u64>` so the INFO log only fires on actual init (CR2-R1-M1), and both the INFO and DEBUG logs were stripped of the raw `MCP_JWT_TTL_SECS=<raw>` echo (SA2-R1-M1, Decision-14 logging policy). The parse-failure WARN that still needs to surface raw input routes it through a new char-boundary-safe `bound_log_value(s, max_chars)` helper that truncates at 16 chars + ellipsis, with a dedicated UTF-8-aware test pinning the panic-safety invariant. Two extra unit tests landed in `719b5c8` to cover the SA2-driven bounding contract — total `--bin mnemonic-mcp` test count is 185 (up from the pre-task baseline of 181).

**Reviews:**

*Round 1:*
- code-reviewer-2: APPROVE_WITH_MINOR_NOTES — M1 (re-seed log honesty) + m1 (whitespace WARN assertion gap) + m2 (visibility tightening on the three TTL constants) → `logs/working/task-2/code-reviewer-2-round1.json`
- security-auditor-2: CONDITIONAL_PASS — M1 (raw env-var echo / secret-exposure vector) + L1 (whitespace WARN gap, dup of CR2 m1 / TR2-R1-1) + L2 (informational, `pub mod refresh;` ownership) + I1 (informational) → `logs/working/task-2/security-auditor-2-round1.json`
- test-reviewer-2: CONDITIONAL_PASS — TR2-R1-1 (whitespace WARN not independently asserted) → `logs/working/task-2/test-reviewer-2-round1.json`

*Round 2 (after fixes):*
- code-reviewer-2: APPROVE — M1/m1/m2 all resolved; `get_or_init` idiom confirmed; visibility tightening accepted; no new issues → `logs/working/task-2/code-reviewer-2-round2.json`
- security-auditor-2: PASS — M1 fully addressed (logs carry only resolved numeric TTL; parse-failure WARN bounded at 16 chars); L1 independently asserted; `bound_log_value` UTF-8-safe via `chars()` iterator → `logs/working/task-2/security-auditor-2-round2.json`
- test-reviewer-2: PASS — TR2-R1-1 resolved with minimal and accurate fix → `logs/working/task-2/test-reviewer-2-round2.json`

**Verification:**
- `cargo test -p mnemonic-mcp --features test-support --bin mnemonic-mcp` → 185/185 pass
- `cargo test -p mnemonic-mcp --features test-support --test oauth_loopback` → 4/4 pass
- `cargo test -p mnemonic-mcp --features test-support --test auth_allowlist` → 1/1 pass (regression check per AC)
- `cargo build -p mnemonic-mcp` → OK
- `cargo clippy -p mnemonic-mcp --all-targets --features test-support -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean

## Task 3: OAuthState widening + refresh-grant + discovery + salt validation

**Status:** Done
**Commits:** `3e5c1fe4` (initial implementation), `b3541d2` (round-1 fixes: CR3-R1-C1 + CR3-R1-M1 + CR3-R1-M2 + SA3-R1-M1 + SA3-R1-M2 + TR3-R1 H1-H4 + M1-M2 + L2)
**Agent:** task-3-builder
**Summary:** Wave-2 wiring task. Widens `OAuthState` with 5 new fields (`refresh_store: Arc<Mutex<Connection>>`, `refresh_salt`, `reuse_interval`, `evictor_tick`, `reuse_cache`); `OAuthState::new` opens a second physical `rusqlite::Connection` on `DATABASE_PATH` per Decision 6. Adds `OAuthState::with_defaults(secret)` in-memory test constructor so ~40 test call sites take a one-line diff. Widens `TokenRequest` so every field is `Option<String>` with explicit `grant_type` (default `"authorization_code"` for legacy clients) + `refresh_token`. `token_handler` is now a thin wrapper around `token_handler_inner` that pipes the response through `apply_no_store_headers` (Decision 15 — `Cache-Control: no-store` + `Pragma: no-cache` on every exit). Post-parse dispatch routes `authorization_code` (existing path with explicit non-empty validation; now also mints a refresh-token via `refresh::mint_for_authorization_code`), `refresh_token` (length-cap pre-hash per Decision 16 → `spawn_blocking` → `refresh::rotate`), or `unsupported_grant_type` 400 per RFC 6749 §5.2 / Decision 11. Discovery advertises both grants. Pure `validate_refresh_salt` helper (Decision 2 — `base64::STANDARD` decode + ≥32 byte check) gates boot in `main::run_http`, which also threads `DATABASE_PATH` + intervals + `reuse_cache_cap` into the new constructor and spawns `refresh::start_evictor`. After round-1 reviews, a new `oauth_error_typed(status, error, description)` builder produces the RFC 6749 §5.2 `{"error": code, "error_description": prose}` envelope so user-supplied `grant_type` and internal `{e}` values are never echoed to the wire; `extract_forensic_remote_addr` reads `X-Forwarded-For` (first hop, 64-char bounded via existing UTF-8-safe `bound_log_value`) and a per-request UUIDv4 `request_id` threads into `RotateContext` so D14 production log lines carry the full forensic correlation set.

**Deviations:** Two cross-task deviations, both approved by the team lead:

1. **`refresh.rs` (Task 1 module) — 2-line surface addition to fix AC12.** The round-1 code review (CR3-R1-C1) caught a critical: `RotateOutcome::Rotated` only carried `(new_token, sub, google_sub)`, so the Task 3 handler was minting a SECOND access JWT for the response body. The JWT published to the LRU cache (for Branch B retry idempotency) was therefore byte-different from the one returned to the Branch A caller, breaking AC12. Fix: added `access_jwt: String` field to `RotateOutcome::Rotated` and populated it from the already-minted JWT inside `branch_a_rotate`. All existing `Rotated { .. }` destructures in `refresh.rs::tests` absorb the new field via `..` so no test-side churn. The Task 3 handler now consumes the field directly. Cross-task encroachment is minimal (2 lines: field declaration + populate); cleaner than the alternative of plumbing the cached JWT out via a second channel.

2. **`OAuthState::with_defaults` lives outside `#[cfg(test)]`.** The cfg-gated form was the original intent, but every integration-test crate under `mcp/tests/*.rs` consumes the constructor via the library facade — gating it on `cfg(test)` made all 14 test crates fail to compile (the library is built without `cfg(test)` for the integration tests). Gating on `feature = "test-support"` would force ~14 test crates to opt in and is out of scope for Task 3. The constructor is annotated `#[allow(dead_code)]` for the bin target and the doc comment is explicit: "**NOT for production wiring** — uses an in-memory SQLite database … and a fixed all-`0xAB` salt." Production `main::run_http` always uses the multi-arg `OAuthState::new`.

3. **Smoke verification could not run live in this sandbox.** The release binary aborts on `identity::ensure` (OS keychain) BEFORE reaching `run_http` where the new salt-gate and `OAuthState::new` live. Documented as the same deviation Task 2 used. All AC paths are covered by the 8 TDD anchor unit tests at the handler boundary via axum `oneshot` (`tower::ServiceExt`).

A handler-side proxy log line was also added — the rotate-internal log emissions inside `tokio::task::spawn_blocking` are invisible to `tracing_test::traced_test`'s thread-local subscriber, so the handler emits a parallel D14-shape INFO line on the async task AFTER `spawn_blocking` returns. The production log infrastructure observes both; the test harness observes the handler-side one. This is documented in code comments on the four emission sites.

**Reviews:**

*Round 1:*
- code-reviewer-3: changes-required — 1 critical (CR3-R1-C1 AC12 double-mint) + 2 major (CR3-R1-M1 envelope shape; CR3-R1-M2 forensic fields) + 3 minor → `logs/working/task-3/code-reviewer-3-round1.json`
- security-auditor-3: CONDITIONAL_PASS — 2 minor (SA3-R1-M1 user-grant_type echo; SA3-R1-M2 internal `{e}` echo) → `logs/working/task-3/security-auditor-3-round1.json`
- test-reviewer-3: FAILED — 4 HIGH (TR3-R1-H1 missing presence assertions; TR3-R1-H2 salt absent; TR3-R1-H3 access_token absent; TR3-R1-H4 Branch B' not exercised) + 1 MEDIUM (TR3-R1-M1 Branch D conflated with E) + 2 LOW → `logs/working/task-3/test-reviewer-1.json`

*Round 2 (after fixes in `b3541d2`):*
- code-reviewer-3: APPROVED — all three required findings correctly addressed; 3 minor follow-up notes (`assert!(starts_with(...))` vs exact equality; handler InvalidGrant log carries merged branch label; pre-existing sub-second `as_secs()` boundary in Branch B' — none merge-blocking) → `logs/working/task-3/code-reviewer-3-round2.json`
- security-auditor-3: PASS — both M1/M2 resolved; new `oauth_error_typed` builder design accepted as a structural audit anchor for future callers → `logs/working/task-3/security-auditor-3-round2.json`
- test-reviewer-3: PASSED — all 8 findings resolved; 2 new LOW observations (N1 Branch B `xff_b` not asserted; N2 handler InvalidGrant log merged label) flagged for code reviewer, non-blocking → `logs/working/task-3/test-reviewer-3-round2.json`

**Verification:**
- `cargo build --workspace` → OK
- `cargo clippy --workspace --all-targets --features 'mnemonic-mcp/test-support' -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- `cargo test -p mnemonic-mcp --features 'mnemonic-mcp/test-support' --no-fail-fast` → 535 tests, 0 fails (51 oauth unit tests including the 8 new TDD anchors)

---

## Task 4: TestServerBuilder with-oauth-token flag + integration test helpers

**Status:** Done
**Commit:** ae3c22a (round-1 implementation in e77cefa; round-2 fixes in ae3c22a)
**Agent:** task-4-builder
**Summary:** Wave-3 single-task — extends `TestServerBuilder` in `mcp/tests/_helpers/mod.rs` with `with_oauth_token(bool)`, `with_reuse_interval(Duration)`, `with_evictor_tick(Duration)` setters. Default `with_oauth_token == false` keeps the existing `OAuthState::with_defaults` constructor and does NOT mount `/oauth/token` or `/oauth/authorize` (preserves the AC9 regression guard — `test_anonymous_recall_unchanged` runs against a builder where those routes still 404). On `true`, opens its own tempfile-backed `rusqlite::Connection` per Decision 6, runs `migrate_refresh_tokens`, mounts BOTH `/oauth/token` (POST) and `/oauth/authorize` (GET + POST) on the same tower-stack as `/mcp` via `Router::merge`. Adds four public fixture helpers consumed by Task 5: `bootstrap_oauth -> (code, verifier, redirect_uri)` drives the real production challenge-sign-redeem path through `/oauth/authorize` GET (JSON mode with `pubkey` query param to bind `expected_pubkey`) + raw Ed25519-signed POST — no SQL shortcuts into `issued_codes` (Finding 4 / production parity); `rotate(server, refresh) -> (access, refresh)` returns BOTH freshly-emitted strings so Task 5 AC12 can assert byte-identity on both fields under the 10-parallel rotation; `insert_expired_refresh_for_test -> (plaintext, family_id)` inserts a row with `expires_at = now - 60` via `refresh::hash_refresh_token(salt, plaintext)` and returns both values Task 5 AC5 needs (plaintext to POST, family_id to assert non-revoke); `family_has_unrevoked_rows(server, family_id) -> bool` is a synchronous `SELECT COUNT(*) WHERE family_id = ? AND revoked = 0`. All SQL helpers acquire the `Mutex<Connection>` guard, run one statement, drop the guard before any control flow — no `.await` under the lock (CLAUDE.md hard rule). Adds `TEST_REFRESH_SALT: [u8; 32] = [0xABu8; 32]` and `TEST_REDIRECT_URI: &str = "http://127.0.0.1:9999/cb"` as `pub const`s so Task 5 tests can echo the same values when posting `/oauth/token` directly. New test target `mcp/tests/helpers_smoke.rs` (gated `required-features = ["test-support"]`) carries five smoke tests covering: positive + negative route mount, bootstrap triple drives 200 on authorization_code grant with non-empty access + refresh, rotate returns non-empty `(access, refresh)` pair with `refresh2 != refresh1`, insert_expired returns 43-char plaintext + UUID family_id with stored token_hash matching `hash_refresh_token` recompute, and family_has_unrevoked_rows flips correctly on direct SQL revoke/unrevoke. Pulls `_helpers` via `#[path = "_helpers/mod.rs"] mod _helpers;` so the smoke tests live in their own Cargo target rather than duplicating into every consumer of `_helpers`.

**Deviations:**

1. **`bootstrap_oauth` embeds `client_id = "test-client"` and does NOT return it as part of the triple (T4-M1).** Task 5 tests doing direct POSTs to `/oauth/token` must echo this value as `client_id` when constructing their bodies. Acceptable in V1 because the tech-spec's refresh-grant explicitly does NOT validate `client_id` against the `refresh_tokens` row, so silent coupling on the literal `"test-client"` does not break any AC. If a future hardening pass adds `client_id` validation, `bootstrap_oauth` should be widened to return a 4-tuple `(code, verifier, redirect_uri, client_id)`.

2. **`rotate(server, refresh)` helper sends `content-type: application/json` only (T4-M3).** Task 5 AC10 (form-encoded / JSON parity for VS Code / Claude.ai vs Cursor wire) must inline the `application/x-www-form-urlencoded` POST construction in the test body rather than calling through `rotate`. AC12 (10 parallel rotations) is unaffected because that test is about reuse-interval serialisation, not content-type. The helper is the happy-path shape only.

3. **No production code modified.** `mcp/src/oauth/refresh.rs`, `mcp/src/oauth/mod.rs`, `mcp/src/main.rs`, `mcp/src/test_support.rs` all unchanged. Scope-fenced to `mcp/tests/_helpers/mod.rs` (extended), `mcp/tests/helpers_smoke.rs` (new), and `mcp/Cargo.toml` (single `[[test]]` entry for the smoke target).

4. **Smoke verification ran in-process via `tower::ServiceExt::oneshot`.** No live HTTP boot — the helper drives the same axum router shape used by Tasks 2/3 smoke. The release binary's `identity::ensure` OS-keychain dependency was not reached and not required for these helpers. Same deviation pattern Tasks 2/3 documented.

5. **Security findings F1 (`OAuthState::with_defaults` not cfg-gated) and F2 (`refresh_salt` / `refresh_store` pub fields) explicitly out of Task 4 scope** — both are pre-existing Task 3 design decisions where `with_defaults` was intentionally kept outside `#[cfg(test)]` so every integration test crate consumes it via the library facade (gating it on `cfg(test)` would break all 14 test crates; gating it on `feature = "test-support"` is out of scope per Task 3's decisions.md entry). The `pub` fields are necessary surface for Task 4's helpers themselves (`insert_expired_refresh_for_test` reads `refresh_salt`, `family_has_unrevoked_rows` locks `refresh_store`). Deferred as a hardening item for a future pass.

**Reviews:**

*Round 1:*
- code-reviewer-4: approve_with_minors — 3 optional minor (tempfile keep() intent, rand::thread_rng confirmation, task-history doc trim) + 3 verified-no-action cross-file consistency notes → `logs/working/task-4/code-reviewer-4-round1.json`
- security-auditor-4: APPROVED — 0 blockers; 2 Low (F1 `with_defaults` cfg-gating, F2 `pub` fields on OAuthState) explicitly flagged as out-of-scope Task-3 design decisions; 1 Informational (TEST_REDIRECT_URI loopback citation, cosmetic) → `logs/working/task-4/security-auditor-4-round1.json`
- test-reviewer-4: APPROVE_WITH_MINORS — 3 minor (T4-M1 client_id ergonomic, T4-M2 misleading evictor sweep comment, T4-M3 rotate JSON-only) → `logs/working/task-4/test-reviewer-4-round1.json`

*Round 2 (after fixes in `ae3c22a`):*
- code-reviewer-4: approved — all three round-1 minors resolved; 0 new findings; deferred items correctly scoped to decisions.md → `logs/working/task-4/code-reviewer-4-round2.json`
- test-reviewer-4: APPROVED — T4-M2 closed (comment now correctly states no evictor runs and documents the EVICTOR_GRACE_SECS boundary math); T4-M1 + T4-M3 deferred to decisions.md per reviewer recommendation; 0 new findings; Task 5 marked READY → `logs/working/task-4/test-reviewer-4-round2.json`
- security-auditor-4: no round-2 requested (round-1 findings were explicitly out-of-Task-4 scope; no security-relevant code changed in round-2 fixes).

**Verification:**
- `cargo build -p mnemonic-mcp` → OK (pre-condition gate: Tasks 1-3 in tree).
- `cargo test -p mnemonic-mcp --features test-support --test helpers_smoke` → 5/5 pass in 1.6s wall-clock.
- `cargo test -p mnemonic-mcp --features test-support --tests` → 0 failures across all 34 test binaries (no regressions on existing tests with `with_oauth_token == false` path).
- `cargo test -p mnemonic-mcp --features test-support --lib` → 201 pass, 0 fail.
- `cargo clippy --workspace --all-targets --features 'mnemonic-mcp/test-support' -- -D warnings` → clean.
- `cargo fmt --all -- --check` → clean.
