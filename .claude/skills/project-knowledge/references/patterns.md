# Patterns & Conventions

Coding conventions, development workflow, and project-specific practices.
For universal coding standards, see `~/.claude/skills/code-writing/references/universal-patterns.md`.

---

## Project-Specific Code Patterns

### Git submodules

See architecture.md for submodule structure and directory layout. Each submodule has its own `Cargo.toml` and is released independently. Commits that touch multiple submodules require a separate commit per submodule, plus a root-repo commit updating the submodule pointers.

### Dual-target compilation

Feature-gate anything requiring network or OS I/O behind `#[cfg(not(target_arch = "wasm32"))]`. WASM builds must not depend on `tokio`, `std::fs`, or `std::net`. Use `wasm-bindgen-futures` for async in WASM context.

### Embedder trait

All providers implement the `Embedder` trait in `core/src/embed/mod.rs`. Never call a concrete provider directly from business logic — always go through the trait. This keeps the provider fallback chain and the test-only `MockEmbedder` transparent to callers.

### Storage lock discipline

`AttestationStore` wraps `rusqlite::Connection` which is `!Send`. In async contexts, wrap in `std::sync::Mutex` and never hold the lock across an `.await` point.

### Error handling

Use `anyhow::Result` for all fallible functions in `core/` and `mcp/`. Convert to `JsValue` only at the WASM boundary in `core/src/wasm/`. Avoid `unwrap()` outside tests.

### Storage modes

`local` (default) uses SQLite only with synthetic IDs prefixed `local:` — free, instant, offline. `full` uses Arweave + Solana + SQLite and requires a funded keypair. The mode is set at server startup, not per-call. Never mix modes in one database.

---

## Git Workflow

### Branch Structure

- **`main`** — production, tagged releases. Protected. Merge from `dev` after full CI passes.
- **`dev`** — active development. All feature branches merge here.
- **`feat/*`** — branch from `dev`, PR back to `dev`.

### Commit Convention

Conventional Commits with component scope: `feat(core):`, `fix(mcp):`, `feat(webapp):`, `docs:`, `chore:`.

### Testing Requirements

On every commit: run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`. On merge to `dev`: add `wasm-pack test --headless --chrome` for the WASM target. On merge to `main`: full CI including WASM tests.

### Security & Quality Gates

Pre-commit: Gitleaks scans for secrets — API keys, private keys, tokens. Commit is blocked if any are detected. Pre-push: `cargo clippy --workspace` must pass with zero warnings.

---

## Testing & Verification

`cargo test --workspace` runs all unit and integration tests from the repo root. WASM tests run with `wasm-pack test --headless --chrome` inside `core/`. Benchmarks run with `cargo bench -p mnemonic-core`. Full-mode integration tests require arlocal on `:1984` and `solana-test-validator` on `:8899`; set `STORAGE_MODE=local` to skip blockchain in all other tests. See each submodule's README for full invocation details and smoke test procedures.

---

## Business Rules

### TurboQuant bit width

Default is 4 bits (best recall quality). Do not change the bit width for an existing database — old and new embeddings become incomparable for recall ranking. Changing this setting effectively starts a new memory store.

### Compression is lossy

The compressed bytes stored on Arweave are not used for recall. The uncompressed float32 embeddings in SQLite are used for cosine search. The compressed bytes prove the embedding existed at attestation time and enable future cross-node comparison.
