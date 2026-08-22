---
created: 2026-06-30
updated: 2026-06-30
status: ready
type: refactor-plan-detailed
size: XL
---

# Monorepo refactoring — detailed, executable plan

> **Supersedes `work/monorepo-refactor/plan.md`.** That document is the
> high-level source of truth for the target architecture and the six phases.
> This folder makes it *detailed and executable*: exact `git mv` lists, exact
> file edits, validation commands, rollback notes, and a "definition of done"
> per phase. Where the two disagree, **this folder wins**.

## Motivation

The repo reads as "messy" because the **module boundaries don't match the
dependency reality**. The single root cause:

> `core/` (`mnemonic-core`) conflates **(a)** portable domain logic, **(b)**
> native-only integrations, and **(c)** the wasm-bindgen export surface — held
> together by `#[cfg(...)]` gating in `core/src/lib.rs` — and the wasm artifact
> is built **twice** (sdk + webapp) from two overlapping `build-wasm.sh`
> scripts that already note a `core/pkg` / `core/pkg-web` collision risk.

Concretely, in `core/Cargo.toml` the crate is `crate-type = ["cdylib", "rlib"]`
and pulls native-only deps (`solana-sdk`, `keyring`, `fastembed`, `tokio`,
`crypto_box`, plus `rusqlite`/`reqwest` under
`target.'cfg(not(target_arch="wasm32"))'`). In `core/src/lib.rs` the modules are
split three ways by cfg:

- **portable** (compile to wasm): `codec`, `compress`, `identity`, `merkle`,
  `trajectory` (feature-gated `trajectory-experimental`).
- **native-only** (`#[cfg(not(target_arch="wasm32"))]`): `arweave`, `embed`,
  `encrypt`, `lineage`, `rebuild`, `solana`, `storage`.
- **wasm-only** (`#[cfg(all(target_arch="wasm32", feature="wasm"))]`): `wasm`
  (the `core/src/wasm/` bindgen surface).

That gating is *load-bearing complexity*: every contributor must reason about
which target a module compiles to. This plan makes the boundaries explicit
crates so the compiler enforces what cfg currently asks humans to remember.

## Target architecture

```
mnemonic/
├── Cargo.toml                 # Rust workspace
├── package.json               # JS/TS workspace
├── crates/                    # all Rust members grouped
│   ├── core/      mnemonic-core    # PORTABLE only: codec, compress, identity,
│   │                               #   merkle, trajectory + pure lineage logic.
│   │                               #   rlib only (no cdylib). native+wasm clean.
│   ├── native/    mnemonic-native  # arweave, embed, encrypt, solana, storage,
│   │                               #   rebuild, keychain. depends on core.
│   │                               #   native-only (rusqlite/reqwest/fastembed).
│   ├── wasm/      mnemonic-wasm    # the wasm-bindgen surface (was core/src/wasm).
│   │                               #   cdylib. depends on core. THE wasm artifact.
│   ├── prover/    mnemonic-prover  # zigz correspondence prover
│   │                               #   (work/research/computation-proof).
│   └── mcp/       mnemonic-mcp     # server binary. depends on core + native.
├── packages/                  # all JS/TS members grouped
│   ├── sdk/       @mnemonik-xyz/sdk        # consumes the ONE wasm artifact
│   ├── cli/       @mnemonik-xyz/cli
│   ├── extension/ @mnemonik-xyz/extension
│   ├── mcp/       @mnemonik-xyz/mcp        # npm distribution wrapper (≠ Rust mcp)
│   └── webapp/    mnemonic-webapp          # moved under packages/
├── conformance/               # cross-language golden vectors (single source)
├── deploy/                    # Dockerfile, nginx, compose, smithery, ollama, Cross.toml
└── docs/
```

### Dependency DAG (must stay one-way, no cycles)

```
          ┌──────── core (portable) ────────┐
          │            │            │        │
        native        wasm        prover    (tests use core)
          │
         mcp ─────────► native
          │
     (sdk, cli, extension, webapp consume the wasm artifact, not crates)
```

- `crates/core` depends on **nothing** else in the repo. Everything points at it.
- `crates/wasm` depends only on `core` (portable) — so wasm can never
  accidentally pull `solana`/`keyring`. The `cfg(not(wasm))` gating in `core`
  **disappears** because the native modules no longer live in `core`.
- One wasm artifact (`crates/wasm` → `pkg-web/`), consumed by sdk + webapp; the
  two `build-wasm.sh` scripts collapse into one.

## Sequencing — six phases, each ships green

| Phase | Title | Risk | Touches |
|---|---|---|---|
| 0 | Inventory + dep audit | none (read-only) | nothing — produces a move-list |
| 1 | Group, don't split | low | workspace `members`, JS `workspaces`, CI paths, Dockerfile/compose |
| 2 | Extract the wasm exporter | **high (CI gate)** | `core/src/wasm` → `crates/wasm`, both `build-wasm.sh`, `core` drops cdylib |
| 3 | Extract native integrations | **high (conflict files)** | `arweave/embed/encrypt/solana/storage/rebuild` → `crates/native`, `mcp` imports, `core/src/lib.rs` |
| 4 | Land `crates/prover` | low | new member from `work/research/computation-proof/` |
| 5 | Consolidate non-code | low | `conformance/`, `deploy/`, single `docs/` |
| 6 | Docs + rules | low | `CLAUDE.md`, `AGENTS.md`, audit rules, README |

Phase index:

- [phase-0-inventory.md](./phase-0-inventory.md)
- [phase-1-group.md](./phase-1-group.md)
- [phase-2-wasm.md](./phase-2-wasm.md)
- [phase-3-native.md](./phase-3-native.md)
- [phase-4-prover.md](./phase-4-prover.md)
- [phase-5-noncode.md](./phase-5-noncode.md)
- [phase-6-docs.md](./phase-6-docs.md)
- [risks.md](./risks.md)

## The "green at every step" principle

Every phase must end with **all** of these green before the next phase starts.
This is the non-negotiable invariant; no phase may leave the tree red "to be
fixed in the next phase".

```bash
cargo build --workspace
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk
npm run build --workspace=@mnemonik-xyz/cli
```

Plus the CI **`cross-lang-build (gate)`** job (`.github/workflows/ci.yml`),
which is the hard-required gate and exercises exactly:

```
cargo build -p mnemonic-mcp
cargo build -p mnemonic-core --example keychain-roundtrip
npm run build --workspace=@mnemonik-xyz/sdk
npm run build --workspace=@mnemonik-xyz/cli
```

When a phase moves any of those paths (Phases 1–4), the workflow edit lands **in
the same commit** as the move, never after.

## Operating constraints

- **`git mv` everything** — preserve history; update paths, never copy-delete.
- One phase = one branch off `main` = one PR. Phases 2 and 3 (which touch the
  conflict-point files `core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/main.rs`)
  run on a **clean branch, not parallel to feature work** (see risks.md).
- Regenerate `Cargo.lock` and `package-lock.json` per phase; the churn is large
  but mechanical.
- The npm-published `mnemonic-core` package name and the `core/pkg-web` public
  artifact contract are preserved through Phase 2 (or bumped deliberately).
