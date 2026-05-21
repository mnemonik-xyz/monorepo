# Decisions Log: invisible-identity

Append-only. Each entry is a design resolution or a task outcome. Format follows the project's `decisions.md` convention (`## Task N: [title]` for task entries, `## YYYY-MM-DD — [topic]` for cross-cutting notes).

Reviews and audit findings link out to `logs/working/task-N/` JSON reports (populated by agents during implementation).

---

## 2026-05-21 — Design phase resolved

**Status:** Frozen for implementation
**Commits:**
- user-spec: aea3975
- tech-spec: 747666b
- deviations resolved: 9e60f9a
**Authors:** main agent + user

**Summary:** Feature scope and architecture pinned. 15 design decisions and 5 user-spec deviations resolved before any implementation task begins. `work/keypair-sync/` absorbed; deferred archival to ship time (Decision 14).

### Design decisions (captured in tech-spec.md §Decisions)

| # | Decision | Tag | Status |
|---|---|---|---|
| 1 | Keychain coords `service=xyz.mnemonik.identity`, `account=default` | TECHNICAL | resolved |
| 2 | `did:sol:` remains default DID format | USER | resolved |
| 3 | Keychain access is lazy and silent at bootstrap | TECHNICAL | resolved |
| 4 | File-fallback triggers on any keychain error, not just unavailability | TECHNICAL | resolved |
| 5 | Disk layout — `identity.json` (stub or legacy) + `README.txt`, no separate `.pub` file | TECHNICAL + USER | resolved |
| 6 | Migration is in-place, idempotent, no `.bak` rename; atomic rollback on partial failure | SECURITY | resolved |
| 7 | Single stderr line on creation, none on subsequent runs | USER | resolved |
| 8 | No new Rust user CLI; extend Node `@mnemonik-xyz/cli` only | USER | resolved |
| 9 | Node keychain library = `@napi-rs/keyring` | TECHNICAL | resolved |
| 10 | Rust keychain library = `keyring` crate | TECHNICAL | resolved |
| 11 | Drift-detector reads only local state (no network) | TECHNICAL | resolved |
| 12 | Ticket protocol = x25519-wrapped, single-use, 5min TTL, server is momentary broker | SECURITY | resolved |
| 13 | Keychain entry inner format = legacy JSON, byte-equal across languages | TECHNICAL | resolved |
| 14 | `work/keypair-sync/` archived to `work/completed/keypair-sync/` at ship time | PROCESS | resolved |
| 15 | JWT-baked deeplinks for Cursor / VS Code / Claude Desktop install buttons | USER | resolved |

Full text of each decision, including rationale and rejected alternatives, lives in `tech-spec.md §Decisions`. This log is the audit trail of *when* and *by whom*; tech-spec is the source of truth for *what*.

### Deviation resolutions (captured in tech-spec.md §User-Spec Deviations)

All five deviations resolved in chat on 2026-05-21:

| # | Deviation | Resolution |
|---|---|---|
| 1 | Lazy keychain access at bootstrap | ACCEPTED as written |
| 2 | Ticket re-wrap broker (momentary server plaintext) | ACCEPTED; trust model pinned in spec |
| 3 | Promote `crypto_box` + `@noble/curves` to direct deps | ACCEPTED as written |
| 4 | Remove `load_or_create_keypair` immediately, no deprecation cycle | ACCEPTED as adjusted (initial draft kept a deprecation shim; resolved to remove now since project is pre-1.0 with no external Rust consumers) |
| 5 | Webapp install-page UA detection | ACCEPTED as softened (sort-not-hide, ~5 LOC, falls back to original order on detection failure) |

**Deviations:** None beyond the five above; all merged into `tech-spec.md` as `[ACCEPTED]`.

**Verification:**
- `wc -l work/invisible-identity/{user-spec,tech-spec}.md` → 327 + 551 lines
- `grep -c "PENDING USER APPROVAL" work/invisible-identity/tech-spec.md` → 1 (historical context note only, no live unresolved markers)
- `git log --oneline work/invisible-identity/` → 3 commits, branch `claude/analyze-portable-agent-memory-nYOJm`

**Concerns / follow-ups for implementation waves:**

1. **Wave 1 dependency check (TECHNICAL):** verify `keyring = "2"` and `crypto_box = "0.9"` actually publish current versions on crates.io before Task 1; bump version pins if newer majors exist. `cargo search` is the gate.
2. **Wave 2 dependency check (TECHNICAL):** verify `@napi-rs/keyring` prebuilt binaries exist for the CI matrix platforms (darwin-arm64, darwin-x64, linux-x64-gnu, linux-arm64-gnu, win32-x64-msvc) before Task 5; fall back to building-from-source path if any are missing.
3. **Wave 3 interop coverage gap (TECHNICAL):** Linux+gnome-keyring container exercises Secret Service path; macOS Keychain and Windows Credential Manager cannot be exercised in CI without paid runners — manual smoke matrix (Wave 5) is the only coverage. Document this trade-off.
4. **Wave 4 server endpoint precondition (PROCESS):** Task 12 (`/api/cli-bootstrap/issue-from-cli`) requires that `work/mnemonic-cli/`'s Deviation 2 endpoints (`/api/cli-bootstrap/{issue,redeem}`) are already shipped. Verify status before starting Wave 4. If `mnemonic-cli` hasn't shipped that endpoint pair yet, either (a) block on it, or (b) absorb the endpoint pair into this feature's Task 12.
5. **Wave 5 archival timing (PROCESS):** `work/keypair-sync/` archival (Task 18) executes only after this feature passes pre-deploy QA (Task 19). Do not archive earlier — `keypair-sync` may still be referenced by in-flight work outside this branch.

---

<!-- Task entries are appended below by agents as work completes.

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
- `cargo test -p mnemonic-core` → 42 passed
- Manual check → OK

-->
