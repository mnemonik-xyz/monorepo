# Decisions log: arweave-as-source-of-truth

Append-only. Owner decisions, task reports and audit findings go here.

---

## Fixed inputs (owner)

- **2026-09-27.** No SQL storage for anchored memories on the MCP server. Utility SQL is
  acceptable: OAuth, payment nonces, quotas, billing.
- **2026-09-27.** Two modes only. `local` means the memory is stored on the agent's own machine or
  hardware. `anchored` means the memory is stored on Arweave.
- **2026-09-27.** All four restore gaps are in scope: the exact embedding, the unsigned
  `visibility` and `anchor` fields, the implicit TurboQuant seed, and the missing WASM rebuild.
- **2026-09-27.** The `visibility` column as a privacy mechanism is rejected. It is only an SQL
  filter and it encrypts nothing. Private must mean encrypted.

## Spec findings (2026-09-27)

- **F1.** The memory text is inside the signed payload. `content` is in
  `MEMORY_V1.cbor_field_order` (`core/src/codec/schema.rs:238`), and those canonical bytes are what
  `mcp/src/api.rs:588` uploads to Arweave. So an anchored memory publishes its text permanently,
  whatever the `visibility` column says.
- **F2.** The exact-embedding tier is dead on the write side. `core/src/rebuild.rs:18-25` supports
  `Precision::F32` from `metadata.embedding_f32`, and `core/src/arweave/recovery.rs:311` can write
  it, but `sign_memory` never does (`mcp/src/tools.rs:1220-1226`). Every restored embedding today is
  the lossy dequantized copy.
- **F3.** The TurboQuant seed is not in the artifact. It is the constant `42` at
  `core/src/wasm/mod.rs:37` and `mcp/src/main.rs:363`. `metadata.embed_dim` and
  `metadata.turbo_bits` are present, so the seed is the only value a third party cannot read from
  the data.
- **F4.** The reason `rebuild` is native-only is stale. `core/src/lib.rs:31` blames the compressor,
  but `compress` is portable (`core/src/lib.rs:3`), and `rebuild.rs` uses only `codec`, `compress`
  and `base64` — all in the shared dependency table of `core/Cargo.toml`. The native-only table
  holds `rusqlite`, `reqwest`, `rsa` and `rand`, none of which `rebuild.rs` touches.
- **F5.** Solana memo history, not Arweave GraphQL, is the authoritative index.
  `core/src/arweave/recovery.rs:1-17` records a live check on 2026-07-09: 16 memos and 0 GraphQL
  hits, because gateways never indexed the old Irys-bundled items.
- **F6.** Adding an optional field is hash-safe. `to_canonical_cbor` writes only present and
  non-null fields from `cbor_field_order` (`core/src/codec/canonical.rs:28`), so an absent field
  contributes no bytes and existing `content_hash` values do not move.
- **F7.** `rebuild_content_hash` (`mcp/src/tools.rs:2026`) rebuilds an artifact from columns with
  the field set hardcoded at the call site. Any unconditional new artifact field breaks
  verification of every existing row that goes through this path. See D-1.
- **F8.** The path in F7 is used only for rows with no signature. Anchored rows go through
  `verify_participate`, which fetches the bytes from Arweave and calls `verify_artifact` directly
  (`mcp/src/tools.rs` — `arweave.read(ar_tx_id)` then COSE verification). No reconstruction.

---

## Decisions

### D-1. New artifact fields are written only for anchored writes (2026-09-27)

**Problem.** F7: `rebuild_content_hash` reconstructs the artifact from columns to recompute a hash.
Add a field unconditionally and old rows stop verifying. Omit it there and new rows stop verifying.

**Rejected.** An `artifact_rev` column plus a branch in the reconstruction. It works, but it adds a
column and a permanent version branch to a path that should be shrinking.

**Decision.** Write `turbo_seed`, `embedding_f32`, `visibility` and `anchor` only into the artifact
of an **anchored** write. F8 makes this safe: an anchored artifact is always verified from its
Arweave bytes, never reconstructed. `rebuild_content_hash` keeps its current field set, frozen,
serving only legacy unsigned local rows.

**Effect.** No new column, no version branch, and no change to any existing hash. Restoration
concerns anchored rows only, so the fields land exactly where they are needed. Local artifacts keep
the old shape, which is correct, because a local index is never restored from Arweave.

**Follow-up.** When the legacy local rows are gone, `rebuild_content_hash` is deleted with them.
Note that in the Wave 4 task.
