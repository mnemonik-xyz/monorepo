# Recovered Mnemonic docs (staging area)

**Status:** Promoted to `docs/` tree on 2026-04-27.
**Promoted on 2026-04-27 in commits 8ed0b6c (usecases), d8df681 (competitive-landscape), 82704c7 (research), eacb6ff (problems).**
**Recovered:** 2026-04-26 from `sivo4kin/mnemonic-protocol@docs/usecases` (commit `7a68a973`).
**Validation target:** `mnemonik-xyz/monorepo@main` — Rust `core/` + `mcp/`.

## Why this folder exists

These documents were written when Mnemonic lived in the prototype repo `sivo4kin/mnemonic-protocol` and described an earlier design (encrypted blobs, SHA3-256 commitments, Python + Rust backends, ADR-numbered design records). They were accidentally destroyed by an agent and then recovered from the `docs/usecases` branch.

Rather than push them straight into the public `docs/` folder of the monorepo, they are staged here under `.claude/skills/project-knowledge/recovered/` so the team can:

1. Re-read each document against the current Rust monorepo architecture.
2. Decide which docs to promote to `docs/`, which to keep internal, and which to discard.
3. Apply targeted validation edits where the original text contradicts the shipped implementation.

## Layout

```
recovered/
├── README.md                                   <- this file
├── usecases/                                   <- 10 use-case docs + index (evergreen, promoted verbatim in 8ed0b6c)
├── competitive-landscape/
│   ├── DECENTRALIZED_RAG_LANDSCAPE.md          <- academic survey (evergreen, promoted verbatim in d8df681)
│   ├── DRAG_ANALYSIS.md                        <- restored verbatim (1 token replacement applied during promotion in d8df681)
│   └── WEB_RESEARCH_TRUSTLESS_RAG.md           <- restored verbatim (3 token replacements applied during promotion in d8df681)
├── problems/                                   <- 3 problem-statement docs (promoted in eacb6ff)
│   ├── MEMORY_EVICTION.md                      <- open system problem statement
│   ├── CONCURRENT_WRITERS.md                   <- multi-agent shared-context write semantics (2 token replacements applied during promotion in eacb6ff)
│   └── ARWEAVE_PRICING_VALIDATION.md           <- economic-model validation for full-mode persistence
└── research/                                   <- 3 .md + 2 PDFs incl. foundational paper.pdf (promoted in 82704c7)
    ├── TURBOQUANT_DEEP_ANALYSIS.md             <- evergreen (source was truncated; closed cleanly)
    ├── apply-to-agent-memory-architecture.md   <- evergreen
    ├── condensed-principles.md                 <- evergreen
    ├── Agent Identity for Autonomous AI_ ... .pdf  <- mnemonic-positioning analysis paper (PDF)
    └── paper.pdf                               <- foundational scientific paper (PDF)
```

## Validation status per document

| Document | State | Action |
|---|---|---|
| `usecases/*.md` (×10 + README) | Evergreen, aligns with WHITEPAPER §9 | Promoted verbatim to `docs/usecases/` (commit 8ed0b6c) |
| `competitive-landscape/DECENTRALIZED_RAG_LANDSCAPE.md` | Academic survey, no Mnemonic-specific claims | Promoted verbatim to `docs/competitive-landscape/` (commit d8df681) |
| `competitive-landscape/DRAG_ANALYSIS.md` | Restored verbatim; 1 token replacement (line 37 `SHA3-256` → `blake3`) applied during promotion. | Promoted to `docs/competitive-landscape/` with token replacement (commit d8df681) |
| `competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md` | Restored verbatim; 3 token replacements (lines 45/64/132) applied during promotion. | Promoted to `docs/competitive-landscape/` with token replacements (commit d8df681) |
| `research/TURBOQUANT_DEEP_ANALYSIS.md` | Source was truncated mid-Mermaid block; closed cleanly with a recovery note | Promoted verbatim to `docs/research/` (commit 82704c7) |
| `research/apply-to-agent-memory-architecture.md` | Architectural recommendations from TurboQuant — generic enough to apply to current `core/src/compress/` design | Promoted verbatim to `docs/research/` (commit 82704c7) |
| `research/condensed-principles.md` | TurboQuant design principles, evergreen | Promoted verbatim to `docs/research/` (commit 82704c7) |
| `problems/MEMORY_EVICTION.md` | Open system problem statement: lifecycle/retention/pruning policy. | Promoted verbatim to `docs/problems/` (commit eacb6ff) |
| `problems/CONCURRENT_WRITERS.md` | Open problem: shared-context multi-agent write semantics; 2 token replacements (SHA3-256 → blake3, encrypted_blob → canonical CBOR bytes; lines 157/217) applied during promotion. | Promoted to `docs/problems/` with 2 token replacements (commit eacb6ff) |
| `problems/ARWEAVE_PRICING_VALIDATION.md` | Economic-model validation for full-mode persistence. | Promoted verbatim to `docs/problems/` (commit eacb6ff) |
| `research/Agent Identity for Autonomous AI_...pdf` | Mnemonic-positioning analysis paper (PDF). | Promoted verbatim (binary copy) to `docs/research/` (commit 82704c7) |
| `research/paper.pdf` | **Foundational scientific paper that motivated the project.** | Promoted verbatim (binary copy) to `docs/research/` (commit 82704c7); referenced from WHITEPAPER §References and root README. |

## Dropped per owner decision

- `competitive-landscape/MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` — not restored, not archived. `docs/historical/` is not created.
- `CRITICAL_REVIEW.md` (upstream `docs/CRITICAL_REVIEW.md`) — not restored. Outdated. Tracked as a follow-up bullet in `work/docs-actualization/decisions.md` ("Critical review redo against current Rust impl").

## Next steps

Promotion completed on 2026-04-27 (commits 8ed0b6c, d8df681, 82704c7, eacb6ff). This folder is retained as the audit trail of the recovered originals; the live tree lives under `docs/`. Future de-staling and follow-up roadmap items are tracked in `work/docs-actualization/decisions.md`.

## Source pointers

- Recovered from: <https://github.com/sivo4kin/mnemonic-protocol/tree/docs/usecases/docs>
- Companion (newer) restore branch: <https://github.com/sivo4kin/mnemonic-protocol/tree/fix/restore-docs/docs> — contains `IMPLEMENTATION_AUDIT.md` and `IMPLEMENTATION_STATUS.md` that describe the Rust implementation truth as of 2026-04-15.
- Current implementation: <https://github.com/mnemonik-xyz/monorepo>
