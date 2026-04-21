# Decisions Log: mnemonic-core

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

## Task 1: Workspace scaffold + turboquant migration

**Status:** Done
**Commit:** 1f26fba
**Agent:** main agent
**Summary:** Created the Cargo workspace root with `members = ["core", "mcp"]`, scaffolded `mnemonic-core` as a library crate with 8 empty `pub mod` stubs and corresponding `mod.rs` files, copied `mnemonic-protocol/mcp/` into the workspace, replaced the `turboquant` git dependency with `turboquant-plus-rs = "0.1.0"` from crates.io, and updated the two `turboquant::` import paths in `compress.rs` to `turboquant_plus_rs::`. Also removed orphan `.git` gitfile from `mcp/` that was blocking `git add`.
**Deviations:** None.

**Reviews:**

*Round 1:*
- code-reviewer: 3 minor findings (unused import warning potential, compression ratio assertion fragility, `from_bytes_legacy_v1` dead-code risk) → [logs/working/task-1/code-reviewer-1.json]
- security-auditor: 4 major pre-existing findings (OpenAI API key in memory, SQLite no-encryption, no rate limiting, Arweave tx exposure) — all pre-existing, none introduced → [logs/working/task-1/security-auditor-1.json]
- test-reviewer: approved, no new tests required for scaffold task → [logs/working/task-1/test-reviewer-1.json]

**Verification:**
- `cargo build -p mnemonic-core` → Finished (0 errors)
- `cargo build -p mnemonic-mcp` → Finished (41 warnings, 0 errors)
- `cargo test -p mnemonic-mcp` → 59 passed
