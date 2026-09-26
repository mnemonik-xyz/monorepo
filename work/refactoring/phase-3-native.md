---
phase: 3
title: Extract native integrations
status: ready
risk: high
depends_on: [2]
conflict_files: [crates/core/src/lib.rs, crates/mcp/src/tools.rs, crates/mcp/src/main.rs]
---

# Phase 3 — Extract native integrations

## Objective

Move `arweave`, `embed`, `encrypt`, `solana`, `storage`, `rebuild` (+ the
keychain bits in `identity`/storage) out of `crates/core` into a new native-only
crate `crates/native` (`mnemonic-native`, depends on `core`). Re-point every
`mnemonic_core::<native>` import in `mcp` (and examples) to `mnemonic_native::`.
After this, `crates/core` is portable **by construction** — the
`#[cfg(not(target_arch="wasm32"))]` gymnastics in `core/src/lib.rs` disappear.

**Risk:** touches the conflict-point files `crates/core/src/lib.rs`,
`crates/mcp/src/tools.rs`, `crates/mcp/src/main.rs`. Land **alone, on a clean
branch, not parallel to feature work** (see risks.md).

## Exact `git mv` list

```
git mv crates/core/src/arweave   crates/native/src/arweave
git mv crates/core/src/embed     crates/native/src/embed
git mv crates/core/src/encrypt.rs crates/native/src/encrypt.rs
git mv crates/core/src/solana    crates/native/src/solana
git mv crates/core/src/storage   crates/native/src/storage
git mv crates/core/src/rebuild.rs crates/native/src/rebuild.rs
git mv crates/core/src/lineage   crates/native/src/lineage
```

Native examples move with their deps:

```
git mv crates/core/examples/keychain-read.rs       crates/native/examples/keychain-read.rs
git mv crates/core/examples/keychain-roundtrip.rs  crates/native/examples/keychain-roundtrip.rs
git mv crates/core/examples/emit_golden.rs         crates/native/examples/emit_golden.rs   # if it touches native modules
```

> **`lineage` split (per the high-level plan):** if `lineage/` contains pure
> chain-validation logic (`Direction`, `chain_valid` computation) separable from
> storage, keep the pure part in `crates/core/src/lineage` and move only the
> storage-bound queries to `crates/native`. Phase 0 §0.2 flags `lineage`/
> `rebuild` as storage-coupled; the **default** here is to move them whole to
> `native` and pull pure helpers back to `core` only if `mcp` or `wasm` needs
> them portably. Record the split decision in `decisions.md`.

## Exact edits

### 3.1 — New `crates/native/Cargo.toml`

```toml
[package]
name = "mnemonic-native"
version = "0.2.8"
edition = "2021"
description = "Mnemonic Protocol native integrations — Solana, Arweave, SQLite storage, embedders, keychain"

[lib]
crate-type = ["rlib"]

[dependencies]
mnemonic-core = { path = "../core" }

# moved out of core (native integrations):
solana-sdk = "2.2"
spl-memo = "6"
keyring = { version = "3", default-features = false, features = ["apple-native", "windows-native", "sync-secret-service"] }
crypto_box = "0.9"
dirs = "6"
tempfile = "3"
fastembed = { version = "5", optional = true }
tokio = { version = "1", features = ["time", "rt"] }
rusqlite = { version = "0.34", features = ["bundled"] }
reqwest = { version = "0.12", features = ["json", "blocking"] }

# shared portable deps these modules also use directly:
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
futures = "0.3"
bs58 = "0.5"
hex = "0.4"
base64 = "0.22"
chrono = { version = "0.4", features = ["serde"] }
ndarray = "0.16"
turboquant-plus-rs = "0.1.0"

[dependencies.openssl-sys]
version = "0.9"
optional = true

[features]
default = []
local-embed = ["fastembed"]
openssl-vendored = ["openssl-sys/vendored"]

[dev-dependencies]
httpmock = "0.8"
proptest = "1"
criterion = { version = "0.5", features = [] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "test-util"] }

[[example]]
name = "keychain-roundtrip"

[[example]]
name = "keychain-read"
```

> The exact dependency set is derived from Phase 0 §0.1; trim any dep that the
> moved modules don't actually import (run `cargo build -p mnemonic-native` and
> remove unused-crate warnings). Benches that live with `decompress*` /
> `cbor_codec` stay with whichever crate owns `compress`/`codec` (i.e. `core`);
> `decompress_fidelity_real` (local-embed) may need to move to `native` if it
> uses a real embedder — audit and relocate the `[[bench]]` entry accordingly.

### 3.2 — `crates/native/src/lib.rs` (new)

```rust
//! Native-only integrations for the Mnemonic Protocol. Depends on the portable
//! `mnemonic-core` kernel. Never compiled for wasm32.
pub mod arweave;
pub mod embed;
pub mod encrypt;
pub mod lineage;   // or pure part re-exported from core; see Phase 3 decision
pub mod rebuild;
pub mod solana;
pub mod storage;
```

No `#[cfg(not(target_arch="wasm32"))]` guards — the crate is native by
construction and is simply not a dependency of `crates/wasm`.

### 3.3 — `crates/core/src/lib.rs` — delete the native module block

Remove all of:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub mod arweave;
#[cfg(not(target_arch = "wasm32"))]
pub mod embed;
#[cfg(not(target_arch = "wasm32"))]
pub mod encrypt;
#[cfg(not(target_arch = "wasm32"))]
pub mod lineage;
#[cfg(not(target_arch = "wasm32"))]
pub mod rebuild;
#[cfg(not(target_arch = "wasm32"))]
pub mod solana;
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;
```

Leaving `crates/core/src/lib.rs` as purely:

```rust
pub mod codec;
pub mod compress;
pub mod identity;
pub mod merkle;

#[cfg(feature = "trajectory-experimental")]
pub mod trajectory;
```

(`identity` keeps `solana-sdk` for the `Keypair` type per the Phase 0 decision;
that is the one remaining native-ish dep in `core` and it is wasm-clean.)

### 3.4 — `crates/core/Cargo.toml` — drop native deps

Remove the now-unused native deps from `crates/core/Cargo.toml`:

- `spl-memo`, `keyring`, `crypto_box`, `dirs`, `tempfile`, `fastembed`,
  `tokio`, and the whole
  `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` block
  (`rusqlite`, `reqwest`) and the matching dev-dependencies block
  (`httpmock`, `proptest`, `criterion`, `tokio`).
- Remove features `local-embed`, `openssl-vendored` from `core` (they move to
  `native`); keep `trajectory-experimental` (portable) and `golden-fixtures`
  (if its emitter is portable; else move to `native`).
- Keep `solana-sdk` (Keypair type for `identity`), plus all portable deps
  (`sha2`, `serde`, `blake3`, `ciborium`, `coset`, `ndarray`,
  `turboquant-plus-rs`, etc.).
- Re-home the `[[bench]]` entries: `decompress`, `decompress_fidelity`,
  `cbor_codec` stay (portable `compress`/`codec`);
  `decompress_fidelity_real` + the `emit_golden`/`golden-keystore-gen`
  examples follow their module dependencies.

### 3.5 — `crates/mcp/Cargo.toml` — add native dependency + forward features

```toml
[dependencies]
mnemonic-core = { path = "../core" }
mnemonic-native = { path = "../native" }

[features]
# was ["mnemonic-core/local-embed"]:
local-embed = ["mnemonic-native/local-embed"]
# was ["mnemonic-core/openssl-vendored"]:
openssl-vendored = ["mnemonic-native/openssl-vendored"]
# unchanged (trajectory is portable, still in core):
trajectory-experimental = ["mnemonic-core/trajectory-experimental"]
```

### 3.6 — Re-point `mcp` imports (Phase 0 §0.3 set)

In each file below, change `mnemonic_core::<m>` → `mnemonic_native::<m>` for
`m ∈ {arweave, embed, encrypt, solana, storage, lineage, rebuild}`. Portable
imports (`codec`, `compress`, `identity`, `merkle`, `trajectory`) stay on
`mnemonic_core::`.

```
crates/mcp/src/api.rs                 storage
crates/mcp/src/chat.rs                arweave, embed, solana, storage
crates/mcp/src/confirmation_token.rs  storage
crates/mcp/src/mcp.rs                 arweave, embed, solana, storage   (conflict-adjacent)
crates/mcp/src/payment.rs             solana, storage
crates/mcp/src/pending.rs             storage
crates/mcp/src/publish.rs             storage
crates/mcp/src/seed.rs                storage
crates/mcp/src/test_support.rs        arweave, embed, solana, storage
crates/mcp/src/tools.rs               arweave, embed, solana, storage   (conflict file)
crates/mcp/src/trajectory_tools.rs    storage
crates/mcp/src/main.rs                COMPRESS_SEED comment ref; any native use (conflict file)
```

> The `COMPRESS_SEED = 42` constant referenced by `crates/wasm/src/lib.rs`
> "must match `mcp/src/main.rs`" — confirm `main.rs`'s seed still matches and
> update the cross-reference comment to the new crate path.

### 3.7 — Root `Cargo.toml` — register the native member

```toml
[workspace]
members = ["crates/core", "crates/native", "crates/wasm", "crates/mcp"]
resolver = "2"
```

### 3.8 — Dockerfile (interim location at root)

`COPY crates/ crates/` already copies `crates/native`. The build line
`cargo build --release -p mnemonic-mcp --features local-embed` now resolves
`local-embed` through `mnemonic-mcp` → `mnemonic-native/local-embed`. **No
Dockerfile path change**, but confirm the feature still activates fastembed.

### 3.9 — `.github/workflows/ci.yml`

- `cross-lang-build` + `cross-lang-keychain`:
  `cargo build -p mnemonic-core --example keychain-roundtrip` →
  `cargo build -p mnemonic-native --example keychain-roundtrip` (the example
  moved with the keychain code). **Update in the same commit** — this is a hard
  gate line.
- `clippy`/`test` jobs: `--features mnemonic-mcp/test-support` unchanged;
  `local-embed` forwarding now goes through `native` (no CI literal to change).
- Do **not** touch `continue-on-error` on `cross-lang-keychain` (yo-yo rule) —
  the keychain code moving to `crates/native` is exactly the move the rule warns
  must not flip the toggle.
- Audit `nightly.yml` (fastembed ONNX / local-embed) and `release.yml` for
  `-p mnemonic-core --example` references and re-point to `mnemonic-native`.

## Validation

```bash
# core now builds for wasm WITHOUT any cfg(not(wasm32)) gating, proving portability:
cargo build -p mnemonic-core --target wasm32-unknown-unknown   # should compile clean
cargo build -p mnemonic-native                                  # native modules
cargo build --workspace
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support

# wasm artifact unaffected (wasm depends only on portable core):
bash scripts/build-wasm.sh

# JS consumers unchanged by the native split:
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk
npm run build --workspace=@mnemonik-xyz/cli
```

**cross-lang-build gate exercises:** `cargo build -p mnemonic-mcp` (now links
`mnemonic-native`), `cargo build -p mnemonic-native --example keychain-roundtrip`
(moved), sdk + cli `npm run build` (unchanged). The
`cargo build -p mnemonic-core --target wasm32-unknown-unknown` check above is
the proof that the `cfg` removal is correct: core must build for wasm with no
native modules present.

## Rollback

Larger than Phase 2 but still mechanical: `git mv` the seven native modules +
examples back under `crates/core/src/`, restore the
`#[cfg(not(target_arch="wasm32"))] pub mod ...` block in
`crates/core/src/lib.rs`, restore native deps in `crates/core/Cargo.toml`,
revert the `mnemonic_core::` → `mnemonic_native::` import edits across the 11
mcp files, drop `crates/native` from workspace members + `mcp/Cargo.toml`, and
revert the `keychain-roundtrip` example path in `ci.yml`. Because Phase 3 is one
PR, `git revert <merge>` does all of this.

## Definition of done / green check

- [ ] `crates/native` exists; the seven native modules + native examples live
      there; `crates/native/src/lib.rs` has no cfg guards.
- [ ] `crates/core/src/lib.rs` declares only portable modules; **zero**
      `cfg(not(target_arch="wasm32"))` remain in it.
- [ ] `cargo build -p mnemonic-core --target wasm32-unknown-unknown` compiles
      clean (portable-by-construction proof).
- [ ] `crates/core/Cargo.toml` carries no `rusqlite`/`reqwest`/`fastembed`/
      `keyring`/`crypto_box`/`spl-memo`/native-only deps.
- [ ] `mcp` imports re-pointed; `local-embed`/`openssl-vendored` forward through
      `mnemonic-native`.
- [ ] `ci.yml` keychain-example line points at `-p mnemonic-native`, same commit.
- [ ] Full "green at every step" block + `cross-lang-build` gate green;
      `continue-on-error` untouched.
