---
created: 2026-04-20
status: draft
branch: dev
size: L
---

# Tech Spec: mnemonic-core extraction

## Solution

Extract all domain logic from the monolithic MCP server (`mnemonic-protocol/mcp/`) into a standalone Rust library crate `mnemonic-core`. The MCP server becomes a thin wrapper that depends on core as a Cargo workspace member.

The extraction follows a strict phased order: codec → identity → embed → compress → db/storage → arweave/solana → lineage. After each phase, `cargo test` and `cargo clippy` must pass — no broken intermediate states.

Key technical moves:
1. Create workspace root `Cargo.toml` with `[workspace] members = ["core", "mcp"]` and `resolver = "2"`.
2. Create `core/` crate with `lib.rs` re-exporting public modules.
3. Move modules one-by-one, updating imports in `mcp/` to use `mnemonic_core::` paths.
4. Split `db.rs` into storage traits (`AttestationStore`, `LineageStore`) in core and keep payment methods in `mcp/`.
5. Replace `turboquant` git dependency with `turboquant-plus-rs = "0.1.0"` from crates.io.
6. Remove `HashEmbedder` entirely — it prevents embedding decompression.
7. Add httpmock-based tests for `arweave.rs` and `solana.rs` (currently zero coverage).
8. Move benchmarks and proptests to `core/benches/` and `core/tests/`.

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

### Decision 1: Phased extraction order
**Decision:** codec → identity → embed → compress → db/storage → arweave/solana → lineage
**Rationale:** Follows the dependency graph bottom-up. Codec has zero internal dependencies. Identity depends only on solana-sdk. Each subsequent module depends only on previously extracted modules. This ensures `cargo test` stays green after each step. Supports US constraint: "After each step cargo test and cargo clippy green." `[TECHNICAL]`
**Alternatives considered:** Big-bang move (all at once) — rejected because a single broken step would leave the entire codebase uncompilable, violating the user-spec constraint.

### Decision 2: Storage trait split
**Decision:** Define `AttestationStore` trait and `LineageStore` trait in `core/src/storage/`. SQLite implementations live in `core/src/storage/sqlite.rs`. Payment methods (`create_api_key`, `deduct_balance`, `credit_deposit`, `mark_x402_nonce`, `record_attestation_cost`, `get_pnl_stats`, `get_owner_pubkey`, `verify_usdc_transfer`) stay in `mcp/src/payment.rs` operating on the same `rusqlite::Connection`.
**Rationale:** Core consumers need attestation CRUD and search but never need payment logic. Supports US technical decision: "Storage traits: AttestationStore and LineageStore — traits in core; payment methods stay in mcp/." `[TECHNICAL]`
**Alternatives considered:** Move everything including payments to core — rejected because payment logic is MCP-server-specific (axum headers, USDC verification).

### Decision 3: turboquant dependency migration
**Decision:** Replace `turboquant = { git = "..." }` with `turboquant-plus-rs = "0.1.0"` from crates.io. Update all imports from `turboquant::` to `turboquant_plus_rs::`.
**Rationale:** Git dependencies block crates.io publishing of mnemonic-core. Supports US acceptance criterion: "`turboquant-plus-rs = \"0.1.0\"` in core/Cargo.toml — crates.io, not git." Supports US risk 1 mitigation: first step is dep change + import update + cargo test.
**Alternatives considered:** Fork and publish under same name — rejected because `turboquant-plus-rs` already exists on crates.io.

### Decision 4: HashEmbedder removal
**Decision:** Remove `HashEmbedder` from the codebase entirely. Tests that used it switch to mock implementations or skip embedding.
**Rationale:** `HashEmbedder` produces non-invertible hashes instead of real embeddings — decompression returns garbage. Supports US acceptance criterion: "`HashEmbedder` absent in core/ — grep empty." `[TECHNICAL]`
**Alternatives considered:** Keep as test-only utility — rejected per user-spec explicit decision.

### Decision 5: lineage.rs cleanup on move
**Decision:** Fix three issues during lineage extraction: (a) `Direction` becomes an enum instead of string, (b) `chain_valid` becomes `Option<bool>` (unknown vs verified), (c) DB errors propagate via `?` instead of being swallowed.
**Rationale:** Supports US acceptance criterion: "lineage.rs: errors propagate via ?; chain_valid: Option<bool>; Direction — enum." These are bug fixes that are safest to apply during the move when the module is already being modified. `[TECHNICAL]`
**Alternatives considered:** Fix in a separate task — rejected because touching the same code twice increases merge risk.

### Decision 6: httpmock tests for arweave and solana
**Decision:** Add httpmock-based unit tests for `arweave.rs` and `solana.rs` during their extraction. Mock Irys upload/read endpoints and Solana JSON-RPC.
**Rationale:** These modules currently have zero test coverage. Supports US risk 2 mitigation and acceptance criterion: "httpmock-tests for arweave/solana." `[TECHNICAL]`
**Alternatives considered:** Integration tests with arlocal/solana-test-validator — rejected for this iteration because they require external services in CI.

### Decision 7: No WASM concerns in this iteration
**Decision:** All code in core/ targets native only. No `#[cfg(target_arch = "wasm32")]` gates, no `wasm-bindgen` exports, no `wasm/mod.rs`.
**Rationale:** Supports US constraint: "Native-only: WASM, wasm/mod.rs, web_sys, localStorage — out of scope." WASM is iteration 2.
**Alternatives considered:** Add WASM feature gates now — rejected per user-spec scope.

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
`sha2`, `hex`, `base64`, `serde`, `serde_json`, `blake3`, `ciborium`, `coset`, `chrono`, `anyhow`, `thiserror`, `uuid`, `bs58`, `bincode`, `ndarray`, `turboquant-plus-rs`, `solana-sdk`, `spl-memo`, `rusqlite` (bundled), `reqwest` (json), `futures`, `tracing` (optional), `fastembed` (optional, feature = "local-embed")

**mcp/Cargo.toml:**
`axum`, `axum-extra`, `tower-http`, `clap`, `tokio` (full), `tokio-stream`, `dotenvy`, `tracing-subscriber`, `solana-client`, `solana-transaction-status`, `mnemonic-core` (path = "../core")

### Removed packages
- `turboquant = { git = "..." }` — replaced by `turboquant-plus-rs`

## Testing Strategy

**Feature size:** L

### Unit tests
- **codec modules** (existing 24 tests): move to `core/src/codec/` `#[cfg(test)]` blocks, verify they pass after move
- **embed module** (existing 8 tests): move, remove `HashEmbedder` references, replace with mock or `OpenAIEmbedder` with env check
- **compress module** (existing 4 tests): move, update `turboquant` → `turboquant_plus_rs` imports, verify roundtrip still works
- **identity module** (existing 4 tests): move, verify keypair roundtrip with `tempfile`
- **db/storage module** (existing 2 tests): move, verify save+count and search ranking work with trait-based API
- **arweave module** (new, ~4 tests): httpmock tests for `write`, `read`, `health_check`, error handling
- **solana module** (new, ~4 tests): httpmock tests for `write_memo`, `read_memo`, `airdrop`, `get_tx_signers`
- **lineage module** (existing 9 tests): move, update for `Direction` enum and `chain_valid: Option<bool>`, verify all 8+ tests pass including cycle detection

### Integration tests
- **MCP round-trip** (existing + updated): `cargo build -p mnemonic-mcp` compiles; JSON-RPC `tools/list` returns 5 tools; `sign_memory` → `recall` round-trip works in local mode via stdio
- **Codec pipeline** (existing 5 tests): move `tests/integration_cbor.rs` to `core/tests/`, remove inline helper duplication (now imports from lib)
- **Proptest** (existing 1 test): move `tests/proptest_canonical.rs` to `core/tests/`
- **Benchmark** (existing 2 files): move `benches/decompress.rs` and `benches/cbor_codec.rs` to `core/benches/`

### E2E tests
None — no deployed environment. Per user-spec: "E2E tests: not done — no deployed environment."

## Agent Verification Plan

**Source:** user-spec "How to verify" section.

### Verification approach
After each wave, the agent runs `cargo test -p mnemonic-core && cargo clippy -p mnemonic-core -- -D warnings` to confirm no regressions. After the final implementation wave, full verification:
1. `cargo test -p mnemonic-core` — all tests green including new httpmock tests
2. `cargo clippy -p mnemonic-core -- -D warnings` — zero warnings
3. `cargo build -p mnemonic-mcp` — compiles with core as workspace dep
4. MCP local mode round-trip via JSON-RPC stdio
5. `grep -r "HashEmbedder" core/src/` — empty
6. `grep -r "create_api_key\|deduct_balance\|credit_deposit\|mark_x402_nonce\|record_attestation_cost\|get_pnl_stats\|get_owner_pubkey\|verify_usdc_transfer" core/src/` — empty
7. `grep -E "codec/|lineage/" .claude/skills/project-knowledge/references/architecture.md` — both lines found

### Tools required
bash (cargo commands, grep), JSON-RPC stdio client (for MCP round-trip test)

## Risks

| Risk | Mitigation |
|------|-----------|
| turboquant namespace change breaks imports | First task is only dep swap + import update + cargo test. No other changes in that step. |
| arweave/solana zero test coverage hides bugs during move | Add httpmock tests as part of the extraction task, before modifying any logic. |
| fastembed model download in CI | CI sets fastembed cache env var. Tests that need embeddings use mock or check EMBED_PROVIDER env. |
| Circular dependency between core and mcp during partial migration | Phased order ensures each module only depends on already-moved modules. MCP re-imports from core progressively. |
| rusqlite::Connection is !Send — async wrappers break | Storage trait methods are sync. MCP wraps in Mutex per existing pattern (patterns.md). No change needed. |

## User-Spec Deviations

None

## Acceptance Criteria

Technical acceptance criteria (supplement user-spec criteria):

- [ ] Workspace root `Cargo.toml` exists with `members = ["core", "mcp"]` and `resolver = "2"`
- [ ] `core/Cargo.toml` exists with `[lib]` target, all domain dependencies listed
- [ ] `core/src/lib.rs` re-exports: `codec`, `embed`, `compress`, `identity`, `storage`, `arweave`, `solana`, `lineage`
- [ ] `mcp/Cargo.toml` has `mnemonic-core = { path = "../core" }` dependency
- [ ] `mcp/src/tools.rs` imports all domain types from `mnemonic_core::`
- [ ] `cargo test -p mnemonic-core` — all tests green (existing + new httpmock)
- [ ] `cargo clippy -p mnemonic-core -- -D warnings` — zero warnings
- [ ] `cargo build -p mnemonic-mcp` — compiles successfully
- [ ] MCP local mode round-trip works (sign_memory → recall returns same content)
- [ ] `turboquant-plus-rs = "0.1.0"` in core/Cargo.toml
- [ ] `grep -r "HashEmbedder" core/src/` — empty
- [ ] `grep -r "create_api_key\|deduct_balance" core/src/` — empty (all 8 payment methods)
- [ ] Benchmarks in `core/benches/`, proptests in `core/tests/`
- [ ] `lineage.rs`: Direction is enum, chain_valid is `Option<bool>`, errors propagate via `?`
- [ ] `architecture.md` updated with `codec/` and `lineage/` in core/src/ description
- [ ] No regressions in existing MCP functionality

## Implementation Tasks

### Wave 1 (independent)

#### Task 1: Workspace scaffold + turboquant migration
- **Description:** Create workspace root Cargo.toml, core/ crate skeleton with lib.rs, and mcp/ Cargo.toml adjustments. Replace turboquant git dependency with turboquant-plus-rs 0.1.0 from crates.io and update all imports. This is the foundation for all subsequent extraction tasks.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-core && cargo build -p mnemonic-mcp && cargo test -p mnemonic-core`
- **Files to modify:** `Cargo.toml` (new workspace root), `core/Cargo.toml` (new), `core/src/lib.rs` (new), `mcp/Cargo.toml` (update deps)
- **Files to read:** `mcp/Cargo.toml` (current), `mcp/src/compress.rs` (turboquant imports)

### Wave 2 (depends on Wave 1)

#### Task 2: Extract codec module
- **Description:** Move codec/ (schema, canonical, hash, sign) from mcp/src/ to core/src/codec/. Update mcp imports to use mnemonic_core::codec. Codec has zero internal dependencies — cleanest extraction target.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- codec && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/codec/` (new: mod.rs, schema.rs, canonical.rs, hash.rs, sign.rs), `core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/mcp.rs`
- **Files to read:** `mcp/src/codec/` (current source)

#### Task 3: Extract identity module
- **Description:** Move identity.rs from mcp/src/ to core/src/identity/. Contains keypair loading, DID derivation, signing. Depends only on solana-sdk. Update mcp imports.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- identity && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/identity/` (new), `core/src/lib.rs`, `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/identity.rs`

#### Task 4: Extract embed module + remove HashEmbedder
- **Description:** Move embed.rs from mcp/src/ to core/src/embed/. Define Embedder trait, move OpenAIEmbedder and FastEmbedder. Remove HashEmbedder entirely. Update tests to not depend on HashEmbedder.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- embed && grep -r "HashEmbedder" core/src/` (second must be empty)
- **Files to modify:** `core/src/embed/` (new), `core/src/lib.rs`, `core/Cargo.toml` (fastembed optional dep), `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/embed.rs`

#### Task 5: Extract compress module
- **Description:** Move compress.rs from mcp/src/ to core/src/compress/. Uses turboquant-plus-rs (already migrated in Task 1) and ndarray. Update mcp imports.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- compress && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/compress/` (new), `core/src/lib.rs`, `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/compress.rs`

### Wave 3 (depends on Wave 2)

#### Task 6: Extract storage with trait split
- **Description:** Create AttestationStore and LineageStore traits in core/src/storage/. Move SQLite implementation from db.rs to core/src/storage/sqlite.rs. Exclude payment methods — they stay in mcp/src/payment.rs. Move existing db tests.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- storage && grep -r "create_api_key\|deduct_balance" core/src/` (second must be empty)
- **Files to modify:** `core/src/storage/` (new: mod.rs, traits.rs, sqlite.rs), `core/src/lib.rs`, `mcp/src/payment.rs`, `mcp/src/tools.rs`, `mcp/src/main.rs`
- **Files to read:** `mcp/src/db.rs`, `mcp/src/payment.rs`

#### Task 7: Extract arweave module + add httpmock tests
- **Description:** Move arweave.rs from mcp/src/ to core/src/arweave/. Add httpmock-based tests mocking Irys endpoints for write, read, health_check, and error scenarios. Currently zero test coverage.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- arweave && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/arweave/` (new), `core/src/lib.rs`, `core/Cargo.toml` (httpmock dev-dep), `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/arweave.rs`

#### Task 8: Extract solana module + add httpmock tests
- **Description:** Move solana.rs from mcp/src/ to core/src/solana/. Add httpmock-based tests mocking Solana JSON-RPC for write_memo, read_memo, airdrop, get_tx_signers. Currently zero test coverage.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- solana && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/solana/` (new), `core/src/lib.rs`, `mcp/src/main.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/solana.rs`

### Wave 4 (depends on Wave 3)

#### Task 9: Extract lineage module + cleanup
- **Description:** Move lineage.rs from mcp/src/ to core/src/lineage/. Apply three fixes during move: Direction becomes enum, chain_valid becomes Option<bool>, DB errors propagate via ?. Update all 9 existing tests for new types.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core -- lineage && cargo clippy -p mnemonic-core -- -D warnings`
- **Files to modify:** `core/src/lineage/` (new), `core/src/lib.rs`, `mcp/src/tools.rs`
- **Files to read:** `mcp/src/lineage.rs`, `mcp/src/codec/schema.rs` (ParentRef, MAX_* constants)

#### Task 10: Move integration tests, proptests, and benchmarks
- **Description:** Move tests/integration_cbor.rs and tests/proptest_canonical.rs to core/tests/. Move benches/decompress.rs and benches/cbor_codec.rs to core/benches/. Remove inline helper duplication in integration tests — they now import directly from mnemonic_core.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core --tests && cargo bench -p mnemonic-core --no-run`
- **Files to modify:** `core/tests/` (new), `core/benches/` (new), `core/Cargo.toml` (criterion dev-dep, bench targets)
- **Files to read:** `mcp/tests/integration_cbor.rs`, `mcp/tests/proptest_canonical.rs`, `mcp/benches/decompress.rs`, `mcp/benches/cbor_codec.rs`

### Wave 5 (depends on Wave 4)

#### Task 11: MCP server rewiring + full verification
- **Description:** Final cleanup of mcp/ imports — ensure all domain types come from mnemonic_core::. Remove leftover module files from mcp/src/. Verify MCP local mode round-trip via JSON-RPC stdio (tools/list → sign_memory → recall). Confirm no domain logic remains in mcp/.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-mcp && cargo test --workspace && cargo clippy --workspace -- -D warnings`
- **Verify-user:** Run `cargo run -p mnemonic-mcp` in local mode, call `mnemonic_whoami` via Cursor or Claude Desktop — verify pubkey matches pre-migration keypair.
- **Files to modify:** `mcp/src/tools.rs`, `mcp/src/mcp.rs`, `mcp/src/main.rs`, `mcp/src/payment.rs`, `mcp/Cargo.toml`
- **Files to read:** `core/src/lib.rs` (public API), `mcp/src/` (all remaining files)

#### Task 12: Update architecture.md documentation
- **Description:** Update .claude/skills/project-knowledge/references/architecture.md to reflect the new core/ crate structure including codec/ and lineage/ modules. Ensure grep for "codec/" and "lineage/" in architecture.md returns matches.
- **Skill:** documentation-writing
- **Reviewers:** code-reviewer
- **Verify-smoke:** `grep -E "codec/|lineage/" .claude/skills/project-knowledge/references/architecture.md` (both lines found)
- **Files to modify:** `.claude/skills/project-knowledge/references/architecture.md`
- **Files to read:** `core/src/lib.rs`, `.claude/skills/project-knowledge/references/architecture.md` (current)

### Audit Wave

#### Task 13: Code Audit
- **Description:** Full-feature code quality audit. Read all source files created/modified in this feature (core/src/, mcp/src/, Cargo.toml files). Review holistically for cross-component issues: duplicate resource initialization, import consistency, public API surface correctness, architectural consistency between core and mcp separation.
- **Skill:** code-reviewing
- **Reviewers:** none

#### Task 14: Security Audit
- **Description:** Full-feature security audit. Read all source files created/modified in this feature. Analyze for OWASP Top 10 across all components, verify no secret material in core/, check that payment methods are properly isolated in mcp/, validate httpmock test safety.
- **Skill:** security-auditor
- **Reviewers:** none

#### Task 15: Test Audit
- **Description:** Full-feature test quality audit. Read all test files created in this feature (core unit tests, httpmock tests, integration tests, proptests, benchmarks). Verify coverage of newly extracted modules, meaningful assertions, test pyramid balance.
- **Skill:** test-master
- **Reviewers:** none

### Final Wave

#### Task 16: Pre-deploy QA
- **Description:** Acceptance testing: run all tests (`cargo test --workspace`, `cargo clippy --workspace -- -D warnings`), verify all acceptance criteria from user-spec and tech-spec. Full verification checklist execution.
- **Skill:** pre-deploy-qa
- **Reviewers:** none
