# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**Mnemonic Protocol** — verifiable, persistent memory for AI agents. Memories are semantically embedded, TurboQuant-compressed, canonicalized to deterministic CBOR, blake3-hashed, COSE_Sign1-signed with an Ed25519 identity, and optionally anchored on Arweave (durable storage) + Solana (SPL Memo timestamp). Exposed over MCP.

**Default branch:** `main`. Feature branches are created from `main` (`feat/*`, `claude/*`, etc.) and PR'd back to `main`. Tagged releases (`v*`) are cut from `main`.

**Library docs:** use Context7 MCP automatically when you need API/setup info — don't ask the user first.

## Common commands

```bash
cargo build --workspace                            # build everything
cargo build -p mnemonic-mcp --release              # release MCP binary
cargo test --workspace --no-fail-fast              # full test suite (90+ tests)
cargo test -p mnemonic-core <test_name>            # single test by name
cargo test -p mnemonic-core --test integration_cbor   # one integration test file
cargo clippy --workspace --all-targets -- -D warnings  # lint gate (CI enforces)
cargo fmt --all -- --check                         # format gate (CI enforces)
cargo bench -p mnemonic-core                       # benchmarks (decompress, cbor_codec)
```

Run the MCP server locally:

```bash
STORAGE_MODE=local PAYMENT_MODE=none \
  cargo run -p mnemonic-mcp --release -- --transport http --port 3000

# or stdio for Cursor / Claude Desktop
cargo run -p mnemonic-mcp --release -- --transport stdio
```

The server **requires** an embedder. By default `EMBED_PROVIDER=fastembed`, which needs `--features local-embed` (downloads a ~22MB ONNX model on first run). Otherwise set `EMBED_PROVIDER=openai` + `OPENAI_API_KEY`. Without one, startup aborts.

## Architecture

Cargo workspace, `resolver = "2"`, two members:

- **`core/` (`mnemonic-core` library)** — all domain logic. Eight public modules, each `pub mod` from `core/src/lib.rs`: `codec`, `embed`, `compress`, `identity`, `storage`, `arweave`, `solana`, `lineage`. Native-only — no WASM, no axum, no clap.
- **`mcp/` (`mnemonic-mcp` binary)** — thin server: `main.rs`, `mcp.rs` (JSON-RPC dispatch), `tools.rs` (5 MCP tools), `payment.rs`, `pricing.rs`, `config.rs`. **All domain types come from `mnemonic_core::`** — no `mod codec;` etc. in `mcp/src/`.

**Hard architectural rules** (audit-enforced):

1. Payment methods (`create_api_key`, `deduct_balance`, `credit_deposit`, `mark_x402_nonce`, `record_attestation_cost`, `get_pnl_stats`, `get_owner_pubkey`, `verify_usdc_transfer`) live **only** in `mcp/src/payment.rs`. None in `core/`.
2. `verify_usdc_transfer` is a **standalone function** taking `&SolanaClient`, not a method on it.
3. `pricing.rs` lives in `mcp/`, never in `core/`.
4. No `HashEmbedder` anywhere — use `MockEmbedder` in `#[cfg(test)]` blocks for tests.
5. `core/` has zero references to anything in `mcp/`. The dependency graph is one-way.

**Data flow (`sign_memory`):** text → `embed::Embedder` → `compress::EmbeddingCompressor` (TurboQuant 2/3/4 bits) → `codec::canonical::to_canonical_cbor` → `blake3` hash → `codec::sign` (COSE_Sign1 with Ed25519) → in `full` mode: Arweave bytes + Solana SPL Memo → `storage::AttestationStore` (SQLite). `recall` reads SQLite + cosine search over decompressed embeddings.

**Storage modes** (`STORAGE_MODE`): `local` (default) — SQLite only, synthetic `local:` tx IDs, free, offline. `full` — Arweave + Solana writes, requires funded Ed25519 keypair. Mode is set at startup, not per-call. Never mix in one DB.

**Payment modes** (`PAYMENT_MODE`, HTTP + `full` only): `none` | `balance` (Bearer token, balance checked against live pricing engine) | `x402` (HTTP 402 + retry with `X-Payment` header) | `both`. Only `mnemonic_sign_memory` is paid.

**Storage lock discipline:** `rusqlite::Connection` is `!Send`. Wrap in `std::sync::Mutex` in async contexts; **never** hold the lock across `.await`.

**Embedder trait:** all providers (`OpenAIEmbedder`, `FastEmbedder` behind `local-embed`, `MockEmbedder` test-only) implement `Embedder` in `core/src/embed/mod.rs`. Never call a concrete provider directly from business logic.

## Spec-driven workflow

Features and bugs live in `work/<feature>/`. Each feature has:

- `user-spec.md` — Russian, what and why
- `tech-spec.md` — English, how (architecture, decisions, testing, tasks)
- `tasks/<n>.md` — atomic units with `status`, `depends_on`, `wave`, `skills`, `reviewers` frontmatter
- `decisions.md` — append-only log of decisions and audit findings

Tasks come in waves; within a wave, tasks may run in parallel if they don't touch shared files (`core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/main.rs` are common conflict points). Audit waves (code/security/test) are read-only and write to `decisions.md`. Pre-deploy QA gates the merge.

## Project knowledge skill

Deeper docs live in `.claude/skills/project-knowledge/references/`:

- `architecture.md` — full module map, dependencies, data model
- `patterns.md` — coding conventions, git workflow, security gates
- `deployment.md` — CI/CD, secrets, environments, rollback
- `project.md` — purpose, audience, MVP scope
- `ux-guidelines.md` — webapp tone, design system

Read these via the `project-knowledge` skill — they're the source of truth.

## Conventions

- Conventional Commits with component scope: `feat(core):`, `fix(mcp):`, `docs:`, `chore:`, `style:`.
- `anyhow::Result` for fallible functions; convert to `JsValue` only at the WASM boundary (none yet — iteration 2). No `unwrap()` outside tests.
- All SQL in `core/src/storage/sqlite.rs` uses `rusqlite` parameterized queries.
- `Direction` is an enum; `chain_valid` is `Option<bool>` (None = unverified, Some(false) = broken, Some(true) = verified). DB errors propagate via `?`.
- TurboQuant bit width: default 4. **Never change for an existing database** — old and new embeddings become incomparable for recall.
- Compressed bytes on Arweave are **proof of existence only**. Recall uses uncompressed f32 embeddings in SQLite.

## CI

`.github/workflows/ci.yml` runs on push to `main` and every PR: rustfmt check, clippy with `-D warnings`, `cargo test --workspace`, gitleaks (working tree + full history). `.github/workflows/release.yml` runs on `v*` tags: cross-compile mcp binary, build Docker image, publish to GHCR/crates.io. Toolchain pinned via `rust-toolchain.toml`.

## Stream Timeout Prevention

1. Do each numbered task ONE AT A TIME. Complete one task fully, confirm it worked, then move to the next.
2. Never write a file longer than ~150 lines in a single tool call. If a file will be longer, write it in multiple append/edit passes.
3. Start a fresh session if the conversation gets long (20+ tool calls). The error gets worse as the session grows.
4. Keep individual grep/search outputs short. Use flags like `--include` and `-l` (list files only) to limit output size.
5. If you do hit the timeout, retry the same step in a shorter form. Don't repeat the entire task from scratch.
