---
phase: 0
title: Inventory + dependency audit
status: ready
risk: none
---

# Phase 0 — Inventory + dependency audit

## Objective

Produce the exact, grounded move-list and dependency map that de-risks every
later phase. **No code moves in this phase** — output is documentation only.

## Why this phase exists

Phases 2 and 3 split `core/` along the native/portable line. Before moving a
single file we must know, precisely: which `core` modules pull which native
deps, every `use mnemonic_core::...` call site in `mcp` (and the JS consumers of
the wasm artifact), and every CI path filter / build step that references the
moved paths. Getting this wrong turns a mechanical move into a debugging
session against the hard `cross-lang-build` gate.

## Files moved

None. This is a read-only audit.

## Exact work — produce these artifacts (commit them under `work/refactoring/`)

### 0.1 — Crate-type + dep inventory of `core/Cargo.toml`

Record (already captured here from the real file):

- `[lib] crate-type = ["cdylib", "rlib"]` — `cdylib` exists *only* for
  `wasm-pack build core`. Phase 2 removes `cdylib`; `crates/wasm` takes it.
- Unconditional native deps that must follow the native modules to
  `crates/native`: `solana-sdk = "2.2"`, `spl-memo = "6"`,
  `keyring = "3"` (apple-native/windows-native/sync-secret-service),
  `crypto_box = "0.9"`, `fastembed = "5"` (optional, `local-embed`),
  `tokio = "1"` (time/rt), `dirs = "6"`, `tempfile = "3"`.
- `target.'cfg(not(target_arch="wasm32"))'.dependencies`:
  `rusqlite = "0.34"` (bundled), `reqwest = "0.12"`.
- Truly portable deps that stay in `core`: `sha2`, `hex`, `base64`, `serde`,
  `serde_json`, `blake3`, `ciborium`, `coset`, `chrono`, `anyhow`, `thiserror`,
  `uuid`, `bs58`, `bincode`, `ndarray`, `turboquant-plus-rs`, `futures`,
  `tracing`.
- `target.'cfg(target_arch="wasm32")'.dependencies`: `wasm-bindgen = "=0.2.100"`,
  `wasm-bindgen-futures`, `js-sys`, `serde-wasm-bindgen`, `getrandom` (0.2 `js`)
  + `getrandom_v03` (0.3 `wasm_js`). These follow the wasm surface to
  `crates/wasm`.
- Features today: `default`, `local-embed`, `openssl-vendored`, `wasm`,
  `golden-fixtures`, `trajectory-experimental`. Track which feature lands in
  which crate post-split (see §0.4).
- `[[bench]]` × 4 (`decompress`, `decompress_fidelity`,
  `decompress_fidelity_real` [local-embed], `cbor_codec`), `[[example]]` ×
  (`emit_golden` [golden-fixtures], `golden-keystore-gen`). Plus the examples on
  disk in `core/examples/`: `emit_golden.rs`, `golden-keystore-gen.rs`,
  `keychain-read.rs`, `keychain-roundtrip.rs`.

### 0.2 — Module → target → destination map (`core/src/lib.rs`)

| Module (`core/src/...`) | Current cfg | Pulls native deps? | Phase-3 destination |
|---|---|---|---|
| `codec/` | portable | no | `crates/core` |
| `compress/` | portable | no | `crates/core` |
| `identity/` | portable | no (uses solana `Keypair` types) | `crates/core` *(see note)* |
| `merkle.rs` | portable | no | `crates/core` |
| `trajectory/` | `feature=trajectory-experimental` | no | `crates/core` |
| `arweave/` | `cfg(not(wasm32))` | reqwest | `crates/native` |
| `embed/` | `cfg(not(wasm32))` | fastembed/reqwest | `crates/native` |
| `encrypt.rs` | `cfg(not(wasm32))` | crypto_box | `crates/native` |
| `lineage/` | `cfg(not(wasm32))` | storage | `crates/native` (split: pure logic → core) |
| `rebuild.rs` | `cfg(not(wasm32))` | compressor + storage | `crates/native` |
| `solana/` | `cfg(not(wasm32))` | solana-sdk/spl-memo | `crates/native` |
| `storage/` | `cfg(not(wasm32))` | rusqlite | `crates/native` |
| `wasm/` | `cfg(all(wasm32, feature=wasm))` | wasm-bindgen | `crates/wasm` |

> **`identity` note:** `core/src/wasm/mod.rs` imports
> `solana_sdk::signature::Keypair` and `crate::identity::{pubkey_base58,
> sign_bytes}`. `identity` is *portable* (it compiles to wasm today) but depends
> on `solana-sdk` for the `Keypair` type. Phase 0 must confirm whether
> `solana-sdk` compiles to `wasm32-unknown-unknown` as a pure type dependency
> (it does today, since `core` already builds for wasm with `solana-sdk` in
> `[dependencies]`). **Decision needed (record in decisions.md):** keep
> `solana-sdk` as a `core` dependency for the `Keypair` type, OR introduce a
> thin keypair newtype in `core` and confine `solana-sdk` to `native`. The
> existing wasm build proves option A is viable; default to A to minimize Phase
> 3 risk.

### 0.3 — Every `use mnemonic_core::<native>::...` call site

From the audited `mcp/src/` tree, the native-module call sites Phase 3 must
re-point from `mnemonic_core::` to `mnemonic_native::`:

```
mcp/src/api.rs                 mnemonic_core::storage
mcp/src/chat.rs                arweave, embed, solana, storage
mcp/src/confirmation_token.rs  storage
mcp/src/mcp.rs                 arweave, embed (×2), solana, storage (×10)
mcp/src/payment.rs             solana (×2), storage (×2)
mcp/src/pending.rs             storage
mcp/src/publish.rs             storage
mcp/src/seed.rs                storage (×2)
mcp/src/test_support.rs        arweave, embed, solana, storage
mcp/src/tools.rs               arweave, embed, solana, storage (×3)
mcp/src/trajectory_tools.rs    storage
```

Portable `mnemonic_core::{codec,compress,identity,merkle,trajectory}` imports
stay pointing at `mnemonic_core::` and are untouched by Phase 3.

> **Refresh command** (run at the start of Phases 2/3 to catch drift; these are
> the canonical greps, run via the repo's search tooling, not committed code):
> search for `mnemonic_core::(storage|solana|arweave|embed|encrypt|lineage|rebuild)`
> across `mcp/src` and `core/examples`. `core/examples/keychain-*.rs` and
> `emit_golden.rs` also reference native modules and must be re-homed in Phase 3.

### 0.4 — Feature-flag fan-out post-split

- `local-embed` → moves to `crates/native` (`fastembed`); `mnemonic-mcp`'s
  `local-embed = ["mnemonic-native/local-embed"]` (was
  `["mnemonic-core/local-embed"]`).
- `wasm` → moves to `crates/wasm` (or becomes the default for that crate, which
  only ever builds for wasm).
- `golden-fixtures` → stays with whichever crate owns the golden emitter
  example (`emit_golden` depends on `compress` + signing; if it touches only
  portable code it can stay in `core`, else split).
- `trajectory-experimental` → stays in `core` (`trajectory/` is portable).
- `openssl-vendored` → follows `reqwest` to `crates/native`; `mnemonic-mcp`
  forwards to `mnemonic-native/openssl-vendored`.

### 0.5 — CI / infra path-reference inventory

Every place that names a moved path (so the same-commit edits in later phases
are complete):

- `.github/workflows/ci.yml`:
  - `paths-ignore` filters (`docs/**`, `work/**`, `.claude/**`) — unaffected by
    code moves but extend in Phase 5.
  - `cross-lang-build` + `cross-lang-keychain`: `cargo build -p mnemonic-mcp`,
    `cargo build -p mnemonic-core --example keychain-roundtrip`,
    `npm run build --workspace=@mnemonik-xyz/sdk|cli`.
  - `webapp` job: `working-directory: webapp` (Phase 1 → `packages/webapp`).
  - `smithery-schema` job: `yamale -s scripts/smithery-schema.yaml smithery.yaml`
    (Phase 5 moves `smithery.yaml` → `deploy/`).
  - `test-stdio` job: `tests/cross-lang/keychain.sh` (Phase 5 → `conformance/`).
- `.github/workflows/{release.yml,node-test.yml,deploy-mcp.yml,deploy-webapp.yml,
  ext-e2e.yml,nightly.yml,docs-link-check.yml}` — audit each for `core/`,
  `mcp/`, `webapp/`, `tests/`, `smithery.yaml`, `Dockerfile` references.
  `release.yml` and `node-test.yml` are known to reference cross-lang/wasm/pkg
  paths.
- `Dockerfile`: `COPY core/ core/`, `COPY mcp/ mcp/` (Phase 1 → `crates/`),
  `cargo build --release -p mnemonic-mcp --features local-embed`.
- `docker-compose.yml`: `./webapp/dist` (Phase 1 → `packages/webapp/dist`),
  `build context: .` + `dockerfile: Dockerfile` (Phase 5 if Dockerfile moves to
  `deploy/`), `./ollama` (Phase 5 → `deploy/ollama`), `./nginx.conf`,
  `./keypair`.
- `package.json` root scripts: `build:wasm:webapp` →
  `webapp/scripts/build-wasm.sh`, `build:wasm:sdk` →
  `packages/sdk/scripts/build-wasm.sh` (Phases 1 & 2).

## Validation

Read-only phase — validation is *coverage of the audit*, not a build:

```bash
# Confirm the audit captured every native-module call site (expect the §0.3 set):
#   search mcp/src + core/examples for the native module paths.
# Confirm no other workspace member already references core's native modules.
cargo build --workspace            # baseline green snapshot (must already pass)
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support
```

Establish the **baseline**: capture current green output of the full
"green at every step" command block from `overview.md` so later phases compare
against a known-good starting point.

## Rollback

Nothing to roll back — no code changed. If the audit is found incomplete mid-way
through a later phase, return here and extend §0.3 / §0.5 before proceeding.

## Definition of done / green check

- [ ] Module → target → destination table (§0.2) committed and matches
      `core/src/lib.rs` exactly.
- [ ] Native call-site list (§0.3) committed and reproducible from a fresh grep.
- [ ] CI/infra path-reference inventory (§0.5) committed; every workflow file
      audited, not just `ci.yml`.
- [ ] `identity`/`solana-sdk` wasm decision (§0.2 note) recorded in
      `decisions.md` (default: option A).
- [ ] Baseline green output of the full validation block captured.
