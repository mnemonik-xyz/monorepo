# Execution Plan: docs-actualization

**Branch:** main (per checkpoint; no separate feature branch in this monorepo)
**Total waves:** 6
**Status:** resuming (Wave 1 work completed in prior session via commits 584001f, 39946a4, 6387f69, 21ac4a1)

## Wave 1: Restoration (1 task)
- **Task 1** — Restore 11 upstream files + extend recovered/README.md
  - Work already complete in prior session (4 docs(recovered) commits)
  - Action this session: spawn code-reviewer to formalize review of completed restoration
  - Write decisions.md entry retroactively
  - Mark status: done

## Wave 2: Sanity-grep finalization (1 task, depends on Wave 1)
- **Task 2** — Run final sanity-grep + finalize code-research.md
  - Skill: code-writing
  - Reviewers: code-reviewer, test-reviewer

## Wave 3: Promotion (4 tasks parallel, depends on Wave 2)
- **Task 3** — Promote recovered/usecases → docs/usecases/ (skill: code-writing, reviewer: code-reviewer)
- **Task 4** — Promote recovered/competitive-landscape → docs/competitive-landscape/ (4 token replaces) (skill: code-writing, reviewers: code-reviewer, test-reviewer)
- **Task 5** — Promote recovered/research → docs/research/ (incl. 2 PDFs) (skill: code-writing, reviewer: code-reviewer)
- **Task 6** — Promote recovered/problems → docs/problems/ (2 token replaces) (skill: code-writing, reviewers: code-reviewer, test-reviewer)

## Wave 4: Documentation edits (5 tasks parallel, depends on Wave 3)
- **Task 7** — Expand WHITEPAPER §9 + paper.pdf reference (depends_on: 3,5)
- **Task 8** — README.md Foundational research section (depends_on: 5)
- **Task 9** — PK project.md + architecture.md updates (depends_on: 3,4,5)
- **Task 10** — decisions.md follow-up roadmap
- **Task 11** — lychee CI workflow

All Wave 4 tasks: skill code-writing or documentation-writing, reviewer code-reviewer.

## Wave 5: Audit (3 tasks parallel, depends on Wave 4)
- **Task 12** — Documentation Audit (skill: code-reviewing, reviewers: none)
- **Task 13** — Security Audit (skill: security-auditor, reviewers: none)
- **Task 14** — Validation Audit (skill: test-master, reviewers: none)

## Wave 6: Pre-deploy QA (1 task, depends on Wave 5)
- **Task 15** — Run all 15 verification steps from Agent Verification Plan

## User Checks (post-merge)
- Open PR on GitHub → confirm only `docs-link-check` job ran (no cargo jobs)
- After merge to dev, observe Cloudflare Pages preview build
- Visit a sample doc URL once preview is live

## Conflict-avoidance summary
- Wave 1 owns recovered/README.md
- Wave 2 owns code-research.md (initial)
- Wave 3: Tasks 4 and 6 also append to code-research.md — sequence those edits within their tasks (they touch different sections)
- Wave 4: 5 disjoint write targets — no conflict

## Notes on adaptation
- TeamCreate/SendMessage tools not available in this environment.
- Workflow adapted: lead spawns teammate first; after teammate commits + writes decisions.md, lead spawns reviewer(s) on the resulting diff. If reviewer finds blocking issues, lead spawns ad-hoc fixer.
