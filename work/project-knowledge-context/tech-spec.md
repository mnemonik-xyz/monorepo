# Tech Spec — Versioned, Arweave-anchored Project Knowledge

## Context

`/chat` is a server-side RAG endpoint. At boot, `mcp/src/seed.rs::run`
walks `docs/**/*.md`, chunks each markdown, and `sign_memory`s every chunk
under the **server keypair** with tags `["protocol-knowledge", <rel_path>]`.
`chat_handler` recalls top-k chunks scoped to the server owner pubkey and
feeds them to the LLM. Recall uses the **uncompressed f32 embeddings in
SQLite**; the compressed TurboQuant bytes on Arweave are proof-of-existence
(per `CLAUDE.md`).

PR #173 fixed seed idempotency (mtime-aware + `MNEMONIC_FORCE_RESEED`) so the
corpus tracks `docs/` again. This spec builds the *versioned, anchored,
retrievable* layer on top.

## Goals

- **G1 Anchor** the knowledge corpus on Arweave + Solana (not just SQLite).
- **G2 Version** every chunk with a release id + corpus timestamp, and emit
  a per-release **manifest** that indexes all chunks (one Arweave tx = entry
  point to a whole release's knowledge).
- **G3 Version-aware answers**: `/chat` answers "as of release X" / "as of
  timestamp T"; default = latest.
- **G4 Webapp retrieval**: select a version, fetch+verify from Arweave, RAG.

## Non-goals (this feature)

- Changing TurboQuant bit width (would break recall comparability — see
  `CLAUDE.md`). Versioning is metadata-only on top of existing embeddings.
- Per-user personalized chat (corpus stays server-owned, public).

## Key decisions

### D-VER — Release identity
Each seeded chunk gains two extra tags beyond the existing
`protocol-knowledge` + rel-path:
- `release:<semver>` — from `env!("CARGO_PKG_VERSION")` of `mnemonic-mcp`
  (currently `0.2.4`), overridable via `MNEMONIC_KNOWLEDGE_RELEASE` so a
  docs-only re-seed can stamp a distinct corpus version (e.g. `0.2.4+wp2`).
- `corpus_ts:<RFC3339>` — single seed-run timestamp shared by every chunk in
  the run, so "as_of T" selects a coherent corpus, not a mix of runs.

Tags are already `Vec<String>` per attestation and already round-trip through
recall, so this is additive — no schema migration.

### D-MAN — Release manifest
After signing all chunks, emit `knowledge-manifest.json`:
```
{ "release": "0.2.4", "corpus_ts": "...", "git_sha": "<MNEMONIC_BUILD_SHA|unknown>",
  "chunk_count": N, "embed_provider": "...", "turbo_bits": 4,
  "chunks": [ { "rel_path", "heading", "content_hash", "arweave_tx", "attestation_id" } ] }
```
The manifest is itself `sign_memory`'d (tag `knowledge-manifest` +
`release:<semver>`) so it gets its own Arweave tx — the **single entry
point** a client resolves to enumerate a release. It is also written into the
downloadable `protocol-knowledge.zip` alongside `knowledge.md`/`knowledge.json`.

### D-VAR — Version-aware recall (G3)
`tools::recall` gains an optional `release: Option<&str>` filter and an
`as_of: Option<&str>` (RFC3339) filter, applied as a post-filter on the
`release:`/`corpus_ts:` tags of candidate rows (SQLite search already returns
tags). `chat::ChatRequest` gains optional `release` / `as_of`. Default
(both `None`) = latest `corpus_ts` present → preserves today's behavior.

### D-R — Retrieval model (NEEDS OWNER SIGN-OFF)
Two ways to satisfy "webapp retrieves from Arweave, uncompress, RAG":
- **(a) Server RAG + Arweave for verifiability/version (recommended).**
  Semantic recall stays server-side over SQLite f32 (accurate, cheap). The
  webapp uses the manifest tx to *fetch + verify + download* a release's
  knowledge from Arweave and to drive the version selector. Honors the
  existing "Arweave = proof-of-existence" design.
- **(b) Full client-side trustless RAG.** Webapp pulls compressed embeddings
  from Arweave, decompresses in WASM, runs cosine in-browser. Literal match
  to the ask, but lower precision (2–4 bit quantization), heavy WASM +
  bandwidth. Offer later as an opt-in "trustless mode".

### D-FUND — Anchoring is funding-gated (G1)
Real Arweave+Solana writes need `STORAGE_MODE=full`/per-request `participate`
+ a funded keypair (SOL for the SPL memo, Irys for Arweave). Until funded,
seeding stays `local`: manifest + version tags still work, `arweave_tx` is a
synthetic `local:` id. **The version/manifest layer is mode-agnostic** — build
and test now, flip to on-chain when the keypair is funded.

## Implementation waves

### Wave 1 — versioning + manifest (no funding needed) ← THIS BRANCH
1. `seed.rs`: compute `release` (`MNEMONIC_KNOWLEDGE_RELEASE` or
   `CARGO_PKG_VERSION`) + one `corpus_ts`; append `release:` / `corpus_ts:`
   tags to every chunk. Unit tests on tag synthesis.
2. `seed.rs`: build the manifest struct, `sign_memory` it, add it to the zip.
   Unit test the manifest serializer + that it lands in the artifact.
3. (follow-up task) `tools::recall` + `chat.rs`: optional `release`/`as_of`
   filters, default-latest. Handler tests.

### Wave 2 — on-chain anchoring (needs D-FUND go-ahead)
- Fund the prod keypair; set prod seeding to `participate`/`full`. Expose the
  latest manifest Arweave tx via `/health` (or `GET /knowledge-manifest`).
- Deploy reseed (existing `deploy-mcp.yml force_reseed`) anchors the corpus.

### Wave 3 — webapp retrieval (needs D-R decision)
- Version selector in `webapp` chat; `release`/`as_of` plumbed into `/chat`.
- Mode (a): fetch manifest + chunk bytes from Arweave gateway, verify
  content_hash, offer download / "view source release".
- Mode (b, optional): WASM decompress + in-browser cosine.

## Testing

- Rust unit tests: tag synthesis (release+corpus_ts present, override honored),
  manifest JSON shape, manifest included in zip, recall release/as_of filter
  (latest-by-default preserves current behavior).
- Handler tests: `/chat` with and without `release`/`as_of` (existing httpmock
  harness in `chat.rs`).
- Regression: existing seed/chat tests must stay green (additive tags must not
  break `build_context` or the `protocol-knowledge` filter).

## Risks

- **Tag bloat**: 4 tags/chunk instead of 2. Negligible (tags are small JSON).
- **as_of precision**: `corpus_ts` is per-run, so "as_of" resolves to a whole
  reseed, not individual doc edits — acceptable and matches "release" framing.
- **Manifest staleness**: regenerated every reseed; mtime idempotency (PR #173)
  already gates reseeds, so the manifest tracks the corpus.
