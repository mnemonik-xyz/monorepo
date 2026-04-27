# Recovered Mnemonic docs (staging area)

**Status:** working drafts, not yet promoted to public `docs/`.
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
├── usecases/                                   <- 10 use-case docs + index (evergreen, ready to promote)
├── competitive-landscape/
│   ├── DECENTRALIZED_RAG_LANDSCAPE.md          <- academic survey (evergreen)
│   ├── DRAG_ANALYSIS.md                        <- VALIDATED (blake3/CBOR/COSE corrections applied)
│   ├── WEB_RESEARCH_TRUSTLESS_RAG.md           <- VALIDATED (status updated, encryption framing fixed)
│   └── MCP_SERVER_BACKEND_FEATURES_COMPARISON.md <- ARCHIVED (Python backend retired; links to old code)
├── problems/                                   <- 3 problem-statement docs (NEW)
│   ├── MEMORY_EVICTION.md                      <- open system problem statement
│   ├── CONCURRENT_WRITERS.md                   <- multi-agent shared-context write semantics
│   └── ARWEAVE_PRICING_VALIDATION.md           <- economic-model validation for full-mode persistence
└── research/                                   <- 3 .md + 2 PDFs incl. foundational paper.pdf (NEW)
    ├── TURBOQUANT_DEEP_ANALYSIS.md             <- evergreen (source was truncated; closed cleanly)
    ├── apply-to-agent-memory-architecture.md   <- evergreen
    ├── condensed-principles.md                 <- evergreen
    ├── Agent Identity for Autonomous AI_ ... .pdf  <- mnemonic-positioning analysis paper (PDF)
    └── paper.pdf                               <- foundational scientific paper (PDF)
```

## Validation status per document

| Document | State | Action |
|---|---|---|
| `usecases/*.md` (×10 + README) | Evergreen, aligns with WHITEPAPER §9 | Ready to promote verbatim |
| `competitive-landscape/DECENTRALIZED_RAG_LANDSCAPE.md` | Academic survey, no Mnemonic-specific claims | Ready to promote verbatim |
| `competitive-landscape/DRAG_ANALYSIS.md` | Edited: blake3 over canonical CBOR + COSE_Sign1; encryption reframed as roadmap; ADR refs reframed as design-lineage refs; capability table updated to reflect current `core/` modules | Review and promote |
| `competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md` | Edited: zkTAM table updated; status "Pre-V1" → "active Rust MCP server"; SHA3 → blake3+CBOR+COSE; encryption framing fixed | Review and promote |
| `competitive-landscape/MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` | Archive header added; links to `feat/auth` and `legacy` branches in `sivo4kin/mnemonic-protocol`; current backend pointed at `mcp/` in this monorepo | Keep as historical reference |
| `research/TURBOQUANT_DEEP_ANALYSIS.md` | Source was truncated mid-Mermaid block; closed cleanly with a recovery note | Ready to promote |
| `research/apply-to-agent-memory-architecture.md` | Architectural recommendations from TurboQuant — generic enough to apply to current `core/src/compress/` design | Ready to promote |
| `research/condensed-principles.md` | TurboQuant design principles, evergreen | Ready to promote |
| `problems/MEMORY_EVICTION.md` | Open system problem statement: lifecycle/retention/pruning policy. | Promoted to docs/problems/ |
| `problems/CONCURRENT_WRITERS.md` | Open problem: shared-context multi-agent write semantics. | Promoted to docs/problems/ with 2 token replaces (SHA3-256 → blake3, encrypted_blob → canonical CBOR bytes). |
| `problems/ARWEAVE_PRICING_VALIDATION.md` | Economic-model validation for full-mode persistence. | Promoted to docs/problems/ |
| `research/Agent Identity for Autonomous AI_...pdf` | Mnemonic-positioning analysis paper (PDF). | Promoted to docs/research/ |
| `research/paper.pdf` | **Foundational scientific paper that motivated the project.** | Promoted to docs/research/; referenced from WHITEPAPER §References and root README. |

## Dropped per owner decision

- `competitive-landscape/MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` — not restored, not archived. `docs/historical/` is not created.
- `CRITICAL_REVIEW.md` (upstream `docs/CRITICAL_REVIEW.md`) — not restored. Outdated. Tracked as a follow-up bullet in `work/docs-actualization/decisions.md` ("Critical review redo against current Rust impl").

## Next steps

1. Walk through each document in this folder against the current `core/` and `mcp/` source.
2. Move ready docs to the public `docs/` tree (suggested: `docs/usecases/`, `docs/competitive-landscape/`, `docs/research/`, `docs/historical/MCP_SERVER_BACKEND_FEATURES_COMPARISON.md`).
3. Drop or summarize anything that no longer matches the shipped implementation.

## Source pointers

- Recovered from: <https://github.com/sivo4kin/mnemonic-protocol/tree/docs/usecases/docs>
- Companion (newer) restore branch: <https://github.com/sivo4kin/mnemonic-protocol/tree/fix/restore-docs/docs> — contains `IMPLEMENTATION_AUDIT.md` and `IMPLEMENTATION_STATUS.md` that describe the Rust implementation truth as of 2026-04-15.
- Current implementation: <https://github.com/mnemonik-xyz/monorepo>
