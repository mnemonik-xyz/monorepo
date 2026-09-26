# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**Mnemonic Protocol** — verifiable, persistent memory for AI agents. A memory is embedded, TurboQuant-compressed, canonicalized to deterministic CBOR, blake3-hashed, COSE_Sign1-signed with an Ed25519 identity, and optionally anchored on Arweave (bytes) + Solana (SPL Memo). It is exposed over MCP (Model Context Protocol), a CLI, a TypeScript SDK, a browser extension and a webapp.

**Default branch:** `main`. Branch from `main` (`feat/*`, `fix/*`, `claude/*`) and PR back to `main`. Tagged releases (`v*`) are cut from `main`.

**Library docs:** use Context7 MCP automatically when you need API/setup info — don't ask the user first.

## Common commands

Rust (workspace root):

```bash
cargo build --workspace
cargo build -p mnemonic-mcp --release --features local-embed   # MCP binary with local embedder
cargo fmt --all -- --check                                     # format gate
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support   # full suite (~890 tests)
cargo test -p mnemonic-core <test_name>                        # one core test
cargo test -p mnemonic-mcp --features test-support --test public_read_routes   # one mcp integration file
cargo bench -p mnemonic-core                                   # cbor_codec, decompress, decompress_fidelity*
```

- `mcp/tests/*.rs` import `mnemonic_mcp::test_support`, which exists only with the `test-support` feature. Without it, clippy and tests fail with `unresolved import`.
- The `keyring` crate needs D-Bus headers. On a fresh Linux box: `apt-get install -y libdbus-1-dev pkg-config`.
- SDK byte-parity fixtures: `cargo test --features golden-fixtures -p mnemonic-core --test golden_fixtures emit_fixtures -- --ignored --nocapture`.

Run the MCP server locally:

```bash
STORAGE_MODE=local PAYMENT_MODE=none \
  cargo run -p mnemonic-mcp --bin mnemonic-mcp --release --features local-embed -- --transport http --port 3000
cargo run -p mnemonic-mcp --bin mnemonic-mcp --release --features local-embed -- mcp-stdio   # stdio for Claude Desktop / Cursor
cargo run -p mnemonic-mcp --bin mnemonic-mcp -- identity                                    # print server pubkey and exit
```

The server **requires** an embedder. `EMBED_PROVIDER=fastembed` (default) needs `--features local-embed` (downloads a ~22MB ONNX model on first run). Otherwise set `EMBED_PROVIDER=openai` + `OPENAI_API_KEY`. Without one, startup aborts. All env config is read in `mcp/src/config.rs`.

JS/TS (npm workspaces: `packages/*` + `webapp`):

```bash
npm install                              # at repo root
npm run build:wasm:sdk                   # core → WASM for the SDK (needs wasm-pack)
npm test -w @mnemonik-xyz/sdk            # vitest; same for @mnemonik-xyz/cli, @mnemonik-xyz/mcp
npm run build -w @mnemonik-xyz/sdk       # tsc + wasm + browser bundle
npm run dev -w mnemonic-webapp           # Vite dev server
```

The extension (`packages/extension`) tests run with `bun test` in CI.

## Architecture

Cargo workspace (`resolver = "2"`) with two members, plus an npm workspace:

- **`core/` (`mnemonic-core`)** — all domain logic. Modules in `core/src/lib.rs`:
  - Portable (also build for wasm32): `codec` (canonical CBOR, blake3 hash, COSE sign/verify, schema), `compress` (TurboQuant), `identity` (keypair, OS keychain / file / memory key stores, lazy creation), `merkle`.
  - Native only (`cfg(not(target_arch = "wasm32"))`): `arweave`, `embed`, `encrypt`, `lineage`, `rebuild`, `solana`, `storage`.
  - Feature-gated: `trajectory` (`trajectory-experimental`), `wasm` (wasm32 + `wasm` feature; wasm-bindgen wrappers used by the SDK, webapp and extension).
- **`mcp/` (`mnemonic-mcp`)** — library + binary. `main.rs` (clap, subcommands `mcp-stdio`, `logout`, `identity`), `mcp.rs` (JSON-RPC dispatch, tool list, `McpState`), `tools.rs` (tool handlers), `api.rs` (REST routes), `oauth/` (OAuth 2.1 + PKCE, Google sign-in, refresh tokens), `pending.rs` + `approval.rs` (deferred signing), `payment.rs` / `pricing.rs` (x402). Domain types come from `mnemonic_core::` — never re-declare `codec`, `storage` etc. inside `mcp/src/`.
- **`packages/`** — `sdk` (`@mnemonik-xyz/sdk`, TS client + WASM core), `cli` (`@mnemonik-xyz/cli`), `mcp` (`@mnemonik-xyz/mcp`, npm launcher that downloads the Rust binary per platform), `extension` (browser extension).
- **`webapp/`** — React + Vite site (mnemonik.xyz), prerendered, Playwright e2e.
- **`tests/cross-lang/`** — Rust ↔ Node keychain interop script.

**MCP tools.** Default builds expose 8 tools: `mnemonic_whoami`, `mnemonic_sign_memory`, `mnemonic_check_pending`, `mnemonic_recall`, `mnemonic_verify`, `mnemonic_prove_identity`, `mnemonic_publish_post`, `request_public_write_confirmation`. `--features trajectory-experimental` adds `mnemonic_attest_step`, `mnemonic_attest_verdict`, `mnemonic_verify_trajectory`. Reference: `docs/tools.md`.

**Signing is non-custodial.** Over HTTP, a JWT write returns `{status: "awaiting_signature", correlation_id, approve_url, ...}`. The client signs the canonical bundle locally and posts it to `/api/sign-callback`. `mnemonic_check_pending` resolves the final state. Pending bundles expire after 300 s. The operator key signs inline only when the writer *is* the operator (stdio / single-tenant). `tools.rs` refuses to operator-sign a memory owned by another identity — keep it that way.

**Data flow (`sign_memory`):** text → `embed::Embedder` → `compress` (TurboQuant, `TURBO_BITS`, default 4) → `codec::canonical` (CBOR) → blake3 → `codec::sign` (COSE_Sign1, Ed25519) → for `participate`: Arweave upload + Solana SPL Memo (JSON with `h` = content hash) → `storage::sqlite` (`AttestationStore`). `recall` searches uncompressed f32 embeddings in SQLite.

**Write modes.** `STORAGE_MODE` (default `local`) sets the operator's capability/default, not a global switch. Each `mnemonic_sign_memory` request may set `mode: "local" | "participate"`; requests without it fall back to the env var (legacy clients). Rows carry a `write_mode` column, and recall spans both modes for one owner. A `participate` write succeeds only after the anchored bytes pass a recall + verify round-trip; on failure the row is demoted to `local` and nothing is charged. Rationale: `work/modes-user-choice/`.

**Payment** (`PAYMENT_MODE`, HTTP only): `none` | `x402`. The custodial `balance`/`both` modes were removed; `check_payment` fail-closes on any other value. Only `mnemonic_sign_memory` in `participate` mode is paid.

**Hard architectural rules** (audit-enforced):

1. Payment logic (`check_payment`, `verify_usdc_transfer`, x402 nonce handling, `record_attestation_cost`, `get_pnl_stats`) lives only in `mcp/src/payment.rs`. `pricing.rs` lives in `mcp/`. Nothing payment-related in `core/`.
2. `verify_usdc_transfer` is a standalone function taking `&SolanaClient`, not a method on it.
3. No `HashEmbedder` anywhere — use `MockEmbedder` in `#[cfg(test)]` blocks.
4. `core/` never depends on `mcp/`. The dependency graph is one-way.
5. Business logic calls the `Embedder` trait (`core/src/embed/mod.rs`), never a concrete provider.

**Storage lock discipline:** `rusqlite::Connection` is `!Send`. `McpState.store` is a `std::sync::Mutex<SqliteStore>`; **never** hold the lock across `.await`.

## Spec-driven workflow

Features and bugs live in `work/<feature>/`:

- `user-spec.md` — Russian, what and why
- `tech-spec.md` — English, how (architecture, decisions, testing, tasks)
- `tasks/<n>.md` — atomic units with `status`, `depends_on`, `wave`, `skills`, `reviewers` frontmatter
- `decisions.md` — append-only log of decisions and audit findings

Tasks come in waves. Tasks in one wave may run in parallel if they don't touch shared files (`core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/mcp.rs`, `mcp/src/main.rs` are common conflict points). Audit waves are read-only and write to `decisions.md`.

## Project knowledge skill

Deeper docs live in `.claude/skills/project-knowledge/references/` (read via the `project-knowledge` skill): `architecture.md`, `patterns.md`, `deployment.md`, `project.md`, `ux-guidelines.md`, `crypto-design.md`, `threat-model.md`, `economics.md`, `protocol-integrations.md`.

Note: root `AGENTS.md` is a **public** page for external agents that want to use the hosted service (it pairs with `/.well-known/agent.json`). It is not a contributor guide — keep it accurate to the shipped service.

## Conventions

- **Documentation language: ASD-STE100 (Simplified Technical English)** for all new and edited English docs (README, `docs/`, specs, PR descriptions). Rules: max 20 words per instruction sentence and 25 per descriptive sentence; active voice; one instruction per sentence; simple words with one meaning; define every abbreviation at first use. Do not add a note about the standard inside the documents. Mark each capability as "available now" or "planned"; never describe unimplemented features as existing.
- **Papers:** `docs/WHITEPAPER.md` is the short overview for readers. `docs/YELLOWPAPER.md` is the detailed technical specification. Keep the whitepaper consistent with the code.
- Conventional Commits with component scope: `feat(core):`, `fix(mcp):`, `docs:`, `chore:`, `style:`.
- `anyhow::Result` for fallible functions; convert to `JsValue` only at the WASM boundary (`core/src/wasm`). No `unwrap()` outside tests.
- All SQL in `core/src/storage/sqlite.rs` uses `rusqlite` parameterized queries.
- `Direction` is an enum; `chain_valid` is `Option<bool>` (None = unverified, Some(false) = broken, Some(true) = verified). DB errors propagate via `?`.
- TurboQuant bit width: default 4. **Never change for an existing database** — old and new embeddings become incomparable for recall.
- Compressed bytes on Arweave are **proof of existence only**. Recall uses uncompressed f32 embeddings in SQLite.

## CI

`.github/workflows/ci.yml` runs **nightly (02:00 UTC) and on manual dispatch only** — PR and push triggers are off since 2026-09-23 (owner decision: PRs merge without waiting on CI). Same for `node-test.yml` (02:30), `docs-link-check.yml` (03:00) and `ext-e2e.yml` (03:30). Jobs: rustfmt check, clippy with `-D warnings`, `cargo test --workspace --features mnemonic-mcp/test-support`, gitleaks (working tree + full history). `nightly.yml` (04:00) builds the MCP binary and smoke-tests it with MCP Inspector. Because nothing gates a PR, run the fast local checks (fmt, clippy, changed-crate tests) before pushing, and check the nightly result the next morning. `.github/workflows/release.yml` runs on `v*` tags: builds the mcp binary (Linux, macOS arm64), creates the GitHub Release, and publishes the npm packages with provenance. `build-mcp-image.yml` pushes the Docker image to `ghcr.io/<owner>/mnemonic-mcp` on pushes to `main` that touch `core/` or `mcp/`, and on tags. Toolchain pinned via `rust-toolchain.toml`.

### CI gate policy

The cross-language interop coverage is intentionally split into two jobs with different gate semantics — do not collapse them or flip the toggles without following the procedure below.

- **`cross-lang-build (gate)`** — hard gate *of the nightly run* (PR gating is off, see above). Builds the Rust binaries + SDK WASM + CLI dist that the keychain interop test would need. Deterministic. Never carries `continue-on-error`. If this is red, real build infra is broken (e.g. wasm-pack missing, libdbus header gone) and the PR must block.
- **`cross-lang-keychain (informational)`** — `needs: cross-lang-build`, permanently `continue-on-error: true`. Drives the actual Rust ↔ Node keychain roundtrip under a CI-spawned `gnome-keyring` + D-Bus session. Sub-test B has an unresolved daemon-coupling issue on Ubuntu 24.04 (see commit `fde7f72` — survived 5 rounds of debugging). The job stays in the matrix as a visible signal but does not gate.

**Yo-yo prevention rule:** the `continue-on-error: true` on `cross-lang-keychain` is permanent until the daemon-coupling sub-test B is fixed upstream. Do not flip it on/off — that pattern previously masked a `wasm-pack`-missing build regression that shipped to main untouched during PR #151. If you believe the test is now stable enough to gate, the procedure is:

1. Reproduce 10 consecutive green runs of the script on Ubuntu 24.04.
2. Land a single PR that simultaneously removes `continue-on-error: true` AND updates this section, naming the fix commit that closed the daemon-coupling root cause.
3. Get a reviewer sign-off on that PR explicitly acknowledging the gate change.

If you only want to gate the build path (the actual regression-catcher), the `cross-lang-build` job already does that — no toggling needed.

## Stream Timeout Prevention

1. Do each numbered task ONE AT A TIME. Complete one task fully, confirm it worked, then move to the next.
2. Never write a file longer than ~150 lines in a single tool call. If a file will be longer, write it in multiple append/edit passes.
3. Start a fresh session if the conversation gets long (20+ tool calls). The error gets worse as the session grows.
4. Keep individual grep/search outputs short. Use flags like `--include` and `-l` (list files only) to limit output size.
5. If you do hit the timeout, retry the same step in a shorter form. Don't repeat the entire task from scratch.
