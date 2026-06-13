---
created: 2026-06-13
status: draft
size: M
branch: feat/delm-integration
tracking_issue: mnemonik-xyz/monorepo#169
---

# Tech Spec: DeLM × Mnemonic integration

## Goal

Back DeLM's shared context with Mnemonic so admitted gists are signed, lineage-linked to
their retained raw evidence, recallable, and reusable **across runs** — without changing
`core/` or `mcp/` in v1. Adopt DeLM's deterministic compactor as a gist producer; achieve
lossless restore via lineage, not via decompaction (see `decisions.md` D1–D3).

## Solution

Three layers, but only Layer 1 is on the v1 critical path.

1. **Adapter (delm repo, Python — v1)** — `src/mnemonic_backend.py`:
   a `MnemonicSharedContext` class with the same surface DeLM already calls on
   `SharedLessons`: `admit(note|result)` and `read(budget, policy)`, plus
   `unfold(label)` / `deep_unfold(label)`. It speaks MCP to a Mnemonic server.
   - `admit`: write the raw trajectory/source as a `rag.context` artifact
     (`mnemonic_sign_memory`), run the compactor to produce the gist text, write the gist
     as a `rag.result` with a `ParentRef{ artifact_id: <raw_id>, role: "context" }`, and
     optionally a middle `rag.context` summary node (hierarchical path). Return the gist
     artifact id.
   - `read`: `mnemonic_recall` over the gist layer, then apply DeLM's existing
     priority/window/dedup on the returned gists (reuse `shared_lessons` helpers).
   - `unfold` / `deep_unfold`: resolve the gist's ancestors and recall the signed
     summary / raw content. **This is the restore path** — no compactor inverse.

2. **Compactor reuse (delm repo — v1)** — wrap `extract_structured_summary` behind a
   `GistExtractor` protocol so the SWE-bench regex set is one implementation. Fallback:
   if the extractor yields no signal, store the payload verbatim as the gist's raw
   `rag.context` and emit a minimal gist — never silently drop content.

3. **Pre-sign veracity gate (monorepo — later wave, optional)** — port the *idea* of
   `verifier.py` into a content-veracity check on the `participate` path in
   `mcp/src/tools.rs`, before COSE signing. Off the v1 path; tracked as a follow-on.

## Architecture

### Data mapping (DeLM → Mnemonic)

| DeLM | Mnemonic artifact | Lineage |
|---|---|---|
| raw `rᵢ` (backing `R`) | `rag.context` | leaf |
| summary `Sᵢ` (backing `L`, optional) | `rag.context` | parent: raw (`role:"context"`) |
| gist `Gᵢ` (admitted to `C`) | `rag.result` | parent: summary-or-raw (`role:"context"`) |
| `thread_id` | COSE producer (Ed25519/DID) | — |
| `UNFOLD` / `DEEP_UNFOLD` | `traverse_lineage(Ancestors)` + recall | — |

### Files

- `delm: src/mnemonic_backend.py` — adapter (`MnemonicSharedContext`, MCP client,
  hierarchy writer, unfold-via-lineage).
- `delm: src/gist_extractor.py` — `GistExtractor` protocol + `SweBenchExtractor`
  wrapping `extract_structured_summary`; verbatim fallback.
- `delm: src/config.py` — add a `shared_context_backend: "memory" | "mnemonic"` switch
  and Mnemonic endpoint/keypair settings.
- `delm: tests/test_mnemonic_backend.py` — round-trip + cross-run reuse tests.
- `monorepo: work/delm-comparison/` — findings, specs, decisions (this dir; no src).

### Key invariants

- **Lossless restore = lineage retention.** Every gist MUST have a reachable signed raw
  ancestor; `unfold` MUST resolve to it. The compactor is never inverted.
- **Determinism.** Gist extraction is deterministic; the same trajectory yields the same
  gist bytes (so canonical CBOR / CID is stable). No LLM in the v1 gist path.
- **No `core/`/`mcp/` change in v1.** Adapter is pure MCP client.
- **Mode.** v1 uses `mode: "local"` writes (free, offline) for the eval harness;
  `participate` (anchored) is a config flag, not required to prove the mechanism.

## Decisions

See `decisions.md`: D1 (compactor is lossy; restore via lineage; not a TurboQuant/SQL
substitute), D2 (reuse existing schemas, no new CBOR version), D3 (client-side Python
adapter, monorepo untouched in v1).

## Testing

- **Round-trip:** admit → recall returns the gist; unfold returns the exact signed raw.
- **Cross-run reuse (the differentiator):** populate context in process A; in a fresh
  process B, `read()` returns A's verified gists; unfold still resolves raw. Proves the
  property DeLM's in-RAM `C` cannot offer.
- **Compactor fidelity:** assert the gist is a strict subset/distillation and that the
  raw ancestor reconstructs full detail; assert verbatim fallback on no-signal input.
- **Determinism:** same trajectory → identical gist bytes and identical CID.
- **Eval (stretch):** run a small SWE-bench Verified subset with both backends; compare
  Avg@1 / cost; confirm Mnemonic backend does not regress single-run metrics while adding
  persistence.

## Tasks / waves

- **Wave 1:** T1 adapter skeleton + MCP client; T2 hierarchy writer + lineage; T3
  compactor behind `GistExtractor` + fallback.
- **Wave 2:** T4 unfold-via-lineage; T5 round-trip + cross-run tests; T6 config switch.
- **Wave 3 (optional/follow-on):** T7 pre-sign veracity gate (monorepo); T8 eval harness.
- **Audit:** code + test review; update `decisions.md`.

## Out of scope

DeLM orchestration in `core/`; replacing TurboQuant or SQL; new crypto primitives; new
CBOR schema versions; marketing Mnemonic as a within-run scratchpad.
