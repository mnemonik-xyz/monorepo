---
created: 2026-06-30
updated: 2026-06-30
status: draft
type: refactor-plan
size: XL
---

# Monorepo refactor plan: separate the modules

## Why

`core/` has grown into three crates wearing one hat, and the wasm build is
duplicated across two consumers. The repo reads as "messy" because the
**module boundaries don't match the dependency reality**. This plan makes the
boundaries explicit and the dependency graph one-way, without a big-bang rewrite.

## Current state (grounded inventory)

**Rust workspace** (`Cargo.toml`, `members = ["core", "mcp"]`):

- `core/` — `mnemonic-core` v0.2.8, `crate-type = ["cdylib","rlib"]`. Carries
  native-only deps (`solana-sdk`, `keyring`, `fastembed`, `tokio`, `crypto_box`)
  **and** compiles to wasm. Modules split by `cfg`:
  - **portable** (compile to wasm): `codec`, `compress`, `identity`, `merkle`,
    `trajectory`.
  - **native-only** (`#[cfg(not(target_arch="wasm32"))]`): `arweave`, `embed`,
    `encrypt`, `lineage`, `rebuild`, `solana`, `storage`.
  - **wasm-only** (`#[cfg(all(target_arch="wasm32", feature="wasm"))]`): `wasm`
    (the wasm-bindgen export surface).
- `mcp/` — `mnemonic-mcp` server binary.

**JS/TS workspace** (`package.json`, `workspaces = ["packages/*","webapp"]`):

- `packages/sdk` — `@mnemonik-xyz/sdk`. Builds wasm via `scripts/build-wasm.sh`
  → `core/pkg-web/`, plus a browser bundle.
- `packages/cli` — `@mnemonik-xyz/cli`. Depends on the SDK + keychain.
- `packages/extension` — `@mnemonik-xyz/extension`. React/Vite browser extension.
- `packages/mcp` — `@mnemonik-xyz/mcp`. npm **distribution wrapper** of the Rust
  server binary (dep: `tar`). Distinct from the Rust `mcp/`.
- `webapp/` — `mnemonic-webapp`. React/Vite app; depends on `mnemonic-core` (the
  wasm pkg) and has its **own** `build-wasm.sh` that builds wasm for BOTH itself
  AND the SDK.

**Non-code:** `docs/`, `tests/` (cross-language conformance), `scripts/`,
`ollama/`, deploy artifacts (`Dockerfile`, `nginx.conf`, `docker-compose.yml`,
`smithery.yaml`, `Cross.toml`), `.github/`.

## The core problem (one sentence)

`core/` conflates **(a)** portable domain logic, **(b)** native-only
integrations, and **(c)** the wasm export surface — and the wasm artifact is
built **twice** (sdk + webapp) from overlapping scripts that already note a
`pkg`/`pkg-web` collision risk.

## Module inventory — answering "what else?"

Your list + what the repo actually needs:

| Your list | Status | Target member |
|---|---|---|
| core | exists, **overloaded** | `crates/core` — *portable only* |
| wasm exporter | exists **inside** `core/src/wasm` | `crates/wasm` — its own crate |
| webapp | exists (top-level) | `packages/webapp` |
| MCP server | exists (Rust `mcp/`) | `crates/mcp` |
| browser extension | exists (`packages/extension`) | `packages/extension` |

**What else (missing from your list but present / needed):**

1. **`crates/native`** — pull `solana`, `arweave`, `storage`, `embed`,
   `encrypt`, `keychain` OUT of `core` into a native-only crate. *This is the
   single highest-value move* — it makes `core` truly portable and deletes the
   `cfg(not(wasm))` gymnastics. (Could split further into `chain` (solana+arweave),
   `storage`, `embed` if they grow.)
2. **`crates/prover`** — the zigz correspondence prover from
   `work/computation-proof/`. New member; needs a home in this layout.
3. **`packages/sdk`** — the TS SDK (wasm consumer). Already central; name it.
4. **`packages/cli`** — already exists.
5. **`packages/mcp`** — the npm distribution wrapper (≠ the Rust server).
6. **`conformance/`** — the cross-language golden vectors (COSE/CBOR/blake3),
   today scattered in `tests/` + per-package fixtures. One source of truth, the
   cross-lang CI gate consumes it.
7. **`deploy/`** — `Dockerfile`, `nginx.conf`, `docker-compose.yml`,
   `smithery.yaml`, `ollama/`, `Cross.toml`. Out of the root.
8. **`docs/`** — keep, but make it the only docs home.

## Target architecture

```
mnemonic/
├── Cargo.toml                 # Rust workspace
├── package.json               # JS/TS workspace
├── crates/                    # all Rust members grouped
│   ├── core/      mnemonic-core    # PORTABLE only: codec, compress, identity,
│   │                               #   merkle, lineage, trajectory. native+wasm
│   │                               #   clean. rlib only (no cdylib here).
│   ├── native/    mnemonic-native  # solana, arweave, storage, embed, encrypt,
│   │                               #   keychain. depends on core. native-only.
│   ├── wasm/      mnemonic-wasm    # the wasm-bindgen surface (was core/src/wasm).
│   │                               #   cdylib. depends on core. THE wasm artifact.
│   ├── prover/    mnemonic-prover  # zigz correspondence prover (work/computation-proof).
│   └── mcp/       mnemonic-mcp     # server binary. depends on core + native (+ prover).
├── packages/                  # all JS/TS members grouped
│   ├── sdk/       @mnemonik-xyz/sdk        # consumes the ONE wasm artifact
│   ├── cli/       @mnemonik-xyz/cli
│   ├── extension/ @mnemonik-xyz/extension
│   ├── mcp/       @mnemonik-xyz/mcp        # npm distribution wrapper
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
          │            │            │
         mcp          sdk          mcp
                    ┌──┴───┐
                  cli  extension  webapp
```

- `core` depends on **nothing** in the repo. Everything points at `core`.
- `wasm` depends only on `core` (portable) — so wasm can never accidentally pull
  `solana`/`keyring`. The `cfg(not(wasm))` gating in `core` **disappears**.
- One wasm artifact (`crates/wasm` → `pkg/`), consumed by sdk + webapp. The two
  `build-wasm.sh` scripts collapse into one.

## Design principles

1. **Boundaries follow dependencies, not topics.** A module that needs
   `solana-sdk` cannot live in a crate that compiles to wasm. Split on that line.
2. **`core` is the portable kernel** — rlib, minimal deps, native+wasm clean.
3. **One wasm exporter, one artifact, one build script.**
4. **One-way DAG**, audited (mirrors the existing `core → mcp` rule, generalized).
5. **No big bang.** Every phase keeps `cargo build --workspace`, clippy, fmt,
   tests, the JS workspace builds, AND the `cross-lang-build` gate green.
6. **`git mv` everything** — preserve history; update paths, never copy-delete.

## Phased migration (each phase ships green)

- **Phase 0 — inventory + dep audit.** Exact map: which `core` modules pull which
  native deps, every `use mnemonic_core::...` site, every CI path filter. Output:
  a move-list. No code moves. (De-risks every later phase.)
- **Phase 1 — group, don't split.** `git mv core mcp` under `crates/`; move JS
  members under `packages/` (webapp included). Update workspace `members`, JS
  `workspaces`, CI path filters, `Dockerfile`/compose paths. Pure relocation —
  smallest possible diff, proves the harness still builds.
- **Phase 2 — extract the wasm exporter.** `core/src/wasm` → `crates/wasm`
  (cdylib, depends on `core`). Unify the two `build-wasm.sh` into one that targets
  `crates/wasm`; point sdk + webapp at the single `pkg/`. `core` drops `cdylib`.
  **Riskiest for CI** (the cross-lang-build gate builds SDK WASM) — land alone.
- **Phase 3 — extract native integrations.** Move `arweave, embed, encrypt,
  solana, storage` (+ keychain) into `crates/native`; fix `mcp`/`cli` imports;
  remove the now-dead `cfg(not(wasm))` gates from `core`. After this `core` is
  portable by construction, not by gating. (`lineage`/`rebuild`: split their
  storage-bound parts into `native`, keep pure logic in `core`.)
- **Phase 4 — land `crates/prover`.** The zigz correspondence work plugs in as a
  member here (see `work/computation-proof/tech-spec.md`).
- **Phase 5 — consolidate non-code.** `conformance/` (merge `tests/` +
  per-package golden vectors), `deploy/` (infra artifacts), single `docs/`.
- **Phase 6 — docs + rules.** Update `CLAUDE.md`/`AGENTS.md` architecture section,
  the audit-enforced dependency rules, README; run the audit waves + QA gate.

## Risks & CI considerations

- **The `cross-lang-build (gate)` job is hard-required** (CLAUDE.md): it builds
  Rust binaries + SDK WASM + CLI dist. Phases 1–2 move exactly those paths — update
  the workflow in the SAME commit as each move, or the gate breaks.
- **`cross-lang-keychain (informational)`** stays `continue-on-error: true`; the
  keychain code moving to `crates/native` must not flip that toggle (yo-yo rule).
- **Conflict-point files** (`core/src/lib.rs`, `mcp/src/tools.rs`,
  `mcp/src/main.rs`) are touched in Phases 2–3 — do those phases on a clean branch,
  not parallel to feature work.
- **`mnemonic-core` npm name** (webapp dep) and `core/pkg-web` path are public-ish
  contracts for SDK consumers — Phase 2 must preserve the published artifact name
  or bump deliberately.
- **Cargo.lock / package-lock** churn is large but mechanical; regenerate per phase.

## Open questions for the owner

1. Group Rust under `crates/` and JS under `packages/` (proposed), or keep Rust
   members at root (`core/`, `mcp/`, …) and only group JS?
2. `crates/native` as one crate, or split into `chain` + `storage` + `embed`
   from the start?
3. Does `prover` (zigz) live here, or in the separate `mnemonik-dev/zigz` repo
   with this monorepo depending on it? (Ties to the earlier zigz-scope question.)
4. Move `webapp` under `packages/`, or leave it top-level (it's the one member
   that isn't a library)?
