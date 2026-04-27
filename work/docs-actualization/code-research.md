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

## 4. Sanity-grep results

Term set (final): `SHA3`, `SHA-3`, `sha3`, `mcp-server-rs`, `pre-V1`, `Pre-V1`, `HashEmbedder`, `Python backend`, `mcp-server.py`.

### 4.1 Pre-flight expectations (against upstream `sivo4kin@7a68a973`)

6 token-replace hits in 3 files; ≪50% per file → drop-rule not triggered. All 6 are `replace-token` overrides per Decision 3 (1-token replacement override on delete-only policy).

### 4.2 Final sanity-grep run against restored `recovered/<subdir>/` (run 2026-04-27)

```bash
grep -rInE 'SHA3|SHA-3|sha3|mcp-server-rs|pre-V1|Pre-V1|HashEmbedder|Python backend|mcp-server\.py' \
  .claude/skills/project-knowledge/recovered/competitive-landscape/ \
  .claude/skills/project-knowledge/recovered/research/ \
  .claude/skills/project-knowledge/recovered/problems/
```

**Result:** 9 raw hits across 4 files. 6 hits in restored content files (parity with pre-flight). 3 hits in `recovered/competitive-landscape/README.md` (lines 8, 9, 10) are descriptive prose authored by the upstream owner about the validation work itself ("SHA3 references replaced with `blake3 over canonical CBOR + COSE_Sign1`", "`Pre-V1, prototype validated` → `active Rust MCP server`", "Python (`mcp-server/`) vs Rust (`mcp-server-rs/`) backend comparison") — not stale claims, but historical narrative referencing the very tokens being replaced. Verdict for those: `keep` in `recovered/`; the README will be adapted (drop the MCP_SERVER... bullet) during Task 4 promotion to `docs/competitive-landscape/README.md`, which is a separate, owner-driven rewrite of that index.

**Cross-check against current code (`core/src/**/*.rs`, `mcp/src/**/*.rs`):** zero hits for `SHA3|sha3|HashEmbedder|mcp-server-rs|Pre-V1|pre-V1|Python backend|mcp-server.py`. Only false positive: `core/src/arweave/mod.rs` defines `fn sha384(...)` (Arweave deep-hash), which is the correct shipped algorithm and not a member of the term set. **Conclusion:** all 6 in-content hits are confirmed stale; no new stale terms surfaced; term set unchanged from pre-flight.

### 4.3 Per-hit table (final, with verdicts)

| File | Line | Original | Replacement | Verdict |
|---|---|---|---|---|
| competitive-landscape/DRAG_ANALYSIS.md | 37 | `Mnemonic records: SHA3-256 hash of the encrypted memory blob via Solana memo.` | delete the entire line (encryption is roadmap, not shipped; line is standalone in the table-context and removing it preserves paragraph coherence) | `replace-token` (delete-line variant) |
| competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md | 45 | `\| Status \| Pre-V1, prototype validated \| Live (Kinic-CLI shipped) \|` | `\| Status \| active Rust MCP server \| Live (Kinic-CLI shipped) \|` (replace `Pre-V1, prototype validated` → `active Rust MCP server`) | `replace-token` |
| competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md | 64 | `Mnemonic commits the memory blob (SHA3 hash); V3DB proves the retrieval result` | `Mnemonic commits the memory blob; V3DB proves the retrieval result` (delete ` (SHA3 hash)`) | `replace-token` (delete-tokens variant) |
| competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md | 132 | `\| Mnemonic \| Memory integrity (hash) \| ✅ 4/8-bit \| ✅ \| Arweave+Solana \| Pre-V1 \|` | `\| Mnemonic \| Memory integrity (hash) \| ✅ 4/8-bit \| ✅ \| Arweave+Solana \| v1.0 (active) \|` (replace `Pre-V1` → `v1.0 (active)`) | `replace-token` |
| problems/CONCURRENT_WRITERS.md | 157 | `Adding parent_hashes for DAG structure: each SHA3-256 hash = 64 hex chars.` | `Adding parent_hashes for DAG structure: each blake3 hash = 64 hex chars.` (replace `SHA3-256` → `blake3`; both produce 32-byte / 64-hex-char outputs, table sizing preserved) | `replace-token` |
| problems/CONCURRENT_WRITERS.md | 217 | `The current system commits SHA3-256(encrypted_blob) to Solana.` | `The current system commits blake3(canonical CBOR bytes) to Solana.` (replace `SHA3-256(encrypted_blob)` → `blake3(canonical CBOR bytes)`; aligns with shipped data-flow per architecture.md) | `replace-token` |

All 6 hits are `replace-token` overrides per Decision 3. Each before/after will be re-logged in this file during Wave 3 (Tasks 4 and 6) at promotion time.

**Files passing verbatim (no hits):** all PDFs (binary, skipped by grep without `-a`), `competitive-landscape/DECENTRALIZED_RAG_LANDSCAPE.md`, `research/TURBOQUANT_DEEP_ANALYSIS.md`, `research/apply-to-agent-memory-architecture.md`, `research/condensed-principles.md`, `problems/MEMORY_EVICTION.md`, `problems/ARWEAVE_PRICING_VALIDATION.md`.

**No `drop-file` verdicts.** No `keep` verdicts on stale-content lines. Owner-policy drops (`MCP_SERVER_BACKEND_FEATURES_COMPARISON.md`, `CRITICAL_REVIEW.md`) are handled in Wave 1 (not restored at all) per Decision 2 and remain out of scope for the sanity-grep table.

Sanity-grep run completed 2026-04-27 against `recovered/{competitive-landscape,research,problems}/`. Total 6 in-content hits across 3 files (plus 3 narrative mentions in `recovered/competitive-landscape/README.md` classified `keep`). 6 token-replace overrides applied; no further stale terms detected; no drop-file verdicts.

### 4.4 Wave 3 Task 4 — promotion log

Applied during promotion of `recovered/competitive-landscape/` → `docs/competitive-landscape/` on 2026-04-27. Each entry quotes the exact line as written in the destination file after edit.

**Entry 1 — `docs/competitive-landscape/DRAG_ANALYSIS.md` line 37 (delete-line)**
- Before: `Mnemonic records: SHA3-256 hash of the encrypted memory blob via Solana memo.`
- After: (line removed; preceding line `D-RAG records: source reliability scores via smart contracts.` is now followed by the blank line and the `Both systems are cheap to run on-chain ...` paragraph)

**Entry 2 — `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md` line 45 (replace-token)**
- Before: `| Status | Pre-V1, prototype validated | Live (Kinic-CLI shipped) |`
- After: `| Status | active Rust MCP server | Live (Kinic-CLI shipped) |`

**Entry 3 — `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md` line 64 (replace-token, delete-tokens variant)**
- Before: `- Mnemonic commits the memory blob (SHA3 hash); V3DB proves the retrieval result`
- After: `- Mnemonic commits the memory blob; V3DB proves the retrieval result`

**Entry 4 — `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md` line 132 (replace-token)**
- Before: `| Mnemonic | Memory integrity (hash) | ✅ 4/8-bit | ✅ | Arweave+Solana | Pre-V1 |`
- After: `| Mnemonic | Memory integrity (hash) | ✅ 4/8-bit | ✅ | Arweave+Solana | v1.0 (active) |`

Verify-smoke: `grep -RIE 'SHA3|Pre-V1|pre-V1' docs/competitive-landscape/` returned empty; `ls docs/competitive-landscape/*.md | wc -l` returned `4`.

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
