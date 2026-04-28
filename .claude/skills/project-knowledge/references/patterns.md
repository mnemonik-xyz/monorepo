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

Use `anyhow::Result` for all fallible functions in `core/` and `mcp/`. Convert to `JsValue` only at the WASM boundary (the wasm-bindgen bridge lives in `webapp/src/wasm/`). Avoid `unwrap()` outside tests.

### Storage modes

`local` (default) uses SQLite only with synthetic IDs prefixed `local:` — free, instant, offline. `full` uses Arweave + Solana + SQLite and requires a funded keypair. The mode is set at server startup, not per-call. Never mix modes in one database.

### Tenant isolation via `owner_pubkey`

Every attestation row in hosted mode carries an `owner_pubkey` derived from the OAuth Bearer token's `sub` claim, set at write time by `tools.rs::sign_memory` and filtered at read time by `recall`/`verify`. Legacy rows with NULL `owner_pubkey` cannot match any caller — there is no escape hatch. The OAuth user pubkey is the only identity that crosses the trust boundary; never look up rows by `signer_pubkey` for tenant scoping (that's the server's signing key, not the user's).

### Browser-mediated signing for hosted-mode `sign_memory`

In hosted deployments the server never sees the user's private key. `sign_memory` returns a `pending` envelope containing a `correlation_id` and a redirect URL; the webapp `Sign.tsx` page completes the COSE_Sign1 envelope in-browser via the WASM `sign_cose_payload` export, then POSTs the signed bytes to `/api/sign-callback`. The capability is the unguessable UUID — `/api/pending/{id}` and `/api/sign-callback` deliberately do **not** require a Bearer JWT, because the calling browser tab may be a different OAuth tenant from the MCP client (e.g. Cursor authorized one identity, the user's webapp tab is logged into another). Tenant binding is enforced inside the callback handler against `entry.jwt_sub` recorded when the bundle was queued.

### COSE vs raw Ed25519 signing surfaces

Two distinct browser signing entry points: `sign_cose_payload(server_cbor_bytes, keypair)` wraps server-provided canonical-CBOR bytes verbatim in a COSE_Sign1 envelope (used by `Sign.tsx` for `mnemonic_sign_memory` finalization). `sign_challenge(challenge_bytes, keypair)` produces a raw Ed25519 signature over the OAuth PKCE-bound challenge (used by `Consent.tsx` for `/oauth/authorize`). Don't confuse the two — the OAuth flow expects raw Ed25519 over the challenge blob, not a COSE wrapper.

### CORS allowlist via predicate, not literal list

`mcp/src/cors_policy.rs` exposes `allowed_origin` as a `tower-http` predicate that allows the literal first-party origins (`https://mnemonik.xyz`, `https://mcp.mnemonik.xyz`), suffix-matches the AI-tool family (`*.claude.ai`, `*.cursor.sh`, `*.chatgpt.com`, `*.anthropic.com`, `*.openai.com`), and accepts `http://localhost:*` / `http://127.0.0.1:*` only over HTTP for dev. Add new clients here, not as `allow_any_origin`.

### JSON-RPC notifications return 202

Per MCP spec 2025-06-18 §2.4, JSON-RPC requests with no `id` field are notifications — the server must accept and dispatch them but must not return a response body. `mcp.rs::JsonRpcRequest.id` is `Option<Value>`; when `None`, `mcp_handler` short-circuits to `StatusCode::ACCEPTED` (202) with an empty body. The MCP transport response Content-Type is plain `application/json` (not `application/x-ndjson`) — most clients reject NDJSON.

### OAuth Bearer-auth allowlist

`mcp/src/oauth.rs::bearer_auth_layer` enforces JWT validation on `/mcp` and `/` POSTs *except* for: discovery (`/.well-known/*`), liveness (`/health`), the OAuth flow itself (`/oauth/*`), the capability-authed pending APIs (`/api/pending/*`, `/api/sign-callback`), and JSON-RPC methods `initialize`, `tools/list`, plus any `notifications/*`. Any new tool added to `tools.rs` is paid + authenticated by default; explicit allowlist edits are required to expose anonymous methods.

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
