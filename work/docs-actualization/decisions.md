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
