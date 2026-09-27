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

### D-2. What "no SQL for anchored memories" means precisely (2026-09-27)

The owner's instruction is "no SQL for anchored memories storage; utility SQL is ok". Two
dependencies found while scoping Wave 4 make the boundary matter:

- **F9.** The replay guard behind `already_anchored` (`find_anchored_by_content_hash`, added by
  #244) reads server-stored anchored rows. Remove them and an artifact can be anchored twice:
  a duplicate chain write and a second free grant spent for one memory.
- **F10.** The public ledger (`/artifacts`, `list_public_artifacts`) serves `content` from the
  `attestations` table.

**Decision.** The server keeps **no memory** — no `content`, no embedding. It keeps an **anchor
index**: `attestation_id`, `content_hash`, the two transaction ids, `owner_pubkey`,
`created_at`, `write_mode`, `visibility`. A hash and a transaction id are not memory; they are
the operator's own bookkeeping, and the replay guard and verify routing depend on them. The
memory itself — the text and the vector — lives on Arweave only.

**Consequences that still need the owner.** Both are deferred to Wave 4b rather than guessed:

- **Q-1. Hosted semantic recall of anchored memories becomes impossible**, because the server
  holds no embedding. Client-side recall over a restored index is the replacement. Confirm that
  hosted `mnemonic_recall` returning nothing for anchored memories is acceptable, or whether the
  operator should keep a public-only derived index.
- **Q-2. `/artifacts` can no longer show content** from the database. Options: fetch from Arweave
  per request, keep a derived public-only cache rebuilt by `core/src/restore/` (tech-spec
  Decision 7), or show hashes and links only.

### D-3. Wave 4 splits; hosted `local` writes are retired first (2026-09-27)

Wave 4 as specified bundles four changes with a wide blast radius. Landing them together would
mix a bounded, reviewable change with a product decision that is still open (D-2 Q-1 and Q-2).

Shipped now (Wave 4a): an explicit `mode: "local"` over HTTP returns `-32010 UnsupportedMode`.
`local` means the agent's own machine; a hosted deploy cannot provide it, and quietly storing
the memory in the operator's database under that name was the dishonest part. This retires the
"free hosted SQLite" tier that `work/binary-mode-cleanup/` calls a deploy anti-pattern.

Only an **explicit** `local` is refused. A caller that sends no `mode` still resolves from the
operator's configuration, so clients that never learned the field keep working. Moving them onto
a paid path is a billing decision and not this change's to make.

Deferred to Wave 4b: dropping `content` and the embedding from anchored rows, chain-based anchor
confirmation (tech-spec Decision 5), and the public read surface (D-2 Q-2).

Test impact, recorded because it looks like lost coverage and is not: five tests asserted the
retired tier. Three became one test asserting the refusal. Two — the local sign-then-verify round
trip and its tamper-detection counterpart — now drive the stdio transport, where `local` is still
legal, so the verify coverage is unchanged.
