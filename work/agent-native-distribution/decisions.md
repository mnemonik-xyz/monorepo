# Decisions Log: agent-native-distribution

Agent reports on completed tasks. Each entry is written by the agent that executed the task.

---

<!-- Entries are added by agents as tasks are completed.

Format is strict — use only these sections, do not add others.
Do not include: file lists, findings tables, JSON reports, step-by-step logs.
Review details — in JSON files via links. QA report — in logs/working/.

## Task N: [title]

**Status:** Done
**Commit:** abc1234
**Agent:** [teammate name or "main agent"]
**Summary:** 1-3 sentences: what was done, key decisions. Not a file list.
**Deviations:** None / Deviated from spec: [reason], did [what].

**Reviews:**

*Round 1:*
- code-reviewer: 2 findings → [logs/working/task-N/code-reviewer-1.json]
- security-auditor: OK → [logs/working/task-N/security-auditor-1.json]

*Round 2 (after fixes):*
- code-reviewer: OK → [logs/working/task-N/code-reviewer-2.json]

**Verification:**
- `npm test` → 42 passed
- Manual check → OK

-->

## Task 1: Skill manifests + build-time projection

**Status:** Done
**Commits:** fec5d97 (impl) + ddf7255 (test-reviewer R1 fixes) + ab7380d (security-auditor R1 fixes) + 5255976 (code-reviewer R1 fixes)
**Agent:** t1-coder
**Summary:** Created seven markdown skill manifests under `mcp/assets/skills/` (`help`, `init`, `recall`, `attest`, `checkpoint`, `verify`, `status`) as the single source of truth and wired a `build.rs` that parses each manifest's `## Purpose` + `## Trigger` H2 sections at build time, emitting compile-time constants (`FULL_MARKDOWN`, `PURPOSE_PLUS_TRIGGER`, `PURPOSE_ONE_LINER`) plus an `ALL_SKILLS` table for Task 2 to project into `prompts/*`, `resources/*`, and `tools/list`. Key implementation decision: the markdown parser is shared between `build.rs` and the integration test via `mcp/src/skill_parse.rs` (include!()-d by build.rs, imported by the test as `mnemonic_mcp::skill_parse::...`) so the "missing-section fails build" guard is exercised by the exact same code at test time — drift between test and build is structurally impossible. Security hardening: `fs::symlink_metadata()` rejects symlinks in the assets dir (mirrors Decision 9's lstat discipline on the install side). Per-file `cargo:rerun-if-changed` directives emitted inside the manifest read loop so in-place edits trigger a rebuild on APFS.
**Deviations:** None.

**Reviews:**

*Round 1 (fec5d97):*
- code-reviewer: changes_requested, 2 blocking + 3 non-blocking → [logs/working/task-1/code-reviewer-round1.json](logs/working/task-1/code-reviewer-round1.json)
- security-auditor: PASS_WITH_NOTES, 2 LOW + 1 INFO → [logs/working/task-1/security-auditor-round1.json](logs/working/task-1/security-auditor-round1.json)
- test-reviewer: NEEDS_FIXES, 1 medium-blocking + 1 low + 1 info → [logs/working/task-1/test-reviewer-round1.json](logs/working/task-1/test-reviewer-round1.json)

*Round 2 (after fixes — ddf7255, ab7380d, 5255976):*
- code-reviewer: approved → [logs/working/task-1/code-reviewer-round2.json](logs/working/task-1/code-reviewer-round2.json)
- security-auditor: PASS → [logs/working/task-1/security-auditor-round2.json](logs/working/task-1/security-auditor-round2.json)
- test-reviewer: APPROVED → [logs/working/task-1/test-reviewer-round2.json](logs/working/task-1/test-reviewer-round2.json)

**Verification:**
- `cargo test -p mnemonic-mcp --test skill_manifests` → 5/5 pass (4 TDD anchors + 1 extra-file regression test)
- `cargo clippy -p mnemonic-mcp --all-targets --features test-support -- -D warnings` → clean
- Smoke (manifest missing): renaming `attest.md` → `attest.bak` yields `error: missing required skill manifest: attest.md / expected at: ...`
- Smoke (section missing): tampered `## Purpose` → `## Purposes` in help.md yields `manifest help.md manifest missing required \`## Purpose\` H2 section`
- Smoke (symlink rejection): `attest.md` as symlink → `/tmp/evil-fake-attest.md` is treated as missing (target never opened)
- Smoke (rerun-if-changed): editing `help.md` content triggers a recompile and regenerates `skills_generated.rs`

---

## Task 3: visibility column migration + Visibility enum + storage signatures

**Status:** Done
**Commit:** b5c52ca (impl) + d4eee78 (round 1 fixes)
**Agent:** t3-coder
**Summary:** Added `Visibility { Private, Public }` enum to `core/src/storage/mode.rs` alongside `WriteMode` with parallel Display/FromStr/serde/rusqlite codecs. Added idempotent `migrate_visibility_column()` mirroring `migrate_write_mode_column`'s 7-step recipe; wired into both `SqliteStore::open` and `SqliteStore::in_memory` after the write_mode migration. Extended `AttestationStore::save_attestation` with `visibility: Visibility`, extended `search` with `visibility_filter: Option<Visibility>`, added `visibility` field to `SearchResult`. Internal `mcp/` callsites pass `Visibility::Private` (privacy-by-default per AC13) and `None` for the filter (authenticated callers see all their own rows per Decision 5); Task 5 will wire the JSON-input resolver and the anonymous-recall `Some(Visibility::Public)` branch.
**Deviations:** None.

**Forward flag for Task 5 (from security-auditor):** when wiring the anonymous-recall path (no-JWT caller), the handler MUST pass `Some(Visibility::Public)` to `search`, never `None`, or AC13 is violated. Current code correctly uses `None` only from authenticated callers.

**Reviews:**

*Round 1 (b5c52ca):*
- code-reviewer: approve_with_minor, 3 optional → [logs/working/task-3/code-reviewer-round1.json](logs/working/task-3/code-reviewer-round1.json)
- security-auditor: PASS, 1 LOW + 1 INFO, no blockers → [logs/working/task-3/security-auditor-round1.json](logs/working/task-3/security-auditor-round1.json)
- test-reviewer: needs_improvement, 0 critical / 0 high / 2 medium / 1 low → [logs/working/task-3/test-reviewer-round1.json](logs/working/task-3/test-reviewer-round1.json)

*Round 2 (d4eee78 — addresses CR-T3-1/2/3, SEC-T3-01, F1/F2/F3):*
- test-reviewer: passed, all 3 findings resolved → [logs/working/task-3/test-reviewer-round2.json](logs/working/task-3/test-reviewer-round2.json)
- code-reviewer + security-auditor: round 1 verdicts already covered the merge condition; self-attestation for the optional fixes in commit `d4eee78` body.

**Verification:**
- `cargo test -p mnemonic-core --test integration_storage` → 6/6 pass (5 TDD anchors + owner-isolation under visibility filter)
- `cargo test -p mnemonic-core --lib` → 130/130 pass
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast` → all green (162 mcp tests + others)
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- Smoke (task §Verification Steps): `cargo test -p mnemonic-core integration_storage::migrate_visibility_column_idempotent_on_clean_db` → 1 passed

---

## Task 2: mcp.rs server surfaces + anonymous allowlist + tools/list enrichment

**Status:** Done
**Commits:** 88f8db6 (impl) + db57911 (round 1 code-reviewer fixes: CR2-01 placeholder arm, CR2-02 negative-path tests, CR2-03 OnceLock cache) + d52f2f5 (round 1 test-reviewer F2/F3 fixes)
**Agent:** t2-coder
**Summary:** Wired the four `prompts/*` and `resources/*` dispatch arms plus the embedder metadata block in `initialize`, enriched `tools/list` descriptions with the matching skill manifest's `Purpose+Trigger` via `skill_for_tool()`/`enrich_tool_description()` (drift-impossible: manifest body is the single source of truth), added the 7th tool entry `request_public_write_confirmation` (definition + -32601 placeholder arm pointing to Task 4's handler), and extended `ALLOWLIST_METHODS` in `oauth/mod.rs` so the four new discovery methods are anonymous-OK. Key implementation decisions: `EMBEDDER_MODEL_VERSION` is a `pub const` literal in `mcp.rs` (re-exported from `lib.rs`) because both compilation units (binary `mod mcp;` and library `pub mod mcp;`) compile the same `mcp.rs` source file; sync risk for fastembed bumps documented inline + flagged for Task 13 release checklist. `enriched_tools()` is memoized via `std::sync::OnceLock<Vec<Value>>` after code-reviewer round 1, single allocation per process.
**Deviations:** None.

**Forward flag for Task 4 (from test-reviewer F2):** `recall_owner_isolation.rs:212` carries a NOTE about the AC13/Task 4 contract change — the 401-on-anonymous-recall assertion must flip to `200 + visibility='public'` rows when Task 4 lands the visibility-filter recall path. Task 4's coder should update that assertion alongside the handler change.

**Reviews:**

*Round 1 (88f8db6):*
- code-reviewer: approve_with_minor_findings, 3 minor + 1 informational → [logs/working/task-2/code-reviewer-round1.json](logs/working/task-2/code-reviewer-round1.json)
- security-auditor: PASS → [logs/working/task-2/security-auditor-round1.json](logs/working/task-2/security-auditor-round1.json)
- test-reviewer: CONDITIONAL_PASS, 2 required + 1 optional → [logs/working/task-2/test-reviewer-round1.json](logs/working/task-2/test-reviewer-round1.json)

*Round 2 (db57911 + d52f2f5):*
- code-reviewer: APPROVED → [logs/working/task-2/code-reviewer-round2.json](logs/working/task-2/code-reviewer-round2.json)
- security-auditor: PASS → [logs/working/task-2/security-auditor-round2.json](logs/working/task-2/security-auditor-round2.json)
- test-reviewer: PASS → [logs/working/task-2/test-reviewer-round2.json](logs/working/task-2/test-reviewer-round2.json)

**Verification:**
- `cargo test -p mnemonic-mcp --features test-support --test discovery_anonymous` → 8/8 pass (6 TDD anchors + 2 negative-path tests added in round 2)
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast` → green
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- Smoke (live curl loop) skipped — integration tests exercise the same dispatcher arms through the same axum Router + oauth middleware stack as the production handler; the only thing live smoke would add is a fastembed model_id roundtrip, and the integration test's `mock_state()` calls the same `Embedder::model_id()` trait method the dispatcher uses.

## Task 6: Token-file access for Rust binary + TokenExpired typed error

**Status:** Done
**Commits:** 604fbf5 (impl) + f2faef6 (review round 1 fixes) + 5aa0533 (review round 2 fixes)
**Agent:** t6-coder
**Summary:** Added `core::identity::token_store` — file-backed read/save/delete for `~/.mnemonic/token.json` with the same on-disk shape as the Node CLI (`packages/cli/src/config.ts:39-65`: `{jwt, expires_at: ISO-8601 string, sub}`). Atomic write via `NamedTempFile` + `persist()`, mode 0600 on file + 0700 on parent dir (Unix), `MNEMONIC_CONFIG_DIR` env override for parity with Node CLI. Malformed JSON degrades to `Ok(None)` so the binary re-OAuths rather than crashes; an unparseable timestamp surfaces as `Err(Expired)` so the caller refreshes rather than silently accepting an undated token. Wired the MCP `token_handler` to cache freshly-minted JWTs (best-effort, never fails the OAuth response); on-disk `expires_at` is decoded from the JWT's own `exp` claim via `extract_exp_unix_no_verify` to avoid clock skew with `Utc::now()`. Added `mcp::mcp::token_expired` typed JSON-RPC error `-32099 TokenExpired { kind, expires_at, pubkey }` for the JSON-RPC boundary.
**Deviations:**
- Deviated from spec (V1 scope reduction, documented in code): the task's TDD anchor for `mcp/tests/oauth_loopback.rs` describes a full agent-side OAuth-loopback flow with mock OAuth server and "mock call count == 1" invariant. The Rust binary does NOT act as an OAuth client in V1 per code-research §5 (Node CLI only). The mcp tests instead exercise the actual server-side callsites this task introduced: `cache_minted_token` after JWT mint, `read_token_from` returning `Err(Expired)` and the `token_expired` JSON-RPC helper, and the malformed→`Ok(None)` degradation. The "second-call cache reuse" half is asserted via a second `read_token_from` returning the byte-identical JWT.
- Deferred to Task 5 (documented in code at `mcp/src/mcp.rs:438` and `mcp/tests/oauth_loopback.rs` header): the production callsite that maps `TokenStoreError::Expired` from `mnemonic_core::identity::read_token` to `-32099 TokenExpired` lives at the outbound participate-mode proxy (mcp-stdio's `MNEMONIC_HOSTED_ENDPOINT` path), which Task 5 wires. `token_expired` carries `#[allow(dead_code)]` with a doc comment pointing to the deferred wiring. Round-1 code-reviewer R1-MAJOR-1 acknowledged this deferral.
- Deviated from tech-spec line 332 ("returns None" for unparseable expires_at): implementation returns `Err(Expired)` per the task TDD anchor `expired_token_returns_expired_error`. Rationale documented inline at `core/src/identity/token_store.rs:127`: "I don't know when this expires" is safer than "assume valid" — force a re-OAuth rather than silently accept an undated token. Round-1 test-reviewer F5 acknowledged the choice and asked for the documentation, now in place.

**Reviews:**

*Round 1 (604fbf5):*
- code-reviewer: REVISE, 2 major + 4 minor → [logs/working/task-6/code-reviewer-round1.json](logs/working/task-6/code-reviewer-round1.json)
- security-auditor: CONDITIONAL_PASS, 1 medium-blocking + 2 low → [logs/working/task-6/security-auditor-round1.json](logs/working/task-6/security-auditor-round1.json)
- test-reviewer: APPROVE_WITH_REQUIRED_FIXES, 2 required + 4 advisory → [logs/working/task-6/test-reviewer-round1.json](logs/working/task-6/test-reviewer-round1.json)

*Round 2 (f2faef6 + 5aa0533):*
- code-reviewer: APPROVE_WITH_NOTES, 0 blockers + 3 non-blocking notes (all addressed in 5aa0533) → [logs/working/task-6/code-reviewer-round2.json](logs/working/task-6/code-reviewer-round2.json)
- security-auditor: PASS → [logs/working/task-6/security-auditor-round2.json](logs/working/task-6/security-auditor-round2.json)
- test-reviewer: APPROVED → [logs/working/task-6/test-reviewer-round2.json](logs/working/task-6/test-reviewer-round2.json)

**Forward flag for Task 5:** the outbound participate-mode proxy callsite must call `mnemonic_core::identity::read_token()` and map `TokenStoreError::Expired` to the existing `mcp::mcp::token_expired(expires_at, sub)` JSON-RPC helper. Drop the `#[allow(dead_code)]` at `mcp/src/mcp.rs:446` once the wiring lands. AC11 ("subsequent writes within TTL do not re-trigger loopback") becomes structurally testable at that point.

**Verification:**
- `cargo test -p mnemonic-core --test integration_token` → 8/8 pass
- `cargo test -p mnemonic-core --lib token_store` → 6/6 pass (5 round-1 unit tests + `config_dir_override_routes_through_token_path` added in round-2 follow-up)
- `cargo test -p mnemonic-mcp --features test-support --test oauth_loopback` → 4/4 pass
- `cargo clippy -p mnemonic-core --all-targets -- -D warnings` → clean
- `cargo fmt -p mnemonic-core -- --check` → clean
- AC11 keychain move deferred to v1.1 per task spec post-completion checklist.

## Task 4: sign_memory visibility + HMAC public-write gate + anonymous recall filter

**Status:** Done
**Commits:** 06e63d8 (impl) + 163706f (review round 1 fixes) + 54ad816 (review round 2 fixes)
**Agent:** t4-coder
**Summary:** Wired `Visibility` through `sign_memory` → `save_attestation`; new `resolve_visibility` + `resolve_allow_fallback` resolvers reject `mode=local + visibility=...` at the dispatcher (AC14). Built `ConfirmationLedger` (DashMap-backed HMAC-SHA256 over `content_hash || owner_pubkey || visibility || expires_at || jti`, single-use via `remove_if`, 5-min TTL + 60s background eviction). Anonymous `tools/call mnemonic_recall` allowlisted in `bearer_auth_middleware` via new `ALLOWLIST_TOOLS_CALL_NAMES`; dispatcher passes `(None, Some(Public))` to `search` so the cross-owner public pool surfaces (AC13) without leaking private rows. Extended `core::storage::AttestationStore::search` signature to `Option<&str>` owner with a defensive `(None, None) → Vec::new()` guard. Error catalogue helpers added for `-32095 PublicWriteRequiresConfirmation`, `-32098 EmbedderInvalid`, plus dead-code-allowed placeholders for the Task 5/6 typed errors (`-32096`, `-32099 LocalStorageBusy`, `-32094`, `-32011 HostedUnavailable`).
**Deviations:**
- Deviated from spec (round-2 / SAR1-M1 security-blocker fix): the round-1 implementation scoped anonymous recall to the dispatcher's owner-fallback (server keypair), hiding cross-user public rows. Round 3 changed `AttestationStore::search`'s `owner_pubkey` from `&str` to `Option<&str>` and added `SEARCH_SQL_CROSS_OWNER_VIS` (`WHERE owner_pubkey IS NOT NULL AND visibility = ?`) so anonymous recall surfaces every owner's public rows per user-spec AC13/Flow 4. Cross-owner test `cross_owner_public_pool_visible` pins the behaviour.
- Added `subtle = "2"` as a direct dep (already transitively present via hmac/sha2) so the consume-path constant-time compare uses `subtle::ConstantTimeEq::ct_eq` instead of a hand-rolled XOR loop (code-reviewer CR-2 / security-auditor SAR1-L1, round 1).
- Added 64-char ASCII-hex format validation on `request_public_write_confirmation::content_hash` at the dispatcher boundary (security-auditor SAR1-L2, round 1) — closes a DashMap-spam DoS vector for authenticated callers.
- `_allow_fallback` parsed at the dispatcher but unused in Task 4 scope; Task 5 wires the soft-fall router in `tools::sign_memory_inline`.
- Some error catalogue helpers (`oauth_timeout`, `local_storage_busy`, `identity_bootstrap_failed`, `hosted_unavailable`) carry `#[allow(dead_code)]` because the production triggers live in Tasks 5/6 (`mcp-stdio` outbound proxy + token-store integration). `error_catalogue.rs::catalogue_typed_helpers_pin_data_shapes` pins the wire shapes for those rows; Task 4 owns the 5 in-scope rows triggered via production code paths.

**Reviews:**

*Round 1 (06e63d8):*
- code-reviewer: REVISE, 1 major + 2 minor + 2 info → [logs/working/task-4/code-reviewer-round1.json](logs/working/task-4/code-reviewer-round1.json)
- security-auditor: CONDITIONAL_PASS, 1 medium-blocking + 2 low + 2 info → [logs/working/task-4/security-auditor-round1.json](logs/working/task-4/security-auditor-round1.json)
- test-reviewer: APPROVE_WITH_NOTES, 3 minor + 2 notes (no blocking) → [logs/working/task-4/test-reviewer-round1.json](logs/working/task-4/test-reviewer-round1.json)

*Round 2 (163706f + 54ad816):*
- code-reviewer: APPROVED → [logs/working/task-4/code-reviewer-round2.json](logs/working/task-4/code-reviewer-round2.json)
- security-auditor: PASS (SAR1-M1 resolved) → [logs/working/task-4/security-auditor-round2.json](logs/working/task-4/security-auditor-round2.json)
- test-reviewer: APPROVE → [logs/working/task-4/test-reviewer-round2.json](logs/working/task-4/test-reviewer-round2.json)

**Forward flag for Task 5:** the soft-fall router must consume the already-parsed `allow_fallback_to_participate` value. Task 5 wires `tools::sign_memory_inline`'s post-failure branch so that when `allow_fallback_to_participate=true` and local execution fails (`-32098 EmbedderInvalid` etc.), `sign_memory` re-dispatches through `MNEMONIC_HOSTED_ENDPOINT` and the response carries `escalated: { from, to, reason }` per Decision 4. Visibility resolution runs AGAIN post-escalation so the public-write confirmation gate from Task 4 still fires; the `-32011 HostedUnavailable` helper (currently `#[allow(dead_code)]`) is the canonical typed error for hosted-unreachable on the escalation path. AC11 / AC15 / R1 become structurally testable once the wiring lands.

**Verification:**
- `cargo test -p mnemonic-mcp --features test-support --lib confirmation_token` → 7/7 pass
- `cargo test -p mnemonic-mcp --features test-support --test sign_memory_visibility` → 5/5 pass
- `cargo test -p mnemonic-mcp --features test-support --test anonymous_recall` → 3/3 pass
- `cargo test -p mnemonic-mcp --features test-support --test confirmation_gate` → 10/10 pass
- `cargo test -p mnemonic-mcp --features test-support --test error_catalogue` → 6/6 pass
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast` → 0 failures
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- Smoke (live curl loop) skipped — the integration tests drive the production dispatcher + middleware + storage stack through the same `axum::Router` + `oauth::bearer_auth_middleware` wiring as the binary. No new external endpoints were added.

## Task 5: mcp-stdio + logout subcommands on Rust binary + MNEMONIC_HOSTED_ENDPOINT gating + soft-fall routing

**Status:** Done
**Commits:** 997f503 (impl) + 158ddc9 (round 2 fixes) + e26c99d (round 3 fixes)
**Agent:** t5-coder
**Summary:** Added `mcp-stdio` subcommand wired to the existing `run_stdio()` path (reusing the `Arc<McpState>` singleton) and `logout` subcommand that deletes the token file before any `McpState` build. Implemented Decision 12 `--allow-custom-endpoint` global clap flag gating `MNEMONIC_HOSTED_ENDPOINT` with `is_safe_hosted_endpoint()` URL validation (rejects non-HTTPS non-loopback, file://, cloud-metadata IPs, lookalike hosts, userinfo credentials) returning a typed `HostedEndpointWarning` enum. Wired soft-fall routing in `tools::sign_memory_inline` per Decision 4 (three conditions: `allow_fallback=true`, error code in soft-fallable catalogue, non-empty hosted endpoint) via `proxy_participate`, which strips `allow_fallback_to_participate` from proxied args, injects the `escalated` response field, maps `TokenStoreError::Expired` to `-32099 TokenExpired` before any HTTP call, scrubs URL/credentials from `reqwest` error messages, and guards against malformed hosted responses with `-32011 HostedUnavailable`.
**Deviations:** None.

**Reviews:**

*Round 1 (997f503):*
- code-reviewer: approve_with_minor_fixes, 4 minor → [logs/working/task-5/code-reviewer-round1.json](logs/working/task-5/code-reviewer-round1.json)
- security-auditor: CONDITIONAL_PASS, 1 medium-blocking (SAR5-M1) + 2 low + 1 info + 2 others → [logs/working/task-5/security-auditor-round1.json](logs/working/task-5/security-auditor-round1.json)
- test-reviewer: needs_improvement, 5 findings → [logs/working/task-5/test-reviewer-round1.json](logs/working/task-5/test-reviewer-round1.json)

*Round 2 (158ddc9):*
- code-reviewer: approve_with_minor_fixes, R1-001/R1-002/R1-004 deferred, R1-003 fixed, 1 new minor doc defect (R2-001) → [logs/working/task-5/code-reviewer-round2.json](logs/working/task-5/code-reviewer-round2.json)
- security-auditor: PASS → [logs/working/task-5/security-auditor-round2.json](logs/working/task-5/security-auditor-round2.json)
- test-reviewer: passed → [logs/working/task-5/test-reviewer-round2.json](logs/working/task-5/test-reviewer-round2.json)

*Round 3 (e26c99d):*
- code-reviewer: approved — all R1 findings resolved; R2-001 (error_catalogue.rs dim comment) accepted as known technical debt → [logs/working/task-5/code-reviewer-round3.json](logs/working/task-5/code-reviewer-round3.json)
- security-auditor: PASS — SAR5-M1/L1/L2/INFO3 all closed, no new findings → [logs/working/task-5/security-auditor-round3.json](logs/working/task-5/security-auditor-round3.json)
- test-reviewer: passed — all 4 new round-3 tests correct, env-mutex serialisation sound → [logs/working/task-5/test-reviewer-round3.json](logs/working/task-5/test-reviewer-round3.json)

**Forward flag:** None. The `#[allow(dead_code)]` on `token_expired()` in `mcp/src/mcp.rs` was dropped in round 3 (helper is now live via `proxy_participate`). R2-001 (wrong dim in error_catalogue.rs comment) is open minor documentation debt with no runtime impact.

**Verification:**
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast` → green (per coder round 3 report)
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- Release-binary smoke: `mcp-stdio` subcommand and `logout` subcommand verified via spawned-binary tests in `mcp/tests/cli_subcommands.rs`

## Task 7: @mnemonik-xyz/mcp npm shim — lazy install, host-config wiring, doctor

**Status:** Done
**Commits:** 4e2e111 (impl) + c0fc702 (code-reviewer R1 fixes) + fbc7f38 (test-reviewer R1 fixes) + 427ab4c (security-auditor R1 SAR7-M2 fix)
**Agent:** t7-coder
**Summary:** New `packages/mcp/` npm workspace shipping the `mnemonik-mcp` Node bin. No `postinstall` (Decision 8) — first invocation lazy-installs the matching Rust binary from GitHub Releases via `ensureBinaryCached`: SHA256 verified against `SHA256SUMS`, `gh attestation verify` pinned to `--owner mnemonik-xyz --repo mnemonik-xyz/monorepo --signer-workflow .github/workflows/release.yml` (fails closed on missing `gh`), zip-slip-hardened `tar.extract` (rejects `..`/absolute paths and SymbolicLink + Link entry types), tar dep pinned to `^7.4.0` (CVE-2021-32803/04 line). Manifest sidecar records the cached binary's on-disk SHA256 (used by doctor's `binary-integrity` check — no re-download, closes the Round-1 audit's circular-trust finding) AND the original SHA256SUMS entry for audit. `install` subcommand (Decision 9) merges `mcpServers.mnemonik` into any present `~/.claude.json`, Claude Desktop, or `~/.cursor/mcp.json` with `lstat`-based symlink-out-of-home refusal, atomic temp+rename, per-candidate atomicity, idempotent, output ends with the exact AC9 restart-instruction line. `install --check` is mtime-stable dry-run (restart line suppressed). `doctor` runs 6 diagnostics (host config presence, /health, binary integrity, local config dir r/w, identity, token) with repair hints. `MNEMONIK_MCP_RELEASE_BASE_URL` validated as `https://` only; `MNEMONIK_MCP_ALLOW_HTTP=1` is the test-only escape hatch (analogous to Rust Decision 12).

**Deviations:** None.

**Reviews:**

*Round 1 (4e2e111):*
- code-reviewer: request_changes, 1 major + 2 minor → [logs/working/task-7/code-reviewer-round1.json](logs/working/task-7/code-reviewer-round1.json)
- security-auditor: CONDITIONAL_PASS, 2 medium + 2 low → [logs/working/task-7/security-auditor-round1.json](logs/working/task-7/security-auditor-round1.json)
- test-reviewer: conditional_pass, 2 required + 1 recommended + 2 advisory → [logs/working/task-7/test-reviewer-round1.json](logs/working/task-7/test-reviewer-round1.json)

*Round 2 (c0fc702 + fbc7f38 + 427ab4c):*
- code-reviewer: approved → [logs/working/task-7/code-reviewer-round2.json](logs/working/task-7/code-reviewer-round2.json)
- security-auditor: PASS → [logs/working/task-7/security-auditor-round2.json](logs/working/task-7/security-auditor-round2.json)
- test-reviewer: pass → [logs/working/task-7/test-reviewer-round2.json](logs/working/task-7/test-reviewer-round2.json)

**Verification:**
- `cd packages/mcp && npx vitest run` → 27/27 pass (15 TDD anchors + 12 supplemental incl. 6 new SAR7-M2 URL-validation tests)
- `cd packages/mcp && npx tsc --noEmit` → clean
- `cd packages/mcp && npm run build` → emits `dist/bin/mnemonik-mcp.js`
- Smoke (task §Verification Steps): tempdir-HOME `install --check` mtime-stable; `install` adds `mcpServers.mnemonik` while preserving unrelated keys; restart line printed only in apply mode.
- Tar version pinned: `^7.4.0` (CVE-2021-32803/04 mitigation).
- `gh` CLI required at runtime — README documents the install link.

## Task 8: release.yml SHA256SUMS + GitHub artifact attestation + @mnemonik-xyz/mcp publish

**Status:** Done
**Commits:** f865d51 (impl) + 7c3eba5 (round 1 review fixes: DR8-M1 concurrency, SAR8-M1 normalization guard, SAR8-M2 attestation scope docs, SAR8-INFO1 + DR8-L1 comment expansion)
**Agent:** t8-coder
**Summary:** Extended `.github/workflows/release.yml` with three additions for the npm shim distribution path. (1) New `Generate SHA256SUMS` step in the `release` job: `find` + `sha256sum -b` over every `mnemonic-mcp-*.tar.gz` artifact, sed-normalized to strip the `download-artifact@v4` subdirectory prefix, then a post-normalization guard (`grep -q '/' && exit 1`) that fails the release at CI time if a future build-matrix layout change produces nested artifact paths (instead of silently breaking every install). Published to the GitHub Release as a separate asset. (2) `actions/attest-build-provenance@v1` step over the same tarball glob; `release` job permissions block now includes `id-token: write` + `attestations: write` alongside the existing `contents: write` (minimal sigstore surface). Tarball-only `subject-path` is intentional per Decision 8 — the shim's `install-binary.ts` runs `gh attestation verify <tarball>` BEFORE extraction. (3) New `publish-mcp-shim` job mirroring `publish-npm` (Trusted Publishing via OIDC — no NPM_TOKEN, `--access public --provenance`, skip-if-already-published guard, `if: startsWith(github.ref, 'refs/tags/v')` gate). Gated `needs: [release]` so SHA256SUMS + attestation are visible on the GitHub Release BEFORE the npm shim is pullable (`ensureBinaryCached` would otherwise 404 on first install). Intentionally NOT gated on `publish-npm`: `packages/mcp` has zero workspace dependency on `@mnemonik-xyz/sdk` or `@mnemonik-xyz/cli`. Workflow-level `concurrency: release-${{ github.ref }}` block with `cancel-in-progress: true` prevents duplicate/force-pushed tag refs from racing on Release asset uploads or OIDC token reuse. Existing `publish-npm` job byte-for-byte unchanged.

**Deviations:** None.

**Reviews:**

*Round 1 (f865d51):*
- code-reviewer: approved_with_minor — CR8-001 (sed nesting-depth assumption, doc clarification) + CR8-002 (no-action) → [logs/working/task-8/code-reviewer-round1.json](logs/working/task-8/code-reviewer-round1.json)
- security-auditor: APPROVE_WITH_FINDINGS — SAR8-M1 (medium, post-norm guard), SAR8-M2 (medium, attest scope docs), SAR8-INFO1 (info) → [logs/working/task-8/security-auditor-round1.json](logs/working/task-8/security-auditor-round1.json)
- deploy-reviewer: CONDITIONAL_PASS — DR8-M1 (medium, blocking — concurrency group), DR8-L1 (low — sed comment), 2 INFO → [logs/working/task-8/deploy-reviewer-round1.json](logs/working/task-8/deploy-reviewer-round1.json)

*Round 2 (7c3eba5):*
- code-reviewer: approved — CR8-001 fully resolved; one CR8-R2-INFO1 (empty SHA256SUMS — structurally impossible per `needs: [build-linux, build-macos]`) accepted as non-actionable → [logs/working/task-8/code-reviewer-round2.json](logs/working/task-8/code-reviewer-round2.json)
- security-auditor: APPROVED — all three R1 findings closed, concurrency block recognized as net security improvement → [logs/working/task-8/security-auditor-round2.json](logs/working/task-8/security-auditor-round2.json)
- deploy-reviewer: PASS — DR8-M1 + DR8-L1 closed, only the same 3 pre-existing actionlint warnings remain in untouched build-linux/build-macos jobs → [logs/working/task-8/deploy-reviewer-round2.json](logs/working/task-8/deploy-reviewer-round2.json)

**Verification:**
- `actionlint .github/workflows/release.yml` → 3 warnings, all pre-existing (lines 68, 91, 105 in build-linux/build-macos; confirmed via `git stash` baseline diff). Zero new findings introduced by this diff.
- SHA256SUMS smoke test (per task §Verification Steps): fixture tarball + sha256sum -b + sed normalization produces parseable line `^[0-9a-f]{64} \*mnemonic-mcp-.+\.tar\.gz$`.
- SAR8-M1 guard regression smoke: simulated nested layout (`artifacts/macos/aarch64-apple-darwin/<tarball>`) correctly trips `exit 1` with the leftover `aarch64-apple-darwin/` prefix in the stderr error output.
- Structural YAML parse confirms job DAG: `build-linux`, `build-macos` → `release` → `publish-mcp-shim`; `publish-npm` runs in parallel (decoupled from binary builds).
- Existing `publish-npm` job sed-extracted and diffed against `HEAD~2:.github/workflows/release.yml` — byte-for-byte identical.

**Forward flag:** Before the first tagged release (Task 13 fires), `@mnemonik-xyz/mcp` must be pre-registered on npm.com with the same Trusted-Publisher OIDC configuration as the existing `@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli` packages (publisher: `mnemonik-xyz/monorepo`, workflow: `.github/workflows/release.yml`, environment: none). Without this, the first `npm publish --provenance` will 403. DR8-INFO1 and the task post-completion checklist both call this out — log it explicitly in Task 13's deployment runbook.
