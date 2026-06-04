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
