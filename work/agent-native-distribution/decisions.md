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
