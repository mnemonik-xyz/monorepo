# Patterns & Conventions

Coding conventions, development workflow, and project-specific practices.
For universal coding standards, see `~/.claude/skills/code-writing/references/universal-patterns.md`.

---

## Project-Specific Code Patterns

### Git submodules

`core/`, `mcp/`, and `webapp/` are independent git repositories registered as submodules. Clone with `git clone --recurse-submodules`. Update all submodules with `git submodule update --remote`. Each submodule has its own `Cargo.toml` and is released independently. Run tests in each submodule directory separately: `cd core && cargo test`, `cd mcp && cargo test`.

### Dual-target compilation

Feature-gate anything requiring network or OS I/O behind `#[cfg(not(target_arch = "wasm32"))]`. WASM builds must not depend on `tokio`, `std::fs`, or `std::net`. Use `wasm-bindgen-futures` for async in WASM context. WASM-specific exports live in `core/src/wasm/mod.rs`.

### Embedder trait

All providers implement the `Embedder` trait in `core/src/embed/mod.rs`. Never call a concrete provider directly from business logic — always go through the trait. This keeps the hash fallback and WASM-safe stub transparent to callers.

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

### Test Infrastructure

Tests run with `cargo test --workspace` from repo root. WASM-specific tests run with `wasm-pack test --headless --chrome` inside `core/`. Benchmarks run with `cargo bench -p mnemonic-core`.

For full-mode integration tests that need blockchain: start arlocal on port 1984 (`npx arlocal`) and `solana-test-validator` on port 8899. Set `STORAGE_MODE=local` to skip blockchain calls in all other tests.

### Agent Verification Methods

Attestation round-trip: call `sign_memory`, capture `solana_tx` and `arweave_tx`, call `verify`, assert `status == "verified"`.

Recall: save five items with known content, call `recall` with a related query, assert the top result matches the expected item.

WASM smoke test: build with `wasm-pack build --target web` inside `core/`, import in a minimal HTML page, call `whoami()`, assert the response contains a valid `public_key`.

MCP smoke test: start the `mcp/` server in local mode, send a `tools/list` JSON-RPC request, assert five tools are returned, call `mnemonic_whoami`, assert a valid response.

---

## Business Rules

### TurboQuant bit width

Default is 4 bits (best recall quality). Do not change the bit width for an existing database — old and new embeddings become incomparable for recall ranking. Changing this setting effectively starts a new memory store.

### Compression is lossy

The compressed bytes stored on Arweave are not used for recall. The uncompressed float32 embeddings in SQLite are used for cosine search. The compressed bytes prove the embedding existed at attestation time and enable future cross-node comparison.
