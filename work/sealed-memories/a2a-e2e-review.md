# Review status: superseded storage draft

The earlier review below describes the rejected SQL artifact-storage draft.
Its storage assertions and test totals are historical, not current evidence.
See [current contract and validation](../../docs/sealed-a2a.md) and the merged
[A2A recovery spec](a2a-recovery-spec.md). Tasks 18–21 remain in progress pending
review and complete acceptance; #61/#74 obligations remain open.

# Sealed A2A implementation review

Base: main `a8f34fcce514ad3da5ef78113f0bc274768a7369`.
Compared with `cd68d215c1aec2bee6e264ac7480b0dad4983aaa`, task 14,
[issue #242](https://github.com/mnemonik-xyz/monorepo/issues/242) and the client
signing constraint in [#233](https://github.com/mnemonik-xyz/monorepo/issues/233).

The earlier commit already supplied chunk primitives, DataPart helpers,
Ed25519-to-X25519 extraction and SDK conformance tests. Those do not need to
be reimplemented. The missing runtime path is implemented in this branch.

| Files | Missing behavior now implemented |
|---|---|
| `core/src/codec/a2a/signed.rs`, `core/src/codec/a2a/mod.rs` | Client signs routing/lineage/time plus public sealed carrier; signs sealed CBOR and every targeted grant; verifies detached pinned-card JWS before recipient discovery; opens only locally |
| `core/src/sealed/stream.rs`, `core/src/sealed/mod.rs` | Completed sealed stream binds header, stream ID, index, previous hash and finality; verifies complete chain before releasing plaintext |
| `core/src/wasm/mod.rs` | Real client prepare/verify/open APIs and correct Ed25519-seed-to-X25519-scalar derivation |
| `mcp/src/mcp.rs`, `mcp/src/tools.rs` | Runtime `signed`/`sealed` schema and dispatch; JWT signer matching; all transports require client signatures; validation before paywall; anonymous recall cannot inherit server identity |
| `mnemonic-a2a/src/signed.rs`, `mnemonic-a2a/src/lib.rs` | Verify immutable signed binding instead of re-signing or dropping parent metadata |
| `core/src/storage/sqlite.rs` | Atomic original-envelope/ciphertext/grant/index persistence; no sealed plaintext or embeddings; idempotent writes; accessible same-context parents; author or active-grant recall, filters before LIMIT |
| `packages/sdk/src/{a2a,client,types,index,wasm,wasm.browser}.ts`, `scripts/build-wasm.sh` | Sign/seal before network; preserve actual kind/time/COSE/hash/context/parent/stream; expected-author verification and local open; public verification without private key; defensive keypair copies |
| `packages/cli/src/commands/a2a.ts`, `packages/cli/bin/mnemonic.ts` | Local signer binding, pinned recipient file, completed chunk size, sealed recall and local `--open-author` |
| `core/examples/emit_sealed_a2a.rs`, `core/tests/sealed_a2a_vectors.rs`, SDK fixture and real-WASM test | Native seal/open fixtures and exact Rust/WASM signed-byte parity; outsider, tamper, trusted-author and repeated-client-signing checks |
| `mcp/tests/integration_a2a_mcp_tools.rs`, SDK HTTP test script, CLI tests, `node-test.yml` | Actual SDK → HTTP MCP → SQLite → recipient recall → client open; payment, isolation, restart, parent, withdrawal and filtering checks; nightly/on-demand HTTP gate |
| `docs/sealed-a2a.md`, tool/SDK/CLI docs, task 8 link, task 14 and append-only decisions | Contract, usage, metadata leakage, trust/rotation, withdrawal and stream completion limits |

Two existing TypeScript build blockers are corrected narrowly:
`sdk/src/erc8004/self-promotion.ts` rejects an invalid/null decoded owner;
`cli/src/commands/erc8004.ts` omits undefined exact-optional arguments.

## Own sealed-chain review

- Fresh nonce prefix and one K per stream; nonce binds index/final flag.
- AEAD binds stream/header/previous hash and the signed manifest binds head.
- Dense indexes, links and a single final chunk are verified before open.
- Reorder, omission, truncation, splice, wrong prefix/header/key fail.
- Even a repaired public hash chain with modified ciphertext or forged shorter
  finality fails AEAD. Hash-chain validity alone is not claimed as decryptability.
- Provisional plaintext chunks are zeroized on error; client keypair copies are
  cleared without clearing the retained identity.

## Remaining scope and closing conditions

Only two feature gaps remain against the original issue scope:

1. Actual #61 per-SSE `A2A_STREAM_CHUNK_V1` integration. The completed signed
   chunk manifest here is not that event transport. Task 14 stays in progress
   until the real #61 chain carries these sealed chunk hashes with integration
   tests for omission/reorder/finality and recipient opening.
2. `did:mnemonic` recipient key discovery (#74), listed in #242. Explicitly
   supplied, cryptographically verified AgentCards work. #242 can close after
   merge/conformance and #61 integration plus #74 discovery, or after an
   explicit decision to defer DID discovery from its V1 scope.

Task 14 also requires its native/WASM conformance and threat-note evidence to
remain green after integration/merge. This patch does not close #233's other
tasks or claim a completed independent security audit.

No changes to the unrelated A-16-2/A-16-3 wire-format work are proposed. The A2A
path preserves the original JSON inside ciphertext and has its own signed
binding, avoiding lossy conversion at this boundary.

See [the protocol contract and reproducible commands](../../docs/sealed-a2a.md).

## Validation results (2026-10-01)

- Native core library: 322 passed, 1 existing ignored; completed-stream,
  signed-A2A and committed-vector targets rechecked after final hardening.
- Workspace unit/integration tests: 1265 passed. Initial doctest artifact
  mismatch during overlapping builds disappeared on the sequential workspace
  doctest rerun; all workspace doctests then passed.
- A2A MCP target: 6 passed; the real SDK/WASM HTTP test was explicitly run
  despite its default build-dependent ignore and passed all four scenarios.
- SDK: 331 passed with real WASM conformance enabled.
- CLI: 186 passed, 2 existing skipped. SDK/CLI TypeScript build and SDK browser
  bundle build passed.
- `cargo clippy --workspace --lib --bins -- -D warnings`: passed.
- All-targets strict clippy is blocked by existing dead-code failure in
  `bridge-a2a/tests/common/mod.rs::row_count` when compiled by
  `strict_failure_mode`. Global rustfmt-check is blocked by existing formatting,
  starting with untouched `bridge-a2a/src/attest.rs`. These gates are not claimed
  green. `git diff --check` passes; unrelated formatting was excluded.
