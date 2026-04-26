---
created: 2026-04-26
status: draft
branch: dev
size: M
---

# Tech Spec: docs-actualization

## Solution

Promote 11 evergreen documents (9 .md + 2 PDF) recovered from `sivo4kin/mnemonic-protocol@docs/usecases` into the public `docs/` tree with surgical de-staling, expand `WHITEPAPER.md §9` to cover all 10 use cases, anchor PK references to the new tree, and seed a follow-up roadmap in `decisions.md`. Implementation is sequenced through five waves followed by audit and pre-deploy QA, totalling 15 tasks across one feature branch `feat/docs-actualization` → PR to `dev`.

Pipeline:

1. Restore missing files into `.claude/skills/project-knowledge/recovered/` (`competitive-landscape/`, `research/`, new `problems/`) using `git -C /Users/syi/src/mnemonic-protocol show 'origin/docs/usecases:<path>'`. Extend `recovered/README.md` to reflect the additions and the dropped files.
2. Finalize `code-research.md` with the per-hit sanity-grep table and 6 token-replace overrides (DRAG_ANALYSIS:37, WEB_RESEARCH:45/64/132, CONCURRENT_WRITERS:157/217). All other files pass verbatim.
3. Promote each `recovered/<subdir>/` to its mirror under `docs/` (4 parallel tasks). Apply token replacements during promotion of competitive-landscape and problems. Drop `MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` and `CRITICAL_REVIEW.md` outright; do not create `docs/historical/`.
4. In parallel: expand `WHITEPAPER.md §9` to all 10 use cases with deep-dive links; add a §References entry for `docs/research/paper.pdf` (foundational paper); add a `paper.pdf` reference to repo `README.md`; update PK files (`project.md` Use Case Roles section + `architecture.md` competitive-landscape and TurboQuant pointers); seed `work/docs-actualization/decisions.md` with the Browser-WASM verification UI sub-section + 8-bullet roadmap; add a lychee link-check workflow.
5. Audit (3 read-only reviews) and pre-deploy QA (lychee, sanity-grep, acceptance checklist).

No code changes in `core/`, `mcp/`, `webapp/`. No `Cargo.toml` or `package.json` edits. CI cargo jobs do not trigger (paths-ignore in `.github/workflows/ci.yml` after dd395fd).

## Architecture

### What we're building/modifying

- **`docs/`** (regular directory in monorepo, per architecture.md) gains four new subfolders: `usecases/`, `competitive-landscape/`, `research/`, `problems/`. `WHITEPAPER.md` is edited (§9 expansion + §References entry). `docs/historical/` is **not** created.
- **Repo `README.md`** gains a reference to `docs/research/paper.pdf`.
- **`.claude/skills/project-knowledge/references/`**: `project.md` and `architecture.md` get pointer additions. `patterns.md` is unchanged unless sanity-grep flags something. `recovered/README.md` (in the same `project-knowledge/` skill) is extended with new rows and a promotion note.
- **`.claude/skills/project-knowledge/recovered/`** gains restored content under `competitive-landscape/`, new `research/` (3 .md + 2 PDF), new `problems/` (3 .md). `usecases/` already exists from commit afa20da and is unchanged here.
- **`.github/workflows/docs-link-check.yml`** (new) — lychee CI gate triggered only on `docs/**` or `*.md` changes; does not touch existing `ci.yml`.
- **`work/docs-actualization/`** gets `code-research.md` (already drafted in this phase) and a new `decisions.md` with the follow-up roadmap.

### How it works

The feature is a one-time migration; there is no runtime "system". Sequencing is enforced by file-conflict avoidance:

- Wave 1 owns `.claude/skills/project-knowledge/recovered/` writes (including `recovered/README.md`).
- Wave 2 owns `work/docs-actualization/code-research.md` writes.
- Wave 3 owns `docs/<subdir>/` writes — one task per subdir, no shared files between tasks.
- Wave 4 spans five disjoint write targets in parallel: `docs/WHITEPAPER.md`, `README.md`, `.claude/skills/project-knowledge/references/{project,architecture}.md`, `work/docs-actualization/decisions.md`, `.github/workflows/docs-link-check.yml`.
- Audit Wave is read-only, parallel.
- Final Wave is QA over the cumulative tree.

After merge to `dev` (and later to `main`), Cloudflare Pages auto-deploys `docs/` per `deployment.md`. Cargo CI does not run.

### Shared resources

None. No DB, no API client, no ML model. Only the local clone at `/Users/syi/src/mnemonic-protocol` is shared by Wave 1 tasks (read-only via `git show`).

## Decisions

### Decision 1: Restoration via `git show` against local clone (variant A)
**Decision:** Restore each upstream file with `git -C /Users/syi/src/mnemonic-protocol show 'origin/docs/usecases:<path>' > <target>`. No fresh clone, no `git remote add`, no `git subtree`.
**Rationale:** Matches the pattern already used for `usecases/` in commit afa20da. Local clone is verified: `origin/docs/usecases` HEAD = `7a68a973` (matches the pin in `recovered/README.md`). All 11 source files pre-flight-confirmed present. Supports user-spec constraint **Source-of-truth** and **Restoration mechanism**. `[Supports US: Source-of-truth pin]`
**Alternatives considered:** Fresh clone to `/tmp` — rejected (extra step, redundant). `git fetch + checkout` — rejected (taints worktree, harder to roll back). `git subtree pull` — rejected (overkill for one-shot import).

### Decision 2: Owner-overrides on recovered/README.md classification
**Decision:** `MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` is **dropped entirely** (not restored, not archived; `docs/historical/` is not created). `CRITICAL_REVIEW.md` is **not restored** at all and only appears as a follow-up bullet in decisions.md.
**Rationale:** Both decisions came from the owner during user-spec interview Q10. They override `recovered/README.md`'s ARCHIVED classification for MCP_SERVER... and bring CRITICAL_REVIEW out of the unmentioned set into a roadmap concern. `[Supports US: Что делаем item 6 — outdated drops]`
**Alternatives considered:** Honour the README's archive classification verbatim — rejected per owner instruction.

### Decision 3: 1-token replacement override on delete-only policy
**Decision:** When sanity-grep surfaces a stale claim and strict deletion would break a table, leave a stranded reference, or destroy paragraph coherence, allow a 1-token replacement. Pre-flight identified 6 such hits (DRAG_ANALYSIS:37, WEB_RESEARCH:45/64/132, CONCURRENT_WRITERS:157/217). Each replacement is logged in `code-research.md` with before/after.
**Rationale:** User-spec policy was "delete-outdated, no rewrites" with a `≥50% drop file` threshold. Strict deletion would either break Markdown tables or leave dangling antecedents; that exceeds the cosmetic damage the owner accepted. Preserving information by replacing one token (e.g., `SHA3-256` → `blake3`) keeps the doc readable while staying within "no surgical rewriting". `[Supports US: Constraints "Delete-outdated, не rewrite (с минимальным override'ом)"]`
**Alternatives considered:** Strict delete-only — rejected by owner in user-spec Q11 ("A"). Free rewrite — rejected: out of policy scope.

### Decision 4: Sequential waves to avoid `recovered/README.md` and `WHITEPAPER.md` conflicts
**Decision:** Wave 1 holds all writes to `recovered/README.md`. Wave 4 holds the only `WHITEPAPER.md` write (combining §9 expansion and §References paper.pdf entry into a single task). No two tasks in the same wave touch the same file.
**Rationale:** Both files are common conflict points. `recovered/README.md` summarizes 11 restored files plus drop notes — splitting that across multiple tasks invites merge churn within the same branch. `WHITEPAPER.md` §9 and §References are different sections but the same file; one task atomically updates both. `[TECHNICAL]`
**Alternatives considered:** Restore-per-subdir parallel tasks each with its own README mini-edit — rejected (3-way merge on README.md inside a feature branch). Two WHITEPAPER tasks (§9 + §References) — rejected (same-file conflict).

### Decision 5: lychee CI gate as part of this feature, not a separate chore
**Decision:** Add `.github/workflows/docs-link-check.yml` running `lychee --offline docs/` and `*.md` files at the repo root. Triggers only on changes to `docs/**` or `*.md` (no `paths-ignore` for itself). Adds ~10s to PR time on doc changes; zero impact on cargo PRs.
**Rationale:** User-spec lists lychee as the gating validation gate. Without a CI workflow, the gate is manual-only and degrades over time. Cost is small (~5 lines of YAML, plus the `lycheeverse/lychee-action@v2` step). Including it here means future doc-only PRs are protected from day one. `[Supports US: Validation gates → lychee --offline docs/ exits 0]`
**Alternatives considered:** Manual-only lychee at PR review time — rejected (no enforcement). Add to existing `ci.yml` — rejected (pulls in the cargo `paths-ignore`, complicates the matrix).

### Decision 6: paper.pdf reference placement in WHITEPAPER §References and README
**Decision:** In `WHITEPAPER.md`, add a new entry numbered 8 to the `## References` ordered list pointing to `docs/research/paper.pdf` with the relative link form `[Mnemonic Protocol Foundational Paper](./research/paper.pdf)`. Title finalised by opening the PDF during Task 7. In `README.md`, insert a new "## Foundational research" H2 immediately after the Introduction block, with a one-sentence summary and the same relative link `[paper.pdf](docs/research/paper.pdf)`.
**Rationale:** Keeps citation style consistent with existing 7 references in WHITEPAPER (numbered, descriptive title + URL). Adds a discoverable section to README without touching its existing copy. Both links are relative — works on GitHub web rendering, GitHub raw, and Cloudflare Pages. `[Supports US: Mandatory references]`
**Alternatives considered:** Inline reference in README Introduction — rejected (less discoverable). Footer reference in WHITEPAPER body — rejected (deviates from the existing References section convention).

### Decision 7: Audit Wave adapted to docs-only feature
**Decision:** Audit Wave still has 3 parallel tasks but the standard skills are repurposed for documentation:
- "Code Audit" (skill: `code-reviewing`) → reviews docs/ tree consistency, link integrity, file naming, README presence per subfolder.
- "Security Audit" (skill: `security-auditor`) → scans restored content for accidentally captured secrets, API keys, internal hostnames, Telegram tokens, mainnet keypairs.
- "Test Audit" (skill: `test-master`) → reviews lychee config, sanity-grep coverage, code-research.md audit-trail completeness, token-replace before/after log.
**Rationale:** Skill template requires Audit Wave with these three skills. For a docs feature the agents are still valuable (link/format/secret/audit-trail review) — repurposed prompt makes this explicit. `[TECHNICAL]`
**Alternatives considered:** Skip Audit Wave — rejected (skill says always present, and value is real). Use a single combined "Documentation Audit" — rejected (loses parallelism and security focus).

### Decision 8: No deploy task; Cloudflare auto-deploys
**Decision:** Final Wave has no Deploy task. Cloudflare Pages docs project auto-deploys on push to `main` per `deployment.md`. Pre-deploy QA's Verify-user step includes a post-merge spot-check.
**Rationale:** No new infrastructure. Existing CI/Cloudflare pipeline handles deploy. `[Supports US: deploy_approach — auto-deploys]`
**Alternatives considered:** Manual deploy verification task — folded into QA Verify-user instead.

## Data Models

None. Pure documentation feature.

## Dependencies

### New packages
- **`lycheeverse/lychee-action@v2`** (CI only) — link checker GitHub Action. No local install required. Pinned by tag.

### Using existing (from project)
- `git` (local clone read-only against `/Users/syi/src/mnemonic-protocol`)
- `ripgrep` / `grep` (sanity-grep)
- `bash` (file ops, redirect for binary PDF restoration)

### Removed packages
None.

## Testing Strategy

**Feature size:** M

### Unit tests
None. No code under test.

### Edge case tests
Validation against pre-flight expectations is captured in `code-research.md`. Edge cases handled at implementation time:
- **Truncated upstream** (TURBOQUANT_DEEP_ANALYSIS.md, 181 lines, ends mid-Mermaid): restore as-is with the existing recovery note inline; do not reconstruct.
- **Token-replace conflict in tables**: validated pre-flight that 1-token replacements preserve table column count.
- **PDF binary integrity**: `wc -c` of restored PDF matches upstream `git show ... | wc -c` byte count.
- **Sanity-grep against current code beyond the initial term set**: implementation may surface new stale terms; if so, append to `code-research.md` and apply the same `delete-section` / `replace-token` policy.

### Integration tests
- **Lychee link check** (gating CI):
  ```bash
  lychee --offline docs/ README.md docs/WHITEPAPER.md
  ```
  Exit 0 required. Triggered automatically via `.github/workflows/docs-link-check.yml`.
- **Sanity-grep regression** (gating, manual or QA-task):
  ```bash
  grep -RIE 'SHA3|mcp-server-rs|pre-V1|Pre-V1|HashEmbedder|Python backend' docs/ README.md
  ```
  Zero hits required. `-I` skips binaries.
- **docs/ layout**:
  ```bash
  ls docs/usecases/ docs/competitive-landscape/ docs/research/ docs/problems/
  ```
  Counts: 11 / 4 / 6 / 4. `docs/historical/` absent.
- **PDF restoration round-trip**:
  ```bash
  diff <(git -C /Users/syi/src/mnemonic-protocol show origin/docs/usecases:research/paper.pdf) docs/research/paper.pdf
  ```
  Empty diff.

### E2E tests
None. No live environment to drive. Cloudflare preview rebuild is observed post-merge as a Verify-user step in QA.

## Agent Verification Plan

**Source:** user-spec "Как проверить" section.

### Verification approach
After each wave, the executing agent runs the wave's Verify-smoke command. After Wave 4, full validation passes:

1. **Restoration completeness:** `ls .claude/skills/project-knowledge/recovered/competitive-landscape/*.md .claude/skills/project-knowledge/recovered/research/*.md .claude/skills/project-knowledge/recovered/research/*.pdf .claude/skills/project-knowledge/recovered/problems/*.md` — counts: 3 / 3 / 2 / 3.
2. **Promotion completeness:** `ls docs/usecases/ docs/competitive-landscape/ docs/research/ docs/problems/` — counts 11 / 4 / 6 / 4.
3. **No `docs/historical/`:** `test ! -d docs/historical || (echo FAIL && exit 1)`.
4. **lychee link check:** `lychee --offline docs/ README.md docs/WHITEPAPER.md` — exit 0.
5. **Sanity-grep regression:** `grep -RIE 'SHA3|mcp-server-rs|pre-V1|Pre-V1|HashEmbedder|Python backend' docs/ README.md` — empty.
6. **WHITEPAPER §9 fully expanded:** `grep -cE '^### 9\.' docs/WHITEPAPER.md` — output `>=10`.
7. **WHITEPAPER references paper.pdf:** `grep 'docs/research/paper.pdf' docs/WHITEPAPER.md` and `grep 'paper.pdf' docs/WHITEPAPER.md` — non-empty.
8. **README references paper.pdf:** `grep 'docs/research/paper.pdf' README.md` — non-empty.
9. **PK project.md updated:** `grep -E 'Use Case Roles|docs/usecases' .claude/skills/project-knowledge/references/project.md` — non-empty.
10. **PK architecture.md updated:** `grep -E 'docs/competitive-landscape|docs/research/condensed-principles' .claude/skills/project-knowledge/references/architecture.md` — non-empty.
11. **decisions.md follow-ups:** `grep -E 'Follow-up roadmap items|Browser-WASM verification UI|for further validation' work/docs-actualization/decisions.md` — all 3 patterns matched.
12. **code-research.md exists with per-hit table + token-replace log:** `test -f work/docs-actualization/code-research.md && grep -c 'replace-token' work/docs-actualization/code-research.md` — output `>=6`.
13. **recovered/ retained with promotion note:** `grep 'Promoted on' .claude/skills/project-knowledge/recovered/README.md` — non-empty.
14. **lychee CI workflow exists:** `test -f .github/workflows/docs-link-check.yml`.
15. **Cargo CI did not run:** check the PR's CI run summary on GitHub UI — only the docs link-check job ran (Verify-user).

### Tools required
bash (git, grep, ls, test, lychee CLI). No MCP tools needed. `lychee` installed locally for pre-merge verification (`cargo install lychee` or `brew install lychee`).

## Risks

| Risk | Mitigation |
|------|-----------|
| Upstream `sivo4kin@docs/usecases` HEAD drifts before the feature lands | Pin to `7a68a973` in restoration commit messages; if local clone is updated, restoration command still works (commit hash is immutable). |
| `paper.pdf` (862KB) bloats repo | Acceptable for one-time addition; PDF is canonical foundational material per owner. No git LFS needed at this size. |
| Token-replace surfaces unexpected secondary stale claims during implementation | Append to `code-research.md` per-hit table; apply same policy. Audit Wave double-checks. |
| lychee finds external dead links (paper URLs) blocking the gate | Use `--offline` flag (already specified) — only internal/relative links checked. External URL rot is post-merge concern. |
| WHITEPAPER §9 expansion creates contradiction with §10 (Related Work) or §13 (Limitations) | Owner-resolved in user-spec edge cases: §9 phrasing wins; update §10/§13 only if grep finds concrete contradictions. Expected to be no-op. |
| Cloudflare Pages docs project not yet provisioned (per deployment.md note "TBD, confirm names after first deploy") | If deploy fails post-merge, raise as separate issue; not gating for this feature. QA Verify-user spot-checks, does not block. |
| `recovered/usecases/` README references files that this feature renames or relocates | Pre-flight: usecases/ tree is unchanged here (already committed in afa20da); only competitive-landscape/, research/, problems/ are touched. |

## User-Spec Deviations

None. Tech-spec implements user-spec decisions verbatim plus a clarification: `lychee CI workflow` is added in Wave 4 Task 12 rather than left to a future chore (Decision 5). User-spec testing strategy already specified `lychee --offline docs/` as the gating validation; whether to add a CI workflow was not explicitly settled. This expansion is non-breaking and serves user-spec acceptance criteria.

## Acceptance Criteria

Technical acceptance criteria (supplement user-spec criteria):

- [ ] Workspace contains `feat/docs-actualization` branch with 13 atomic commits in user-spec-defined order
- [ ] PR opened against `dev` titled `feat(docs): actualize protocol documentation from recovered staging`
- [ ] No diff under `core/`, `mcp/`, `webapp/`, root `Cargo.toml`, or any `Cargo.lock`
- [ ] `git status --porcelain` after merge confirms no leftover untracked files in `recovered/`
- [ ] `recovered/README.md` extended with rows for problems/ (3 files), both PDFs in research/ (2 files), and explicit drop notes for MCP_SERVER_BACKEND_FEATURES_COMPARISON.md and CRITICAL_REVIEW.md
- [ ] `recovered/README.md` top section gains "Promoted on YYYY-MM-DD in commit `<hash>`" line
- [ ] `docs/historical/` directory does not exist
- [ ] `code-research.md` ends with "Sanity-grep run completed; 6 token-replace overrides applied; no further stale terms detected" (or with extended table if more found)
- [ ] All 6 token replacements logged in `code-research.md` with before/after exact quotes
- [ ] WHITEPAPER §9 contains exactly 10 `### 9.X` subsections each ending with `[See deep-dive in docs/usecases/<file>.md.]`
- [ ] WHITEPAPER §References has 8th entry pointing to `./research/paper.pdf`
- [ ] README has new `## Foundational research` H2 section with link to `docs/research/paper.pdf`
- [ ] PK project.md has new "Use Case Roles" section with bullet list of 10 use-case roles linking to `docs/usecases/<file>.md`
- [ ] PK architecture.md has pointer paragraph linking to `docs/competitive-landscape/` and `docs/research/condensed-principles.md`
- [ ] decisions.md has "Follow-up roadmap items" section: 1 detailed sub-section (Browser-WASM verification UI) + 8-bullet list each tagged "for further validation"
- [ ] `.github/workflows/docs-link-check.yml` exists with `paths` filter on `docs/**` and `*.md`
- [ ] On the PR run, the docs-link-check job exits 0 and no cargo job runs
- [ ] Cloudflare Pages preview build completes for the PR (Verify-user post-merge spot check; not gating)
- [ ] Plus all user-spec acceptance criteria pass

## Implementation Tasks

### Wave 1: Restoration

#### Task 1: Restore 11 upstream files + extend recovered/README.md
- **Description:** Run the 11 `git -C /Users/syi/src/mnemonic-protocol show 'origin/docs/usecases:<path>'` commands to materialize files into `recovered/competitive-landscape/`, `recovered/research/` (incl. 2 PDFs), `recovered/problems/`. Extend `recovered/README.md` with new table rows for problems/ + both PDFs + drop notes for MCP_SERVER_BACKEND_FEATURES_COMPARISON.md and CRITICAL_REVIEW.md. Single task because all writes converge on the same `recovered/README.md`. Three commits inside this task per user-spec commit plan (commits 1–3) plus one for README update (commit 4).
- **Skill:** code-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `ls .claude/skills/project-knowledge/recovered/competitive-landscape/*.md .claude/skills/project-knowledge/recovered/research/*.md .claude/skills/project-knowledge/recovered/research/*.pdf .claude/skills/project-knowledge/recovered/problems/*.md | wc -l` returns `11`. `wc -c .claude/skills/project-knowledge/recovered/research/paper.pdf` returns `861881`.
- **Files to modify:** `.claude/skills/project-knowledge/recovered/competitive-landscape/{DRAG_ANALYSIS,WEB_RESEARCH_TRUSTLESS_RAG,DECENTRALIZED_RAG_LANDSCAPE}.md` (new), `.claude/skills/project-knowledge/recovered/research/{TURBOQUANT_DEEP_ANALYSIS,apply-to-agent-memory-architecture,condensed-principles}.md` (new), `.claude/skills/project-knowledge/recovered/research/Agent Identity for Autonomous AI_ Protocols, Mnemonic Analysis, and the Path to a Minimal Primitive.pdf` (new), `.claude/skills/project-knowledge/recovered/research/paper.pdf` (new), `.claude/skills/project-knowledge/recovered/problems/{MEMORY_EVICTION,CONCURRENT_WRITERS,ARWEAVE_PRICING_VALIDATION}.md` (new), `.claude/skills/project-knowledge/recovered/README.md` (extend table + drop notes)
- **Files to read:** `.claude/skills/project-knowledge/recovered/README.md` (current), `work/docs-actualization/code-research.md` (file mapping table), `work/docs-actualization/user-spec.md`

### Wave 2: Sanity-grep finalization (depends on Wave 1)

#### Task 2: Run final sanity-grep + finalize code-research.md
- **Description:** Run sanity-grep against the restored `recovered/<subdir>/` tree using the term set from `code-research.md` §4. Confirm pre-flight 6 hits + log any new hit. Append the final table form to `code-research.md` and add a closing "Sanity-grep run completed" paragraph with run date. Commit as `chore(docs): sanity-grep report → code-research.md (incl. 6 token-replace overrides)`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `grep -c 'replace-token' work/docs-actualization/code-research.md` returns `>=6`. `grep -c 'Sanity-grep run completed' work/docs-actualization/code-research.md` returns `1`.
- **Files to modify:** `work/docs-actualization/code-research.md`
- **Files to read:** `.claude/skills/project-knowledge/recovered/competitive-landscape/*.md`, `.claude/skills/project-knowledge/recovered/research/*.md`, `.claude/skills/project-knowledge/recovered/problems/*.md`, `core/src/**/*.rs`, `mcp/src/**/*.rs` (for cross-checking new stale terms if any)

### Wave 3: Promotion (depends on Wave 2; 4 tasks parallel — disjoint subdirs)

#### Task 3: Promote recovered/usecases → docs/usecases/
- **Description:** Copy 10 .md + README from `recovered/usecases/` to `docs/usecases/` verbatim (this folder was committed in afa20da and is evergreen). No edits needed. Single commit: `docs: promote recovered/usecases → docs/usecases/`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `ls docs/usecases/*.md | wc -l` returns `11`.
- **Files to modify:** `docs/usecases/{shared-memory-layer,provenance-attestation-layer,trust-reputation-layer,portable-memory-wallet,settlement-aware-memory-infrastructure,task-memory-ledger,shared-project-memory-namespace,artifact-attestation-service,agent-continuity-layer,reliability-oracle-for-orchestration}.md` (new), `docs/usecases/README.md` (new)
- **Files to read:** `.claude/skills/project-knowledge/recovered/usecases/*.md`

#### Task 4: Promote recovered/competitive-landscape → docs/competitive-landscape/ (apply 4 token replaces)
- **Description:** Copy 3 .md (DRAG_ANALYSIS, WEB_RESEARCH_TRUSTLESS_RAG, DECENTRALIZED_RAG_LANDSCAPE) and README from `recovered/competitive-landscape/` into `docs/competitive-landscape/`. Apply the 4 token replacements during copy (DRAG_ANALYSIS:37 delete-line; WEB_RESEARCH:45,64,132). README content adapted to drop the MCP_SERVER... row. Log each before/after edit in `code-research.md` (append-only).
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `grep -RIE 'SHA3|Pre-V1|pre-V1' docs/competitive-landscape/` returns empty. `ls docs/competitive-landscape/*.md | wc -l` returns `4`.
- **Files to modify:** `docs/competitive-landscape/{DRAG_ANALYSIS,WEB_RESEARCH_TRUSTLESS_RAG,DECENTRALIZED_RAG_LANDSCAPE}.md` (new), `docs/competitive-landscape/README.md` (new), `work/docs-actualization/code-research.md` (append before/after log entries)
- **Files to read:** `.claude/skills/project-knowledge/recovered/competitive-landscape/*.md`, `work/docs-actualization/code-research.md`

#### Task 5: Promote recovered/research → docs/research/ (incl. 2 PDFs)
- **Description:** Copy 3 .md (TURBOQUANT_DEEP_ANALYSIS with truncation note kept inline, apply-to-agent-memory-architecture, condensed-principles) + 2 PDF (Agent-Identity, paper.pdf) from `recovered/research/` to `docs/research/`. Add a new `docs/research/README.md` describing the contents and the foundational status of `paper.pdf`. PDFs copied as binaries; verify byte equality. No token replacements expected here.
- **Skill:** code-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `ls docs/research/*.md docs/research/*.pdf | wc -l` returns `>=5`. `cmp docs/research/paper.pdf .claude/skills/project-knowledge/recovered/research/paper.pdf` exits 0.
- **Files to modify:** `docs/research/{TURBOQUANT_DEEP_ANALYSIS,apply-to-agent-memory-architecture,condensed-principles}.md` (new), `docs/research/Agent Identity for Autonomous AI_ Protocols, Mnemonic Analysis, and the Path to a Minimal Primitive.pdf` (new), `docs/research/paper.pdf` (new), `docs/research/README.md` (new)
- **Files to read:** `.claude/skills/project-knowledge/recovered/research/*`

#### Task 6: Promote recovered/problems → docs/problems/ (apply 2 token replaces)
- **Description:** Copy 3 .md (MEMORY_EVICTION, CONCURRENT_WRITERS, ARWEAVE_PRICING_VALIDATION) from `recovered/problems/` to `docs/problems/`. Apply 2 token replacements during CONCURRENT_WRITERS copy (lines 157, 217). Add new `docs/problems/README.md` describing the section as "open system problem statements + pricing validation that inform further roadmap". Log before/after entries in `code-research.md`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `grep -RIE 'SHA3' docs/problems/` returns empty. `ls docs/problems/*.md | wc -l` returns `4`.
- **Files to modify:** `docs/problems/{MEMORY_EVICTION,CONCURRENT_WRITERS,ARWEAVE_PRICING_VALIDATION}.md` (new), `docs/problems/README.md` (new), `work/docs-actualization/code-research.md` (append entries)
- **Files to read:** `.claude/skills/project-knowledge/recovered/problems/*.md`, `work/docs-actualization/code-research.md`

### Wave 4: Documentation edits + decisions + CI gate (depends on Wave 3; 5 tasks parallel — disjoint files)

#### Task 7: Expand WHITEPAPER §9 + add §References entry for paper.pdf
- **Description:** Edit `docs/WHITEPAPER.md`: rewrite §9 from 4 subsections to 10 subsections matching the 10 use-case docs (mapping in `code-research.md` §8); each subsection is 1-2 sentences + `[See deep-dive in docs/usecases/<file>.md.]`. Add 8th entry to `## References` ordered list pointing to `[Mnemonic Protocol Foundational Paper](./research/paper.pdf)`; open the PDF metadata to populate the title precisely. Single commit.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `grep -cE '^### 9\.' docs/WHITEPAPER.md` returns `>=10`. `grep './research/paper.pdf' docs/WHITEPAPER.md` returns non-empty.
- **Files to modify:** `docs/WHITEPAPER.md`
- **Files to read:** `docs/WHITEPAPER.md` (current §9 + §References), `docs/usecases/*.md`, `docs/research/paper.pdf` (read PDF title metadata)

#### Task 8: Add Foundational research section to README.md
- **Description:** Insert new `## Foundational research` H2 section immediately after the Introduction block in repo `README.md` with a one-sentence summary and a relative link `[paper.pdf](docs/research/paper.pdf)`. Do not modify Introduction copy.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `grep -A2 'Foundational research' README.md | grep 'docs/research/paper.pdf'` returns non-empty.
- **Files to modify:** `README.md`
- **Files to read:** `README.md` (current), `docs/research/paper.pdf` (title metadata)

#### Task 9: Update PK project.md + architecture.md
- **Description:** In `.claude/skills/project-knowledge/references/project.md`, add "Use Case Roles" H2 section with a bullet list of all 10 use-case roles, each linking to `docs/usecases/<file>.md`. In `.claude/skills/project-knowledge/references/architecture.md`, add a short pointer paragraph at the end of the file linking to `docs/competitive-landscape/` (positioning) and `docs/research/condensed-principles.md` (TurboQuant principles for knowledge-DB ref). Patterns.md unchanged unless grep flags.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `grep -E 'Use Case Roles|docs/usecases' .claude/skills/project-knowledge/references/project.md` and `grep -E 'docs/competitive-landscape|docs/research/condensed-principles' .claude/skills/project-knowledge/references/architecture.md` both non-empty.
- **Files to modify:** `.claude/skills/project-knowledge/references/project.md`, `.claude/skills/project-knowledge/references/architecture.md`
- **Files to read:** `.claude/skills/project-knowledge/references/project.md` (current), `.claude/skills/project-knowledge/references/architecture.md` (current), `docs/usecases/`, `docs/competitive-landscape/`, `docs/research/condensed-principles.md`

#### Task 10: Seed work/docs-actualization/decisions.md with follow-up roadmap
- **Description:** Create `work/docs-actualization/decisions.md`. Add "Follow-up roadmap items" section with: (a) detailed Browser-WASM verification UI sub-section (Problem, Proposed Approach, Dependencies, Open Questions, Source-doc refs); (b) 8-bullet list (encryption, ZK proofs, shared-namespaces, reliability oracle, compressed shadow-index recall, lifecycle policy, economic model validation, critical review redo) — each 1-2 sentences + ref + tag `for further validation`. Order matches `code-research.md` §9.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `grep -E 'Follow-up roadmap items|Browser-WASM verification UI|for further validation' work/docs-actualization/decisions.md` matches all 3 patterns.
- **Files to modify:** `work/docs-actualization/decisions.md` (new)
- **Files to read:** `work/docs-actualization/code-research.md` §9, `work/docs-actualization/user-spec.md`, `docs/competitive-landscape/DRAG_ANALYSIS.md`, `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md`, `docs/problems/CONCURRENT_WRITERS.md`, `docs/problems/MEMORY_EVICTION.md`, `docs/problems/ARWEAVE_PRICING_VALIDATION.md`, `docs/usecases/reliability-oracle-for-orchestration.md`, `docs/usecases/shared-project-memory-namespace.md`, `docs/research/apply-to-agent-memory-architecture.md`, `docs/WHITEPAPER.md`

#### Task 11: Add lychee CI workflow
- **Description:** Create `.github/workflows/docs-link-check.yml` that runs `lycheeverse/lychee-action@v2` on push to `dev`/`main` and on PR, scoped to `docs/**` and `*.md` paths only. Uses `--offline` flag. Job name `lychee-link-check`. No matrix; single ubuntu-latest run. Job must succeed (non-zero on broken links).
- **Skill:** code-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `test -f .github/workflows/docs-link-check.yml`. Locally: `lychee --offline docs/ README.md docs/WHITEPAPER.md` exits 0.
- **Files to modify:** `.github/workflows/docs-link-check.yml` (new)
- **Files to read:** `.github/workflows/ci.yml` (paths-ignore convention)

### Audit Wave (depends on Wave 4; 3 tasks parallel)

#### Task 12: Documentation Audit
- **Description:** Holistic review of docs/ tree for consistency: link integrity (every relative link resolves), README.md present in each subfolder, file naming convention consistent, WHITEPAPER §9 cross-references correct, PK refs land at the right anchors. Also check `recovered/README.md` extension reflects the actual restored content.
- **Skill:** code-reviewing
- **Reviewers:** none
- **Files to read:** `docs/**/*.md`, `docs/**/README.md`, `README.md`, `.claude/skills/project-knowledge/references/{project,architecture,patterns}.md`, `.claude/skills/project-knowledge/recovered/README.md`
- **Files to modify:** N/A (analysis only; produces report)

#### Task 13: Security Audit
- **Description:** Scan all restored content (in docs/ and recovered/) for accidentally captured sensitive material: API keys, OpenAI/Anthropic/HF tokens, private keys, mainnet keypairs, internal hostnames, `.env` lines, Telegram bot tokens, Cloudflare API tokens. Particular focus on PDFs (open in text-extraction tool to verify no embedded keys) and on tables in DRAG_ANALYSIS / WEB_RESEARCH that may contain redacted-looking strings.
- **Skill:** security-auditor
- **Reviewers:** none
- **Files to read:** `docs/**`, `.claude/skills/project-knowledge/recovered/**`, `README.md`, `docs/WHITEPAPER.md`
- **Files to modify:** N/A (analysis only)

#### Task 14: Validation Audit
- **Description:** Audit the validation evidence: `code-research.md` per-hit table is complete, all 6 token replacements have before/after entries, no missing audit entries; `lychee` workflow correctness; sanity-grep regression command in tech-spec matches the implemented set; recovered/README.md promotion note populated with real commit hash post-merge. If any gap found, recommend a fix (no direct edits).
- **Skill:** test-master
- **Reviewers:** none
- **Files to read:** `work/docs-actualization/code-research.md`, `work/docs-actualization/user-spec.md`, `work/docs-actualization/tech-spec.md`, `.github/workflows/docs-link-check.yml`, `.claude/skills/project-knowledge/recovered/README.md`
- **Files to modify:** N/A (analysis only)

### Final Wave (depends on Audit Wave)

#### Task 15: Pre-deploy QA
- **Description:** Run all 15 verification steps from the Agent Verification Plan. Run `lychee --offline docs/ README.md docs/WHITEPAPER.md` locally and confirm exit 0. Run sanity-grep regression. Confirm 13 atomic commits in feature branch matching user-spec commit plan. Confirm no diff under `core/`, `mcp/`, `webapp/`, `Cargo.toml`, `Cargo.lock`. Verify all user-spec acceptance checkboxes are met. After PR merges to dev, spot-check Cloudflare Pages preview rebuild.
- **Skill:** pre-deploy-qa
- **Reviewers:** none
- **Verify-smoke:** All 15 steps in Agent Verification Plan pass.
- **Verify-user:** Open PR on GitHub → confirm only `docs-link-check` job ran (no cargo jobs). After merge to dev, observe Cloudflare Pages preview build for the dev branch refreshes; visit a sample doc URL once preview is live.
- **Files to read:** `work/docs-actualization/user-spec.md`, `work/docs-actualization/tech-spec.md`, `work/docs-actualization/code-research.md`, all changed files in the PR
- **Files to modify:** N/A (verification only)
