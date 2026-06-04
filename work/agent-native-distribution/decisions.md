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
