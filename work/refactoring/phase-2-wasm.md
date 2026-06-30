---
phase: 2
title: Extract the wasm exporter
status: ready
risk: high
depends_on: [1]
conflict_files: [crates/core/src/lib.rs, crates/core/Cargo.toml]
---

# Phase 2 — Extract the wasm exporter

## Objective

Move the wasm-bindgen surface out of `crates/core/src/wasm` into its own crate
`crates/wasm` (`mnemonic-wasm`, `cdylib`, depends only on portable `core`),
collapse the two `build-wasm.sh` scripts into one that builds `crates/wasm`, and
drop `cdylib` from `crates/core`. **This is the riskiest phase for CI** — the
`cross-lang-build` gate builds the SDK WASM — so it lands **alone, on a clean
branch**.

## Preserve the public contract

The npm package the webapp depends on is named **`mnemonic-core`** (see
`packages/webapp/package.json` → `"mnemonic-core": "0.2.4"`), and the SDK + tests
consume the artifact at `core/pkg-web/` with file names
`mnemonic_core.js` / `mnemonic_core_bg.wasm` / `mnemonic_core.d.ts` /
`mnemonic_core_bg.wasm.d.ts`. **These names are a public-ish contract.**

> **Decision (record in decisions.md):** keep the *emitted package name*
> `mnemonic_core` (wasm-pack derives it from the crate's `name`), even though the
> crate directory is `crates/wasm` and the workspace member is `mnemonic-wasm`.
> wasm-pack's emitted JS module name comes from the crate `name` in
> `Cargo.toml`, **not** the directory. To keep `mnemonic_core_bg.wasm` /
> `mnemonic_core.js` stable for SDK + webapp consumers without a coordinated
> npm bump, set the new crate's `name = "mnemonic-core-wasm"` **only if** a
> rename is acceptable; otherwise keep wasm-pack output names by setting the
> `[package] name` such that wasm-pack emits `mnemonic_core_*`. Default:
> **keep `mnemonic_core_*` output filenames** to avoid touching every SDK/webapp
> import path in this phase. Renaming the published artifact is a deliberate,
> separately-versioned follow-up.

## Exact `git mv` list

```
git mv crates/core/src/wasm crates/wasm/src
```

(`crates/core/src/wasm/mod.rs` → `crates/wasm/src/mod.rs`; rename to
`crates/wasm/src/lib.rs` in the edit step below — a `git mv` then content edit,
or `git mv crates/core/src/wasm/mod.rs crates/wasm/src/lib.rs`.)

```
git mv crates/core/src/wasm/mod.rs crates/wasm/src/lib.rs
```

## Exact edits

### 2.1 — New `crates/wasm/Cargo.toml`

```toml
[package]
name = "mnemonic-wasm"
version = "0.2.8"
edition = "2021"
description = "Mnemonic Protocol wasm-bindgen surface — browser-mediated COSE_Sign1 signing"

[lib]
# The ONE cdylib in the workspace. Emits mnemonic_core_bg.wasm via wasm-pack.
# (Output JS/wasm filenames are controlled to stay `mnemonic_core_*` for the
# SDK/webapp public contract — see Phase 2 decisions.md note.)
crate-type = ["cdylib", "rlib"]

[dependencies]
mnemonic-core = { path = "../core" }
serde = { version = "1", features = ["derive"] }

# Keypair type used by the bindgen surface (core/src/wasm imports
# solana_sdk::signature::Keypair). Kept here because the surface needs it and
# core already builds for wasm with solana-sdk present.
solana-sdk = "2.2"

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "=0.2.100"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
serde-wasm-bindgen = "0.6"
getrandom = { version = "0.2", features = ["js"] }
getrandom_v03 = { package = "getrandom", version = "0.3", features = ["wasm_js"] }

[target.'cfg(target_arch = "wasm32")'.dev-dependencies]
wasm-bindgen-test = "=0.3.50"

[features]
default = []
```

> These wasm deps are *moved out of* `crates/core/Cargo.toml`'s
> `cfg(target_arch="wasm32")` blocks — they no longer belong in core once core
> never compiles a bindgen surface.

### 2.2 — `crates/wasm/src/lib.rs`

The former `core/src/wasm/mod.rs` becomes the crate root. Two edits:

- Drop the inner-module cfg guard `#![cfg(all(target_arch = "wasm32", feature =
  "wasm"))]` — the whole crate is the wasm surface; gate the crate's lib build
  on the wasm target via `cfg(target_arch="wasm32")` at the item level only
  where needed, but the crate compiles for wasm by construction (cdylib).
- Re-point imports from `crate::...` to `mnemonic_core::...`:
  `crate::codec::canonical::to_canonical_cbor` →
  `mnemonic_core::codec::canonical::to_canonical_cbor`; likewise
  `mnemonic_core::codec::hash::hash_bytes`,
  `mnemonic_core::codec::schema::MEMORY_V1`,
  `mnemonic_core::codec::sign::sign_cose`,
  `mnemonic_core::compress::{CompressedEmbedding, EmbeddingCompressor}`,
  `mnemonic_core::identity::{pubkey_base58, sign_bytes}`. (`solana_sdk::...`
  imports stay.)
- The `COMPRESS_SEED: u64 = 42` constant moves with the file; its
  "must match `mcp/src/main.rs`" comment is updated to the new path.

### 2.3 — `crates/core/Cargo.toml` — drop cdylib + wasm-bindgen surface deps

```toml
[lib]
# Portable kernel — rlib only. The cdylib (wasm artifact) now lives in
# crates/wasm. mcp links core as an rlib; nothing else needs cdylib.
crate-type = ["rlib"]
```

Remove from `crates/core/Cargo.toml`:

- the `wasm` feature (`wasm = []`) from `[features]`.
- the entire `[target.'cfg(target_arch = "wasm32")'.dependencies]` block
  (`wasm-bindgen`, `wasm-bindgen-futures`, `js-sys`, `serde-wasm-bindgen`,
  `getrandom`, `getrandom_v03`) and the matching
  `[target.'cfg(target_arch = "wasm32")'.dev-dependencies]`
  (`wasm-bindgen-test`).

> `core` keeps `solana-sdk` (for the `Keypair` type used by `identity`) and the
> `cfg(not(target_arch="wasm32"))` native deps **for now** — those leave in
> Phase 3. Phase 2 only removes the *wasm export surface* and the cdylib.

### 2.4 — `crates/core/src/lib.rs` — remove the `wasm` module declaration

Delete:

```rust
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub mod wasm;
```

The portable + native module declarations remain unchanged in Phase 2.

### 2.5 — Root `Cargo.toml` — add the new member

```toml
[workspace]
members = ["crates/core", "crates/native"  /* not yet */, "crates/wasm", "crates/mcp"]
resolver = "2"
```

In Phase 2 (native not yet extracted):

```toml
members = ["crates/core", "crates/wasm", "crates/mcp"]
```

### 2.6 — Collapse the two `build-wasm.sh` into one

Replace both `packages/sdk/scripts/build-wasm.sh` and
`packages/webapp/scripts/build-wasm.sh` with a **single** script (canonical
location: `scripts/build-wasm.sh` at repo root, or keep
`packages/sdk/scripts/build-wasm.sh` as the home and have webapp call it). It:

- runs `wasm-pack build crates/wasm --target web` (replaces
  `wasm-pack build crates/core ...` / `wasm-pack build core ...`).
- still produces `crates/wasm/pkg`, renames to `crates/wasm/pkg-web` and (for
  the SDK golden test) `crates/wasm/pkg-nodejs` exactly as the sdk script does
  today.
- mirrors the `--target web` artifact to `packages/sdk/dist/wasm/` (SDK tarball)
  AND `packages/webapp/src/wasm/` (Vite) — merging the two current mirror
  behaviours into one pass.
- preserves the emitted filenames `mnemonic_core.js` / `mnemonic_core_bg.wasm` /
  `mnemonic_core.d.ts` / `mnemonic_core_bg.wasm.d.ts` (Phase 2 contract note).
- keeps the `wasm-opt -Oz --strip-debug --strip-producers` post-step.

Update root `package.json` scripts to point both at the one script:

```json
"scripts": {
  "build:wasm": "bash scripts/build-wasm.sh",
  "build:wasm:webapp": "bash scripts/build-wasm.sh",
  "build:wasm:sdk": "bash scripts/build-wasm.sh"
}
```

> Any SDK/webapp source that imports from `core/pkg-web` (e.g.
> `packages/sdk/test/cose.golden.test.ts` reads `core/pkg-nodejs/`) must be
> re-pointed to `crates/wasm/pkg-nodejs` / `crates/wasm/pkg-web`. Audit
> `packages/sdk/src/wasm.ts`, `packages/sdk/test/`, and
> `packages/webapp/src/` for literal `core/pkg` paths and update them. The
> emitted *module name* stays `mnemonic_core`, so JS `import` of the module API
> is unchanged — only the on-disk artifact directory moves.

### 2.7 — `.github/workflows/ci.yml`

The `cross-lang-build` / `cross-lang-keychain` step
`cargo build -p mnemonic-core --example keychain-roundtrip` is unaffected (the
`keychain-roundtrip` example is a native example, still on `mnemonic-core` in
Phase 2). The SDK build step `npm run build --workspace=@mnemonik-xyz/sdk` now
shells to the unified `scripts/build-wasm.sh` → `wasm-pack build crates/wasm`.
**No job-name or gate-semantics change** — do not touch `continue-on-error` on
`cross-lang-keychain` (yo-yo rule). Audit `release.yml` + `node-test.yml` for
`wasm-pack build core` / `core/pkg` literals and re-point to `crates/wasm`.

## Validation

```bash
# Native workspace: core now rlib-only, wasm a separate cdylib member.
cargo build --workspace
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support

# The wasm artifact, via the unified script:
bash scripts/build-wasm.sh
wasm-pack build crates/wasm --target web      # sanity: builds standalone

# JS consumers see the same emitted module name + mirrored bytes:
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk
npm run build --workspace=@mnemonik-xyz/cli
npm run build --workspace=mnemonic-webapp
```

**cross-lang-build gate exercises:** `cargo build -p mnemonic-mcp`,
`cargo build -p mnemonic-core --example keychain-roundtrip` (both still on
`mnemonic-core`, unaffected), then `npm run build` for sdk + cli — the sdk build
is the critical path because it now runs `wasm-pack build crates/wasm`. **Run
all four gate steps locally before opening the PR.**

## Rollback

Revert is a single commit: `git mv crates/wasm/src/lib.rs` back to
`crates/core/src/wasm/mod.rs`, restore `crate-type = ["cdylib","rlib"]` and the
wasm dep blocks in `crates/core/Cargo.toml`, restore the `pub mod wasm;`
declaration in `crates/core/src/lib.rs`, restore the two original
`build-wasm.sh` scripts, and remove `crates/wasm` from the workspace members.
Because the *emitted artifact name* was held constant, no JS consumer import
needs reverting (only the on-disk artifact path).

## Definition of done / green check

- [ ] `crates/wasm` exists with its own `Cargo.toml` (cdylib), `src/lib.rs`
      importing `mnemonic_core::...`.
- [ ] `crates/core` is `crate-type = ["rlib"]`; no `wasm` feature; no
      `cfg(target_arch="wasm32")` dep blocks; no `pub mod wasm;`.
- [ ] One `build-wasm.sh`; both old scripts deleted; root `package.json`
      scripts point at it.
- [ ] Emitted artifact filenames unchanged (`mnemonic_core_bg.wasm` etc.);
      SDK + webapp import the same module API.
- [ ] All SDK/webapp source references to `core/pkg*` re-pointed to
      `crates/wasm/pkg*`.
- [ ] Full "green at every step" block + `cross-lang-build` gate green.
- [ ] `continue-on-error` on `cross-lang-keychain` untouched.
