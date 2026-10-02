# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**Mnemonic Protocol** — signed memory artifacts and recovery tools for AI agents. Client-prepared sealed memory is encrypted and signed locally; external delivery retains original bytes. Embeddings and TurboQuant support particular search/artifact paths, not every memory. Plain stdio-local rows are unsigned. Arweave/Irys provides current external delivery; Solana memos remain a legacy verification/discovery source. Source capabilities, supported artifact/backend combinations and release limits are listed in [docs/source-capabilities.md](docs/source-capabilities.md).

**Default branch:** `main`. Branch from `main` (`feat/*`, `fix/*`, `claude/*`) and PR back to `main`. Tagged releases (`v*`) are cut from `main`.

**Library docs:** use Context7 MCP automatically when you need API/setup info — don't ask the user first.

## Common commands

Rust (workspace root):

```bash
cargo build --workspace
cargo build -p mnemonic-mcp --release --features local-embed   # MCP binary with local embedder
cargo fmt --all -- --check                                     # format gate
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support   # full workspace suite
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

Cargo workspace (`resolver = "2"`) with four members, plus an npm workspace:

- **`core/` (`mnemonic-core`)** — all domain logic. Modules in `core/src/lib.rs`:
  - Portable (also build for wasm32): `codec` (canonical CBOR, blake3 hash, COSE sign/verify, schema), `compress` (TurboQuant), `identity` (keypair, OS keychain / file / memory key stores, lazy creation), `merkle`, `rebuild` (reconstruct a recall row from signed artifact bytes, so a client can restore its index from Arweave).
  - Native only (`cfg(not(target_arch = "wasm32"))`): `arweave`, `embed`, `encrypt`, `lineage`, `restore` (rebuild the index from Arweave), `solana`, `storage`.
  - Feature-gated: `trajectory` (`trajectory-experimental`), `wasm` (wasm32 + `wasm` feature; wasm-bindgen wrappers used by the SDK, webapp and extension).
- **`mcp/` (`mnemonic-mcp`)** — library + binary. `main.rs` (clap, subcommands `mcp-stdio`, `logout`, `identity`, `restore`, `export`), `mcp.rs` (JSON-RPC dispatch, tool list, `McpState`), `tools.rs` (tool handlers), `api.rs` (REST routes), `oauth/` (OAuth 2.1 + PKCE, Google sign-in, refresh tokens), `pending.rs` + `approval.rs` (deferred signing), `payment.rs` / `pricing.rs` (x402). Domain types come from `mnemonic_core::` — never re-declare `codec`, `storage` etc. inside `mcp/src/`.
- **`mnemonic-a2a/` (`mnemonic-a2a`)** — A2A models and attestation integration.
- **`bridge-a2a/` (`bridge-a2a`)** — A2A JSON-RPC sidecar bridge using `mnemonic-a2a`.
- **`packages/`** — `sdk` (`@mnemonik-xyz/sdk`, TS client + WASM core), `cli` (`@mnemonik-xyz/cli`), `mcp` (`@mnemonik-xyz/mcp`, npm launcher that downloads the Rust binary per platform), `extension` (browser extension).
- **`webapp/`** — React + Vite site (mnemonik.xyz), prerendered, Playwright e2e.
- **`tests/cross-lang/`** — Rust ↔ Node keychain interop script.

**MCP tools.** Default builds expose 11 tools: `mnemonic_whoami`, `mnemonic_sign_memory`, `mnemonic_check_pending`, `mnemonic_recall`, `mnemonic_verify`, `mnemonic_prove_identity`, `mnemonic_publish_post`, `request_public_write_confirmation`, `mnemonic_share`, `mnemonic_attest_a2a`, `mnemonic_recall_a2a`. `--features trajectory-experimental` adds `mnemonic_attest_step`, `mnemonic_attest_verdict`, `mnemonic_verify_trajectory`. Reference: `docs/tools.md`.

**Signing and privacy boundaries.** Legacy HTTP `mnemonic_sign_memory` sends
plaintext to the operator for artifact preparation, then uses client COSE signing
through `/api/sign-callback`. This is not client-private preparation. Explicit
HTTP `mode: "local"` is rejected. Omitted legacy modes follow server configuration.
Plain stdio local writes are unsigned; client-prepared sealed local artifacts are
signed and require the identity key. Only the owner/operator's stdio identity
may sign a memory inline; preserve that guard.

**Client-prepared delivery.** SDK `sealMemory` encrypts and signs locally.
`mode: "store"` means a session-only client cache, requiring caller-owned durable
backup. `mode: "anchor"` submits complete COSE bytes to `/api/ingest-artifact`.
Public MEMORY_V1 preparation is also local and publication requires explicit
consent. Memory and HTTP A2A adapters validate signatures, identity and parents
before shared quota/payment/delivery coordination. Original bytes are fetched
and compared exactly; receipt persistence is a separate diagnostic. New hosted
receipts contain metadata, not memory/grant/vector payloads. Legacy retained
staging is migration data and must not be mistaken for the new storage boundary.

**Recall.** Hosted semantic recall searches available hosted index rows;
metadata-only anchored receipts have no vector and do not become searchable
plaintext. HTTP recall never decrypts sealed payloads. Hosted recall-key sessions,
new general-memory grant storage and hosted `/api/store-sealed` writes are retired.
SDK sealed recall opens the local cache and requires an explicit local embedder.
Client recovery preserves original signatures and reports source failures; a
successful index scan alone does not prove complete history. See
[client-prepared memory](docs/client-prepared-memory.md) and
[recovery checkpoints](docs/recovery-checkpoints.md).

**Backing up.** `mnemonic-mcp export` emits this identity's existing index rows as JSON Lines, oldest first. It is owner-scoped and is not a complete artifact/key backup. New prepared-artifact receipts do not create plaintext index rows. Retain original signed envelopes and use the client checkpoint/encrypted-backup APIs for independent recovery. Legacy public or server-prepared index content can still exist.

**Restoring the local index.** `mnemonic-mcp restore` merges candidate discovery from configured provider-specific GraphQL and historical Solana memos. Both are discovery sources, not authorities for authorship or completeness. `enumerate_anchored` reports each source's exhaustion, failure, partial results or budget limit. Successful candidates survive another source's failure; the command reports incomplete discovery after retaining verified imports. `fetch_restorable` verifies envelopes, supports `memory.v1`, checks signed producer identity and reports sealed items as requiring client-local recovery. `apply_restore` imports only this identity's records after network work completes; `--dry-run` skips writes. Never hold the `!Send` store mutex across `.await`.

**Private and graph recovery.** `rebuild::recover_memory` and `restore::fetch_recovered_memories` accept a pinned author and optional X25519 key in the client process. They recover supported public/sealed content without relying on operator SQL. `recover_memory_set` requires author and exact-envelope digest pins for every head and ancestor, because general-memory IDs alone are not content-addressed. A2A has separate signed parent-hash/context/recipient checks. A successful scan cannot prove a newest head or global completeness; authenticated checkpoints define only expected ancestry. See [recovery checkpoints](docs/recovery-checkpoints.md) and [storage portability](docs/storage-portability.md).

**Mode names.** The two modes say *where the memory lives*: `local` = the agent's own machine only, `anchored` = Arweave. `anchored` was called `participate` before 2026-09-27. The old token is still accepted on input (`WriteMode::from_str_strict`, a serde alias, and a `write_mode` column normalising migration) but is never emitted. Remove the alias once the release that introduced `anchored` is the oldest supported client. See `work/arweave-as-source-of-truth/`.

**Anchored artifacts are self-describing.** An `anchored` write adds four OPTIONAL signed fields so the Arweave copy alone can rebuild a recall row: top-level `visibility` and `anchor`, plus `metadata.turbo_seed` and — for public memories only — `metadata.embedding_f32` (the exact vector). A plain legacy `sign_memory` local write adds none of them, because those rows carry no signature and are verified by reconstruction from columns (`rebuild_content_hash`), whose field list is fixed. `embedding_f32` is withheld from private memories on purpose: an embedding inverts to an approximation of its source text, and Arweave is permanent and public. Never make these fields required and never reorder `MEMORY_V1.cbor_field_order` — either moves every existing `content_hash`.

**`local` is refused over HTTP.** An explicit `mode: "local"` on the HTTP transport returns `-32010 UnsupportedMode` with `supported: ["anchored"]`. `local` means the memory stays on the agent's own machine, and a hosted deploy cannot provide that — a "local" write there would be a row in the operator's database, which is the custodial tier the binary mode model retires (`work/binary-mode-cleanup/`). Install the server locally for free local storage, or use `anchored`. A request that omits `mode` still resolves from the operator's configuration, so clients that never learned the field are unaffected. On stdio, `local` works as before: there the operator key *is* the agent's own identity.

**Write modes.** `STORAGE_MODE` (default `local`) sets the operator's capability/default, not a global switch. Each `mnemonic_sign_memory` request may set `mode: "local" | "anchored"`; requests without it fall back to the env var (legacy clients). Rows carry a `write_mode` column, and recall spans both modes for one owner. Delivery succeeds after exact-byte external fetch and signature/author verification; SQL receipt persistence is separate. Payment settlement can precede delivery. Retry and remedy states are distinct; never promise no charge on every delivery failure. Rationale: `work/modes-user-choice/`.

**Sealed write mode** is orthogonal to local/external storage. The identity key
is needed for encryption/signing/opening, including sealed local writes. Plain
legacy local writes need no keychain access. The agent may store its own sealed
bytes locally; the current hosted ingestion path retains metadata only.

**Payment** (`PAYMENT_MODE`, HTTP only): `none` | `x402`. The removed `balance`/`both` modes fail closed. Shared prepared-memory/A2A ingestion supports the Universal Paywall exact rail; other rails fail closed there. Legacy deferred callbacks retain separate interfaces. Read-only verification and A2A recall do not invoke payment.

**Free quota.** On paid HTTP deployments, a Google-linked signer may qualify under account, real-client-IP and global UTC-day limits. Source defaults are `MNEMONIC_FREE_ANCHORS_PER_DAY=10`, `MNEMONIC_FREE_ANCHORS_PER_IP_PER_DAY=20`, `MNEMONIC_FREE_ANCHORS_GLOBAL_PER_DAY=1000`, and `MNEMONIC_FREE_ANCHOR_MAX_BYTES=16384`; these are configurable defaults, not a service entitlement. `PAYMENT_MODE=none` and stdio do not consume those counters. The legacy pre-parking gate only peeks. Shared ingestion validates the artifact first, claims eligible quota against the durable operation, and avoids another allocation for revalidated existing exact bytes. Cancellation or failure refunds a reservation only after durable payment eligibility is safely reset; otherwise the reservation stays consumed. The global budget remains spent after an external write attempt. Quota refunds are not financial refunds.

**Financial state and retention.** Payment acceptance and delivery are independent. Retry uses the same original bytes, operation and validated provider receipt. Lost financial metadata requires reconciliation. Terminal failed delivery records `remedy_pending`; refunds or credits require an operator action and evidence. New delivery/callback staging records contain metadata only. Historical paid payload staging remains an explicit upgrade exception until identical client resubmission drains it; backups and journals have separate retention. Never delete the only undelivered historical copy merely to claim a payload-free database. See [payment retries](docs/artifact-payment-retries.md).

**Hard architectural rules** (audit-enforced):

1. Rail and price policy live in `mcp/src/payment.rs` and `pricing.rs`. `ingestion.rs`, `delivery_operation.rs` and `paid_operation.rs` coordinate immutable operations and retries. Nothing payment-related belongs in `core/`.
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

Several `work/` folders are steps of one larger effort, and their order matters more than it looks — `work/DECOUPLING-SEQUENCE.md` names that order and why each step blocks the next. Read it before starting any of `chain-agnostic`, `pluggable-anchoring`, `pluggable-storage`, `dual-key-identity` or `multi-suite-signing`.

Tasks come in waves. Tasks in one wave may run in parallel if they don't touch shared files (`core/src/lib.rs`, `mcp/src/tools.rs`, `mcp/src/mcp.rs`, `mcp/src/main.rs` are common conflict points). Audit waves are read-only and write to `decisions.md`.

## Project knowledge skill

Deeper docs live in `.claude/skills/project-knowledge/references/` (read via the `project-knowledge` skill): `architecture.md`, `patterns.md`, `deployment.md`, `project.md`, `ux-guidelines.md`, `crypto-design.md`, `threat-model.md`, `economics.md`, `protocol-integrations.md`.

Note: root `AGENTS.md` is a **public** page for external agents that want to use the hosted service (it pairs with `/.well-known/agent.json`). It is not a contributor guide — keep it accurate to the shipped service.

## Conventions

- **Docs change with code.** Every code change updates the affected docs in the same commit or PR: `docs/tools.md`, `docs/how-it-works.md`, package READMEs, `docs/QUICKSTART.md`, the white/yellow papers and this file when they describe the changed behaviour. A PR that changes behaviour without a doc update is incomplete.
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
