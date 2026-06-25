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

## Wave 2 — manifest endpoint (done)

- Seeding writes `RAG_CHUNK_DIR/knowledge-manifest.json` (standalone, in
  addition to inside the zip). `GET /knowledge-manifest` serves it.
- On-chain anchoring of the chunks is already handled by `sign_memory` when
  `STORAGE_MODE=full` — so D-FUND is the operational flip (fund keypair + set
  prod to full/participate), not new code.

## Wave 3 — client-side trustless RAG

- **D-R resolved: mode (b)** — owner chose full client-side WASM RAG over
  Arweave.
- **Done (core, tested):** `core/src/rag.rs` (`cosine_similarity`,
  `verify_and_extract_memory`) + WASM bindings. The browser verifies each
  chunk's COSE signature itself, so it trusts the bytes, not the gateway.
- **Done (webapp, tested):** `webapp/src/lib/rag.ts` — manifest fetch + Arweave
  fetch + verify + cosine rank, dependency-injected + vitest-covered (8 tests).
  `wasm.ts` typings extended for the two new exports.
- **D-EMB (embedder parity):** the browser must embed the query with the same
  model the corpus used — server fastembed = `all-MiniLM-L6-v2` (384-dim),
  which has a browser twin (`Xenova/all-MiniLM-L6-v2`). BLOCKER: both
  `@xenova/transformers` and `@huggingface/transformers` pull `sharp`, whose
  native libvips binary can't download behind this session's proxy (and would
  risk the Cloudflare/VPS build). Deferred `embedder.ts` + the dep until the
  build env can fetch sharp, or we adopt a sharp-free embedding path (e.g.
  onnxruntime-web directly, or a `transformers.js` build with the Node image
  backend excluded). Owner decision needed on the lib/build approach.
- **Remaining:** (1) D-EMB embedder integration once the lib/build path is
  settled; (2) server `/chat` extension — accept optional client-supplied
  `context` (skip server recall) + `release`/`as_of` (D-VAR) for the version
  selector; (3) UI — version dropdown + "trustless mode" toggle wired through
  `retrieveContext` → `/chat`.
