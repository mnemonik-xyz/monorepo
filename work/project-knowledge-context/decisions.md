# Decisions — Versioned, Arweave-anchored Project Knowledge

Append-only log.

## 2026-06-25 — Feature framed on top of PR #173

- PR #173 (merged, `7df94c2`) fixed the live "no context" chat bug
  (seed idempotency: `store.count(signer)` froze the corpus after the first
  user write) and added `deploy-mcp.yml` / `deploy-webapp.yml`. Prod unblock
  is gated on 6 repo secrets being set (VPS_* + CLOUDFLARE_*) — owner action.
- This feature is the *next layer*: version-stamping, release manifest,
  version-aware chat, on-chain anchoring, webapp Arweave retrieval.

## Open decisions for the owner

- **D-R (retrieval model)**: (a) server RAG + Arweave-for-verifiability
  [recommended] vs (b) full client-side WASM RAG over Arweave. Default to (a);
  (b) is a later opt-in.
- **D-FUND (anchoring)**: fund the prod keypair to flip seeding from `local`
  to `participate`/`full` so the corpus actually lands on Arweave + Solana.
  Until then, version/manifest machinery runs in `local` with synthetic
  `local:` tx ids.

## Wave 1 implementation notes

- Release id source: `MNEMONIC_KNOWLEDGE_RELEASE` env override, else
  `CARGO_PKG_VERSION` (mcp crate = 0.2.4). Lets a docs-only reseed mint a
  distinct corpus version without a crate bump.
- `corpus_ts` is one timestamp per seed run (not per chunk) so "as_of"
  selects a coherent corpus.
- Additive tags only — no DB migration; `protocol-knowledge` filter and
  `build_context` are untouched.
