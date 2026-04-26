---
created: 2026-04-26
status: research
type: feature-research
size: M
---

# Code Research: docs-actualization

> Pre-implementation research consolidating: upstream restoration sources, target docs/ layout,
> stale-term sweep, validation tooling, CI behaviour. Updated by sanity-grep tasks during implementation.

## 1. Upstream source

- **Local clone:** `/Users/syi/src/mnemonic-protocol`
- **Branch:** `origin/docs/usecases`
- **HEAD:** `7a68a973a37fb0cc4df548bf13e0687f4ff2b39c` (matches pin in `recovered/README.md`)
- **Commit message:** `docs: add A2A use cases and integration patterns`

## 2. Pre-flight inventory of files to restore (11)

| upstream path | size (bytes / lines) | destination in monorepo |
|---|---|---|
| `docs/DRAG_ANALYSIS.md` | 12984 / 179 | `recovered/competitive-landscape/` |
| `docs/WEB_RESEARCH_TRUSTLESS_RAG.md` | 11563 / 173 | `recovered/competitive-landscape/` |
| `docs/DECENTRALIZED_RAG_LANDSCAPE.md` | 56003 / 325 | `recovered/competitive-landscape/` |
| `docs/TURBOQUANT_DEEP_ANALYSIS.md` | 30641 / 181 | `recovered/research/` (truncated upstream, restore as-is) |
| `docs/apply-to-agent-memory-architecture.md` | 12328 / 431 | `recovered/research/` |
| `docs/condensed-principles.md` | 4696 / 144 | `recovered/research/` |
| `Agent Identity for Autonomous AI_ Protocols, Mnemonic Analysis, and the Path to a Minimal Primitive.pdf` (root) | 55683 | `recovered/research/` (binary) |
| `research/paper.pdf` | 861881 | `recovered/research/paper.pdf` (foundational) |
| `docs/MEMORY_EVICTION.md` | 13177 / 335 | `recovered/problems/` |
| `docs/CONCURRENT_WRITERS.md` | 18309 / 347 | `recovered/problems/` |
| `docs/ARWEAVE_PRICING_VALIDATION.md` | 3793 / 79 | `recovered/problems/` |

**Dropped per owner decision:**
- `docs/MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` — not restored, not archived. `docs/historical/` not created.
- `docs/CRITICAL_REVIEW.md` — not restored. Replaced by follow-up bullet "redo critical review against current Rust impl" in decisions.md.

**Other upstream files (out of scope, explicitly outdated):** `MVP_SPEC.md`, `MVP_VERIFICATION.md`, `DEMO_SPEC.md`, `PROJECT_STATE.md`, `report.md`, `v0/SCOPE.md`, `v1/V1_*.md` (×9), `v1.1/SCOPE.md`, `mcp_server_rs/{API,SPEC}.md`, `diagrams/*.mmd`.

## 3. Restoration command pattern

```bash
git -C /Users/syi/src/mnemonic-protocol show 'origin/docs/usecases:<upstream-path>' \
  > .claude/skills/project-knowledge/recovered/<subdir>/<filename>
```

For PDF files: same command, redirect captures binary bytes correctly.

If local clone is not at `origin/docs/usecases`: `git -C /Users/syi/src/mnemonic-protocol fetch origin` (branch already tracked).

## 4. Sanity-grep results (pre-flight against upstream content)

Initial term set: `SHA3`, `SHA-3`, `mcp-server-rs`, `pre-V1`, `Pre-V1`, `HashEmbedder`, `Python backend`, `mcp-server.py`, `sha3`.

**Hits requiring action (6 token replaces in 3 files; ≪50% per file → drop-rule not triggered):**

| File | Line | Original | Replacement |
|---|---|---|---|
| DRAG_ANALYSIS.md | 37 | `Mnemonic records: SHA3-256 hash of the encrypted memory blob via Solana memo.` | delete the line (encryption is roadmap, not shipped) |
| WEB_RESEARCH_TRUSTLESS_RAG.md | 45 | `\| Status \| Pre-V1, prototype validated \| Live (Kinic-CLI shipped) \|` | replace `Pre-V1, prototype validated` → `active Rust MCP server` |
| WEB_RESEARCH_TRUSTLESS_RAG.md | 64 | `Mnemonic commits the memory blob (SHA3 hash); V3DB proves the retrieval result` | delete `(SHA3 hash)` |
| WEB_RESEARCH_TRUSTLESS_RAG.md | 132 | `\| Mnemonic \| Memory integrity (hash) \| ✅ 4/8-bit \| ✅ \| Arweave+Solana \| Pre-V1 \|` | replace `Pre-V1` → `v1.0 (active)` |
| CONCURRENT_WRITERS.md | 157 | `Adding parent_hashes for DAG structure: each SHA3-256 hash = 64 hex chars.` | replace `SHA3-256` → `blake3` (sizes match) |
| CONCURRENT_WRITERS.md | 217 | `The current system commits SHA3-256(encrypted_blob) to Solana.` | replace `SHA3-256(encrypted_blob)` → `blake3(canonical CBOR bytes)` |

All 6 are `replace-token` overrides per user-spec policy A. Each must be re-logged in this file with before/after during implementation Wave 3 (promotion).

**Files passing verbatim (no hits):** all PDFs (binary, skipped), DECENTRALIZED_RAG_LANDSCAPE.md, TURBOQUANT_DEEP_ANALYSIS.md, apply-to-agent-memory-architecture.md, condensed-principles.md, MEMORY_EVICTION.md, ARWEAVE_PRICING_VALIDATION.md.

## 5. Current state of monorepo paths affected

| path | current state | action in feature |
|---|---|---|
| `docs/` | only `WHITEPAPER.md` (335 lines) | new subfolders: usecases/, competitive-landscape/, research/, problems/ |
| `docs/WHITEPAPER.md` | §9 covers 4 use cases; §References has 7 entries (1-7) | edit §9 to cover all 10; add reference 8 → `docs/research/paper.pdf` |
| `README.md` (root) | introduction + repo layout; no References section | add ref to `docs/research/paper.pdf` (placement: in Introduction or new "Foundational research" subsection) |
| `.claude/skills/project-knowledge/references/project.md` | covers Project Overview, Target Audience, Core Problem, Key Features, MVP Scope, Out of Scope | add "Use Case Roles" section linking to `docs/usecases/` |
| `.claude/skills/project-knowledge/references/architecture.md` | tech-stack, dependencies, data flow, data model | add pointer paragraph to `docs/competitive-landscape/` and to `docs/research/condensed-principles.md` |
| `.claude/skills/project-knowledge/references/patterns.md` | conventions, git workflow, business rules | unchanged unless sanity-grep flags |
| `.claude/skills/project-knowledge/recovered/usecases/` | 10 .md + README (committed in afa20da) | **already present**, promote verbatim to `docs/usecases/` |
| `.claude/skills/project-knowledge/recovered/competitive-landscape/` | only README.md | restore 3 .md, then promote |
| `.claude/skills/project-knowledge/recovered/research/` | not exists | create + restore 3 .md + 2 PDF |
| `.claude/skills/project-knowledge/recovered/problems/` | not exists | create + restore 3 .md |
| `.claude/skills/project-knowledge/recovered/README.md` | classification table for usecases/ + competitive-landscape/ + research/ | extend with problems/, both PDF rows; record drops; add "Promoted on <date> in commit <hash>" line at top after merge |

## 6. CI / tooling state

- **`.github/workflows/ci.yml`** has `paths-ignore` covering `*.md`, `docs/**`, `work/**`, `.claude/**` (per commit dd395fd). Cargo CI **will not run** for any commit in this feature → no risk of red CI from doc changes. To preserve this, no edits to non-doc paths.
- **No lychee or markdownlint configured.** Need to add a CI step to run `lychee --offline docs/` on PR (gating). Two options:
  - (A) New workflow file `.github/workflows/docs-link-check.yml` — runs only when `docs/**` or `*.md` changes (no `paths-ignore`).
  - (B) Run lychee locally before PR; no CI gate.
  Recommendation: (A), so the gate is enforced automatically. Cost: ~5 lines of YAML, ~10s CI time per PR.
- **markdownlint:** no repo config; advisory only — out of CI scope.
- **Cloudflare Pages docs deploy:** per `deployment.md`, configured to auto-deploy on push to main. Verify post-merge that the preview rebuilds.

## 7. Validation tooling

- **lychee:** released as a Rust binary; install via `cargo install lychee` or `lychee-action@v2` in CI (no Rust toolchain needed for the latter, image-based).
- **markdownlint:** Node.js based (`markdownlint-cli`); not used here.
- **grep:** ripgrep available locally. Sanity-grep over upstream files runs against pre-flight `/tmp/` dump or against in-tree `recovered/` after restoration.

## 8. WHITEPAPER §9 expansion target structure

Current §9 has 4 subsections (`9.1 Shared Project Memory`, `9.2 Provenance And Attestation`, `9.3 Portable Memory Wallet`, `9.4 Settlement-Aware Memory Infrastructure`). Target: 10 subsections matching `recovered/usecases/`:

| §  | Use case | source doc |
|----|----------|-----------|
| 9.1 | Shared Project Memory | usecases/shared-project-memory-namespace.md |
| 9.2 | Shared Memory Layer | usecases/shared-memory-layer.md |
| 9.3 | Provenance And Attestation Layer | usecases/provenance-attestation-layer.md |
| 9.4 | Trust And Reputation Layer | usecases/trust-reputation-layer.md |
| 9.5 | Portable Memory Wallet | usecases/portable-memory-wallet.md |
| 9.6 | Settlement-Aware Memory Infrastructure | usecases/settlement-aware-memory-infrastructure.md |
| 9.7 | Task Memory Ledger | usecases/task-memory-ledger.md |
| 9.8 | Artifact Attestation Service | usecases/artifact-attestation-service.md |
| 9.9 | Agent Continuity Layer | usecases/agent-continuity-layer.md |
| 9.10 | Reliability Oracle For Orchestration | usecases/reliability-oracle-for-orchestration.md |

Each subsection: 1-2 sentences quoting/paraphrasing the use-case doc + `[See deep-dive in docs/usecases/<file>.md.]`. ~30-40 lines added to WHITEPAPER total.

## 9. decisions.md follow-up roadmap target structure

1. **Browser-WASM verification UI** — full sub-section: Problem (1-2 paragraphs), Proposed Approach (~1 paragraph), Dependencies (mnemonic-core WASM target from completed mnemonic-core feature; webapp can reuse `whoami`/`verify` exports), Open Questions (4-5 bullets), Source-doc refs.
2-9. Bullet list (1-2 sentences each + ref + `for further validation`):
- Encryption (AES-256-GCM at-rest+in-transit) — DRAG_ANALYSIS + WHITEPAPER §13.2
- ZK proofs (zkTAM-style embedding/retrieval correctness) — WEB_RESEARCH §4
- Shared namespaces multi-writer semantics — CONCURRENT_WRITERS + usecases/shared-project-memory-namespace
- Reliability oracle — usecases/reliability-oracle-for-orchestration
- Compressed shadow-index recall path — WHITEPAPER §4 + research/apply-to-agent-memory-architecture
- Memory lifecycle policy / eviction — MEMORY_EVICTION + WHITEPAPER §13
- Economic model validation / Arweave pricing — ARWEAVE_PRICING_VALIDATION
- Critical review redo against current Rust impl — sivo4kin@7a68a973:docs/CRITICAL_REVIEW.md (not restored)

## 10. Open implementation questions

- **paper.pdf title in §References citation.** PDF title not extracted yet; either open the PDF during implementation to copy the title, or use a generic citation `Mnemonic Protocol Foundational Paper (paper.pdf)` and link relatively. Decision deferred to Task 7.
- **README.md placement.** "Introduction" is currently 30 lines from top, then "Repository layout". Cleanest insertion: a new line in Introduction "This work builds on the foundational research in [paper.pdf](docs/research/paper.pdf)." Or a new H2 section "## Foundational research" right after Introduction. Decision deferred to Task 8.
- **lychee CI job.** Add it in this feature or as a separate small chore? Recommendation: add inside this feature (Task 12) so the gate exists from day one.
