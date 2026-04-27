# Decisions Log: docs-actualization

Append-only log of decisions and audit findings during feature execution.

---

## Task 1 — Restore 11 upstream files + extend recovered/README.md

**Status:** done (executed in prior session, formalized in this session)
**Wave:** 1
**Date:** 2026-04-27

**Summary:** Materialized all 11 upstream files (3 competitive-landscape .md, 3 research .md, 2 research PDFs, 3 problems .md) from `sivo4kin/mnemonic-protocol@7a68a973` (origin/docs/usecases) into `.claude/skills/project-knowledge/recovered/{competitive-landscape,research,problems}/` via `git -C /Users/syi/src/mnemonic-protocol show 'origin/docs/usecases:<path>'`. Extended `recovered/README.md` with new rows for problems/, both PDFs, and drop notes for MCP_SERVER_BACKEND_FEATURES_COMPARISON.md and CRITICAL_REVIEW.md.

**Commits (4):**
- `584001f` docs(recovered): restore competitive-landscape (3 files) from sivo4kin@7a68a973
- `39946a4` docs(recovered): restore research (3 .md + 2 PDFs) from sivo4kin@7a68a973
- `6387f69` docs(recovered): restore problem statements + pricing analysis from sivo4kin@7a68a973
- `21ac4a1` docs(recovered): extend recovered/README.md with problems/ + PDFs + drop notes

**Verify-smoke results:**
- competitive-landscape .md count: 3 (+ 1 pre-existing README) — pass
- research .md count: 3 — pass
- research .pdf count: 2 — pass
- problems .md count: 3 — pass
- paper.pdf size: 861881 bytes — pass (matches expected)

**Review:** Code-reviewer review JSON at `logs/working/task-1/code-reviewer-round1.json`.

**Round 1 fixes:** Addressed 3 major + 3 minor findings on recovered/README.md (commit a9dcd99).

---

## Task 2 — Run final sanity-grep + finalize code-research.md

**Status:** done
**Wave:** 2
**Date:** 2026-04-27

**Summary:** Ran the sanity-grep term set against the restored `recovered/{competitive-landscape,research,problems}/` tree. Confirmed parity with the pre-flight 6 in-content hits (DRAG_ANALYSIS:37, WEB_RESEARCH:45/64/132, CONCURRENT_WRITERS:157/217). Cross-checked `core/src/**/*.rs` and `mcp/src/**/*.rs` — zero stale-term hits, only the expected `sha384` Arweave deep-hash false-positive. Extended `code-research.md` §4 with a per-hit table including `verdict` column (all 6 = `replace-token`), and appended the closing run paragraph. No new stale terms surfaced; no drop-file verdicts.

**Commit:** `4e9d929`

**Verify-smoke results:**
- `grep -c 'replace-token' work/docs-actualization/code-research.md` → 8 (>=6) — pass
- `grep -c 'Sanity-grep run completed' work/docs-actualization/code-research.md` → 1 — pass

**Reviewers:** code-reviewer, test-reviewer (round 1 in progress)

---

## Task 3 — Promote recovered/usecases → docs/usecases/

**Status:** done
**Wave:** 3
**Date:** 2026-04-27

**Summary:** Copied 10 use-case .md + README from recovered/usecases/ to docs/usecases/ verbatim.
**Commit:** 8ed0b6c
**Verify-smoke:** ls docs/usecases/*.md | wc -l == 11 — pass.

---

## Task 5 — Promote recovered/research → docs/research/ (incl. 2 PDFs)

**Status:** done
**Wave:** 3
**Date:** 2026-04-27

**Summary:** Copied 3 .md (TURBOQUANT_DEEP_ANALYSIS, apply-to-agent-memory-architecture, condensed-principles) and 2 PDF (paper.pdf, Agent-Identity .pdf) from `recovered/research/` to `docs/research/` verbatim. Authored new `docs/research/README.md` describing each artefact and flagging `paper.pdf` as the foundational scientific paper (linking WHITEPAPER.md and repo README). TURBOQUANT_DEEP_ANALYSIS.md preserved as-is with upstream mid-Mermaid truncation; README documents this. No token replacements applied (research/ tree passed sanity-grep verbatim per Task 2).
**Commit:** 82704c7
**Verify-smoke:**
- `cmp docs/research/paper.pdf .claude/skills/project-knowledge/recovered/research/paper.pdf` → exit 0 (silent) — pass
- `cmp` for Agent-Identity .pdf → exit 0 (silent) — pass
- `wc -c docs/research/paper.pdf` → 861881 — pass
- `ls docs/research/*.md docs/research/*.pdf | wc -l` → 6 (3 .md + 2 PDF + README) — pass
- `grep -RIE 'SHA3|Pre-V1|pre-V1|HashEmbedder|Python backend' docs/research/` → empty — pass
- `grep 'foundational' docs/research/README.md` → non-empty — pass

---

## Task 4 — Promote recovered/competitive-landscape → docs/competitive-landscape/ (apply 4 token replaces)

**Status:** done
**Wave:** 3
**Date:** 2026-04-27

**Summary:** Copied 3 .md (DRAG_ANALYSIS, WEB_RESEARCH_TRUSTLESS_RAG, DECENTRALIZED_RAG_LANDSCAPE) from `recovered/competitive-landscape/` to `docs/competitive-landscape/` and authored an adapted README dropping the `MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` reference per Decision 2 (now a 3-doc set rather than 4). Applied the 4 pre-flight token replacements during promotion:
- `docs/competitive-landscape/DRAG_ANALYSIS.md:37` (delete-line) — before `Mnemonic records: SHA3-256 hash of the encrypted memory blob via Solana memo.` → after: line removed.
- `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md:45` — before `| Status | Pre-V1, prototype validated | Live (Kinic-CLI shipped) |` → after `| Status | active Rust MCP server | Live (Kinic-CLI shipped) |`.
- `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md:64` (delete-tokens) — before `- Mnemonic commits the memory blob (SHA3 hash); V3DB proves the retrieval result` → after `- Mnemonic commits the memory blob; V3DB proves the retrieval result`.
- `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md:132` — before `| Mnemonic | Memory integrity (hash) | ✅ 4/8-bit | ✅ | Arweave+Solana | Pre-V1 |` → after `| Mnemonic | Memory integrity (hash) | ✅ 4/8-bit | ✅ | Arweave+Solana | v1.0 (active) |`.
DECENTRALIZED_RAG_LANDSCAPE.md copied verbatim. Appended a "Wave 3 Task 4 — promotion log" sub-section to `code-research.md` §4.4 with the 4 before/after pairs.

**Commit:** `d8df681`

**Verify-smoke results:**
- `grep -RIE 'SHA3|Pre-V1|pre-V1' docs/competitive-landscape/` → empty — pass
- `ls docs/competitive-landscape/*.md | wc -l` → 4 (3 .md + README) — pass

---

## Task 6 — Promote recovered/problems → docs/problems/ (apply 2 token replaces)

**Status:** done
**Wave:** 3
**Date:** 2026-04-27

**Summary:** Copied 3 .md (MEMORY_EVICTION, CONCURRENT_WRITERS, ARWEAVE_PRICING_VALIDATION) from `recovered/problems/` to `docs/problems/`. MEMORY_EVICTION.md and ARWEAVE_PRICING_VALIDATION.md copied byte-equal to source. Applied the 2 pre-flight token replacements during CONCURRENT_WRITERS promotion:
- `docs/problems/CONCURRENT_WRITERS.md:157` — before `... Adding `parent_hashes` for DAG structure: each SHA3-256 hash = 64 hex chars. Two parents = 128 chars. Fits comfortably.` → after `... Adding `parent_hashes` for DAG structure: each blake3 hash = 64 hex chars. Two parents = 128 chars. Fits comfortably.`
- `docs/problems/CONCURRENT_WRITERS.md:217` — before `The current system commits `SHA3-256(encrypted_blob)` to Solana. With multiple concurrent writers, two approaches for history verification:` → after `The current system commits `blake3(canonical CBOR bytes)` to Solana. With multiple concurrent writers, two approaches for history verification:`
Authored new `docs/problems/README.md` framing the section as open system problem statements + pricing validation that inform the follow-up roadmap and linking back to this decisions.md. Appended a "Wave 3 Task 6 — promotion log" sub-section to `code-research.md` §4.5 with the 2 before/after pairs.

**Commit:** `eacb6ff`

**Verify-smoke results:**
- `grep -RIE 'SHA3|Pre-V1|pre-V1' docs/problems/` → empty — pass
- `ls docs/problems/*.md | wc -l` → 4 (3 .md + README) — pass
- `sed -n '157p' docs/problems/CONCURRENT_WRITERS.md` contains `blake3 hash = 64 hex chars` — pass
- `sed -n '217p' docs/problems/CONCURRENT_WRITERS.md` contains `blake3(canonical CBOR bytes)` (no `encrypted_blob`) — pass
- `cmp docs/problems/MEMORY_EVICTION.md .claude/skills/project-knowledge/recovered/problems/MEMORY_EVICTION.md` → exit 0 — pass
- `cmp docs/problems/ARWEAVE_PRICING_VALIDATION.md .claude/skills/project-knowledge/recovered/problems/ARWEAVE_PRICING_VALIDATION.md` → exit 0 — pass
- `grep 'decisions.md' docs/problems/README.md` → non-empty — pass

---

## Follow-up roadmap items

Evergreen direction-of-travel hints surfaced during the docs-actualization feature. The Browser-WASM verification UI is the only candidate the owner flagged as "serious" and so receives a full sub-section. The remaining 8 items are listed once each as research / scoping leads tagged `for further validation` — they are NOT committed roadmap and must be validated against current code before any of them is promoted to a feature spec.

### Browser-WASM verification UI

**Problem:** Today's verification path requires a runtime that can run the Rust `verify` tool — the MCP server or the native binary. Browser users cannot independently verify a recalled attestation without contacting a backend, which weakens the core "anyone can verify" property of the Mnemonic Protocol.

**Proposed Approach:** Surface the existing `verify` and `whoami` exports from the `mnemonic-core` WASM build inside the webapp as a verification panel. The panel takes an `attestation_id` (or full bundle), fetches the Arweave payload, runs blake3 + COSE_Sign1 verification entirely in-browser, optionally cross-checks the Solana SPL Memo anchor, and renders OK / tampered / not-found badges. Reuse design tokens from `ux-guidelines.md`.

**Dependencies:**
- `mnemonic-core` WASM target — completed feature, `whoami` and `verify` exports already available.
- Webapp Vite build already imports the WASM package; only a UI surface is missing.

**Open Questions:**
- Does verifying a >10MB Arweave payload entirely in-browser create UX issues (memory, time-to-result, mobile)?
- Should the panel accept a user-supplied Solana RPC URL, or rely on a public default with graceful degradation?
- How to surface `tampered` vs `not_found` distinctively without alarming users for legitimate not-yet-anchored attestations?
- Should the panel optionally render the full COSE_Sign1 protected header for transparency / auditor inspection?
- Mobile browsers — feasible target, or desktop-first with explicit "verify on desktop" affordance on mobile?

**Source-doc refs:** [docs/usecases/agent-continuity-layer.md](../../docs/usecases/agent-continuity-layer.md); [docs/research/apply-to-agent-memory-architecture.md](../../docs/research/apply-to-agent-memory-architecture.md); [docs/WHITEPAPER.md](../../docs/WHITEPAPER.md) §4 layered pipeline.

### Open follow-ups (8)

1. **Encryption / privacy** — AES-256-GCM at-rest+in-transit and a key-recovery story; today artifacts are signed-and-public, encryption was an upstream "default" and is now framed as roadmap rather than a current property. Ref: [docs/competitive-landscape/DRAG_ANALYSIS.md](../../docs/competitive-landscape/DRAG_ANALYSIS.md), [docs/WHITEPAPER.md](../../docs/WHITEPAPER.md) §13.2. _for further validation._
2. **ZK proofs / verifiable recall** — zkTAM-style proofs of embedding correctness and retrieval correctness; credible long-horizon direction once prover cost and proof latency settle, complementing today's hash-and-sign integrity guarantee. Ref: [docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md](../../docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md) §4. _for further validation._
3. **Shared namespaces semantics** — multi-writer authorization, conflict resolution, and namespace isolation rules for multi-agent collaboration on a shared memory surface. Ref: [docs/problems/CONCURRENT_WRITERS.md](../../docs/problems/CONCURRENT_WRITERS.md), [docs/usecases/shared-project-memory-namespace.md](../../docs/usecases/shared-project-memory-namespace.md). _for further validation._
4. **Reliability oracle (full design)** — source-quality scoring across shared-memory contributors, used by orchestrators to weight retrieval results; needs a full design pass covering signal definition, gaming resistance, and on-chain vs off-chain placement. Ref: [docs/usecases/reliability-oracle-for-orchestration.md](../../docs/usecases/reliability-oracle-for-orchestration.md). _for further validation._
5. **Compressed shadow-index recall** — first-stage candidate generation over compressed (TurboQuant) embeddings, full-precision rerank on the short list; a path to scale recall beyond what the current SQLite + uncompressed-f32 cosine path supports. Ref: [docs/WHITEPAPER.md](../../docs/WHITEPAPER.md) §4, [docs/research/apply-to-agent-memory-architecture.md](../../docs/research/apply-to-agent-memory-architecture.md). _for further validation._
6. **Lifecycle policy / eviction tuning** — append/merge/overwrite semantics, retention classes, and deletion semantics across local SQLite vs anchored full-mode storage. Ref: [docs/problems/MEMORY_EVICTION.md](../../docs/problems/MEMORY_EVICTION.md), [docs/WHITEPAPER.md](../../docs/WHITEPAPER.md) §13. _for further validation._
7. **Economic model validation (Arweave pricing, x402 micropayments)** — refresh full-mode persistence cost calibration against current Arweave $/GB and validate the x402 micropayment path end-to-end against the live pricing engine. Ref: [docs/problems/ARWEAVE_PRICING_VALIDATION.md](../../docs/problems/ARWEAVE_PRICING_VALIDATION.md). _for further validation._
8. **Critical review redo** — the upstream `CRITICAL_REVIEW.md` was outdated and intentionally not restored; a fresh critical review against the current Rust implementation is needed to retire or refute its findings. Ref: `sivo4kin/mnemonic-protocol@7a68a973:docs/CRITICAL_REVIEW.md` (not in this repo). _for further validation._

---

## Task 7 — Expand WHITEPAPER §9 + add §References paper.pdf

**Status:** done · **Wave:** 4 · **Date:** 2026-04-27
**Commit:** `654178d`
**Summary:** Expanded `docs/WHITEPAPER.md` §9 from 4 to 10 subsections (one per use-case doc), each closing with `[See deep-dive in docs/usecases/<file>.md.]`; added 8th `## References` entry pointing to `./research/paper.pdf`.
**Verify-smoke:** `grep -cE '^### 9\.' docs/WHITEPAPER.md` == 10 — pass; `./research/paper.pdf` link present — pass.
**Reviewers:** code-reviewer round 1 approved.

---

## Task 8 — README.md Foundational research section

**Status:** done · **Wave:** 4 · **Date:** 2026-04-27
**Commit:** `f64dac6`
**Summary:** Inserted `## Foundational research` H2 in repo `README.md` after the Introduction with one-sentence summary and relative link `docs/research/paper.pdf`. Introduction copy unchanged.
**Verify-smoke:** `grep -A2 'Foundational research' README.md | grep 'docs/research/paper.pdf'` non-empty — pass.
**Reviewers:** code-reviewer round 1 approved.

---

## Task 9 — PK project.md + architecture.md updates

**Status:** done · **Wave:** 4 · **Date:** 2026-04-27
**Commit:** `bc8cbd6`
**Summary:** Added `## Use Case Roles` H2 (10 bulleted roles linking to `docs/usecases/<file>.md`) to `.claude/skills/project-knowledge/references/project.md`. Appended `## Further reading` to `.claude/skills/project-knowledge/references/architecture.md` linking `docs/competitive-landscape/`, `docs/research/condensed-principles.md`, and `docs/research/paper.pdf`. patterns.md untouched.
**Verify-smoke:** both grep patterns matched — pass.
**Reviewers:** code-reviewer round 1 approved (3 minor non-blocking nits noted).

---

## Task 11 — lychee CI workflow

**Status:** done · **Wave:** 4 · **Date:** 2026-04-27
**Commit:** `124e663`
**Summary:** Created `.github/workflows/docs-link-check.yml`. Triggers on push to dev/main and pull_request, paths-filtered to `docs/**` and `*.md`. Single `lychee-link-check` job on `ubuntu-latest` using `actions/checkout@v4` + `lycheeverse/lychee-action@v2` with `--offline` and fail-on-broken-links. ci.yml untouched.
**Verify-smoke:** `test -f .github/workflows/docs-link-check.yml` — pass. Local lychee run skipped (CLI not installed); CI will validate.
**Reviewers:** code-reviewer round 1 approved (1 minor non-blocking redundancy nit).

---

## Task 12 — Documentation Audit (read-only)

**Status:** done · **Wave:** 5 · **Date:** 2026-04-27
**Verdict:** issues_found
**Summary:** Link integrity, README presence in all 4 subdirs, WHITEPAPER §9 deep-dive targets, PK reference paths, and the sanity-grep regression all pass. Two minor findings about `recovered/README.md` staleness ("working drafts, not yet promoted" claim and pending-Wave-3 annotations are no longer accurate) — these are scheduled for normalization in Task 15 (QA), which must populate the "Promoted on YYYY-MM-DD in commit <hash>" line. Three cosmetic nits about naming/linking style. No blocking or major issues.
**Report:** logs/working/audit/documentation-auditor.json
**Findings count:** blocking=0, major=0, minor=2, nit=3

---

## Task 13 — Security Audit (read-only)

**Status:** done · **Wave:** 5 · **Date:** 2026-04-27
**Verdict:** clean
**Summary:** Comprehensive secret scan over all restored content (50 files across `docs/` and `.claude/skills/project-knowledge/recovered/`, including 2 unique PDFs extracted with `pdftotext`). Patterns checked: OpenAI/Anthropic/HuggingFace API keys, Telegram bot tokens, AWS access keys, PEM private keys, GitHub/Google/JWT tokens, Solana base58 keypairs and byte-array keypair JSON, `.env`-style env assignments, generic password/secret/token/api_key/private_key assignments, internal hostnames and RFC1918 IPs, redacted-looking placeholders, raw 64-hex hash strings. The only secret-shaped match was `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` in the Agent-Identity PDF — the well-known public Solana SPL Memo program ID, intentionally cited as a protocol primitive (no private key, no remediation). All HTTP(S) URLs encountered are public references. Token-replacement focus areas (DRAG_ANALYSIS, WEB_RESEARCH_TRUSTLESS_RAG, CONCURRENT_WRITERS, ARWEAVE_PRICING_VALIDATION) contain no hard-coded keys/addresses or redacted strings.
**Report:** logs/working/audit/security-auditor.json
**Findings count:** blocking=0, major=0, minor=0, nit=0
**Stats:** files_scanned=50, pdfs_scanned=2, secret_candidates_found=1 (explained as public Solana SPL Memo program ID), confirmed_real_secrets=0

---

## Task 14 — Validation Audit (read-only)

**Status:** done · **Wave:** 5 · **Date:** 2026-04-27
**Verdict:** issues_found
**Summary:** Validation evidence is substantively complete and reproducible. All 6 token-replace overrides are logged with before/after pairs in `code-research.md` §4.3 (verdict table) and §4.4/§4.5 (post-promotion logs); spot-checks against commits `d8df681` and `eacb6ff` plus on-disk lines (DRAG_ANALYSIS:37 blank, WEB_RESEARCH:45/64/132, CONCURRENT_WRITERS:157/217) all match the claims. The lychee CI workflow correctly filters on `docs/**` + `*.md`, uses `--offline`, sets `fail: true`, and pins both `actions/checkout@v4` and `lycheeverse/lychee-action@v2`. The `decisions.md` follow-up roadmap has the required Browser-WASM sub-section (5 parts) plus 8 numbered bullets each tagged `for further validation`. Sanity regression `grep -RIE 'SHA3|mcp-server-rs|pre-V1|Pre-V1|HashEmbedder|Python backend' docs/ README.md` returns exit 1 (zero hits). Two non-blocking gaps remain that Task 15 must close: (a) `recovered/README.md` still lacks the required `Promoted on YYYY-MM-DD in commit <hash>` line and three rows still say `(token replacements pending Wave 3)` despite Wave 3 being complete — these are explicit acceptance criteria; (b) sanity-grep term set drift between tech-spec/user-spec (6 patterns) and code-research.md §4 (9 patterns; adds `SHA-3`, `sha3`, `mcp-server.py`) — silent today since none of the dropped terms appear in `docs/`, but a future agent reproducing from tech-spec alone gets a narrower sweep than what was actually run.
**Report:** logs/working/audit/validation-auditor.json
**Findings count:** blocking=0, major=1, minor=2, info=4
**Stats:** files_audited=6, commits_spot_checked=3, on-disk_lines_verified=6, replace-token_entries_validated=6/6, sanity_regression=clean

---

## Audit fixer — recovered/README.md promotion stamp

**Status:** done · **Wave:** 5 (post-audit) · **Date:** 2026-04-27
**Commit:** c57c492
**Summary:** Addressed major finding from Task 12 + 14 audits. recovered/README.md now carries the "Promoted on 2026-04-27" stamp with all 4 promotion commit hashes; validation-table rows reworded to past tense reflecting completed Wave 3.

---

## Task 15 — Pre-deploy QA

**Status:** done · **Wave:** 6 · **Date:** 2026-04-27
**Verdict:** pass
**Summary:** All 15 Agent Verification Plan steps passed (14 verified locally, 1 deferred to CI). Restoration counts 3/3/2/3, promotion 11/4/6/4, no docs/historical, zero stale-term hits in docs/+README.md, WHITEPAPER §9 expanded to 10 subsections with paper.pdf §References entry, README foundational-research section, PK pointers in project.md + architecture.md, decisions.md follow-ups complete, code-research.md token-replace log has 13 entries, recovered/README.md carries the promotion stamp, docs-link-check workflow file exists, and no diff under core/, mcp/, webapp/, Cargo.toml, Cargo.lock since the feature began (since 2839122).
**Report:** logs/working/task-15/qa-report.json
**Deferred to CI:** lychee --offline docs/ README.md docs/WHITEPAPER.md — runs via .github/workflows/docs-link-check.yml on PR (lychee CLI not installed locally).
**Deferred to user (Verify-user):** (a) Open PR and confirm only docs-link-check job runs (cargo jobs skipped via paths-ignore). (b) After merge to dev, spot-check Cloudflare Pages preview rebuild and visit a sample doc URL — non-blocking per tech-spec if the docs project is not yet wired in Cloudflare.
