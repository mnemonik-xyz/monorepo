---
created: 2026-09-27
status: draft
size: L
branch: feat/arweave-source-of-truth
---

# Tech Spec: Arweave as the source of truth

## Solution

The operator stops storing memory. Arweave holds the durable copy. The client holds the index.

Four layers change:

1. **Artifact completeness (`mnemonic-core`)** — the signed artifact gains every field that a
   restore needs. Three additions: the exact embedding, the visibility label, and the TurboQuant
   seed. TurboQuant is the embedding compressor. After this change the COSE_Sign1 envelope on
   Arweave is sufficient to rebuild a row with no loss.
2. **Portable restore (`core/src/rebuild.rs`)** — the rebuild path moves to every target, so a
   browser client restores from Arweave without a native binary.
3. **Server storage removal (`mcp/`)** — the memory tables leave the operator schema. Utility
   tables stay. The anchor confirmation stops using a database read.
4. **Client storage and migration** — `local` means the agent's own machine. An export command
   moves legacy rows to the owner. The hosted server refuses `local` writes over HTTP.

## Architecture

### Files modified

- `core/src/codec/schema.rs` — `MEMORY_V1` gains `visibility` and `anchor` in `optional_fields`
  and in `cbor_field_order`. `MEMORY_V1` stays version 1. See Decision 1.
- `mcp/src/tools.rs` — `sign_memory_inline` writes `metadata.turbo_seed`, and writes
  `metadata.embedding_f32` when Decision 2 allows it. It also writes `visibility` and `anchor`
  into the artifact.
- `core/src/lib.rs` — `pub mod rebuild` moves to the portable block. See Decision 6.
- `core/src/storage/sqlite.rs` — the memory tables leave the schema. See Decision 4.
- `mcp/src/mcp.rs`, `mcp/src/api.rs` — the anchor confirmation changes. See Decision 5.
- `mcp/src/tools.rs` — `resolve_write_mode` refuses `local` over HTTP. See Decision 8.
- `CLAUDE.md`, `docs/tools.md`, `docs/how-it-works.md`, `docs/YELLOWPAPER.md` — the sentence
  "Recall uses uncompressed f32 embeddings in SQLite" becomes false. Every document that states
  it must change in the same pull request.

### Files added

- `core/src/restore/mod.rs` — the enumerate-then-rebuild driver. It calls
  `arweave::recovery::snapshot_chain` for discovery and `rebuild::rebuild_rows` for
  reconstruction, then writes into any `AttestationStore`.
- `packages/cli/src/commands/restore.ts` — `mnemonic restore` for the end user.
- `packages/cli/src/commands/export.ts` — `mnemonic export` for the migration. Issue #47 already
  asks for this, so that issue closes here.

### Rules preserved

- `core/` never depends on `mcp/`.
- No payment logic in `core/`.
- Never hold the SQLite mutex across an `.await`.
- The client signs. The server verifies and pays fees.

## Decisions

### Decision 1 — add optional signed fields; do not create `MEMORY_V2`

`to_canonical_cbor` walks `cbor_field_order` and writes only present and non-null fields
(`core/src/codec/canonical.rs:28`). An absent field contributes no bytes.

Therefore a new **optional** field does not change the canonical bytes of any existing artifact.
Old rows keep their `content_hash`. Verification of old rows still passes. New rows carry the new
fields and hash differently, which is correct, because they are new.

This is the fact that makes the whole feature cheap. A `MEMORY_V2` would fork the schema registry
and force a dual-verification path for no benefit.

Constraint: never reorder `cbor_field_order`, and never make the new fields required. Both would
change existing bytes.

### Decision 2 — write the exact embedding only when the content is already public

An embedding inverts to an approximation of its source text. So a full-precision vector next to
public ciphertext leaks the plaintext.

Rule:

- `visibility: public` — write `metadata.embedding_f32`. The text is public anyway, so the vector
  adds no disclosure.
- `visibility: private`, not sealed — do **not** write it. Restore falls back to the compressed
  copy, so recall precision is coarse.
- `visibility: private`, sealed — the vector belongs inside the ciphertext. `work/sealed-memories/`
  owns that format. This specification only reserves the position.

Consequence to state in the documents: private anchored memories have coarse semantic recall until
sealed mode ships. This is a real, temporary limitation.

### Decision 3 — record the TurboQuant seed in the artifact

`rebuild_row` needs a compressor with the same dimension, bit width and seed as the producer used.
`metadata.embed_dim` and `metadata.turbo_bits` are already present. The seed is not: it is the
constant `42` at `core/src/wasm/mod.rs:37` and at `mcp/src/main.rs:363`.

A third-party implementation cannot read the seed from the data, so it cannot reproduce the
decompression. Add `metadata.turbo_seed`. Freeze the value at `42` for `MEMORY_V1`. Also state it
in `docs/YELLOWPAPER.md` and in the conformance document.

### Decision 4 — remove the memory tables, keep the utility tables

Delete from the operator schema:

- `attestations` — full text, tags and anchor ids.
- `attestation_embeddings` — the uncompressed f32 vectors.
- `lineage_edges` — never written by production code today.

Keep, because none of it is user memory and none of it can live on a chain or on a client:

- The OAuth tables — sessions, refresh-token rotation, revocation.
- `x402_nonces` — payment replay protection.
- `free_anchor_usage` — the daily free-anchor quota.
- `payment_events`, `attestation_costs`, `paid_operations`, `paid_artifact_staging`,
  `paid_wallet_links` — billing and reconciliation.
- `api_keys`.

`attestation_costs` currently joins to `attestations`. Break that link: keep the `content_hash` as
an opaque string, with no foreign key.

### Decision 5 — confirm an anchor from the chain, not from a database read

Today a `participate` write succeeds only after a recall-and-verify round trip against the
operator's own row. With no row, that check cannot run.

New check, after the Arweave upload and the Solana memo:

1. Fetch the item back from the Arweave gateway.
2. Verify the COSE_Sign1 signature.
3. Recompute the blake3 hash of the payload.
4. Compare it against the hash in the Solana memo.

This is strictly stronger than the current check. It proves the durable copy is readable and
correct, instead of proving the operator's cache is readable. On failure, refund the free-anchor
quota and report the failure. There is no row to demote, so the demotion path is deleted.

### Decision 6 — make the rebuild path portable

`core/src/lib.rs:31` gates `rebuild` to native targets and says the reason is the compressor. That
reason is stale. `compress` is in the portable block (`core/src/lib.rs:3`), and `rebuild.rs` uses
only `codec::canonical`, `codec::sign`, `compress` and `base64`. All four are in the shared
dependency table of `core/Cargo.toml`. The native-only table holds `rusqlite`, `reqwest`, `rsa`
and `rand`, and `rebuild.rs` uses none of them.

So move `pub mod rebuild` into the portable block, and prove it with a real
`wasm32-unknown-unknown` build plus a `wasm-bindgen-test`. Add the WASM exports the SDK needs.

Discovery is a separate matter. `arweave::recovery` and `arweave::graphql` use `reqwest`, so they
stay native. A browser client performs discovery with `fetch` in TypeScript and calls the WASM
rebuild for each item. Therefore the new `core/src/restore/` driver is native-only, and the WASM
surface exposes `rebuild_row` alone.

### Decision 7 — public read routes use a derived cache, never a source of truth

**This decision needs owner sign-off, because it keeps one SQL table of anchored public content.**

`/blog`, `/stats` and `/artifacts` are public read routes. Serving them from Arweave GraphQL on
every request is slow and depends on gateway availability.

Recommendation: the server keeps one derived table of **public** artifacts only, rebuilt from
Arweave by `core/src/restore/`. Properties: it contains nothing private, it is dropped and rebuilt
on demand, and no write path reads it for correctness. Name it `public_cache` so no reader mistakes
it for a store.

The alternative is a live GraphQL query per request, with no table at all. That is purer and
slower. If the owner prefers zero memory tables of any kind, take the alternative and accept the
latency.

### Decision 8 — `local` over HTTP returns an error

`work/binary-mode-cleanup/` already specifies this. `tools::resolve_write_mode` returns
`-32010 UnsupportedMode` for `mode: "local"` on the HTTP transport. The hint names the two valid
paths: install the server locally for free local storage, or use `anchored`.

`mnemonic_whoami` then advertises `supported_modes: ["anchored"]` on a hosted deployment.

Rename the wire value `participate` to `anchored`, and accept `participate` as a deprecated alias
for one release. The owner's model names two modes, and the documents must use those names.

### Decision 9 — migrate by export, then freeze

Existing hosted rows follow three steps:

1. Ship `mnemonic export` first. It writes the owner's rows as JSON Lines, with the COSE envelope
   where one exists.
2. Announce the change and give a window for export.
3. Refuse new writes to the legacy tables. Keep reads working until the window closes.

Rows labelled `private` that are already anchored get the honest label. Set `visibility: public`
and add `plaintext_on_arweave: true`. Tell the affected users. Nothing can be deleted from Arweave,
and the announcement must say that plainly. This is decision D-8 of `work/sealed-memories/`.

### Decision 10 — write the new fields only for anchored writes

`rebuild_content_hash` (`mcp/src/tools.rs:2026`) rebuilds an artifact from database columns to
recompute a hash. Its field set is hardcoded at the call site. An unconditional new artifact field
therefore breaks verification of every existing row that uses this path.

The path is used only for rows that hold no signature. An anchored row is verified by
`verify_participate`, which fetches the bytes from Arweave and calls `verify_artifact`. It never
reconstructs.

So write `turbo_seed`, `embedding_f32`, `visibility` and `anchor` into the artifact of an
**anchored** write only. `rebuild_content_hash` keeps its current field set, frozen, and serves only
legacy unsigned local rows. No new column, no version branch, and no existing hash moves.

Restoration concerns anchored rows alone, so the fields land exactly where they are needed. Delete
`rebuild_content_hash` in Wave 4 with the legacy rows it serves.

## Testing

- **Unit, hashing.** An artifact without the new fields produces byte-identical canonical CBOR
  before and after the schema change. This is the regression test that protects Decision 1.
- **Unit, restore fidelity.** Sign an artifact with `embedding_f32`, rebuild it, and assert the
  vector is bit-identical and `precision == F32`. Repeat without the field and assert
  `precision == Compressed`.
- **Unit, seed.** Rebuild with a compressor built from `metadata` alone, including `turbo_seed`,
  and assert success. This proves a third party needs no out-of-band constant.
- **Integration, full restore.** Write N memories through the anchored path against a local
  Arweave and Solana stub. Drop the client store. Run restore. Assert every row returns, assert
  each signature verifies, and assert the Merkle root of the restored set equals the root of the
  original set.
- **Integration, chain confirmation.** Make the gateway fetch fail and assert the anchor write
  reports failure and refunds the free-anchor quota.
- **Integration, no memory tables.** Assert the operator schema has no `attestations` table, and
  assert an anchored write leaves no memory row.
- **WASM.** A `wasm-bindgen-test` rebuilds a golden artifact in the browser target and asserts
  byte parity with the native result. This runs under the hard `cross-lang-build` gate.
- **Negative.** A tampered artifact fails rebuild. An unsigned artifact fails rebuild. A rebuild
  with the wrong seed produces a different vector and must be detectable.

## Sequencing (waves)

- **Wave 1 — artifact completeness. DONE (#246).** `MEMORY_V1` optional fields, `turbo_seed`, `embedding_f32`
  with the visibility rule, and the byte-compatibility regression test. Nothing else compiles on a
  complete artifact until this lands.
- **Wave 2 — portable rebuild. DONE.** Moved the module, added `rebuild_row_self_describing`
  and the `rebuild_row` WASM export, and covered it from Rust and from JS against the
  real WASM artifact.
- **Wave 3 — restore driver. DONE (restore).** `core/src/restore/` plus the
  `mnemonic-mcp restore` subcommand. `mnemonic export` moves to Wave 5, where the
  migration actually needs it (it also closes #47).
- **Wave 4 — server removal.** Drop the memory tables, change the anchor confirmation, refuse
  `local` over HTTP, rename the mode, update the envelope.
- **Wave 5 — migration and documents.** The relabel, the user announcement, and every affected
  document.
- **Wave 6 — audit.** Read-only. Confirm no private content reaches Arweave in plaintext, and
  confirm no memory row remains on the server.

Conflict points to keep inside single tasks: `core/src/lib.rs`, `core/src/codec/schema.rs`,
`mcp/src/tools.rs`, `mcp/src/mcp.rs`.

## Risks

- **The f32 embedding enlarges every public item.** A 384-dimension vector costs about 1.5 KB
  before base64, and about 2 KB after. The operator pays that for each public anchored write.
  Mitigation: measure the cost in Wave 1 and expose the tier as a per-write flag if it is
  material.
- **Restoring depends on gateway availability.** If no Arweave gateway answers, a client with no
  local index cannot read its memory. Mitigation: document the liveness assumption, and keep the
  local index as the normal read path rather than an emergency one.
- **The Solana memo history is the real index.** `core/src/arweave/recovery.rs` records a live
  check from 2026-07-09: 16 memos and 0 GraphQL hits. So discovery cannot rely on Arweave GraphQL
  alone. Mitigation: keep both enumeration sources, and test with GraphQL returning nothing.
- **Losing the key loses the memory.** With no operator copy, key loss is final for anchored
  private content. Mitigation: this is the point of the design, so say it in the user-facing text
  during key creation, not in a footnote.
- **Coarse recall for private anchored memories** until sealed mode ships. Mitigation: state the
  limitation, and sequence `work/sealed-memories/` tasks 1 to 5 next, since they have no
  dependency on this feature.
