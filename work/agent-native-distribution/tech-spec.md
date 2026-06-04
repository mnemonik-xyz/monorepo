---
created: 2026-06-04
status: draft
branch: dev
size: L
---

# Tech Spec: Agent-Native Distribution

## Solution

Ship three coordinated pieces in one release, all wired against existing code surfaces identified in `code-research.md`:

1. **Server-side skill propagation.** Add seven markdown skill manifests under `mcp/assets/skills/` (new dir, no precedent). At build time `include_str!` projects two sections of each manifest (`## Purpose`, `## Trigger`) into `tool_definitions()` (`mcp.rs:427-497`) — same content lives in the binary verbatim for the new `prompts/*` and `resources/*` MCP methods. Extend the `bearer_auth_middleware` JSON-RPC allowlist (`oauth/mod.rs:1235`) so the four discovery methods are anonymous-OK. Extend `attestations` schema with a `visibility` column via a new `migrate_visibility_column()` following the exact pattern of `migrate_write_mode_column()` (`sqlite.rs:282-350`). `sign_memory` (`mcp.rs:1054-1088`) parses two new typed args (`visibility`, `allow_fallback_to_participate`) via resolvers that mirror `resolve_write_mode`'s shape. Anonymous `recall` filters by `visibility = 'public'`.

2. **Rust binary `mcp-stdio` subcommand.** Add a thin subcommand on the existing `mnemonic-mcp` binary. It reuses the same `Arc<McpState>` wired by `run_stdio()` (`main.rs:576-617`), but dispatches between two routes per request: discovery and local-mode tool calls execute against the binary's own state (fastembed + SQLite + COSE), participate-mode tool calls proxy via HTTPS to `mcp.mnemonik.xyz/mcp`. Token storage moves from `~/.mnemonic/token.json` (today's plaintext file written by Node CLI at `packages/cli/src/config.ts:39-65`) to the OS keychain via the same `keyring` crate the binary already uses for identity (`core/src/identity/keystore_os.rs`).

3. **npm shim package `@mnemonik-xyz/mcp`.** New package under `packages/mcp/`. Two responsibilities: (a) on `npm install -g`, download the platform-matching `mnemonic-mcp` artifact from GitHub Releases, verify its checksum against a `SHA256SUMS` manifest emitted by the release pipeline, install it as `mnemonik-mcp` in the platform-standard bin location; (b) expose three subcommands — `install` (PNL-pattern host-config wiring), `mcp-stdio` (passthrough to the underlying binary, for host subprocess use), `doctor` (diagnostic). `release.yml` (`.github/workflows/release.yml:14-216`) gets two additions: a SHA256SUMS step in the build matrix's release job, and a new npm publish step for the shim alongside the existing SDK+CLI publish.

The Node CLI (`@mnemonik-xyz/cli` v0.2.x) gains a token-storage migration of its own — switches from `~/.mnemonic/token.json` to the OS keychain via the existing `@napi-rs/keyring` dependency it already uses for identity. One-shot file→keychain migration runs on the next `login`/`status` invocation.

## Architecture

### What we're building/modifying

- **`mcp/assets/skills/` (new)** — seven manifests (`help.md`, `init.md`, `recall.md`, `attest.md`, `checkpoint.md`, `verify.md`, `status.md`). Single source of truth; build-time projection to MCP surfaces. No runtime fetching.
- **`mcp/src/mcp.rs` (edit)** — extend `handle_request_with_resolved_mode()` dispatch (`mcp.rs:526-547`) with four new arms (`prompts/list`, `prompts/get`, `resources/list`, `resources/read`); enrich `tool_definitions()` (`mcp.rs:427-497`) descriptions from manifests; surface `embedder.model_id` + `embedder.model_version` in the `initialize` response (`mcp.rs:526` arm); two new typed-arg resolvers (`resolve_visibility`, `resolve_allow_fallback`) used from `handle_tool_call` (`mcp.rs:1054-1088`).
- **`mcp/src/oauth/mod.rs` (edit)** — extend `ALLOWLIST_METHODS` (`oauth/mod.rs:1235`) with the four discovery method names.
- **`mcp/src/main.rs` (edit)** — register a `mcp-stdio` clap subcommand that calls existing `run_stdio()` path (no behavioral change for default; the subcommand is an alias and a hook for future dual-routing).
- **`mcp/src/tools.rs` (edit)** — `sign_memory()` (`tools.rs:240`) accepts `visibility` + `allow_fallback_to_participate` typed args; routes default-no-soft-fall vs explicit-opt-in escalation through `confirm_delivery_or_demote`-like helper; rejects `mode=local + visibility=...` with `invalid_params`; `recall` (or its anonymous variant) filters by `visibility='public'` for unauthenticated callers.
- **`core/src/storage/sqlite.rs` (edit)** — new `migrate_visibility_column()` following `migrate_write_mode_column` pattern (`sqlite.rs:282-350`); extends `save_attestation` (`sqlite.rs:488-532`) with a `visibility` parameter; `search` (`sqlite.rs:596-653`) gets an optional `visibility_filter` arg.
- **`core/src/storage/traits.rs` (edit)** — `AttestationStore::save_attestation` signature gains `visibility: Visibility`.
- **`core/src/identity/token_store.rs` (new)** — token-in-keychain helpers (read/write/delete) using existing `keyring::Entry` plumbing.
- **`packages/cli/src/identity/token-store.ts` (new)** — Node side of the same, using `@napi-rs/keyring`. Migration helper reads legacy `~/.mnemonic/token.json` on first call, writes to keychain, deletes the file.
- **`packages/mcp/` (new package)** — npm shim. `package.json` (`name: "@mnemonik-xyz/mcp"`, `bin: { "mnemonik-mcp": "./dist/bin/mnemonik-mcp.js" }`), `src/install.ts` (download + verify + cache binary), `src/install-hosts.ts` (PNL-pattern config merge), `src/doctor.ts` (diagnostics), `src/mcp-stdio.ts` (subprocess passthrough). Bundled as ESM via tsc.
- **`.github/workflows/release.yml` (edit)** — generate `SHA256SUMS` after all build matrices finish; attach to release; new `publish-mcp-shim` job analogous to the existing `publish-npm` job.

### How it works

**Anonymous discovery (Cursor or vanilla MCP Inspector → mcp.mnemonik.xyz):**

1. Client POSTs `{method: "initialize", ...}` (no `Authorization`).
2. `bearer_auth_middleware` (`oauth/mod.rs:1254`) peeks the body, finds `initialize` in `ALLOWLIST_METHODS`, passes through.
3. `handle_request()` dispatches; the `"initialize"` arm now returns `{protocolVersion, capabilities: {tools: {}, prompts: {}, resources: {}}, serverInfo, embedder: {model_id, model_version}}`.
4. Client follows up with `prompts/list`, `resources/list`, `tools/list` — all anonymous-OK. Each returns content baked into the binary at build time from `mcp/assets/skills/*.md`.

**Local sign through the shim (Claude Code → mnemonik-mcp mcp-stdio):**

1. Host spawns `/path/to/mnemonik-mcp mcp-stdio` as subprocess.
2. Subprocess is the same Rust `mnemonic-mcp` binary; `mcp-stdio` subcommand → existing `run_stdio()` (`main.rs:576-617`) flow.
3. JSON-RPC over stdin: `tools/call sign_memory { mode: "local", content: ... }` arrives.
4. `mcp_handler` → `handle_request_with_resolved_mode` → `handle_tool_call`. `resolve_write_mode` returns `Local`; `resolve_visibility` returns default `Private` (and rejects if caller sent it explicitly with `invalid_params` per AC14); `resolve_allow_fallback` returns `false` default.
5. `sign_memory` takes the inline path (`tools.rs:289`); embed via `state.embedder` (fastembed already on the path); TurboQuant compress; canonical CBOR; COSE_Sign1 with the local Ed25519 identity from `ensure()` (`core/src/identity/ensure.rs`); `save_attestation` with `visibility = Private` and synthetic `local:` tx IDs.
6. No network calls. Caller receives `{attestation_id, content_hash, write_mode: "local"}`.

**Participate sign with explicit fallback opt-in:**

1. `tools/call sign_memory { mode: "local", visibility: "public", allow_fallback_to_participate: true, content: ... }` arrives.
2. Resolvers accept all three args.
3. Local embedder attempt fails (e.g., model file missing).
4. Because `allow_fallback_to_participate = true`, `sign_memory` re-dispatches as participate-mode: same `handle_tool_call` flow but proxies through HTTPS to `mcp.mnemonik.xyz/mcp` (the binary needs to be configured to know its hosted peer; default `MNEMONIC_HOSTED_ENDPOINT=https://mcp.mnemonik.xyz/mcp`). OAuth-loopback fires if no token cached.
5. Response carries `{attestation_id, ..., escalated: {from: "local", to: "participate", reason: "embedder_unavailable"}}`. Stderr line warns about chain anchor.

**Install (shim, npm-distributed):**

1. User runs `npm install -g @mnemonik-xyz/mcp`. npm runs the package's `postinstall` (or `install.ts` invoked via `bin`).
2. `install.ts` reads platform + arch, picks the matching artifact name from a hardcoded map (`mnemonic-mcp-${tag}-${target}.tar.gz`).
3. Fetches GitHub Releases asset URL, downloads, fetches `SHA256SUMS` (also a release asset), verifies the line matching the artifact filename.
4. Extracts `mnemonic-mcp` from the tarball, renames the binary to `mnemonik-mcp`, places in `~/.local/share/@mnemonik-xyz/mcp/bin/` (or platform-appropriate location).
5. User runs `mnemonik-mcp install`. Script reads three candidate config paths (only-if-exists), parses each as JSON, sets `mcpServers.mnemonik = { command: "<full path to cached binary>", args: ["mcp-stdio"] }`, writes back preserving all other keys.

### Shared resources

| Resource | Owner (creates) | Consumers | Instance count |
|----------|----------------|-----------|----------------|
| `Box<dyn Embedder>` (fastembed instance) | `McpState::build()` in `main.rs` | `tools::sign_memory`, `tools::recall`, `initialize` response builder | 1 (singleton via `Arc<McpState>`) |
| `SqliteStore` | `McpState::build()` in `main.rs` | `tools::sign_memory`, `tools::recall`, `tools::verify`, lineage | 1 (singleton, `!Send` wrapped in `Arc<Mutex<...>>` per patterns.md) |
| Cached binary (`~/.local/share/@mnemonik-xyz/mcp/bin/mnemonik-mcp`) | shim `install.ts` | `mcp-stdio` subprocess spawned by every MCP host | 1 per user, multiple readers (host subprocesses) |
| OS keychain entry for token | Rust binary (first participate write) or Node CLI (login) — whichever runs first | Rust binary (read on every authenticated proxy call), Node CLI (read on `whoami` / `recall` / `sign`) | 1 per user |
| OS keychain entry for identity | already exists (PRs #154/#157, invisible-bootstrap) | Rust binary (sign), Node CLI (sign) | 1 per user |

## Decisions

### Decision 1: Single markdown source → three MCP surfaces via build-time projection
**Decision:** Manifests live as `mcp/assets/skills/*.md`. `build.rs` (or a `proc-macro` if `build.rs` proves awkward) parses each manifest into three named string constants: `FULL_MARKDOWN`, `PURPOSE_PLUS_TRIGGER`, and `PURPOSE_ONE_LINER`. The constants feed: `resources/read` (full markdown), `prompts/get` (full markdown wrapped as a prompt message), and `tools/list` description (Purpose+Trigger concatenation injected into the existing inline `json!` blocks in `tool_definitions()`). All three derive from one file; drift physically impossible.
**Rationale:** Serves user-spec "Что делаем" item 2 and AC15 ("Tool descriptions inline manifest sections at build time. Drift … physically impossible"). Build-time projection rules out the runtime fetching anti-pattern adequacy validator flagged in round 2 (always-proxy discovery violates offline cold-start).
**Alternatives considered:** (a) Server-only manifests, binary fetches on `mcp-stdio` startup — rejected because offline cold-start breaks. (b) Bundled into the npm shim instead of the Rust binary — rejected because then the Rust binary's `tools/list` descriptions can't reference them, and we'd have two stores.

### Decision 2: New `visibility` column via `migrate_visibility_column()` following `migrate_write_mode_column` precedent
**Decision:** Add `attestations.visibility TEXT NOT NULL DEFAULT 'private'` via a new migration helper that mirrors `migrate_write_mode_column()` at `sqlite.rs:282-350` line-for-line: column existence check via `PRAGMA table_info`, `BEGIN IMMEDIATE`, conditional `ALTER TABLE`, backfill UPDATE, `CREATE INDEX IF NOT EXISTS idx_attestations_visibility ON (visibility)`, COMMIT/ROLLBACK. Wired into both `SqliteStore::open` (`sqlite.rs:378`) and `SqliteStore::in_memory` (`sqlite.rs:394`).
**Rationale:** Serves user-spec "Что делаем" item 5 and AC13. Established precedent eliminates schema-design risk; identical lock discipline.
**Alternatives considered:** Separate `attestation_visibility` table joined on `attestation_id` — rejected because `recall`'s hot path adds an extra join for every row, and a per-row column is the minimum-overhead lookup the filter needs.

### Decision 3: `Visibility` is only valid with `mode: "participate"` — `mode=local + visibility=...` is `invalid_params`
**Decision:** `resolve_visibility(args, resolved_mode)` returns `Err(invalid_params("visibility", "visibility is only valid with mode=participate"))` if `args.visibility.is_some() && resolved_mode == WriteMode::Local`. If `mode=participate` and `visibility` is absent, default `Private`.
**Rationale:** Serves user-spec AC14. Eliminates the under-defined `{local, public}` matrix cell user flagged in interview. Local writes never leave the machine — sharing concept doesn't apply.
**Alternatives considered:** Accept `visibility` on local writes as future-promote intent — rejected because there's no promote mechanism in v1, the flag would be dead metadata, and adequacy validator round 1 explicitly flagged the under-specification.

### Decision 4: Soft-fall is explicit opt-in via `allow_fallback_to_participate: bool` (default `false`), escalation visible in response
**Decision:** New typed `bool` arg on `sign_memory`. Default `false`. When `false` and local execution fails, `sign_memory` returns the underlying JSON-RPC typed error. When `true` and local fails, `sign_memory` re-dispatches the same arguments through the participate path (which auto-triggers OAuth-loopback if no cached token); response includes `escalated: { from: "local", to: "participate", reason: "<machine readable enum>" }`. Stderr line logged. If escalation itself fails (no network), the error returned is the *escalation* failure (`-32011` family for hosted unavailable), not the original local-failure code — agent sees the actual failure point.
**Rationale:** Serves user-spec AC5 + AC6. Closes adequacy validator round 1 finding ("silent escalation surprises the caller"). Explicit opt-in + response-surfaced escalation gives the agent enough information to surface to the user without depending on stderr (which most MCP hosts don't expose).
**Alternatives considered:** (a) Always silent soft-fall on `visibility=public` — rejected per validator. (b) Always loud fail — rejected because the user-spec explicitly carves out the opt-in for callers who'd rather get a working write than a clean error.

### Decision 5: Anonymous recall filters `WHERE visibility = 'public'` via new `search` arg
**Decision:** Extend `SqliteStore::search` (`sqlite.rs:596-653`) to accept an optional `visibility_filter: Option<Visibility>`. When `Some(Visibility::Public)`, the WHERE clause adds `AND a.visibility = 'public'`. `recall` tool handler passes `Some(Public)` iff the caller has no JWT (`Claims` from `bearer_auth_middleware` is `None`); otherwise `None` (authenticated callers see all their own rows).
**Rationale:** Serves user-spec AC13. Reuses `bearer_auth_middleware`'s existing claim-or-no-claim distinction; no new auth surface.
**Alternatives considered:** Two separate SQL queries (anonymous variant fully separate) — rejected because the join + cosine-distance core stays identical; the filter is one AND clause.

### Decision 6: `mcp-stdio` subcommand on the existing Rust binary, not a separate binary
**Decision:** Add a `mcp-stdio` subcommand to `mnemonic-mcp`'s clap parser in `main.rs`. The subcommand re-uses the existing `run_stdio()` (`main.rs:576-617`) entry point (already feeds `Arc<McpState>` into the same `handle_request` dispatcher used by the HTTP path). No new state-construction code; default `MNEMONIC_HOSTED_ENDPOINT=https://mcp.mnemonik.xyz/mcp` env var allows the binary to know its hosted peer for participate-mode proxying.
**Rationale:** Serves user-spec "Что делаем" Кусок 3 item 12. Reuses the entire existing tool pipeline; no behavioral drift between the same tool executed by mcp.mnemonik.xyz and locally. `Arc<McpState>` is already shared between `run_stdio` and `run_http` per code-research §10.
**Alternatives considered:** Separate binary `mnemonic-mcp-stdio` — rejected because it doubles release surface and risks code drift; the existing dispatcher handles everything we need.

### Decision 7: Token storage moves to OS keychain via existing `keyring` crate (Rust) and `@napi-rs/keyring` (Node)
**Decision:** New `core/src/identity/token_store.rs` exposes `read_token() -> Option<TokenJson>`, `save_token(&TokenJson)`, `delete_token()` against a fixed keychain coordinate `service = "xyz.mnemonik.token"`, `account = "default"` (mirrors the identity entry's `xyz.mnemonik.identity`/`default` convention). Token JSON shape is unchanged from today's `~/.mnemonic/token.json`. Both the Rust binary (when caching after OAuth-loopback) and Node CLI (when persisting after `login`) read/write the same coordinate. On Rust side, a one-shot migration helper detects the legacy file and adopts its content if the keychain entry is absent; legacy file deleted post-migration. On Node side, same migration in `packages/cli/src/identity/token-store.ts`.
**Rationale:** Serves user-spec AC12 + AC12b. Closes adequacy validator round 1 finding ("two-store anomaly: identity in keychain, token in plaintext"). Reuses already-loaded `keyring` infrastructure on both runtimes — zero new deps.
**Alternatives considered:** (a) Keep file but encrypt with a fresh OS-randomness key in keychain — rejected because adds a key-management layer for zero benefit over just storing the token directly in keychain. (b) Token-only file; coordinate Rust+CLI release simultaneously — rejected because user-spec AC12b explicitly requires migration-on-first-use without simultaneous release.

### Decision 8: npm shim downloads pinned-tag binary from GitHub Releases on install, verifies against pipeline-emitted SHA256SUMS
**Decision:** `@mnemonik-xyz/mcp` package.json includes a `postinstall` script that runs `dist/scripts/install-binary.js`. The script consults `dist/binary-version.json` (committed alongside the shim release) for the target tag, computes the GitHub Releases asset URL pattern `https://github.com/mnemonik-xyz/monorepo/releases/download/${tag}/mnemonic-mcp-${tag}-${target}.tar.gz`, downloads, also downloads `SHA256SUMS` from the same release, picks the matching line, verifies the digest with Node's `crypto.createHash("sha256")`. On mismatch — refuses to install; on network failure — clear error message. release.yml gains a `release.checksums` step that emits `SHA256SUMS` from all collected artifacts before `softprops/action-gh-release@v2` runs.
**Rationale:** Serves user-spec R4 + R6 + AC17 (doctor checks integrity). Closes adequacy validator round 2 finding ("'hash verified' has nothing trustworthy to verify against today"). SHA256SUMS is the minimum-overhead manifest format; future upgrade to sigstore is a v2 concern.
**Alternatives considered:** (a) sigstore-signed manifest — rejected as scope creep; SHA256SUMS satisfies "verified against same tagged pipeline" requirement. (b) Bundle binary directly in npm package per-platform (esbuild pattern) — rejected because the binary is already published to GitHub Releases for `cargo install` parity; double-publishing would double download paths and split the verification surface.

### Decision 9: Three host candidates hardcoded (macOS only); install written with full binary path (no `npx`)
**Decision:** `install.ts` candidates list: `~/.claude.json`, `~/Library/Application Support/Claude/claude_desktop_config.json`, `~/.cursor/mcp.json`. Each entry written as `"command": "<resolved absolute path to cached mnemonik-mcp>", "args": ["mcp-stdio"]`. Only-if-file-exists guard; absent hosts skipped silently; output reports per-host status.
**Rationale:** Serves user-spec "Что делаем" Кусок 2 items 6-8 and AC7-AC10. Direct binary path keeps the host's subprocess spawn offline (user-spec AC3 explicitly forbids `npx -y` since it pings registry). macOS-only matches PNL's actual coverage; Linux/Windows are v1.1 (user-spec scope).
**Alternatives considered:** `npx -y @mnemonik-xyz/mcp mcp-stdio` — rejected per adequacy validator round 1.

### Decision 10: `initialize` response surfaces embedder identity for client-side parity warning
**Decision:** `initialize` arm in `mcp.rs:526` extended to include `embedder: { model_id: <Embedder::model_id()>, model_version: "<env or build constant>", dim: <Embedder::dim()> }`. The Rust binary running as `mcp-stdio` reads its own embedder's values at startup; on first `initialize` reply from a remote `mcp.mnemonik.xyz` (when proxying participate-mode discovery, though discovery is local by Decision 1), compares; mismatch logged to stderr as `[mnemonik-mcp] embedder version mismatch: local=<x> remote=<y>, cross-mode recall not guaranteed consistent`.
**Rationale:** Serves user-spec AC15. Decoupled-versioning surface using the `Embedder` trait that already exposes `model_id()` and `dim()` (code-research §10).
**Alternatives considered:** Force lockstep version pin between binary and `mcp.mnemonik.xyz` — rejected per user-spec ("documented that cross-mode recall semantics undefined at mismatch").

### Decision 11: `Visibility` enum lives in `core/src/storage/types.rs` next to `WriteMode`
**Decision:** New `pub enum Visibility { Private, Public }` with `Display`/`FromStr`/serde impls following `WriteMode` precedent. Wired through `save_attestation`, `search`, `SearchResult`, and the SQLite codec. Default-derived `Visibility::default()` returns `Private`.
**Rationale:** Established pattern in the repo (`WriteMode` is the closest analogue); reviewers know what to expect. **[TECHNICAL]** — derived from user-spec requirement to track visibility; the exact module placement is a code-organization detail.

## Data Models

### `attestations` table (after migration)

```sql
-- Columns existing pre-feature:
attestation_id TEXT PRIMARY KEY,
content        TEXT NOT NULL,
content_hash   TEXT NOT NULL,
tags           TEXT NOT NULL DEFAULT '[]',
solana_tx      TEXT NOT NULL,
arweave_tx     TEXT NOT NULL,
signer_pubkey  TEXT NOT NULL,
created_at     TEXT NOT NULL,
owner_pubkey   TEXT,                                       -- migrated earlier
correlation_id TEXT,                                       -- migrated earlier
write_mode     TEXT NOT NULL DEFAULT 'participate',        -- migrated earlier

-- New column (this feature):
visibility     TEXT NOT NULL DEFAULT 'private'             -- 'private' | 'public'
```

New index: `CREATE INDEX IF NOT EXISTS idx_attestations_visibility ON attestations(visibility);`

### `sign_memory` JSON-RPC `arguments` after this feature

```json
{
  "content": "string (required)",
  "tags": "array of strings (optional)",
  "mode": "'local' | 'participate' (optional, default per write_mode resolver)",
  "visibility": "'public' (optional, ONLY valid when mode='participate' — rejected with -32602 on local writes)",
  "allow_fallback_to_participate": "bool (optional, default false; opt-in for local→participate escalation)"
}
```

### `sign_memory` response (escalation case)

```json
{
  "attestation_id": "uuid",
  "content_hash": "blake3 hex",
  "write_mode": "participate",
  "visibility": "public",
  "solana_tx": "...",
  "arweave_tx": "...",
  "escalated": {
    "from": "local",
    "to": "participate",
    "reason": "embedder_unavailable | local_storage_busy | identity_bootstrap_failed"
  }
}
```

### `initialize` response (added fields)

```json
{
  "protocolVersion": "2025-06-18",
  "capabilities": {
    "tools": {},
    "prompts": {},
    "resources": {}
  },
  "serverInfo": { "name": "mnemonic-mcp", "version": "..." },
  "embedder": {
    "model_id": "Xenova/all-MiniLM-L6-v2",
    "model_version": "compile-time constant",
    "dim": 384
  }
}
```

### Token keychain entry (both runtimes)

- Service: `xyz.mnemonik.token`
- Account: `default`
- Secret payload: same JSON shape as today's `~/.mnemonic/token.json` (see `packages/cli/src/config.ts:39-65`).

### `SHA256SUMS` manifest format

Standard `sha256sum -b` output, one line per artifact:

```
<64-hex digest>  mnemonic-mcp-${tag}-aarch64-apple-darwin.tar.gz
<64-hex digest>  mnemonic-mcp-${tag}-x86_64-apple-darwin.tar.gz
<64-hex digest>  mnemonic-mcp-${tag}-x86_64-unknown-linux-gnu.tar.gz
<64-hex digest>  mnemonic-mcp-${tag}-aarch64-unknown-linux-gnu.tar.gz
```

Attached to the GitHub Release as a separate asset.

## Dependencies

### New packages

- (Rust) **none** — `keyring`, `serde`, `serde_json`, `rusqlite`, `tracing`, `clap` are already in workspace. Skill markdown parsing uses string-slicing on `## Purpose` / `## Trigger` headers via `str::split` — no new crate.
- (Node, `packages/mcp/`) **`tar`** — for extracting tarball downloads. Standard `decompress` alternatives have native deps that would defeat the purpose. **`undici`** — Node 22's built-in fetch is sufficient; only fall back to `undici` if Node 20 support is needed (TBD with user; defaulting to Node 22+ since that's what CI already requires).
- (Node, `packages/mcp/` devDeps) — TypeScript tooling matching `packages/cli/`'s existing setup (`typescript`, `vitest`, `tsx`).

### Using existing (from project)

- `core/src/identity/keystore_os.rs` — existing `OsKeyStore` wraps `keyring::Entry`; `token_store.rs` mirrors the wrapper for the new `xyz.mnemonik.token` coordinate.
- `core/src/storage/sqlite.rs::migrate_write_mode_column` — line-for-line precedent for the new `migrate_visibility_column`.
- `mcp/src/mcp.rs::{JsonRpcRequest, JsonRpcError, invalid_params, …}` — existing JSON-RPC types and helpers; new errors follow the `data: { kind, … }` discriminator convention already used by `-32010` and `-32011`.
- `mcp/src/oauth/mod.rs::ALLOWLIST_METHODS` — extended in-place.
- `mcp/src/main.rs::run_stdio` — re-used by the `mcp-stdio` subcommand.
- `@napi-rs/keyring` (already in `packages/cli/package.json:48`) — Node CLI token migration uses the existing dep.
- `.github/workflows/release.yml` — new SHA256SUMS step + new shim-publish job grafted onto the existing release matrix.

## Testing Strategy

**Feature size:** L

### Unit tests

- Skill manifest parser: every manifest in `mcp/assets/skills/*.md` is well-formed (has `## Purpose` and `## Trigger` H2 sections); duplicate manifest filenames are a build error.
- `Visibility` enum: `Display`/`FromStr`/serde roundtrip; default `Private`.
- `resolve_visibility(args, mode)`: explicit `"public"`/`"private"` accepted under participate; explicit value rejected with `invalid_params` under local; absent value defaults `Private` for both modes (but local-mode signing never reads it).
- `resolve_allow_fallback(args)`: explicit `true`/`false` accepted; non-bool rejected; absent defaults `false`.
- `migrate_visibility_column`: clean DB gains the column; existing DB with no `visibility` gains the column and backfills `'private'`; re-run is no-op.
- Token store (Rust + Node): write/read/delete roundtrip against a `MemoryKeyStore`-style mock; one-shot legacy migration from a tempfile.
- `install-binary.ts`: SHA256 verify accepts matching digest, rejects mismatch (table-driven against fixture digests).
- `install-hosts.ts`: JSON merge preserves unrelated keys (table-driven against synthetic configs with pre-existing MCP servers); idempotent on re-run; absent file is silently skipped.

### Integration tests

- **Anonymous discovery (AC1):** spin up `mnemonic-mcp` locally in `STORAGE_MODE=local`, POST `initialize` / `prompts/list` / `resources/list` / `tools/list` from `reqwest` without `Authorization`. Assert ≥7 prompts, ≥7 resources, 5+ tool descriptions each ≥500 bytes mentioning Purpose+Trigger.
- **Anonymous recall filter (AC13):** seed DB via direct SQL with one row `visibility='private'` matching a query string and one row `visibility='public'` matching the same string. Call `recall` without `Authorization`. Assert the response contains only the public row.
- **Visibility rejected on local writes (AC14):** call `tools/call sign_memory` with `mode=local, visibility=public`. Assert JSON-RPC error code `-32602` and message naming `visibility`.
- **Soft-fall opt-in semantics (AC5/AC6):** monkey-patch the embedder to raise on first call; call `sign_memory` with `allow_fallback_to_participate=false` → assert typed embedder-failure error and zero outbound TCP (netns-isolated test). Call again with `allow_fallback_to_participate=true` + reachable hosted endpoint → assert success and `escalated` field in response.
- **Token migration (AC12b):** seed `~/.mnemonic/token.json` in a tempdir HOME, invoke `mnemonic_login`-equivalent flow via `mnemonic-mcp` binary, assert keychain entry present + file deleted.
- **Install idempotent + non-destructive (AC7-AC10):** create synthetic configs with unrelated `mcpServers` entries; run `install.ts` twice; diff against original — only `mnemonik` key added/replaced, unrelated entries byte-identical; absent host configs silently skipped.

### E2E tests

- **Offline cold-start of local sign (AC3):** in network-isolated CI runner, spawn `mnemonic-mcp mcp-stdio` with embedder model pre-cached, drive it via JSON-RPC stdin, assert `sign_memory { mode: "local" }` succeeds with zero outbound TCP (`tcpdump`-style assertion at the network namespace boundary).
- **Embedder parity surface (AC15):** start binary, observe `initialize` response carries `embedder.model_id` and `embedder.model_version`; simulate mismatched values from a mock hosted peer; assert stderr warning line.

## Agent Verification Plan

**Source:** user-spec "Как проверить" section (12 rows in agent-checks table + 2 manual user steps).

### Verification approach

For Implementation Tasks below, every server-side task includes a curl/bash `Verify-smoke` that exercises the new method or argument against a locally-running `mnemonic-mcp` in `STORAGE_MODE=local`. CLI shim tasks have shell `Verify-smoke` for the install/doctor paths. Post-deploy verification runs the MCP Inspector smoke against the production `mcp.mnemonik.xyz` once the new server build is live.

### Tools required

- `curl` — direct JSON-RPC against the running MCP server (anonymous discovery, sign_memory, recall)
- `bash` — fixture seeding, tempdir HOME for token-migration tests, netns isolation via `unshare -n` for offline tests
- `cargo test --workspace` — unit + integration
- `npx vitest run` — Node-side unit tests for shim package
- `npx @modelcontextprotocol/inspector` — pre-release manual smoke against production

No Playwright / Telegram / Stripe MCPs required; this feature has no browser UI.

## Risks

| Risk | Mitigation |
|------|-----------|
| `mcp/assets/skills/*.md` skill content quality drives adoption — bad triggers = over- or under-attest | Trigger boundaries explicit in `attest.md` per user-spec R7; positive AND negative examples in each manifest; internal review before tagging release |
| `migrate_visibility_column` race with another process opening the same DB | Existing precedent (`migrate_write_mode_column`) uses `BEGIN IMMEDIATE` for serialization; new migration reuses the pattern verbatim |
| Token migration runs before keychain is unlocked (first boot, login items) | Token migration is opportunistic — first failure → keep file, retry on next call. Doctor reports persistent failure |
| `mcp-stdio` subprocess hangs on `Arc<McpState>` build (fastembed model download on first run) | Background warmup on subcommand entry, with a deadline; failure surfaces as typed error in first tool call, not subprocess hang |
| Linux `local-embed` build still broken (libdbus, separate task) blocks linux artifacts in release pipeline → shim install on Linux fails | v1 is macOS-only per user-spec; shim refuses to install on unsupported platforms with a clear message; Linux/Windows queued for v1.1 |
| SHA256SUMS download is the trust root for the binary — if release.yml's checksum step is compromised, integrity is broken | Same trust root as the existing artifact uploads; tagged-release pipeline is the boundary, no new attack surface beyond what existed pre-feature |
| Concurrent MCP-host subprocesses contend on local SQLite (Claude Code + Cursor open at once) | WAL mode + bounded busy-timeout retry (typed `LocalStorageBusy` error on budget exceedance); precedent in storage lock discipline (patterns.md) |
| `keyring` crate's `sync-secret-service` feature pulls libdbus on Linux, complicating linux build | Same blocker as the existing `local-embed` Linux issue; documented as v1.1 scope; v1 macOS works today |

## User-Spec Deviations

- **AC12b (token migration via existing CLI):** user-spec says "не требует одновременного релиза CLI и shim'а." Tech-spec implements this for the Rust binary (Decision 7) but ALSO writes equivalent migration code in `packages/cli/src/identity/token-store.ts`. The CLI change is non-blocking — the keychain entry coordinate is fixed, so a not-yet-updated CLI continues reading the legacy file until it gets rebuilt with token-store wiring, at which point it migrates. **Reason:** without the CLI-side change, users who run `mnemonic login` from the CLI (not via the shim) would still write plaintext, leaving a partial migration. Tech-spec ships both halves but they remain independently shippable. → No approval needed — strengthens user-spec intent without changing semantics.
- **Added: `MNEMONIC_HOSTED_ENDPOINT` environment variable** on `mnemonic-mcp` binary (default `https://mcp.mnemonik.xyz/mcp`). Reason: participate-mode proxying needs to know where to send HTTPS calls; for testing the binary must be redirectable to a local instance. User-spec implies `mcp.mnemonik.xyz` everywhere but doesn't make it env-configurable. → No approval needed — operational hygiene, doesn't change user-visible behavior.
- **Added: `embedder.model_version` field type left to tech-spec.** User-spec AC15 says "embedder.model_id + embedder.model_version" without committing to whether `model_version` is a semver string, ONNX file hash, or build-time constant. Tech-spec picks a build-time constant string for v1, derivable from the fastembed crate version + model name. → No approval needed — tech-spec-level detail, surface contract unchanged.

## Acceptance Criteria

Технические критерии приёмки (дополняют пользовательские из user-spec):

- [ ] `cargo test --workspace` passes; `cargo clippy --workspace -- -D warnings` clean
- [ ] `npx vitest run` in `packages/mcp/` passes
- [ ] No regression in existing integration tests (`mcp/tests/`, `core/tests/`)
- [ ] `migrate_visibility_column` is idempotent (running open() twice on the same DB is a no-op after first)
- [ ] `mcp-stdio` subcommand reuses the same `Arc<McpState>` as `run_http` (no duplicate embedder/store construction; code-review attestable via singleton check)
- [ ] All five new JSON-RPC method arms (`prompts/list`, `prompts/get`, `resources/list`, `resources/read`, and the enriched `initialize`) return MCP-spec-conformant responses on anonymous calls
- [ ] `bearer_auth_middleware`'s `ALLOWLIST_METHODS` extension is the sole code change required for anonymous-OK semantics (no body-parsing changes needed per code-research §2)
- [ ] release.yml emits `SHA256SUMS` as a release asset matching all `mnemonic-mcp-*.tar.gz` artifacts
- [ ] `packages/mcp/` ships as `@mnemonik-xyz/mcp` with Trusted Publishing (no NPM_TOKEN) — same pattern as existing SDK+CLI publish jobs

## Implementation Tasks

### Wave 1 (independent foundation — server-side skills + discovery)

#### Task 1: Skill manifests + build-time projection
- **Description:** Create `mcp/assets/skills/` directory with seven manifests (`help.md`, `init.md`, `recall.md`, `attest.md`, `checkpoint.md`, `verify.md`, `status.md`). Each manifest has `## Purpose`, `## Trigger`, plus body sections (context, tool, guardrails, examples). Wire `build.rs` (or proc-macro if cleaner) to parse the H2 sections and emit string constants for downstream use. Manifest content is per user-spec; trigger boundary explicit per R7 mitigation.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-mcp` succeeds; deliberately rename one manifest, expect build to fail with clear missing-manifest error
- **Files to modify:** `mcp/build.rs` (new), `mcp/Cargo.toml` (build dep if needed), `mcp/assets/skills/*.md` (new)
- **Files to read:** `mcp/src/mcp.rs:427-497` (current `tool_definitions()`), `work/agent-native-distribution/user-spec.md`

#### Task 2: prompts/* + resources/* MCP methods + anonymous allowlist
- **Description:** Add four arms (`prompts/list`, `prompts/get`, `resources/list`, `resources/read`) to the dispatcher in `mcp.rs`. Each pulls from the manifest constants built in Task 1. Extend `ALLOWLIST_METHODS` in `oauth/mod.rs` with the four new method names. Extend `initialize` capabilities to include `prompts: {}` and `resources: {}`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `curl -s -X POST localhost:3000/mcp -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"prompts/list","params":{}}'` returns ≥7 prompts without Authorization
- **Files to modify:** `mcp/src/mcp.rs` (dispatcher + initialize), `mcp/src/oauth/mod.rs` (allowlist)
- **Files to read:** `mcp/src/mcp.rs:499-547`, `mcp/src/oauth/mod.rs:1226-1336`, `mcp/assets/skills/` (Task 1 output)

#### Task 3: Enriched tools/list descriptions + initialize embedder surface
- **Description:** Inject Purpose+Trigger from each skill manifest into the matching tool's `description` field in `tool_definitions()`. Extend the `initialize` response to include `embedder: { model_id, model_version, dim }` derived from `state.embedder` (which already exposes `Embedder::model_id()` and `Embedder::dim()`).
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `curl ... initialize` response shows `embedder.model_id`; `curl ... tools/list` response shows each tool description ≥500 bytes mentioning Purpose+Trigger
- **Files to modify:** `mcp/src/mcp.rs` (`tool_definitions()` + `initialize` arm)
- **Files to read:** `mcp/src/mcp.rs:427-497`, `core/src/embed/mod.rs` (`Embedder` trait)

### Wave 2 (storage + tool args — depends on nothing in Wave 1)

#### Task 4: visibility column migration + Visibility enum + storage signatures
- **Description:** New `core/src/storage/types.rs::Visibility` enum (Private | Public) with Display/FromStr/serde. New `migrate_visibility_column()` in `sqlite.rs` mirroring `migrate_write_mode_column`. Extend `AttestationStore::save_attestation` trait signature with `visibility: Visibility`. Extend `search` with optional `visibility_filter`. Wire migration into `SqliteStore::open` and `SqliteStore::in_memory`.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Files to modify:** `core/src/storage/types.rs` (new file), `core/src/storage/sqlite.rs`, `core/src/storage/traits.rs`, `core/src/storage/mod.rs` (re-export)
- **Files to read:** `core/src/storage/sqlite.rs:282-350` (migration precedent), `core/src/storage/sqlite.rs:488-532` (save_attestation), `core/src/storage/sqlite.rs:596-653` (search)

#### Task 5: sign_memory accepts visibility + allow_fallback args; anonymous recall filters public
- **Description:** Add `resolve_visibility(args, mode)` and `resolve_allow_fallback(args)` in `tools.rs` following `resolve_write_mode` shape; reject `mode=local + visibility=...` with `invalid_params`. Thread `visibility` through `sign_memory` into `save_attestation`. Update `recall` handler to pass `Some(Visibility::Public)` to `search` when caller has no `Claims`, else `None`. Add `escalated` field to `sign_memory` response when soft-fall fires (paired with Task 7 wiring).
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `curl ... tools/call sign_memory { mode: "local", visibility: "public" }` returns `-32602`; anonymous `recall` returns only public seeded row
- **Files to modify:** `mcp/src/tools.rs`, `mcp/src/mcp.rs` (`handle_tool_call` arg extraction)
- **Files to read:** `mcp/src/tools.rs:100-134` (resolve_write_mode), `mcp/src/mcp.rs:1054-1088` (current sign_memory arg extraction)

### Wave 3 (CLI + binary — depends on Wave 2)

#### Task 6: mcp-stdio subcommand on Rust binary + MNEMONIC_HOSTED_ENDPOINT env var
- **Description:** Add `clap` subcommand `mcp-stdio` to `mnemonic-mcp` (`main.rs`) that re-uses the existing `run_stdio()` entry point. Add `MNEMONIC_HOSTED_ENDPOINT` env var (default `https://mcp.mnemonik.xyz/mcp`) for participate-mode proxying. Behavior preserved: existing stdio flow continues to work unchanged.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, test-reviewer
- **Verify-smoke:** `mnemonic-mcp mcp-stdio` accepts JSON-RPC on stdin (echo a `tools/list` request, expect response)
- **Files to modify:** `mcp/src/main.rs`
- **Files to read:** `mcp/src/main.rs:576-617` (current run_stdio)

#### Task 7: Soft-fall opt-in routing (local→participate on opt-in)
- **Description:** When `allow_fallback_to_participate=true` and local execution fails, `sign_memory` re-dispatches the same call through the participate path (HTTPS to `MNEMONIC_HOSTED_ENDPOINT`). On success, response carries `escalated: {from, to, reason}`. On hosted unavailability, error is the hosted-unavailable code (not the original local-failure code). Stderr warning logged. Wire this into `sign_memory` inline path.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** rm fastembed cache, post `sign_memory` with `allow_fallback_to_participate=true` against running binary, expect success + `escalated` field
- **Files to modify:** `mcp/src/tools.rs`, `mcp/src/mcp.rs` (route hookup)
- **Files to read:** `mcp/src/tools.rs:240-...` (current sign_memory routing)

#### Task 8: Token storage in OS keychain + migration (Rust binary + Node CLI)
- **Description:** New `core/src/identity/token_store.rs` with `read_token`/`save_token`/`delete_token` against `xyz.mnemonik.token/default`. Migration helper that adopts `~/.mnemonic/token.json` on first call and deletes the file. New `packages/cli/src/identity/token-store.ts` mirroring the same coordinate via `@napi-rs/keyring`. Update Node CLI's `loadToken`/`saveToken` callsites (`config.ts:367, 399`) to delegate.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `mnemonic login` (Node CLI) leaves no `~/.mnemonic/token.json` afterwards; `security find-generic-password -s xyz.mnemonik.token -a default` (macOS) returns entry
- **Files to modify:** `core/src/identity/token_store.rs` (new), `core/src/identity/mod.rs` (re-export), `mcp/src/oauth/mod.rs` (callsites that read/write token), `packages/cli/src/identity/token-store.ts` (new), `packages/cli/src/config.ts:367,399` (delegate)
- **Files to read:** `packages/cli/src/config.ts:39-65` (current TokenJson + paths), `core/src/identity/keystore_os.rs` (Rust keychain wrapper precedent), `packages/cli/src/identity/keystore-os.ts:16` (Node keychain wrapper precedent)

### Wave 4 (npm shim package — depends on Wave 3 Task 6)

#### Task 9: @mnemonik-xyz/mcp shim package skeleton + binary download/verify
- **Description:** New `packages/mcp/` directory with `package.json` (`name: "@mnemonik-xyz/mcp"`, `bin: { "mnemonik-mcp": "./dist/bin/mnemonik-mcp.js" }`), TypeScript config, vitest setup mirroring `packages/cli/`. Bin entrypoint dispatches between subcommands (`install`, `mcp-stdio`, `doctor`). On `postinstall`, downloads the platform binary from GitHub Releases, verifies SHA256 against the release's SHA256SUMS asset, extracts, places at `~/.local/share/@mnemonik-xyz/mcp/bin/mnemonik-mcp`. `mcp-stdio` subcommand on the shim spawns the cached binary as subprocess.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `npm install` in shim dir runs postinstall; cached binary present; SHA256 verify with fixture digest passes (positive case) and fails on mismatch (negative case)
- **Files to modify:** `packages/mcp/package.json` (new), `packages/mcp/tsconfig.json` (new), `packages/mcp/src/install-binary.ts` (new), `packages/mcp/src/bin/mnemonik-mcp.ts` (new), `packages/mcp/src/mcp-stdio.ts` (new)
- **Files to read:** `packages/cli/package.json` (peer pattern), `packages/cli/bin/mnemonic.ts` (subcommand dispatch pattern)

#### Task 10: install + doctor subcommands (PNL-pattern config wiring + diagnostics)
- **Description:** `install` subcommand reads three candidate host config paths (only-if-exists), parses JSON, sets `mcpServers.mnemonik = { command: <absolute path to cached binary>, args: ["mcp-stdio"] }`, writes back preserving unrelated keys. Idempotent on re-run. `--check` flag prints plan without writing. `doctor` subcommand reports: presence of `mnemonik` entry in each host config, ping to `mcp.mnemonik.xyz/health`, binary integrity (re-verify hash), local SQLite read/write, identity accessibility, keychain accessibility for token. Pass/fail per check + repair hint.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** seed unrelated `mcpServers.foo` entry in tempdir HOME's `~/.claude.json`; run `mnemonik-mcp install`; diff shows `mnemonik` added and `foo` untouched; re-run shows no diff
- **Files to modify:** `packages/mcp/src/install-hosts.ts` (new), `packages/mcp/src/doctor.ts` (new), `packages/mcp/src/bin/mnemonik-mcp.ts` (dispatch new subcommands)
- **Files to read:** Task 9 output (shim skeleton)

### Wave 5 (release pipeline — depends on Waves 1-4)

#### Task 11: release.yml SHA256SUMS emission + @mnemonik-xyz/mcp publish step
- **Description:** Add a step to `.github/workflows/release.yml` that, after all build matrices complete (`needs: [build-linux, build-macos]`), generates a `SHA256SUMS` file from all collected `mnemonic-mcp-*.tar.gz` artifacts and attaches it to the GitHub Release as a separate asset. Add a new `publish-mcp-shim` job analogous to the existing `publish-npm` job: Trusted Publishing via OIDC (no NPM_TOKEN), `npm publish --access public --provenance` on `packages/mcp/`. Skip-if-already-published guard.
- **Skill:** deploy-pipeline
- **Reviewers:** code-reviewer, security-auditor, deploy-reviewer
- **Verify-smoke:** create a draft release in a forked repo (or use `act` if available) and confirm SHA256SUMS asset appears + shim publish job runs end-to-end
- **Files to modify:** `.github/workflows/release.yml`
- **Files to read:** `.github/workflows/release.yml:14-216` (full file — modifications span build matrix + release job + new publish-mcp-shim)

### Audit Wave

#### Task 12: Code Audit
- **Description:** Full-feature code quality audit. Read all source files created/modified in Tasks 1-11. Review holistically for cross-component issues: SQLite lock discipline (no .await held across mutex), Arc<McpState> singleton compliance, error code conventions (-32xxx ranges consistent with existing -32001/-32010/-32011 helpers), no `unwrap()` outside tests, manifest content quality (positive AND negative triggers in `attest.md`). Write audit report.
- **Skill:** code-reviewing
- **Reviewers:** none

#### Task 13: Security Audit
- **Description:** Full-feature security audit. Read all source files created/modified in Tasks 1-11. Analyze for OWASP Top 10 + protocol-specific: bearer allowlist correctness (no new methods accidentally exposed beyond intended four), SHA256 verification correctness in shim's binary download path, no token leakage in logs, visibility filter cannot be bypassed via SQL injection through the recall query path, install path doesn't follow symlinks out of `~/.local/share`. Write audit report.
- **Skill:** security-auditor
- **Reviewers:** none

#### Task 14: Test Audit
- **Description:** Full-feature test quality audit. Read all test files created in Tasks 1-11. Verify: unit-test coverage of resolvers + migration + Visibility enum roundtrip; integration tests assert the actual JSON-RPC error codes and `data` shapes (not just error presence); shim tests exercise SHA256 mismatch path (negative case); netns-isolated offline test is genuinely network-namespace-isolated (not just `--offline`); test pyramid balance (no over-mocked integration tests). Write audit report.
- **Skill:** test-master
- **Reviewers:** none

### Final Wave

#### Task 15: Pre-deploy QA
- **Description:** Acceptance testing: run `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `npx vitest run` in packages/mcp/ and packages/cli/. Verify each user-spec acceptance criterion (AC1–AC17) and each tech-spec criterion against a freshly-built local binary + shim install in a tempdir HOME. Cross-check verification-table rows 1–14 from user-spec "Как проверить" pass against the locally-running binary.
- **Skill:** pre-deploy-qa
- **Reviewers:** none

#### Task 16: Deploy (tag + publish)
- **Description:** Bump versions: `mcp/Cargo.toml` (binary), `packages/sdk/package.json`, `packages/cli/package.json`, `packages/mcp/package.json`. Update CHANGELOG. Tag `v<x.y.z>` and push. CI release.yml emits artifacts + SHA256SUMS + publishes SDK, CLI, and new `@mnemonik-xyz/mcp`. Watch the Trusted Publishing flow complete. Update `dist/binary-version.json` in the shim to reference the new tag.
- **Skill:** deploy-pipeline
- **Reviewers:** none

#### Task 17: Post-deploy verification
- **Description:** Live-environment checks against `mcp.mnemonik.xyz` after server rebuild + `@mnemonik-xyz/mcp` after npm publish:
  - Anonymous discovery via MCP Inspector — tool: `npx @modelcontextprotocol/inspector https://mcp.mnemonik.xyz/mcp`
  - Anonymous recall filter against production DB (seeded test fixture earlier) — tool: curl
  - `npm install -g @mnemonik-xyz/mcp` from a fresh tempdir HOME on macOS; `mnemonik-mcp install --check`; `mnemonik-mcp install`; open Claude Code and verify the binary spawns + `tools/list` works offline (airplane mode) — tool: bash + manual UI check
  - `mnemonik-mcp doctor` reports all checks pass on the fresh install — tool: bash
  Tools: `curl`, `bash`, MCP Inspector CLI, macOS host.
- **Skill:** post-deploy-qa
- **Reviewers:** none
