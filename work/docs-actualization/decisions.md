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
