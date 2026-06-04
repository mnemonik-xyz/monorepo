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

2. **Rust binary `mcp-stdio` subcommand.** Add a thin subcommand on the existing `mnemonic-mcp` binary. It reuses the same `Arc<McpState>` wired by `run_stdio()` (`main.rs:576-617`), but dispatches between two routes per request: discovery and local-mode tool calls execute against the binary's own state (fastembed + SQLite + COSE), participate-mode tool calls proxy via HTTPS to `mcp.mnemonik.xyz/mcp`. Token storage stays in `~/.mnemonic/token.json` (today's path written by Node CLI at `packages/cli/src/config.ts:39-65`); the binary reads/writes the same file via a thin `token_store.rs` wrapper. Moving tokens into OS keychain is deferred to v1.1 per Decision 7.

3. **npm shim package `@mnemonik-xyz/mcp`.** New package under `packages/mcp/`. Two responsibilities: (a) on `npm install -g`, download the platform-matching `mnemonic-mcp` artifact from GitHub Releases, verify its checksum against a `SHA256SUMS` manifest emitted by the release pipeline, install it as `mnemonik-mcp` in the platform-standard bin location; (b) expose three subcommands — `install` (PNL-pattern host-config wiring), `mcp-stdio` (passthrough to the underlying binary, for host subprocess use), `doctor` (diagnostic). `release.yml` (`.github/workflows/release.yml:14-216`) gets two additions: a SHA256SUMS step in the build matrix's release job, and a new npm publish step for the shim alongside the existing SDK+CLI publish.

The Node CLI (`@mnemonik-xyz/cli` v0.2.x) is **unchanged** for tokens in v1 — keeps `loadToken`/`saveToken` (`packages/cli/src/config.ts:367,399`) on `~/.mnemonic/token.json` exactly as today. Both runtimes share the same file path; no migration. Keychain move deferred to v1.1 per Decision 7.

## Architecture

### What we're building/modifying

- **`mcp/assets/skills/` (new)** — seven manifests (`help.md`, `init.md`, `recall.md`, `attest.md`, `checkpoint.md`, `verify.md`, `status.md`). Single source of truth; build-time projection to MCP surfaces. No runtime fetching.
- **`mcp/src/mcp.rs` (edit)** — extend `handle_request_with_resolved_mode()` dispatch (`mcp.rs:526-547`) with four new arms (`prompts/list`, `prompts/get`, `resources/list`, `resources/read`); enrich `tool_definitions()` (`mcp.rs:427-497`) descriptions from manifests; surface `embedder.model_id` + `embedder.model_version` in the `initialize` response (`mcp.rs:526` arm); two new typed-arg resolvers (`resolve_visibility`, `resolve_allow_fallback`) used from `handle_tool_call` (`mcp.rs:1054-1088`).
- **`mcp/src/oauth/mod.rs` (edit)** — extend `ALLOWLIST_METHODS` (`oauth/mod.rs:1235`) with the four discovery method names.
- **`mcp/src/main.rs` (edit)** — register a `mcp-stdio` clap subcommand that calls existing `run_stdio()` path (no behavioral change for default; the subcommand is an alias and a hook for future dual-routing).
- **`mcp/src/tools.rs` (edit)** — `sign_memory()` (`tools.rs:240`) accepts `visibility` + `allow_fallback_to_participate` typed args; routes default-no-soft-fall vs explicit-opt-in escalation through `confirm_delivery_or_demote`-like helper; rejects `mode=local + visibility=...` with `invalid_params`; `recall` (or its anonymous variant) filters by `visibility='public'` for unauthenticated callers.
- **`core/src/storage/sqlite.rs` (edit)** — new `migrate_visibility_column()` following `migrate_write_mode_column` pattern (`sqlite.rs:282-350`); extends `save_attestation` (`sqlite.rs:488-532`) with a `visibility` parameter; `search` (`sqlite.rs:596-653`) gets an optional `visibility_filter` arg.
- **`core/src/storage/traits.rs` (edit)** — `AttestationStore::save_attestation` signature gains `visibility: Visibility`.
- **`core/src/identity/token_store.rs` (new)** — file-based token helpers (read/write/delete) over `~/.mnemonic/token.json`; returns typed `-32099 TokenExpired` when `expires_at` precedes current time.
- **(No new Node-side file)** — Node CLI's existing `loadToken`/`saveToken` (`packages/cli/src/config.ts:367,399`) is unchanged in v1.
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
| `~/.mnemonic/token.json` file | Rust binary (cache after OAuth-loopback) or Node CLI (login) — whichever runs first | Rust binary (read on every authenticated proxy call), Node CLI (read on `whoami` / `recall` / `sign`) | 1 per user — shared between both runtimes |
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

### Decision 4: Soft-fall is explicit opt-in via `allow_fallback_to_participate: bool` (default `false`), escalation visible in response, visibility re-resolves on escalation
**Decision:** New typed `bool` arg on `sign_memory`. Default `false`. When `false` and local execution fails, `sign_memory` returns the underlying JSON-RPC typed error. When `true` and local fails, `sign_memory` re-dispatches the same arguments through the participate path (which auto-triggers OAuth-loopback if no cached token); response includes `escalated: { from: "local", to: "participate", reason: "<machine readable enum>" }`. Stderr line logged. If escalation itself fails (no network), the error returned is the *escalation* failure (`-32011` family for hosted unavailable), not the original local-failure code — agent sees the actual failure point.

**Critical**: visibility resolution runs AGAIN after escalation. The caller's original `mode=local` rejected `visibility=public` per Decision 3 (AC14). After escalation, `mode` is now effectively `participate` — `visibility=public` is now legal but Decision 5b's public-write confirmation gate still applies. So the path `mode=local + visibility=public + allow_fallback_to_participate=true` can only succeed if the call also carries `public_write_confirmation` matching the bound-challenge contract — otherwise post-escalation visibility resolution returns `-32095 PublicWriteRequiresConfirmation` and the escalation is undone (no chain write). This closes the security-audit finding "allow_fallback composes with visibility-rejection bypass."

**Rationale:** Serves user-spec AC5 + AC6. Closes adequacy validator round 1 finding ("silent escalation surprises the caller") and security audit round 1 finding ("allow_fallback bypasses visibility rejection").
**Alternatives considered:** (a) Always silent soft-fall — rejected. (b) Always loud fail — rejected because user-spec carves out the opt-in. (c) Disallow `visibility=public` with `allow_fallback=true` entirely — rejected because forcing the agent to know in advance that local will fail defeats the opt-in's purpose.

### Decision 5b: Server-side public-write confirmation gate (HMAC-bound token)
**Decision:** Any `sign_memory` with `mode=participate + visibility=public` requires an additional `public_write_confirmation` field. Flow:

1. Agent calls new `request_public_write_confirmation { content_hash }` tool. The tool **requires JWT auth** (NOT in `ALLOWLIST_METHODS`), so `owner_pubkey` is derived server-side from `claims.sub` — never client-supplied.
2. Server mints `confirmation_token = HMAC_SHA256(secret, content_hash || owner_pubkey || visibility || expires_at || jti)`. `jti` is a 128-bit random nonce. `expires_at` is now+5min. Returns `{confirmation_token, jti, expires_at}` bound to that specific content + owner + visibility.
3. Agent surfaces the content to the user; on approval, supplies `confirmation_token` (and `jti`) to `sign_memory`.
4. Server verifies: HMAC reconstructs from request fields, `claims.sub == owner_pubkey_from_request`, token not yet consumed, not expired. Stores `jti` in single-use ledger (DashMap, in-process). On any check failure: `-32095 PublicWriteRequiresConfirmation { content_hash, suggested_action }`.

**HMAC secret lifecycle:** process-scoped, random-generated at `McpState::build()`, never persisted. Server restart invalidates all in-flight tokens — this is intentional. Tokens are single-use *and* short-lived, so loss-on-restart is a graceful-degradation (agent re-runs the ceremony, ~3s user friction) rather than a security incident.

**DashMap eviction:** background task sweeps expired `jti` entries every 60s to prevent unbounded growth under abuse. The single-use removal happens under atomic compare-and-swap (DashMap `remove_if`), so two parallel `sign_memory` calls presenting the same token: exactly one wins.

**Horizontal scaling note:** if `mcp.mnemonik.xyz` ever scales to >1 instance, the in-process DashMap becomes incorrect (token minted on instance A, consumed on instance B → not found → false rejection). v1 operates as single-instance per the deployment description; multi-instance migration requires moving the ledger to Redis or signed-cookie equivalent — flagged as v2 concern.

**Rationale:** Serves user-spec R1 ("server-side public-write confirmation gate"). HMAC binding tuple includes `visibility` to future-proof against enum extension, `owner_pubkey` to prevent cross-owner replay (security audit round-2 critical), `jti` for single-use enforcement against TOCTOU windows, `expires_at` for TTL. JWT-required mint ensures the owner identity is server-authoritative, not client-supplied.
**Alternatives considered:** (a) Skill-manifest only — rejected per round-1 audit (prompt injection bypass). (b) `confirmed: true` bool — rejected (zero replay protection). (c) Persistent ledger in SQLite — rejected for v1 (extra table, lock-discipline cost) since in-process restart loss is acceptable.

### Decision 5: Anonymous recall filters `WHERE visibility = 'public'` via new `search` arg
**Decision:** Extend `SqliteStore::search` (`sqlite.rs:596-653`) to accept an optional `visibility_filter: Option<Visibility>`. When `Some(Visibility::Public)`, the WHERE clause adds `AND a.visibility = 'public'`. `recall` tool handler passes `Some(Public)` iff the caller has no JWT (`Claims` from `bearer_auth_middleware` is `None`); otherwise `None` (authenticated callers see all their own rows).
**Rationale:** Serves user-spec AC13. Reuses `bearer_auth_middleware`'s existing claim-or-no-claim distinction; no new auth surface.
**Alternatives considered:** Two separate SQL queries (anonymous variant fully separate) — rejected because the join + cosine-distance core stays identical; the filter is one AND clause.

### Decision 6: `mcp-stdio` subcommand on the existing Rust binary, not a separate binary
**Decision:** Add a `mcp-stdio` subcommand to `mnemonic-mcp`'s clap parser in `main.rs`. The subcommand re-uses the existing `run_stdio()` (`main.rs:576-617`) entry point (already feeds `Arc<McpState>` into the same `handle_request` dispatcher used by the HTTP path). No new state-construction code; default `MNEMONIC_HOSTED_ENDPOINT=https://mcp.mnemonik.xyz/mcp` env var allows the binary to know its hosted peer for participate-mode proxying.
**Rationale:** Serves user-spec "Что делаем" Кусок 3 item 12. Reuses the entire existing tool pipeline; no behavioral drift between the same tool executed by mcp.mnemonik.xyz and locally. `Arc<McpState>` is already shared between `run_stdio` and `run_http` per code-research §10.
**Alternatives considered:** Separate binary `mnemonic-mcp-stdio` — rejected because it doubles release surface and risks code drift; the existing dispatcher handles everything we need.

### Decision 7: Token storage stays in `~/.mnemonic/token.json` for v1 — keychain move deferred to v1.1
**Decision:** Tokens continue to live in `~/.mnemonic/token.json` as today (`packages/cli/src/config.ts:39-65`, `loadToken`/`saveToken` at lines 367/399). The Rust binary's new `mcp-stdio` subcommand reads/writes the same file via a thin `core/src/identity/token_store.rs` that wraps the existing path. No keychain coordinate is added for tokens in v1.

**Token TTL still applies**: Cached tokens remain valid until `expires_at` from the JWT claim (1h post-issue). Pre-expiry calls reuse the cached token. On expiry, the read path returns `-32099 TokenExpired`; the agent re-initiates participate flow which triggers OAuth-loopback. This behavior is independent of where the token is stored.

**Rationale:** Serves user-spec AC11 (1h TTL semantics) at minimum cost. Tokens are short-lived (1h) and re-OAuth-loopback on expiry is cheap — moving them into OS keychain adds (a) coordinated Rust + Node release of two keychain wrappers, (b) keychain-unlock UX cost on each cold-start (interactive on Linux Secret Service in particular), (c) test fixture complexity, in exchange for closing a window of plaintext exposure for a credential that's already short-lived. The cost/benefit doesn't carry v1.

The identity-vs-token asymmetry (identity in keychain via PRs #154/#157, token in plaintext) is a deliberate scope reduction documented in User-Spec Deviations and the user-spec's v1.1 Follow-ups section so we don't lose it.
**Alternatives considered:** (a) Move to keychain per the original draft — rejected for v1 per user direction; deferred to v1.1. (b) Encrypted file with key in keychain — rejected (key-management layer with zero gain).

### Decision 8: npm shim downloads pinned-tag binary from GitHub Releases on install, verifies against pipeline-emitted SHA256SUMS + GitHub artifact attestation pinned to our publisher identity
**Decision:** `@mnemonik-xyz/mcp` ships `bin/mnemonik-mcp` as a stub that invokes lazy install (no `postinstall` script — those run with unrestricted ambient permissions and silently fail under `--ignore-scripts`). The first invocation runs `install-binary.ts`, which:

1. Consults `dist/binary-version.json` (committed in the shim) for the target tag.
2. Downloads `mnemonic-mcp-${tag}-${target}.tar.gz` and `SHA256SUMS` from `github.com/mnemonik-xyz/monorepo/releases/download/${tag}/...`.
3. Verifies SHA256 against the SHA256SUMS line for that exact filename.
4. **Additionally** verifies via `gh attestation verify <tarball> --owner mnemonik-xyz --repo mnemonik-xyz/monorepo --signer-workflow .github/workflows/release.yml`. The `--owner`, `--repo`, and `--signer-workflow` pins constrain the attestation to OUR pipeline specifically — without them, any GitHub-published attestation over matching bytes would pass (security audit round-2 major). Rejection on missing/mismatched attestation OR missing/mismatched publisher identity.
5. Extracts using zip-slip-hardened tar: every entry's resolved absolute path must be a strict descendant of the cache directory; symlinks are skipped entirely; the tar dep is pinned to a version known to mitigate CVE-2021-32803/04.
6. Cache directory is created with mode `0o700`; binary set executable mode `0o755`.

`mnemonik-mcp doctor` re-verifies the cached binary against a sidecar `manifest.json` written at install time (containing the original SHA256SUMS line, the attestation bundle, AND the publisher identity tuple). The doctor's verify is NOT against a re-download (would be circular per security audit) — it's against the manifest captured at install time.

release.yml gains a `release.checksums` step emitting `SHA256SUMS` and uses GitHub's built-in artifact attestation action (`actions/attest-build-provenance@v1`) before the release publishes.

**Rationale:** Serves user-spec R4 + R6 + AC17. Closes security audit round 1 critical finding ("SHA256SUMS-only is supply-chain weak — same-URL fetch breaks the trust root"). `gh attestation verify` is free, OIDC-rooted, and natively compatible with the Trusted Publishing flow we already use for npm. Stub-install instead of `postinstall` closes the npm install-script attack surface flagged in round 1.
**Alternatives considered:** (a) `postinstall` with SHA256SUMS only — rejected per security audit. (b) Sigstore independently — rejected because GitHub artifact attestation IS Sigstore under the hood, but with less plumbing on our side. (c) Bundle binaries directly in npm — rejected because dual download paths split the verification surface.

### Decision 9: Three host candidates hardcoded (macOS only); install written with full binary path (no `npx`); atomic write + symlink hardening
**Decision:** `install.ts` candidates list: `~/.claude.json`, `~/Library/Application Support/Claude/claude_desktop_config.json`, `~/.cursor/mcp.json`. Each entry written as `"command": "<resolved absolute path to cached mnemonik-mcp>", "args": ["mcp-stdio"]`. Only-if-file-exists guard; absent hosts skipped silently; output reports per-host status AND ends with the line `"If any of these agents is already running, please restart them."` (asserted by AC9 test).

**Atomic write + symlink hardening**: For each candidate path: (1) `lstat()` the target — if it's a symlink whose resolved target is outside the user's home directory, refuse with a clear error; (2) read the contents, deserialize, mutate; (3) serialize, write to `<candidate>.mnemonik.tmp` in the same directory; (4) `fsync()` the temp file; (5) `rename()` over the original. The temp-then-rename guarantees no partial-write window where the host could read a corrupted JSON.

**TTY confirmation for agent-headless invocation**: `install` defaults to non-interactive (writes immediately). A new `--confirm` flag forces an interactive prompt before each host. Agents that headlessly invoke `install` see the write happen; users who want a TTY prompt opt in via `--confirm`. This is documented in `mnemonik-mcp install --help` and in the `mnemonik-attest` skill manifest's "guardrails" section.

**Rationale:** Serves user-spec Кусок 2 items 6-8 and AC7-AC10 (especially the previously-unverified restart-instruction line). Closes security audit round 1 majors ("no symlink check", "no atomic write", "agent headless invocation"). Direct binary path keeps the host's subprocess spawn offline (user-spec AC3 forbids `npx -y`).
**Alternatives considered:** `npx -y @mnemonik-xyz/mcp mcp-stdio` — rejected per adequacy round 1. Always-interactive install — rejected because the headline flow is one-command install; mandatory TTY breaks scripting.

### Decision 10: `initialize` response surfaces embedder identity for client-side parity warning
**Decision:** `initialize` arm in `mcp.rs:526` extended to include `embedder: { model_id: <Embedder::model_id()>, model_version: "<env or build constant>", dim: <Embedder::dim()> }`. The Rust binary running as `mcp-stdio` reads its own embedder's values at startup; on first `initialize` reply from a remote `mcp.mnemonik.xyz` (when proxying participate-mode discovery, though discovery is local by Decision 1), compares; mismatch logged to stderr as `[mnemonik-mcp] embedder version mismatch: local=<x> remote=<y>, cross-mode recall not guaranteed consistent`.
**Rationale:** Serves user-spec AC15. Decoupled-versioning surface using the `Embedder` trait that already exposes `model_id()` and `dim()` (code-research §10).
**Alternatives considered:** Force lockstep version pin between binary and `mcp.mnemonik.xyz` — rejected per user-spec ("documented that cross-mode recall semantics undefined at mismatch").

### Decision 11: `Visibility` enum lives in `core/src/storage/mode.rs` next to `WriteMode`
**Decision:** New `pub enum Visibility { Private, Public }` added to `core/src/storage/mode.rs` (the existing file containing `WriteMode`), with `Display`/`FromStr`/serde impls following `WriteMode` precedent. Wired through `save_attestation`, `search`, `SearchResult`, and the SQLite codec. Default-derived `Visibility::default()` returns `Private`.
**Rationale:** Established pattern in the repo (`WriteMode` is the closest analogue); same file makes the pair discoverable. **[TECHNICAL]** — derived from user-spec requirement to track visibility; the exact module placement is a code-organization detail.

### Decision 12: `MNEMONIC_HOSTED_ENDPOINT` env override is gated behind an explicit `--allow-custom-endpoint` flag
**Decision:** The binary's default hosted peer is the compile-time-baked constant `https://mcp.mnemonik.xyz/mcp`. The `MNEMONIC_HOSTED_ENDPOINT` env var is read ONLY when the binary is invoked with `--allow-custom-endpoint`. Without the flag, the env var is ignored (with a single-line stderr warning if it's set). This means local malware that injects the env var into a user's shell cannot silently redirect participate-mode writes + OAuth token exchange to an attacker-controlled server.
**Rationale:** Serves the security audit round 1 major finding ("MNEMONIC_HOSTED_ENDPOINT redirection vector"). Trades a slight ergonomic cost for test/dev (need `--allow-custom-endpoint` in any test that points at a local instance) for closing the attack vector.
**Alternatives considered:** (a) Remove the env var entirely — rejected because integration tests need a way to point at localhost. (b) Read env var unconditionally — rejected per security audit. (c) Require a signed config file — rejected as scope creep.

### Decision 13: Reuse existing WAL + busy_timeout config (already in code), keep 5000ms
**Decision:** `core/src/storage/sqlite.rs:371` already sets `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;` immediately after connection open, before migrations. **No change needed.** On `SQLITE_BUSY` after the 5000ms internal retry window, the `save_attestation` callsite catches the error and surfaces typed JSON-RPC `-32099 LocalStorageBusy { retry_after_ms: 500 }` so the agent retries with backoff.
**Rationale:** Serves user-spec R8 ("точные значения — tech-spec") by **referencing** the existing 5000ms value rather than introducing a new one. The existing 5000ms was tuned for payment race tests (comment in `sqlite.rs:360-363`); regressing it to 2000ms would risk breaking those tests. The 5000ms upper bound is still well within an interactive agent's tolerance.
**Alternatives considered:** (a) Lower to 2000ms (my round-1 draft) — rejected by mirage validator round 2; breaks existing payment race tests. (b) Outer Rust-level retry loop — rejected as duplicate-effort over SQLite's built-in. (c) Higher (10s) — too long for an interactive agent path.

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

### Error Catalogue (canonical list for this feature)

The user-spec AC16 says "точный list — tech-spec". This is that list. Every entry has a matching parametrized integration test in `mcp/tests/error_catalogue.rs` that triggers the condition and asserts `code`, `data.kind`, and each documented data field.

| Code | `data.kind` | Trigger condition | `data` fields |
|------|-------------|-------------------|---------------|
| `-32602` | `"InvalidParams"` | `sign_memory { mode: "local", visibility: ... }` | `field: "visibility"`, `received: "<value>"` |
| `-32602` | `"InvalidParams"` | `sign_memory { allow_fallback_to_participate: <non-bool> }` | `field`, `received` |
| `-32010` | `"UnsupportedMode"` | `mode=participate` on local-only deploy | `requested`, `supported` |
| `-32011` | `"DeliveryNotConfirmed"` | Existing; unchanged | `arweave_tx`, `solana_tx`, `stage`, `row_demoted_to: "local"`, `attestation_id` |
| `-32011` | `"DeliveryQuotaExceeded"` | Existing; unchanged | `window_secs`, `threshold` |
| `-32011` | `"HostedUnavailable"` | mcp-stdio's participate proxy cannot reach `MNEMONIC_HOSTED_ENDPOINT` | `last_error`, `retry_after_ms` |
| `-32095` | `"PublicWriteRequiresConfirmation"` | `mode=participate + visibility=public` without `public_write_confirmation` (Decision 5b) | `content_hash`, `suggested_action` |
| `-32096` | `"OAuthTimeout"` | OAuth-loopback exceeded `MNEMONIC_OAUTH_TIMEOUT_SECS` (default 120s) without callback | `sign_url`, `expires_at`, `attempted_at` |
| `-32098` | `"EmbedderInvalid"` | Local embedder cannot produce vectors (model missing, corrupted, ONNX crash) | `reason`, `repair_hint`, `fallback_available` |
| `-32099` | `"LocalStorageBusy"` | SQLite `SQLITE_BUSY` after 2s busy-timeout | `retry_after_ms` |
| `-32099` | `"TokenExpired"` | Cached token's `expires_at` precedes current time on read | `expires_at`, `pubkey` |
| `-32094` | `"IdentityBootstrapFailed"` | `ensure()` fails (no keychain, no file fallback) | `reason`, `repair_hint` |

### Token file (both runtimes — v1)

- Path: `~/.mnemonic/token.json` (existing per `packages/cli/src/config.ts:64`)
- Shape: `{jwt: string, expires_at: number (unix seconds), sub: string}` (existing per `TokenJson` interface at `config.ts:39-65`)
- Read/write: Rust binary via new `core/src/identity/token_store.rs`; Node CLI via existing `loadToken`/`saveToken` (unchanged)
- Keychain coordinates `xyz.mnemonik.token/default` are NOT used in v1; reserved for v1.1 when the migration lands.

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

- (Rust) **`hmac`** + **`sha2`** for the confirmation-token HMAC in Decision 5b. `sha2` is already a transitive dep via blake3/solana-sdk; `hmac` is small and well-audited (RustCrypto). Alternative `blake3::keyed_hash` was considered but `hmac::Hmac<sha2::Sha256>` is the more conventional choice and matches the JWT HS256 plumbing already in `mcp/src/oauth/mod.rs`. Other workspace deps (`keyring`, `serde`, `serde_json`, `rusqlite`, `tracing`, `clap`, `dashmap`) are already in. Skill markdown parsing uses string-slicing on `## Purpose` / `## Trigger` headers — no new crate.
- (Node, `packages/mcp/`) **`tar`** — for extracting tarball downloads. Standard `decompress` alternatives have native deps that defeat the purpose. **`undici`** — Node 22's built-in fetch is sufficient (Node 22+ baseline matches existing CI).
- (Node, `packages/mcp/` devDeps) — TypeScript tooling matching `packages/cli/`'s setup (`typescript`, `vitest`, `tsx`).

### Using existing (from project)

- `core/src/identity/keystore_os.rs` — existing `OsKeyStore` wraps `keyring::Entry` for identity (NOT touched by this feature; token storage uses a file, not keychain, in v1).
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
- Token store (Rust): write/read/delete roundtrip against tempdir HOME; expired-token returns None; malformed JSON returns None (not panic).
- HMAC confirmation token internals: (a) HMAC secret is process-scoped — instantiate two `McpState` instances back-to-back, tokens minted by instance A are rejected by instance B. (b) Atomic single-use: spawn two tokio tasks both calling the consume path on the same token via `DashMap::remove_if`; exactly one returns `Ok`, other returns `Err(Replayed)`. (c) TTL eviction: advance test clock past `expires_at`, run the eviction sweep; the entry is removed from the DashMap (not just marked expired) so the map can't grow unboundedly.
- `install-binary.ts`: SHA256 verify accepts matching digest, rejects mismatch (table-driven). `gh attestation verify` invocation is constructed with `--owner mnemonik-xyz --repo mnemonik-xyz/monorepo --signer-workflow .github/workflows/release.yml`; mock the gh CLI binary to return success only when all three flags are present.
- `install-hosts.ts`: JSON merge preserves unrelated keys (table-driven); idempotent on re-run; absent file silently skipped; symlink-out-of-home refuses to write; all-absent paths exit 0 with "no host configs found"; `--check` mode mtime unchanged + no temp files.

### Integration tests

- **Anonymous discovery (AC1):** spin up `mnemonic-mcp` locally in `STORAGE_MODE=local`, POST `initialize` / `prompts/list` / `resources/list` / `tools/list` from `reqwest` without `Authorization`. Assert 7 prompts (exact, per Decision 1), 7 resources (exact), 7 tool descriptions (6 pre-existing + new `request_public_write_confirmation` from Decision 5b), each of the 6 pre-existing tools' descriptions containing the literal Purpose+Trigger sub-strings from `mcp/assets/skills/<corresponding>.md` (byte-for-byte snapshot check), no manifest body containing placeholder tokens `TBD|TODO|XXX|FIXME`. (Note: `request_public_write_confirmation` is in `tools/list` count but is gate infrastructure; its description doesn't need a skill manifest.)
- **Anonymous recall filter (AC13):** seed DB via direct SQL with one row `visibility='private'` matching a query string and one row `visibility='public'` matching the same string. Call `recall` without `Authorization`. Assert the response contains only the public row.
- **Local recall finds local-written (AC4):** with `STORAGE_MODE=local` binary, `sign_memory { mode: "local", content: "distinctive test string XYZ" }` → capture `attestation_id`. Then `recall { query: "distinctive test string XYZ", limit: 5 }` → assert `results[0].attestation_id == captured_id` AND `results[0].score > 0.5`. Variant: write 2 unrelated + 1 target, recall on target text, assert target top-1 with score gap > 0.1 over runners-up. Exercises embed → compress → store → search round-trip.
- **Visibility rejected on local writes (AC14):** call `tools/call sign_memory` with `mode=local, visibility=public`. Assert JSON-RPC error code `-32602`, `data.kind == "InvalidParams"`, `data.field == "visibility"`.
- **Soft-fall opt-in semantics (AC5/AC6):** inject failure via test-only `McpState` constructor variant that takes `Box<dyn Embedder>` directly; pass a `FailingEmbedder { fail_on_call: 1 }` test impl in `core/tests/support/`. For sub-cases (b)/(c), additionally use a `wiremock`-rs HTTP mock bound to a local port set as `MNEMONIC_HOSTED_ENDPOINT` with `--allow-custom-endpoint`. (a) call `sign_memory` with `allow_fallback_to_participate=false` → assert typed `-32098 EmbedderInvalid` AND zero outbound TCP (`unshare -rn` test). (b) Call with `allow_fallback_to_participate=true` + reachable wiremock + `visibility=public + public_write_confirmation=<valid token>` → assert success, `escalated` field present, AND `wiremock.received_requests().len() == 1` (escalation actually proxied; not stubbed). (c) Call with `allow_fallback_to_participate=true + visibility=public` WITHOUT `public_write_confirmation` → assert post-escalation re-resolution returns `-32095 PublicWriteRequiresConfirmation` (validates Decision 4 + 5b interaction).
- **Public-write confirmation gate (Decision 5b):** (a) `request_public_write_confirmation` called WITHOUT JWT → `-32001 unauthorized` (NOT in `ALLOWLIST_METHODS`). (b) call with JWT → returns `{confirmation_token, jti, expires_at}`. Pass it to `sign_memory` with the same JWT → success. (c) Replay same token (second consume) → `-32095`. (d) Pass a token issued for a different `content_hash` → `-32095`. (e) Token expired (advance clock past `expires_at`) → `-32095`. (f) **Cross-owner replay**: authenticate as owner A and mint a token for content_hash H; then authenticate as owner B (different JWT, different `claims.sub`) and call `sign_memory { content_hash: H, public_write_confirmation: A's token }` → `-32095` (HMAC reconstruction fails because B's owner_pubkey from JWT doesn't match the A's owner bound in the HMAC tuple). (g) Concurrent consumption race: spawn two tokio tasks both presenting the same valid token; exactly one succeeds, the other gets `-32095`.
- **OAuth-loopback + cached-token reuse (AC11):** (a) Fresh-install path: no `~/.mnemonic/token.json` present, mock OAuth server, first `sign_memory` triggers loopback and persists token to file; second `sign_memory` within TTL succeeds without invoking the OAuth mock (mock call count == 1). (b) Token expired path: first call within TTL succeeds (mock count 1); advance clock past `expires_at`; second call returns `-32099 TokenExpired`, agent re-initiates loopback. (c) Corrupted token path: write malformed bytes (truncated JSON, missing `expires_at` field) to `~/.mnemonic/token.json`; assert `token_store.read_token` returns `None` (NOT a panic), OAuth-loopback is re-initiated. Token storage remains in `~/.mnemonic/token.json` per Decision 7.
- **Install idempotent + non-destructive (AC7, AC8):** create synthetic configs with unrelated `mcpServers` entries; run `install.ts` twice; diff against original — only `mnemonik` key added/replaced, unrelated entries byte-identical.
- **Install resilient (AC9):** All three candidate paths absent → install exits 0, stdout reports "no host configs found", stderr empty. 1-of-3 present → only the present one modified, other two untouched (`lstat()` mtime check before/after).
- **Install --check (AC10):** populate tempdir candidates; capture mtime_ns for each; run `mnemonik-mcp install --check`; assert mtime_ns unchanged AND no `.mnemonik.tmp` files in any candidate dir AND stdout contains a per-host plan line.
- **Install restart instruction (AC9 supplemental):** `mnemonik-mcp install` output's final line equals `If any of these agents is already running, please restart them.` (exact string match).
- **Install symlink hardening (Decision 9):** create `~/.claude.json` as a symlink whose target resolves outside `$HOME` (e.g., `/tmp/foo`); run `mnemonik-mcp install` → assert error and the target file is NOT modified.
- **MNEMONIC_HOSTED_ENDPOINT gating (Decision 12):** with `MNEMONIC_HOSTED_ENDPOINT=http://attacker.example` set in env: (a) `mnemonic-mcp mcp-stdio` (no flag) → participate path uses default endpoint, stderr contains warning about ignored env var. (b) `mnemonic-mcp mcp-stdio --allow-custom-endpoint` → uses the env value.
- **Doctor (AC17):** (a) Happy path — clean fixture with valid host config + identity + token file → exit 0, output names all 6 checks as `pass`. (b) Parametrized failures — for each of {missing-host-config, mcp.mnemonik.xyz/health unreachable (mock 503), corrupted-cached-binary, locked-SQLite, denied-keychain-identity, corrupted-token-file (truncated `~/.mnemonic/token.json`)}: induce that failure, assert exit 1 AND the corresponding check is `fail` AND `repair_hint` non-empty for that check.
- **Error Catalogue coverage (AC16):** parametrized test iterating each row of the Error Catalogue table; for each row, triggers the condition **via the production code path that would naturally surface that error** (not a hand-crafted response stub) and asserts `error.code`, `error.data.kind`, and each documented `data` field is present with documented type. Per-row trigger fixtures (codified in the test harness, each row maps to one): `held_sqlite_lock` (LocalStorageBusy), `broken_embedder` (EmbedderInvalid via `FailingEmbedder`), `denied_keychain` (IdentityBootstrapFailed via mock KeyStore), `hung_oauth_mock` (OAuthTimeout via wiremock with delayed response), `unreachable_hosted` (HostedUnavailable via wiremock without a route), `expired_clock` (TokenExpired via `tokio::time::advance`), `confirmation_gate_violation` (PublicWriteRequiresConfirmation via missing/replayed token), `malformed_args` (InvalidParams via crafted JSON). A row is considered covered ONLY if the test induces the condition this way, not by hand-crafting a response.
- **migrate_visibility_column idempotency (Decision 2):** clean DB → column 'visibility' present with default 'private' AND `idx_attestations_visibility` index present. DB that already has the column → re-run produces identical `pragma_table_info` output. DB with legacy rows lacking the column → all rows backfilled to 'private'.

### E2E tests

- **Offline cold-start of local sign (AC3):** spawn `mnemonic-mcp mcp-stdio` inside `unshare -rn` (new network namespace with only `lo`), drive via JSON-RPC on stdin, assert (a) `sign_memory { mode: "local" }` returns success, (b) `ss -tn state established` inside the netns shows zero established non-loopback connections, (c) `strace -e trace=connect` on the process records no `connect()` calls to a non-loopback address. Removes the `--offline`-vs-netns ambiguity flagged by round 1 test review.
- **Embedder parity surface (AC15):** start binary, observe `initialize` response carries `embedder.model_id` and `embedder.model_version`; mock a hosted-peer `initialize` response with mismatched values; assert stderr contains exact warning line `[mnemonik-mcp] embedder version mismatch: local=<x> remote=<y>, cross-mode recall not guaranteed consistent`.

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

- **AC12 + AC12b dropped from v1 — token storage stays in `~/.mnemonic/token.json`.** User-spec AC12 requires token-in-keychain; AC12b requires migration ceremony. Tech-spec Decision 7 keeps both deferred: tokens continue to live in the existing plaintext file for v1, with the Rust binary reading/writing the same path. **Reason:** tokens are short-lived (1h TTL), re-OAuth on expiry is cheap, and the keychain move adds coordinated Rust + Node release complexity plus keychain-unlock UX cost (especially intrusive on Linux Secret Service) for a closed window of plaintext exposure on a 1h-rotating credential. The identity-vs-token asymmetry (identity in keychain, token in file) is deliberately accepted for v1. → **User explicitly approved this deferral in round-2 review; recorded in user-spec.md "Follow-ups (v1.1+)" so we don't lose it.**
- **Added: `MNEMONIC_HOSTED_ENDPOINT` environment variable** on `mnemonic-mcp` binary, gated behind `--allow-custom-endpoint` flag per Decision 12. Reason: participate-mode proxying needs to know where to send HTTPS calls; tests need redirectability. The flag-gating closes the security-audit redirection vector. → No approval needed — tightens user-spec security posture.
- **Added: `embedder.model_version` field type left to tech-spec.** User-spec AC15 says "embedder.model_id + embedder.model_version" without committing to whether `model_version` is a semver string, ONNX file hash, or build-time constant. Tech-spec picks a build-time constant string for v1. → No approval needed — surface contract unchanged.
- **Added: `embedder.dim` field on `initialize` response (Decision 10).** Not requested by user-spec AC15. Reason: a future client wanting to validate that incoming embeddings are compatible with the server's storage needs the dimensionality alongside the model ID; surfacing it costs zero. → No approval needed — additive, no semantic change.
- **Added: `request_public_write_confirmation` MCP tool (Decision 5b).** Not in user-spec's enumerated tools. Current `tool_definitions()` exposes 6 tools (whoami, sign_memory, verify, prove_identity, recall, check_pending); after this feature there are 7 (the new one). Reason: user-spec R1 explicitly names "server-side public-write confirmation gate (typed error)" as the privacy mitigation; the gate requires a ceremony, and the ceremony needs an endpoint. The new tool implements that endpoint. → No approval needed — fulfills user-spec R1 mitigation.
- **Added: `--confirm` flag on `mnemonik-mcp install`.** User-spec defines default-apply (`install`) and dry-run (`install --check`). Tech-spec Decision 9 adds an interactive `install --confirm` mode that prompts before each host. Reason: closes the security-audit-round-1 finding "agent can headlessly invoke install" by giving non-agent operators an opt-in interactive path. Default behavior is unchanged (matches user-spec). → No approval needed — additive opt-in, default unchanged.
- **Decision 11 `[TECHNICAL]`: Visibility enum module placement** (`core/src/storage/mode.rs`, alongside `WriteMode`) is pure code organization — user-spec doesn't speak to module paths. Marked `[TECHNICAL]` for clarity but listed here because the methodology requires `[TECHNICAL]` decisions to be acknowledged. → No approval needed — infrastructure detail.
- **Added: `mnemonik-mcp logout` subcommand on the binary's CLI.** User-spec AC12 describes `logout` removing the token (originally specified as keychain entry; in v1 that's the file at `~/.mnemonic/token.json` per Decision 7). The verb wasn't in any task in the round-1 draft (gap caught by completeness validator). Task 5 now wires `logout` as a clap subcommand calling `token_store::delete_token` which `unlink()`s the file. → No approval needed — closes a gap, not a deviation per se.

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
- **Description:** Create `mcp/assets/skills/` directory with seven manifests (`help.md`, `init.md`, `recall.md`, `attest.md`, `checkpoint.md`, `verify.md`, `status.md`). Each manifest has `## Purpose`, `## Trigger`, plus body sections (context, tool, guardrails, examples). Wire `build.rs` to parse the H2 sections and emit string constants for downstream use. Manifest content per user-spec; trigger boundary explicit per R7 mitigation.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo build -p mnemonic-mcp` succeeds; deliberately rename one manifest, expect build to fail
- **Files to modify:** `mcp/build.rs` (new), `mcp/Cargo.toml` (build dep if needed), `mcp/assets/skills/*.md` (new)
- **Files to read:** `mcp/src/mcp.rs:427-497`, `work/agent-native-distribution/user-spec.md`

#### Task 2: All `mcp.rs` server surfaces (prompts + resources + dispatcher + initialize + tools/list enrichment) + anonymous allowlist
- **Description:** Single owner of all `mcp/src/mcp.rs` edits in Wave 1 to eliminate W1 file-collision. Adds four dispatcher arms (`prompts/list`, `prompts/get`, `resources/list`, `resources/read`) using manifest constants from Task 1. Extends `initialize` capabilities (`prompts: {}`, `resources: {}`) AND adds `embedder: { model_id, model_version, dim }` derived from `state.embedder`. Enriches `tool_definitions()` descriptions with Purpose+Trigger from each manifest. Extends `ALLOWLIST_METHODS` in `oauth/mod.rs` with the four new method names.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `curl -s -X POST localhost:3000/mcp -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"prompts/list","params":{}}'` returns 7 prompts without `Authorization`; `curl ... initialize` carries non-empty `embedder.model_id`; `curl ... tools/list` shows tool descriptions containing the literal Purpose+Trigger substrings from the manifest source files (post-Task-1 wiring)
- **Files to modify:** `mcp/src/mcp.rs` (dispatcher + initialize + tool_definitions), `mcp/src/oauth/mod.rs` (allowlist const)
- **Files to read:** `mcp/src/mcp.rs:427-497`, `mcp/src/mcp.rs:499-547`, `mcp/src/oauth/mod.rs:1226-1336`, `mcp/assets/skills/` (Task 1 output), `core/src/embed/mod.rs` (`Embedder` trait)

### Wave 2 (storage + tool args + WAL config — depends on nothing in Wave 1)

#### Task 3: visibility column migration + Visibility enum + storage signatures
- **Description:** Add `pub enum Visibility { Private, Public }` to **existing** `core/src/storage/mode.rs` (alongside `WriteMode`) with Display/FromStr/serde. New `migrate_visibility_column()` mirroring `migrate_write_mode_column`. Extend `AttestationStore::save_attestation` trait signature with `visibility: Visibility`. Extend `search` with optional `visibility_filter`. Wire migration into `SqliteStore::open` and `SqliteStore::in_memory`. **No change to WAL/busy_timeout** — existing 5000ms config at `sqlite.rs:371` is kept per Decision 13.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core sqlite::migration::visibility_column` covers idempotency (clean → column present + index; existing column → no-op; legacy rows → backfilled to 'private')
- **Files to modify:** `core/src/storage/mode.rs` (extend), `core/src/storage/sqlite.rs`, `core/src/storage/traits.rs`, `core/src/storage/mod.rs`
- **Files to read:** `core/src/storage/sqlite.rs:282-350`, `core/src/storage/sqlite.rs:488-532`, `core/src/storage/sqlite.rs:596-653`, `core/src/storage/mode.rs` (existing WriteMode for pattern)

#### Task 4: sign_memory accepts visibility + allow_fallback args; anonymous recall filters public; public-write confirmation gate per Decision 5b
- **Description:** Add `resolve_visibility(args, mode)` and `resolve_allow_fallback(args)` in `tools.rs` following `resolve_write_mode` shape; reject `mode=local + visibility=...` with `invalid_params`. Thread `visibility` through `sign_memory` into `save_attestation`. Add new `request_public_write_confirmation` MCP tool per Decision 5b (NOT in `ALLOWLIST_METHODS` — JWT required at mint); enforce confirmation token on `sign_memory` with `mode=participate + visibility=public`. HMAC + DashMap mechanics per Decision 5b (do not re-architect here). Update `recall` handler to pass `Some(Visibility::Public)` to `search` when caller has no `Claims`. Add typed error catalogue entries from the Error Catalogue table.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test --test error_catalogue` parametrized test exercises every row; anonymous `curl ... recall` returns only public row; `curl ... sign_memory { mode: "local", visibility: "public" }` returns `-32602`; `request_public_write_confirmation` without JWT returns `-32001 unauthorized`
- **Files to modify:** `mcp/src/tools.rs` (resolvers + new tool + error helpers + HMAC token store), `mcp/src/mcp.rs` (`handle_tool_call` arg extraction + dispatcher arm for the new tool), `mcp/Cargo.toml` (add `hmac` + `sha2`)
- **Files to read:** `mcp/src/tools.rs:100-134`, `mcp/src/mcp.rs:1054-1088`, `mcp/src/oauth/mod.rs:1235` (allowlist to NOT extend with the new tool)

### Wave 3 (CLI + binary — depends on Wave 2)

#### Task 5: mcp-stdio + logout subcommands on Rust binary + MNEMONIC_HOSTED_ENDPOINT gating + soft-fall routing
- **Description:** Add clap subcommands `mcp-stdio` (re-uses existing `run_stdio()`) and `logout` (calls `token_store::delete_token`) to `mnemonic-mcp`. Add `--allow-custom-endpoint` flag per Decision 12; without it, `MNEMONIC_HOSTED_ENDPOINT` env var is ignored (with stderr warning if set). Behavior preserved for default `mnemonic-mcp` invocation. Wire soft-fall routing in `sign_memory`: when `allow_fallback_to_participate=true` and local execution fails, re-dispatch through participate path (HTTPS to resolved endpoint); on success, response carries `escalated: {from, to, reason}`; on hosted unavailability, error is `-32011 HostedUnavailable` (not the original local-failure code); per Decision 4, visibility resolution runs again post-escalation so the public-write confirmation gate from Task 4 still applies.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `mnemonic-mcp mcp-stdio` accepts JSON-RPC on stdin; `mnemonic-mcp logout` removes `~/.mnemonic/token.json` (second invocation is idempotent — file already absent, exits 0); with `MNEMONIC_HOSTED_ENDPOINT=http://attacker.example` set in env and no flag, participate path uses default endpoint and stderr contains warning
- **Files to modify:** `mcp/src/main.rs`, `mcp/src/tools.rs` (soft-fall routing), `mcp/src/mcp.rs` (route hookup)
- **Files to read:** `mcp/src/main.rs:576-617`, `mcp/src/tools.rs:240-...`

#### Task 6: Token-file access for Rust binary + TokenExpired typed error
- **Description:** New `core/src/identity/token_store.rs` exposes `read_token() -> Option<TokenJson>`, `save_token(&TokenJson)`, `delete_token()` reading/writing the same `~/.mnemonic/token.json` file used by the existing Node CLI. Honor `expires_at` — return `-32099 TokenExpired` when current time exceeds `expires_at`. Update Rust callsites in `mcp/src/oauth/mod.rs` to delegate. Node CLI's `loadToken`/`saveToken` (`config.ts:367, 399`) unchanged. **No keychain wrapper** for tokens in v1 per Decision 7.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `cargo test -p mnemonic-core token_store::roundtrip` (write → read → delete) passes against a tempdir HOME; expired-token fixture returns `-32099 TokenExpired`
- **Files to modify:** `core/src/identity/token_store.rs` (new), `core/src/identity/mod.rs` (re-export), `mcp/src/oauth/mod.rs` (callsites)
- **Files to read:** `packages/cli/src/config.ts:39-65` (shared file shape + path), `packages/cli/src/config.ts:367,399`

### Wave 4 (npm shim package skeleton + lazy-install + bin entry — depends on Wave 3 Task 5)

#### Task 7: @mnemonik-xyz/mcp shim — full package (bin + lazy-install + hardened download/verify + all subcommands)
- **Description:** Single owner of all `packages/mcp/` content in this wave to eliminate W4 file-collision. Creates `packages/mcp/` with `package.json` (`name: "@mnemonik-xyz/mcp"`, `bin: { "mnemonik-mcp": "./dist/bin/mnemonik-mcp.js" }`), `tsconfig.json`, vitest setup mirroring `packages/cli/`. Bin entrypoint dispatches between `install` / `install --check` / `mcp-stdio` / `doctor`. No `postinstall` script (security hardening per Decision 8) — first invocation lazy-runs install-binary. `install-binary.ts` downloads tarball + SHA256SUMS, verifies hash, runs `gh attestation verify` (rejects on missing/mismatched attestation), extracts with zip-slip-hardened tar (resolves each entry's absolute path, refuses non-descendants, skips symlinks; `tar` dep pinned to known-safe version), caches binary at platform-standard location with mode 0o755 (parent dir 0o700), writes `manifest.json` sidecar with SHA256 + attestation bundle for doctor's later re-verification. `install` subcommand: per Decision 9 — three hardcoded host candidates, lstat-symlink-out-of-home check, atomic tempfile + rename, non-destructive JSON merge, idempotent, output ends with restart-instruction string. `install --check` is dry-run (verified via mtime-unchanged assertion). `doctor` subcommand: six checks (host-config entry presence, /health ping, binary integrity via `manifest.json` not re-download, local SQLite r/w, identity accessibility, **token file integrity** — checks `~/.mnemonic/token.json` is readable, parses as valid JSON, contains `expires_at` field; reports `expired` as soft warning, malformed as hard fail), structured output, exit 0/1, repair hints. `mcp-stdio` spawns the cached binary as subprocess.
- **Skill:** code-writing
- **Reviewers:** code-reviewer, security-auditor, test-reviewer
- **Verify-smoke:** `npx vitest run packages/mcp/` passes; (a) install-binary against fixture tarball + matching SHA256SUMS → success; (b) install-binary against fixture with mismatched SHA → rejects with clear error; (c) `mnemonik-mcp install` against tempdir HOME with unrelated `mcpServers.foo` entry → diff shows `mnemonik` added, `foo` untouched, restart-instruction in stdout; (d) `mnemonik-mcp install --check` against same tempdir → mtime_ns unchanged; (e) symlink-out-of-home test refuses to write
- **Files to modify:** `packages/mcp/package.json` (new), `packages/mcp/tsconfig.json` (new), `packages/mcp/src/bin/mnemonik-mcp.ts` (new — dispatch all subcommands), `packages/mcp/src/install-binary.ts` (new), `packages/mcp/src/install-hosts.ts` (new), `packages/mcp/src/doctor.ts` (new), `packages/mcp/src/mcp-stdio.ts` (new), `packages/mcp/dist/binary-version.json` (new, committed)
- **Files to read:** `packages/cli/package.json`, `packages/cli/bin/mnemonic.ts`

### Wave 5 (release pipeline — depends on Waves 1-4)

#### Task 8: release.yml SHA256SUMS + GitHub artifact attestation + @mnemonik-xyz/mcp publish
- **Description:** Add a step to `.github/workflows/release.yml` that, after all build matrices complete (`needs: [build-linux, build-macos]`), generates `SHA256SUMS` from all collected `mnemonic-mcp-*.tar.gz` artifacts and attaches as a separate release asset. Add `actions/attest-build-provenance@v1` step to emit GitHub-OIDC-rooted artifact attestations for the same artifacts (the shim's install-binary then verifies via `gh attestation verify`). Add a new `publish-mcp-shim` job analogous to `publish-npm`: Trusted Publishing via OIDC (no NPM_TOKEN), `npm publish --access public --provenance` on `packages/mcp/`, skip-if-already-published guard.
- **Skill:** deploy-pipeline
- **Reviewers:** code-reviewer, security-auditor, deploy-reviewer
- **Verify-smoke:** (a) `actionlint .github/workflows/release.yml` is clean; (b) standalone shell snippet for SHA256SUMS step: `tar -czf /tmp/fake.tar.gz README.md && (cd /tmp && sha256sum -b fake.tar.gz > SHA256SUMS) && grep -E "^[0-9a-f]{64} \*?fake.tar.gz$" /tmp/SHA256SUMS` succeeds; (c) optional `act -j publish-mcp-shim --dryrun` if act available
- **Files to modify:** `.github/workflows/release.yml`
- **Files to read:** `.github/workflows/release.yml:14-216`

### Audit Wave

#### Task 9: Code Audit
- **Description:** Full-feature code quality audit. Read all source files created/modified in Tasks 1-11. Review holistically for cross-component issues: SQLite lock discipline (no .await held across mutex), Arc<McpState> singleton compliance, error code conventions (-32xxx ranges consistent with existing -32001/-32010/-32011 helpers), no `unwrap()` outside tests, manifest content quality (positive AND negative triggers in `attest.md`). Write audit report.
- **Skill:** code-reviewing
- **Reviewers:** none

#### Task 10: Security Audit
- **Description:** Full-feature security audit. Read all source files created/modified in Tasks 1-11. Analyze for OWASP Top 10 + protocol-specific: bearer allowlist correctness (no new methods accidentally exposed beyond intended four), SHA256 verification correctness in shim's binary download path, no token leakage in logs, visibility filter cannot be bypassed via SQL injection through the recall query path, install path doesn't follow symlinks out of `~/.local/share`. Write audit report.
- **Skill:** security-auditor
- **Reviewers:** none

#### Task 11: Test Audit
- **Description:** Full-feature test quality audit. Read all test files created in Tasks 1-11. Verify: unit-test coverage of resolvers + migration + Visibility enum roundtrip; integration tests assert the actual JSON-RPC error codes and `data` shapes (not just error presence); shim tests exercise SHA256 mismatch path (negative case); netns-isolated offline test is genuinely network-namespace-isolated (not just `--offline`); test pyramid balance (no over-mocked integration tests). Write audit report.
- **Skill:** test-master
- **Reviewers:** none

### Final Wave

#### Task 12: Pre-deploy QA
- **Description:** Acceptance testing: run `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `npx vitest run` in packages/mcp/ and packages/cli/. Verify each user-spec acceptance criterion (AC1–AC17) and each tech-spec criterion against a freshly-built local binary + shim install in a tempdir HOME. Cross-check verification-table rows 1–14 from user-spec "Как проверить" pass against the locally-running binary.
- **Skill:** pre-deploy-qa
- **Reviewers:** none

#### Task 13: Deploy (tag + publish)
- **Description:** Bump versions: `mcp/Cargo.toml` (binary), `packages/sdk/package.json`, `packages/cli/package.json`, `packages/mcp/package.json`. Update CHANGELOG. Tag `v<x.y.z>` and push. CI release.yml emits artifacts + SHA256SUMS + publishes SDK, CLI, and new `@mnemonik-xyz/mcp`. Watch the Trusted Publishing flow complete. Update `dist/binary-version.json` in the shim to reference the new tag.
- **Skill:** deploy-pipeline
- **Reviewers:** none

#### Task 14: Post-deploy verification
- **Description:** Live-environment checks against `mcp.mnemonik.xyz` after server rebuild + `@mnemonik-xyz/mcp` after npm publish:
  - Anonymous discovery via MCP Inspector — tool: `npx @modelcontextprotocol/inspector https://mcp.mnemonik.xyz/mcp`
  - Anonymous recall filter against production DB (seeded test fixture earlier) — tool: curl
  - `npm install -g @mnemonik-xyz/mcp` from a fresh tempdir HOME on macOS; `mnemonik-mcp install --check`; `mnemonik-mcp install`; open Claude Code and verify the binary spawns + `tools/list` works offline (airplane mode) — tool: bash + manual UI check
  - `mnemonik-mcp doctor` reports all checks pass on the fresh install — tool: bash
  Tools: `curl`, `bash`, MCP Inspector CLI, macOS host.
- **Skill:** post-deploy-qa
- **Reviewers:** none
