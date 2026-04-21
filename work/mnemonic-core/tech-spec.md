---
created: 2026-04-20
status: approved
branch: dev
size: L
---

# Tech Spec: mnemonic-core extraction

## Solution

Extract all domain logic from the monolithic MCP server (`mnemonic-protocol/mcp/`) into a standalone Rust library crate `mnemonic-core`. The MCP server becomes a thin wrapper that depends on core as a Cargo workspace member.

The extraction follows a strict phased order: codec → identity → embed → compress → db/storage → arweave/solana → lineage. After each phase, `cargo test` and `cargo clippy` must pass — no broken intermediate states. Each phase is a separate wave — no parallel execution within a wave to avoid file conflicts on shared files (`core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/main.rs`).

Key technical moves:
1. Create workspace root `Cargo.toml` with `[workspace] members = ["core", "mcp"]` and `resolver = "2"`.
2. Create `core/` crate with `lib.rs` re-exporting public modules.
3. Move modules one-by-one, updating imports in `mcp/` to use `mnemonic_core::` paths.
4. Split `db.rs` into storage traits (`AttestationStore`, `LineageStore`) in core and keep payment methods in `mcp/`.
5. Move `verify_usdc_transfer` out of `SolanaClient` into `mcp/src/payment.rs` as a standalone function taking `&SolanaClient` — it is a payment concern, not core chain logic.
6. Replace `turboquant` git dependency with `turboquant-plus-rs = "0.1.0"` from crates.io.
7. Remove `HashEmbedder` entirely — replace with a `MockEmbedder` that returns deterministic fixed vectors for testing.
8. Add httpmock-based tests for `arweave.rs` and `solana.rs` (currently zero coverage).
9. Move benchmarks and proptests to `core/benches/` and `core/tests/`.

The MCP server retains: `main.rs`, `mcp.rs`, `tools.rs`, `payment.rs`, `pricing.rs`, `config.rs`, plus all axum/tokio/clap dependencies.

## Architecture

### What we're building/modifying

- **`core/` (new crate)** — Rust library crate containing all domain logic: codec, identity, embed, compress, storage, arweave, solana, lineage. Public API surface for external consumers.
- **`mcp/` (modified)** — MCP server binary. Loses all domain logic modules, gains `mnemonic-core` as workspace dependency. Retains server bootstrap, JSON-RPC dispatch, payment, pricing, config.
- **Workspace root `Cargo.toml` (new)** — Ties core and mcp together with shared resolver.

### How it works

**Before (monolith):**
```
mcp/src/
  main.rs → directly imports codec, embed, compress, identity, db, arweave, solana, lineage, tools, mcp, payment, pricing, config
```

**After (workspace):**
```
Cargo.toml (workspace root)
core/src/
  lib.rs → pub mod codec, embed, compress, identity, storage, arweave, solana, lineage
mcp/src/
  main.rs → imports mnemonic_core::{codec, embed, compress, identity, storage, arweave, solana, lineage}
  tools.rs, mcp.rs, payment.rs, pricing.rs, config.rs — remain here
```

Data flow unchanged. `McpState` in `mcp.rs` still owns `ArweaveClient`, `EmbeddingCompressor`, `AttestationStore`, `Box<dyn Embedder>`, `SolanaClient` — all types now come from `mnemonic_core::`.

### Shared resources

None. This is a library extraction — no shared runtime resources like DB pools or singletons are introduced. Resource ownership remains in the MCP server binary.

## Decisions

### Decision 1: Phased extraction order — sequential waves
**Decision:** codec → identity → embed → compress → db/storage → arweave/solana → lineage. Each extraction is a separate sequential wave because all tasks modify shared files (`core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/main.rs`).
**Rationale:** Follows the dependency graph bottom-up. Codec has zero internal dependencies. Each subsequent module depends only on previously extracted modules. Sequential waves avoid file-level merge conflicts. Supports US constraint: "After each step cargo test and cargo clippy green." `[TECHNICAL]`
**Alternatives considered:** Parallel extraction within waves — rejected because shared file modifications cause conflicts. Big-bang move — rejected because it violates the phased constraint.

### Decision 2: Storage trait split + verify_usdc_transfer isolation
**Decision:** Define `AttestationStore` trait and `LineageStore` trait in `core/src/storage/`. SQLite implementations live in `core/src/storage/sqlite.rs`. Payment methods (`create_api_key`, `deduct_balance`, `credit_deposit`, `mark_x402_nonce`, `record_attestation_cost`, `get_pnl_stats`, `get_owner_pubkey`) stay in `mcp/src/payment.rs`. Additionally, `verify_usdc_transfer` is moved out of `SolanaClient` into `mcp/src/payment.rs` as a standalone async function taking `&SolanaClient` — it is a payment verification concern, not core chain logic.
**Rationale:** Core consumers need attestation CRUD and search but never need payment logic. `verify_usdc_transfer` must be absent from core/ per user-spec AC. Supports US technical decision: "Storage traits: AttestationStore and LineageStore — traits in core; payment methods stay in mcp/." `[TECHNICAL]`
**Alternatives considered:** Keep `verify_usdc_transfer` on `SolanaClient` in core — rejected because user-spec explicitly lists it in the payment-methods grep exclusion. Move all payments to core — rejected because payment logic is MCP-server-specific.

### Decision 3: turboquant dependency migration
**Decision:** Replace `turboquant = { git = "..." }` with `turboquant-plus-rs = "0.1.0"` from crates.io. Update all imports from `turboquant::` to `turboquant_plus_rs::`. Verify crate exists on crates.io before starting Task 1.
**Rationale:** Git dependencies block crates.io publishing of mnemonic-core. Supports US acceptance criterion: "`turboquant-plus-rs = \"0.1.0\"` in core/Cargo.toml — crates.io, not git." Supports US risk 1 mitigation: first step is dep change + import update + cargo test.
**Alternatives considered:** Fork and publish under same name — rejected because `turboquant-plus-rs` already exists on crates.io. **Pre-condition:** verify crate availability with `cargo search turboquant-plus-rs` before implementation.

### Decision 4: HashEmbedder removal + MockEmbedder replacement
**Decision:** Remove `HashEmbedder` from the codebase entirely. Introduce `MockEmbedder` as a `#[cfg(test)]` utility in `core/src/embed/` that returns deterministic fixed-dimension vectors (e.g., normalized sequential floats). Tests in `embed.rs` and `db.rs` that relied on `HashEmbedder` switch to `MockEmbedder`.
**Rationale:** `HashEmbedder` produces non-invertible hashes instead of real embeddings — decompression returns garbage. `MockEmbedder` provides deterministic, decompressible test vectors. Supports US acceptance criterion: "`HashEmbedder` absent in core/ — grep empty." `[TECHNICAL]`
**Alternatives considered:** Keep HashEmbedder as test-only — rejected per user-spec explicit decision. No replacement — rejected because db.rs tests require a concrete Embedder implementation.

### Decision 5: lineage.rs cleanup on move
**Decision:** Fix three issues during lineage extraction: (a) `Direction` becomes an enum instead of string, (b) `chain_valid` becomes `Option<bool>` (unknown vs verified), (c) DB errors propagate via `?` instead of being swallowed. Add a dedicated test for `?`-propagation (DB error returns Err, not panic).
**Rationale:** Supports US acceptance criterion: "lineage.rs: errors propagate via ?; chain_valid: Option<bool>; Direction — enum." These are bug fixes that are safest to apply during the move when the module is already being modified. `[TECHNICAL]`
**Alternatives considered:** Fix in a separate task — rejected because touching the same code twice increases merge risk.

### Decision 6: httpmock tests for arweave and solana
**Decision:** Add httpmock-based unit tests for `arweave.rs` (~6 tests: write, read, write_bytes, health_check, network timeout, malformed response) and `solana.rs` (~7 tests: write_memo, read_memo, airdrop, get_tx_signers, confirm_tx retry exhaustion, health_check, error handling). Mock Irys upload/read endpoints and Solana JSON-RPC. Tests must not contain real mainnet URLs or funded keypairs.
**Rationale:** These modules currently have zero test coverage. Supports US risk 2 mitigation and acceptance criterion: "httpmock-tests for arweave/solana." `[TECHNICAL]`
**Alternatives considered:** Integration tests with arlocal/solana-test-validator — rejected for this iteration because they require external services in CI.

### Decision 7: No WASM concerns in this iteration
**Decision:** All code in core/ targets native only. No `#[cfg(target_arch = "wasm32")]` gates, no `wasm-bindgen` exports, no `wasm/mod.rs`.
**Rationale:** Supports US constraint: "Native-only: WASM, wasm/mod.rs, web_sys, localStorage — out of scope." WASM is iteration 2.
**Alternatives considered:** Add WASM feature gates now — rejected per user-spec scope.

### Decision 8: Storage trait authorization model
**Decision:** `AttestationStore` and `LineageStore` traits do not enforce per-signer authorization. All trait methods operate on the full database. Authorization is the caller's responsibility — documented in trait-level rustdoc.
**Rationale:** The existing `db.rs` has no per-signer scoping. The MCP server enforces authorization via the keypair loaded at startup. Core library consumers must handle authorization at their own layer. Changing the authorization model is out of scope for this extraction. `[TECHNICAL]`
**Alternatives considered:** Add `signer_pubkey` parameter to all trait methods — rejected because it changes the existing API contract and is not in user-spec scope.

## Data Models

No new data models. Existing SQLite schema (`attestations`, `memory_embeddings`, `attestation_costs`, `lineage_edges`) is unchanged. US constraint: "No DB schema changes — existing attestations.db continues working without migrations."

Struct types (`AttestationRow`, `SearchResult`, `PnlStats`, `CompressedEmbedding`, `SignedArtifact`, `VerificationResult`, `LineageResult`, `LineageNode`, `LineageEdge`, `ParentRef`, `ArtifactSchema`) move from `mcp/src/` to `core/src/` with identical field definitions.

## Dependencies

### New packages
- `turboquant-plus-rs = "0.1.0"` — replaces git dependency `turboquant`, same functionality, crates.io-published
- `httpmock = "0.8"` — dev-dependency for mocking HTTP endpoints in arweave/solana tests

### Using existing (from project)
All existing dependencies split between core and mcp per code-research.md section 3:

**core/Cargo.toml:**
`sha2`, `hex`, `base64`, `serde`, `serde_json`, `blake3`, `ciborium`, `coset`, `chrono`, `anyhow`, `thiserror`, `uuid`, `bs58`, `bincode`, `ndarray`, `turboquant-plus-rs`, `solana-sdk`, `spl-memo`, `rusqlite` (bundled), `reqwest` (features: `json`, `blocking` — `blocking` needed for `OpenAIEmbedder`), `futures`, `tracing` (optional), `fastembed` (optional, feature = "local-embed"), `tokio` (features: `time`, `rt` — needed for async fns in arweave/solana modules)

**core/Cargo.toml [dev-dependencies]:**
`httpmock`, `tempfile`, `proptest`, `criterion`, `tokio` (features: `macros`, `rt-multi-thread` — for `#[tokio::test]`)

**mcp/Cargo.toml:**
`axum`, `axum-extra`, `tower-http`, `clap`, `tokio` (full), `tokio-stream`, `dotenvy`, `tracing-subscriber`, `mnemonic-core` (path = "../core")

### Removed packages
- `turboquant = { git = "..." }` — replaced by `turboquant-plus-rs`
- `solana-client` — removed from mcp/ (was used only in solana.rs, which now lives in core/ using reqwest directly)
- `solana-transaction-status` — evaluate if still needed in mcp/; remove if unused after extraction

## Testing Strategy

**Feature size:** L

### Unit tests
- **codec modules** (existing 24 tests): move to `core/src/codec/` `#[cfg(test)]` blocks, verify they pass after move
- **embed module** (existing 8 tests): move, remove `HashEmbedder` references, replace with `MockEmbedder` (returns deterministic normalized f32 vectors of configurable dimension)
- **compress module** (existing 4 tests): move, update `turboquant` → `turboquant_plus_rs` imports, add numerical fidelity test comparing compress→decompress roundtrip MSE stays below threshold after namespace change
- **identity module** (existing 4 tests): move, verify keypair roundtrip with `tempfile`
- **db/storage module** (existing 2 tests + new ~4 contract tests): move existing tests, add contract tests for `AttestationStore` and `LineageStore` traits verifying trait methods work through the SQLite implementation
- **arweave module** (new, ~6 tests): httpmock tests for `write`, `read`, `write_bytes`, `health_check`, network timeout scenario, malformed response handling. No real mainnet URLs or funded keypairs in tests.
- **solana module** (new, ~7 tests): httpmock tests for `write_memo`, `read_memo`, `airdrop`, `get_tx_signers`, `confirm_tx` retry exhaustion, `health_check`, JSON-RPC error handling. No real mainnet URLs or funded keypairs in tests.
- **lineage module** (existing 9 tests + 1 new): move all, update for `Direction` enum and `chain_valid: Option<bool>`, add test verifying DB error propagates via `?` (returns `Err`, not panic)

### Edge case tests
- **embed**: empty string input, zero-dimension embedder configuration
- **compress**: zero-length embedding, single-element embedding
- **storage**: concurrent SQLite access (two `AttestationStore` instances on same file), query with no results, save duplicate attestation_id
- **arweave/solana**: malformed JSON response, HTTP 5xx, connection timeout, empty response body
- **lineage**: cycle detection with self-reference, max depth boundary (MAX_DEPTH = 64)

### Integration tests
- **MCP round-trip** (existing + updated): `cargo build -p mnemonic-mcp` compiles; JSON-RPC `tools/list` returns 5 tools; `sign_memory` → `recall` round-trip works in local mode. Concrete command: `echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | cargo run -p mnemonic-mcp -- --transport stdio`
- **Codec pipeline** (existing 5 tests): move `tests/integration_cbor.rs` to `core/tests/`, remove inline helper duplication (now imports from lib)
- **Proptest** (existing 1 test): move `tests/proptest_canonical.rs` to `core/tests/`
- **Benchmark** (existing 2 files): move `benches/decompress.rs` and `benches/cbor_codec.rs` to `core/benches/`
- **Standalone usage**: `cargo test -p mnemonic-core` succeeds without mnemonic-mcp being built — verifies core is independently usable (user-spec scenario 1)

### E2E tests
None — no deployed environment. Per user-spec: "E2E tests: not done — no deployed environment."

## Agent Verification Plan

**Source:** user-spec "How to verify" section.

### Verification approach
After each wave, the agent runs `cargo test -p mnemonic-core && cargo clippy -p mnemonic-core -- -D warnings` to confirm no regressions. After the final implementation wave, full verification:
1. `cargo test -p mnemonic-core` — all tests green including new httpmock tests
2. `cargo clippy -p mnemonic-core -- -D warnings` — zero warnings
3. `cargo build -p mnemonic-mcp` — compiles with core as workspace dep
4. MCP local mode round-trip: `echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | cargo run -p mnemonic-mcp -- --transport stdio` — returns 5 tools
5. `grep -r "HashEmbedder" core/src/` — empty
6. `grep -r "create_api_key\|deduct_balance\|credit_deposit\|mark_x402_nonce\|record_attestation_cost\|get_pnl_stats\|get_owner_pubkey\|verify_usdc_transfer" core/src/` — empty
7. `grep -E "codec/|lineage/" .claude/skills/project-knowledge/references/architecture.md` — both lines found
8. `cargo test -p mnemonic-core` succeeds without building mnemonic-mcp — confirms standalone usability

### Tools required
bash (cargo commands, grep, echo + pipe for JSON-RPC stdio)

## Risks

| Risk | Mitigation |
|------|-----------|
| turboquant-plus-rs not on crates.io or incompatible API | Pre-condition check: `cargo search turboquant-plus-rs` before Task 1. Fallback: publish turboquant fork ourselves. |
| turboquant namespace change breaks imports | First task is only dep swap + import update + cargo test. No other changes in that step. |
| arweave/solana zero test coverage hides bugs during move | Add httpmock tests as part of the extraction task, before modifying any logic. |
| fastembed model download in CI | CI sets fastembed cache env var. Tests use MockEmbedder by default; fastembed tests gated behind `local-embed` feature. |
| Circular dependency between core and mcp during partial migration | Phased sequential order ensures each module only depends on already-moved modules. |
| rusqlite::Connection is !Send — async wrappers break | Storage trait methods are sync. MCP wraps in Mutex per existing pattern (patterns.md). No change needed. |
| verify_usdc_transfer extraction from SolanaClient breaks mcp/ callers | Task 8 explicitly moves it to mcp/payment.rs as standalone fn. Compile check catches missing references. |

## User-Spec Deviations

None

## Acceptance Criteria

Technical acceptance criteria (supplement user-spec criteria):

- [ ] Workspace root `Cargo.toml` exists with `members = ["core", "mcp"]` and `resolver = "2"`
- [ ] `core/Cargo.toml` exists with `[lib]` target, all domain dependencies listed including `tokio`
- [ ] `core/src/lib.rs` re-exports: `codec`, `embed`, `compress`, `identity`, `storage`, `arweave`, `solana`, `lineage`
- [ ] `mcp/Cargo.toml` has `mnemonic-core = { path = "../core" }` dependency
- [ ] `mcp/src/tools.rs` imports all domain types from `mnemonic_core::`
- [ ] `cargo test -p mnemonic-core` — all tests green (existing + new httpmock + contract tests)
- [ ] `cargo clippy -p mnemonic-core -- -D warnings` — zero warnings
- [ ] `cargo build -p mnemonic-mcp` — compiles successfully
- [ ] MCP local mode round-trip works (sign_memory → recall returns same content)
- [ ] `cargo test -p mnemonic-core` succeeds independently (without building mcp) — standalone usability
- [ ] `turboquant-plus-rs = "0.1.0"` in core/Cargo.toml
- [ ] `grep -r "HashEmbedder" core/src/` — empty
- [ ] `grep -r "create_api_key\|deduct_balance\|credit_deposit\|mark_x402_nonce\|record_attestation_cost\|get_pnl_stats\|get_owner_pubkey\|verify_usdc_transfer" core/src/` — empty (all 8 payment methods including verify_usdc_transfer)
- [ ] Benchmarks in `core/benches/`, proptests in `core/tests/`
- [ ] `lineage.rs`: Direction is enum, chain_valid is `Option<bool>`, errors propagate via `?`
- [ ] `architecture.md` updated with `codec/` and `lineage/` in core/src/ description, stale references removed
- [ ] `pricing.rs` unchanged in mcp/src/ (not moved, not modified)
- [ ] No regressions in existing MCP functionality
- [ ] All SQL in core/src/storage/ uses rusqlite parameterized queries

## Implementation Tasks

### Wave 1: Workspace scaffold

#### Task 1: Workspace scaffold + turboquant migration
- **Description:** Create workspace root Cargo.toml, core/ crate skeleton with lib.rs, and mcp/ Cargo.toml adjustments. Replace turboquant git dependency with turboquant-plus-rs from crates.io and update all imports. Foundation for all subsequent extraction.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo search turboquant-plus-rs && cargo build -p mnemonic-core && cargo build -p mnemonic-mcp`
- **Files to modify:** `Cargo.toml` (new workspace root), `core/Cargo.toml` (new), `core/src/lib.rs` (new), `mcp/Cargo.toml` (update deps)
- **Files to read:** `mcp/Cargo.toml` (current), `mcp/src/compress.rs` (turboquant imports)

### Wave 2: Codec extraction (depends on Wave 1)

#### Task 2: Extract codec module
- **Description:** Move codec/ (schema, canonical, hash, sign) from mcp/src/ to core/src/codec/. Update mcp imports to use mnemonic_core::codec. Codec has zero internal dependencies — cleanest extraction target.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- codec && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/codec/` (new: mod.rs, schema.rs, canonical.rs, hash.rs, sign.rs), `core/src/lib.rs`, `core/Cargo.toml`, `mcp/src/tools.rs`, `mcp/src/mcp.rs`
- **Files to read:** `mcp/src/codec/` (current source)

### Wave 3: Identity extraction (depends on Wave 2)

#### Task 3: Extract identity module
- **Description:** Move identity.rs from mcp/src/ to core/src/identity/. Contains keypair loading, DID derivation, signing. Depends only on solana-sdk. Update mcp imports.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- identity && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/identity/` (new), `core/src/lib.rs`, `core/Cargo.toml`, `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/identity.rs`

### Wave 4: Embed extraction (depends on Wave 3)

#### Task 4: Extract embed module + remove HashEmbedder
- **Description:** Move embed.rs to core/src/embed/. Define Embedder trait, move OpenAIEmbedder and FastEmbedder. Remove HashEmbedder, introduce MockEmbedder (#[cfg(test)]) returning deterministic fixed vectors. Update all tests.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- embed && grep -r "HashEmbedder" core/src/` (second must be empty)
- **Files to modify:** `core/src/embed/` (new), `core/src/lib.rs`, `core/Cargo.toml` (fastembed optional dep), `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/embed.rs`, `mcp/src/db.rs` (HashEmbedder usage in tests)

### Wave 5: Compress extraction (depends on Wave 4)

#### Task 5: Extract compress module
- **Description:** Move compress.rs to core/src/compress/. Uses turboquant-plus-rs (migrated in Task 1) and ndarray. Add numerical fidelity roundtrip test verifying MSE stays below threshold after namespace change.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- compress && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/compress/` (new), `core/src/lib.rs`, `core/Cargo.toml`, `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/compress.rs`

### Wave 6: Storage extraction (depends on Wave 5)

#### Task 6: Extract storage with trait split
- **Description:** Create AttestationStore and LineageStore traits in core/src/storage/. Move SQLite implementation from db.rs, excluding payment methods. Add contract tests verifying trait methods through SQLite impl. Document that authorization is caller's responsibility.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- storage && grep -r "create_api_key\|deduct_balance" core/src/` (second must be empty)
- **Files to modify:** `core/src/storage/` (new: mod.rs, traits.rs, sqlite.rs), `core/src/lib.rs`, `core/Cargo.toml`, `mcp/src/payment.rs`, `mcp/src/tools.rs`, `mcp/src/main.rs`
- **Files to read:** `mcp/src/db.rs`, `mcp/src/payment.rs`

### Wave 7: Arweave extraction (depends on Wave 6)

#### Task 7: Extract arweave module + add httpmock tests
- **Description:** Move arweave.rs to core/src/arweave/. Add ~6 httpmock tests: write, read, write_bytes, health_check, network timeout, malformed response. No real mainnet URLs or funded keypairs in test code.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- arweave && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/arweave/` (new), `core/src/lib.rs`, `core/Cargo.toml` (httpmock dev-dep, tokio dev features), `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/arweave.rs`

### Wave 8: Solana extraction (depends on Wave 7)

#### Task 8: Extract solana module + add httpmock tests + isolate verify_usdc_transfer
- **Description:** Move solana.rs to core/src/solana/, but extract verify_usdc_transfer out of SolanaClient into mcp/src/payment.rs as a standalone async fn taking &SolanaClient. Add ~7 httpmock tests for core SolanaClient methods. No real mainnet URLs or funded keypairs.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- solana && grep -r "verify_usdc_transfer" core/src/` (second must be empty)
- **Files to modify:** `core/src/solana/` (new), `core/src/lib.rs`, `core/Cargo.toml`, `mcp/src/payment.rs`, `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/solana.rs`, `mcp/src/payment.rs`

### Wave 9: Lineage extraction (depends on Wave 8)

#### Task 9: Extract lineage module + cleanup
- **Description:** Move lineage.rs to core/src/lineage/. Apply three fixes: Direction becomes enum, chain_valid becomes Option<bool>, DB errors propagate via ?. Add test for error propagation. Update all 9 existing tests.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- lineage && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/lineage/` (new), `core/src/lib.rs`, `core/Cargo.toml`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/lineage.rs`, `core/src/codec/schema.rs` (ParentRef, MAX_* constants)

### Wave 10: Test artifacts (depends on Wave 9)

#### Task 10: Move integration tests, proptests, and benchmarks
- **Description:** Move tests/integration_cbor.rs and tests/proptest_canonical.rs to core/tests/. Move benches/ to core/benches/. Remove inline helper duplication — import from mnemonic_core directly.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core --tests && cargo bench -p mnemonic-core --no-run`
- **Files to modify:** `core/tests/` (new), `core/benches/` (new), `core/Cargo.toml` (criterion dev-dep, bench targets)
- **Files to read:** `mcp/tests/integration_cbor.rs`, `mcp/tests/proptest_canonical.rs`, `mcp/benches/decompress.rs`, `mcp/benches/cbor_codec.rs`

### Wave 11: Final rewiring (depends on Wave 10)

#### Task 11: MCP server rewiring + full verification
- **Description:** Final cleanup of mcp/ imports — all domain types from mnemonic_core::. Remove leftover module files from mcp/src/. Verify MCP local mode round-trip via JSON-RPC stdio. Confirm no domain logic remains in mcp/.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-mcp && cargo test --workspace && cargo clippy --workspace -- -D warnings`
- **Verify-user:** Run `cargo run -p mnemonic-mcp` in local mode, call `mnemonic_whoami` via Cursor or Claude Desktop — verify pubkey matches pre-migration keypair.
- **Files to modify:** `mcp/src/tools.rs`, `mcp/src/mcp.rs`, `mcp/src/main.rs`, `mcp/src/payment.rs`, `mcp/Cargo.toml`
- **Files to read:** `core/src/lib.rs` (public API), `mcp/src/` (all remaining files)

#### Task 12: Update architecture.md and patterns.md documentation
- **Description:** Update architecture.md to reflect new core/ structure with codec/ and lineage/ modules. Remove stale references: attest/ (now codec/), wasm/ (out of scope), HashEmbedder (removed), three-provider embed (now two). Update patterns.md: remove "hash fallback" from Embedder trait section, remove wasm/mod.rs reference.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `grep -E "codec/|lineage/" .claude/skills/project-knowledge/references/architecture.md` (both found)
- **Files to modify:** `.claude/skills/project-knowledge/references/architecture.md`, `.claude/skills/project-knowledge/references/patterns.md`
- **Files to read:** `core/src/lib.rs`, `.claude/skills/project-knowledge/references/architecture.md`, `.claude/skills/project-knowledge/references/patterns.md`

### Audit Wave

#### Task 13: Code Audit
- **Description:** Full-feature code quality audit. Review all source files created/modified for cross-component issues: import consistency, public API surface correctness, architectural consistency between core and mcp.
- **Skill:** code-reviewing
- **Reviewers:** none
- **Files to read:** `core/src/**/*.rs`, `mcp/src/**/*.rs`, `Cargo.toml`, `core/Cargo.toml`, `mcp/Cargo.toml`
- **Files to modify:** N/A (analysis only)

#### Task 14: Security Audit
- **Description:** Full-feature security audit. Verify no secret material in core/, payment methods isolated in mcp/, httpmock tests contain no real endpoints, SQL uses parameterized queries, keypair file handling is safe.
- **Skill:** security-auditor
- **Reviewers:** none
- **Files to read:** `core/src/**/*.rs`, `mcp/src/payment.rs`, `mcp/src/tools.rs`, `core/Cargo.toml`
- **Files to modify:** N/A (analysis only)

#### Task 15: Test Audit
- **Description:** Full-feature test quality audit. Verify coverage of extracted modules, MockEmbedder adequacy, httpmock scenario completeness, contract test coverage for storage traits, edge case coverage, test pyramid balance.
- **Skill:** test-master
- **Reviewers:** none
- **Files to read:** `core/src/**/*.rs` (test modules), `core/tests/**/*.rs`, `core/benches/**/*.rs`
- **Files to modify:** N/A (analysis only)

### Final Wave

#### Task 16: Pre-deploy QA
- **Description:** Acceptance testing: run all tests (`cargo test --workspace`, `cargo clippy --workspace -- -D warnings`), verify all acceptance criteria from user-spec and tech-spec. Full verification checklist execution.
- **Skill:** pre-deploy-qa
- **Reviewers:** none
- **Files to read:** `work/mnemonic-core/user-spec.md`, `work/mnemonic-core/tech-spec.md`, `core/src/**/*.rs`, `mcp/src/**/*.rs`
- **Files to modify:** N/A (verification only)
