# mnemonic-a2a

Pure-Rust adapter over `mnemonic-core` for attesting, storing, and verifying
[A2A (Agent-to-Agent) protocol v1.0.0-rc](https://google.github.io/A2A/) objects.

## One-way dependency rule

```
mnemonic-a2a  →  mnemonic-core
```

`mnemonic-a2a` depends only on `mnemonic-core`. It **never** depends on `mcp/`
or `bridge-a2a/`. Downstream consumers that need to attest or recall A2A objects
should depend on this crate rather than on `mnemonic-core::storage` directly —
the public surface here is the stable API boundary.

## Relationship to A2A wire types

The A2A wire types (`Message`, `Task`, `A2aArtifact`) are defined in
`mnemonic-core::codec::a2a` (behind the `a2a-experimental` feature) and
re-exported from this crate, so consumers need only one `Cargo.toml` entry.

## Public API

| Function | Description |
|---|---|
| `attest_message` | JCS-canonicalize + COSE_Sign1 + store `Message` under its `contextId` |
| `attest_task` | Same for `Task` |
| `attest_artifact` | Same for `A2aArtifact` (caller supplies `ctx_id`) |
| `recall_by_context` | Return all attested rows for a `contextId`, newest first |
| `verify_a2a_attestation` | Verify a COSE_Sign1 envelope; optionally check signer pubkey |

## Pipeline

Each `attest_*` call runs:

1. `to_jcs_bytes()` — RFC 8785 canonical JSON
2. `sign_cose()` — COSE_Sign1 envelope (Ed25519, Solana keypair)
3. `blake3(jcs_bytes)` — content hash
4. `save_attestation` — persist via `AttestationStore`
5. `set_context_id` — associate the A2A `contextId` column
6. Return `AttestationId` (UUID v4)

## Storage

The `A2aStore` supertrait extends `AttestationStore` with `set_context_id`.
A `SqliteStore` implementation is provided in this crate (non-wasm only).
For tests, use the `InMemoryA2aStore` in `tests/common/`.
