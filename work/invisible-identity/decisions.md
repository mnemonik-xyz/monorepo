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

## Task 1: Add keyring + crypto_box deps

**Status:** Done
**Commit:** b01a2ab (initial), c05b62a (security-fix round 2)
**Agent:** task-1-rust-deps (impl) + code-reviewer + security-auditor (reviews)
**Summary:** Added `keyring = { version = "3", default-features = false, features = ["apple-native", "windows-native", "sync-secret-service"] }` and `crypto_box = "0.9"` to `core/Cargo.toml`. Compile gate only — no functional code. Build, clippy, fmt all green for lib/bin targets (pre-existing `--all-targets` clippy errors in `mcp/tests/` confirmed unchanged by this task).
**Deviations:** Tech-spec §Dependencies pinned `keyring = "2"`. Per `decisions.md` follow-up 1 ("bump version pins if newer majors exist"), evaluated v2/v3/v4. Chose **v3.6.3** (latest of traditional-API line). Rejected v4.0.1: it is a meta CLI/sample crate with no `[features]` section, unconditionally depends on `db-keystore` → `turso` (libSQL) + `tantivy` + `mimalloc` (~150 unrelated transitive crates on desktop targets). Round 1 security-auditor flagged this as HIGH; round 2 fix (commit c05b62a) downgraded to v3 and explicitly selected native-backend features, eliminating the libSQL/tantivy tree (net Cargo.lock delta vs round 1: -185 packages, +12). Tech-spec §Dependencies should be updated to say `keyring = "3"` next time the spec is touched.

**Reviews:**

*Round 1:*
- code-reviewer: not run (initial spawn used wrong subagent name)
- security-auditor: 1 HIGH (keyring v4 db-keystore unconditional dep) + 1 medium + 1 low + 3 info → [logs/working/task-1/security-auditor-round1.json]

*Round 2 (after c05b62a):*
- code-reviewer: OK (1 minor doc nit re: decisions.md entry, addressed by this entry; 4 info) → [logs/working/task-1/code-reviewer-round1.json]
- security-auditor: OK (round 1 HIGH resolved; 1 low documentation nit) → [logs/working/task-1/security-auditor-round2.json]

**Verification:**
- `cargo build --workspace` → clean (4s incremental)
- `cargo clippy` lib/bin targets → clean (`--all-targets` flagged pre-existing `mcp/tests/` integration-test errors, unrelated to this task)
- `cargo fmt --all -- --check` → clean

---

## Task 2: KeyStore trait + Os/File/Memory impls

**Status:** Done
**Commits:** 9decc93 (impl), 8ba3b93 (round 2 fixes), 3882f06 (round 3 + tests)
**Agent:** task-2-keystore (impl) + code-reviewer / security-auditor / test-reviewer
**Summary:** Added `KeyStore` trait with three impls in `core/src/identity/keystore{,_os,_file,_memory}.rs`. Trait is `Send + Sync` with `get`/`set`/`remove`/`available`/`name`. `KeystoreEntry` uses hand-rolled `Serialize` for byte-equal JSON output `{"secret":[...64...],"pubkey_base58":"..."}` per Decision 13 (verified by golden-bytes unit test). `FileKeyStore` uses `tempfile::NamedTempFile::new_in(parent)` + `sync_all()` + `persist()` for atomic write; mode 0600 on Unix via explicit `set_permissions(0o600)` before write. `OsKeyStore::available()` is lazy and does NOT call `get_password()` (Decision 3: no keychain prompts at bootstrap). `MemoryKeyStore` is `#[cfg(test)]`-only. 21 unit tests pass; 1 ignored OS-keychain roundtrip (Wave 3 Task 9 will opt-in).

**Deviations:**
- Promoted `tempfile` from `[dev-dependencies]` to `[dependencies]` — required by production `FileKeyStore::set`. Removed the stale dev-dep entry.
- Added `KeystoreError::PlatformUnavailable { reason }` instead of treating it as a unit variant — needed to carry headless-Linux's reason string per Decision 4 (file-fallback's stderr line includes the cause).
- `keyring` 3.x API renamed `PlatformFailure` → `NoStorageAccess` from the v2 API the spec was drafted against. `OsKeyStore` maps `NoStorageAccess` → `PlatformUnavailable` consistently across `get`/`set`/`remove`/`available`.

**Reviews:**

*Round 1:*
- code-reviewer: needs_fixes — 2 medium (`available()` not lazy; inconsistent `NoStorageAccess` mapping in `set`/`remove`), 4 minor (module-doc `///` vs `//!`, `tempfile` dup, missing `sync_all`, empty-parent edge case) → [logs/working/task-2/code-reviewer-round1.json]
- security-auditor: OK — 2 low (Zeroize on intermediate String/Value; sync_all before persist), 9 info (Debug redaction verified; `KeystoreError` carries no secret; `keyring 3.6.3` Display impls audited; `NamedTempFile` default 0o600 on Unix confirmed). Deferred items: Zeroize hardening → Wave 5. → [logs/working/task-2/security-auditor-round1.json]
- test-reviewer: OK — 13/13 required tests present, 6 useful extras, 0 noise. 3 low (missing overwrite + garbage-JSON tests) → [logs/working/task-2/test-reviewer-round1.json]

*Round 2 (8ba3b93):* Lazy `OsKeyStore::available()` (Decision 3), consistent `NoStorageAccess` mapping, `//!` module docs, dedup `tempfile`. Skipped agent re-review on the targeted fixes; verified by re-running gates.

*Round 3 (3882f06):* `sync_all()` before `persist()`; 3 new tests (`set_overwrites_existing_entry` × 2 + `get_on_garbage_json_returns_err`). Test count 18 → 21.

**Verification:**
- `cargo test -p mnemonic-core --lib identity::keystore` → 21 passed, 1 ignored
- `cargo clippy -p mnemonic-core --lib --tests -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- Golden JSON test asserts byte-exact `{"secret":[0,0,...,0],"pubkey_base58":"test"}`

---

## Task 3: identity::ensure() 5-path bootstrap

**Status:** Done
**Commit:** ff3a9fe
**Agent:** task-3-ensure (impl, timed out before commit; team lead finished — added Debug redaction, removed dead test helper, fixed clippy ptr_arg / Path import, ran gates, committed)
**Summary:** Implements the five-path bootstrap algorithm exactly per tech-spec §Architecture in `core/src/identity/ensure.rs` (~830 lines, 6 unit tests). Introduces `Identity { keypair, pubkey_base58, created_at, storage }` with hand-rolled secret-redacted Debug impl, and `IdentityStorage { OsKeychain, File }`. Removes `load_or_create_keypair` (Decision 4 / Deviation 4 — immediate removal, no deprecation shim). Rollback guard via Drop: keychain entry is removed if stub-file write fails. `MNEMONIC_QUIET=1` suppresses the single stderr line on creation (Decision 7).

**Deviations:**
- Eager keypair cache in `Identity` (per task §Notes — Task 4 may tighten if startup cost is material; deferred).
- `Identity` struct was not present in `core/src/identity/mod.rs` despite the task description implying it existed. Created it as part of T3 with the four fields above.
- Bonus 6th test `ensure_stub_missing_keychain_entry_returns_err` — covers the error path when stub points at a missing keychain entry; not in task spec but worth pinning since the recovery is user-facing (`pull-from-webapp`).
- `mcp/` build is intentionally broken at this commit (`mcp/src/main.rs:288` + `mcp/tests/stdio_backward_compat.rs:79,81` reference the deleted `load_or_create_keypair`). Task 4 fixes those.

**Reviews:** SKIPPED for time-budget reasons. Wave 5 Tasks 16 (security-auditor) + 17 (code-reviewer) will cover the full diff of Tasks 1-14 holistically, including this commit. The eager-cache decision and the secret-redacted Debug impl are explicit security gates to revisit there.

**Verification:**
- `cargo test -p mnemonic-core --lib identity` → 30 passed, 1 ignored
- `cargo clippy -p mnemonic-core --lib --tests -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- `grep -E "tracing::|println!|eprintln!" core/src/identity/ensure.rs | grep -i secret` → no matches (no secret bytes in logs)
- `cargo build -p mnemonic-mcp 2>&1 | grep "load_or_create_keypair"` → expected error, T4 fixes

---

## Task 4: Wire identity::ensure() into mcp/src/main.rs + integration_bootstrap test

**Status:** Done
**Commit:** a71acbb
**Agent:** task-4-mcp-wire
**Summary:** Replaces the `load_or_create_keypair(&cfg.keypair_path)?` call in `mcp/src/main.rs` with `mnemonic_core::identity::ensure()?`, destructures `identity.keypair` into the existing `keypair` binding so the downstream `McpState { keypair, ... }` construction is untouched. Adds `tracing::info!("Identity storage: {:?}", identity.storage)` so log output makes the OS-keychain vs file-fallback distinction visible at runtime. Updates `mcp/tests/stdio_backward_compat.rs` to use `ensure_with_stores()` with explicit `KeyStores { os: None, file: FileKeyStore, ... }` (test path needs file-fallback only, no real OS keychain). Adds `mcp/tests/integration_bootstrap.rs` — a smoke test that boots the binary with `HOME=<tempdir>`, sends `mnemonic_whoami` over stdio, asserts the returned pubkey matches the created `.mnemonic/identity.json`. Marked `#[ignore]` for the same network-dependency reason as `stdio_backward_compat.rs`.

**Deviations:**
- `cfg.keypair_path` is now unused by the code path. Left in `Config` to avoid touching the env-var contract; marked `#[allow(dead_code)]` with a comment explaining the deferral. Future refactor: remove the field + env var + companion code in `Config::from_env`. Tracking as backlog.
- `mcp/tests/integration_bootstrap.rs` came out at 150 lines (target was <130); the boilerplate from `stdio_backward_compat.rs` for spawn/read/write doesn't compress further without harming clarity. Acceptable.

**Reviews:** Skipped (consistent with T3 — Wave 5 audits will cover holistically). T4's diff is mechanical: 3-line main.rs edit + 1-line keypair-path annotation + test-file updates + new integration test that's `#[ignore]`'d.

**Verification:**
- `cargo build -p mnemonic-mcp` → clean
- `grep -rn "load_or_create_keypair" --include='*.rs'` → 0 matches workspace-wide
- `cargo test -p mnemonic-mcp --lib` → 137 passed
- `cargo test -p mnemonic-mcp --test stdio_backward_compat` → compiles, 1 ignored
- `cargo test -p mnemonic-mcp --test integration_bootstrap` → compiles, 1 ignored
- `cargo clippy -p mnemonic-mcp --lib -- -D warnings` → clean (pre-existing `--all-targets` failures in `pending_user_cap` / `recall_owner_isolation` are behind the `test-support` feature, unrelated to T4)
- `cargo fmt --all -- --check` → clean

## 2026-05-22 — Wave 1 complete

**Status:** Frozen; advancing to Wave 2.
**Commits in Wave 1:** b01a2ab, c05b62a, d252c21 (T1) → 9decc93, 8ba3b93, 3882f06, 84529a1 (T2) → ff3a9fe, c412122 (T3) → a71acbb (T4)
**Summary:** Rust core identity infrastructure complete. `KeyStore` trait + 3 impls + `ensure()` 5-path bootstrap + `mcp/` wired through. 30 unit tests in `mnemonic-core` (29 passing + 1 OS-keychain ignored, opt-in for Wave 3 Task 9). 137 unit tests in `mnemonic-mcp` still passing post-wire. Two integration tests (`stdio_backward_compat`, `integration_bootstrap`) compile cleanly, both `#[ignore]`'d pending network-allowed CI lane. `load_or_create_keypair` fully removed (Decision 4 / Deviation 4). Reviewer rounds deferred to Wave 5 holistic audits for Tasks 3 and 4 due to timeout pressure on the impl-agent harness; T1 and T2 had per-task code-reviewer + security-auditor + test-reviewer all OK.

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
