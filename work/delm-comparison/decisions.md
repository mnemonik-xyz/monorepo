# Decisions — DeLM × Mnemonic integration

Append-only log. Newest at bottom.

---

## D1 — The DeLM memory compactor is lossy distillation, NOT reversible compression

**Question raised:** Does `memory_compactor.py` let us restore memory without losing
logic? Can it replace TurboQuant and let us drop SQL?

**Finding — it is one-way and lossy.** `extract_structured_summary(records)` runs a
fixed set of **regexes** over an agent's trajectory and emits a fixed-shape *string*:
edited `.py` files, failing pytest targets, exception class names, `file.py:NNN` refs
(cap 8), recent commands (truncated to 80 chars, cap 8), the latest `PATCH_SUMMARY`
note, and unresolved exception classes. Everything not matched by those regexes is
**discarded**. There is **no inverse function** anywhere in the DeLM repo (confirmed: a
repo-wide search for `restore` / `decompress` / `rehydrate` / `expand` returns zero
hits). You cannot reconstruct the trajectory from the summary.

**Why DeLM still "loses no logic":** not because the compactor is reversible, but because
DeLM **retains the raw trajectory separately** (backing store `R[ℓ]`, paper §A.1/A.2) and
recovers detail by **selective unfolding back to that retained raw** — never by inverting
the compaction. Fidelity = retention + unfold, not decompaction.

**Three further constraints:**
1. **Domain-specific.** The regexes are SWE-bench/Python/pytest-shaped (`_PY_FILE_RE`,
   `_PYTEST_TARGET_RE`, `*Error|*Exception|*Warning`). On arbitrary memory content it
   captures little or nothing. It needs a pluggable extractor set to generalize.
2. **It compresses TEXT, TurboQuant compresses VECTORS — different axes, not
   substitutes.** TurboQuant quantizes the f32 embedding (the *recall key*) for
   portability/anchor cost; recall still uses uncompressed f32 (CLAUDE.md: "Recall uses
   uncompressed f32 embeddings in SQLite"). The compactor distills the *payload text*.
   You cannot run cosine recall over a regex summary. They are complementary
   (compactor shrinks payload, TurboQuant shrinks key), never one-for-the-other.
3. **It is not a store and cannot replace SQL.** SQLite is the hot vector index +
   lineage tables that make semantic recall and `traverse_lineage` possible. The
   compactor outputs a string; removing SQL removes recall and lineage entirely.

**Decision.**
- **Adopt** the compactor as a **deterministic, zero-LLM gist producer** for one artifact
  role (the gist), behind a pluggable-extractor interface. Determinism fits Mnemonic's
  canonical-CBOR reproducibility ethos (unlike the LLM summarizer/verifier).
- **Reject** it as a replacement for TurboQuant or SQL.
- **Lossless restore is achieved by lineage, not by the compactor:** sign the raw
  trajectory as a `rag.context` artifact, sign the gist as a `rag.result`, link
  gist→(summary)→raw via `ParentRef`. "Unfold" = `traverse_lineage(Ancestors)` + recall
  of the signed raw. This gives the property the compactor alone cannot.

---

## D2 — v1 reuses existing schemas; no new crypto, no new CBOR schema version

The gist→summary→raw hierarchy maps onto existing `ArtifactType`s:
`rag.context` (raw + grounded summary) → `rag.result` (gist), linked by `ParentRef`
(`role: "context"`). No change to `core/src/codec/schema.rs` enums or any schema version
in v1 (mirrors the `a2a-bridge` "all reuse" principle). A dedicated `delm.gist` schema is
deferred to a later iteration if metadata needs outgrow `metadata`/`tags`.

---

## D3 — v1 adapter is client-side Python in the `delm` repo; monorepo stays untouched

`mnemonic_sign_memory` / `mnemonic_recall` already exist over MCP, so backing DeLM's
shared context needs **no `core/` or `mcp/` change** for a first cut. The adapter is a
Python module in `mnemonik-dev/delm` implementing the same `read()/admit()` surface as
`SharedLessons`. The pre-sign veracity gate (borrow from `verifier.py`) is a **separate,
later** monorepo change, not on the v1 critical path.
