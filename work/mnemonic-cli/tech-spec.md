---
created: 2026-04-29
status: approved
size: M
branch: dev
---

# Tech Spec: mnemonic-cli (Phase 1 — SDK + CLI)

## Solution

Ship two pure-ESM npm packages under the `@mnemonik-xyz` scope:

1. **`@mnemonik-xyz/sdk`** — runtime-agnostic JavaScript/TypeScript library that wraps the public Mnemonic MCP HTTP surface. Provides a `MnemonicClient` with the 5 tool methods (`whoami`, `signMemory`, `recall`, `verify`, `proveIdentity`), an OAuth 2.1 + PKCE helper supporting both interactive (browser-spawn) and headless (pre-issued JWT) modes, and a pluggable `Signer` interface (Phase 1 ships only `LocalSigner`; future `TurnkeySigner` / `WebAuthnSigner` are drop-in replacements). Distributed as ESM only. Targets: Node ≥20, Bun, Deno, Cloudflare Workers, modern browsers.

2. **`@mnemonik-xyz/cli`** — Node-only CLI binary built on top of the SDK. Implements 7 commands: `init`, `login` (interactive OAuth or `--token <jwt>` headless), `sign`, `recall`, `verify`, `whoami`, `prove`, plus identity bootstrap subcommand (`identity import --file <path>` / `identity export --file <path>`). Output: human-readable on TTY (ANSI color), `--json` for machine consumption, `--quiet` for CI. Persistence (identity file + JWT) is the CLI's responsibility — SDK is stateless.

Both packages live in a new top-level `packages/` directory (npm workspace, not Cargo workspace). The Rust workspace is unaffected. The existing `core/src/wasm/` build (already producing `pkg/` artifacts via `wasm-pack --target web`) is consumed by the SDK as a private workspace dependency. Investigation of correct wasm-pack target for SDK is part of Task 1 (see Decision 3).

## Architecture

### What we're building / modifying

**New packages (top-level `packages/` directory, npm workspace):**

- `packages/sdk/` — `@mnemonik-xyz/sdk` source. Modules: `client.ts` (MnemonicClient + 5 tool methods), `oauth.ts` (PKCE helper + interactive/headless modes), `signer.ts` (`Signer` interface + `LocalSigner` impl), `cose.ts` (thin wrapper around `@mnemonic/core` WASM `sign_cose_payload` / `sign_challenge`), `keypair.ts` (Keypair JSON parse/serialize/generate via WASM `generate_keypair`), `errors.ts` (typed error hierarchy + redaction helpers), `types.ts` (public TS types). `index.ts` re-exports the public surface.
- `packages/cli/` — `@mnemonik-xyz/cli` source. Modules: `bin/mnemonic.ts` (binary entrypoint), `commands/{init,login,sign,recall,verify,whoami,prove,identity}.ts`, `output.ts` (TTY-aware formatter), `config.ts` (`~/.mnemonic/` paths + persistence with proper file-mode enforcement), `errors.ts`.

**Modified files (server side — Decisions 5 + 7):**

- `mcp/src/oauth.rs` — add a redirect-URI allowlist (does not exist today; current code accepts any `redirect_uri`). Allowlist entries: existing webapp / Cursor / VS Code / Claude.ai redirect schemes plus a regex for `http://127.0.0.1:<port>/callback` and `http://[::1]:<port>/callback` gated to `client_id=mnemonic-cli`. Adding the allowlist is a security improvement over today's behavior. PKCE state is bound to verifier per RFC 7636 §4.4.
- `mcp/src/api.rs` — new endpoint `POST /api/cli-bootstrap/issue` (authenticated via Bearer JWT; issues a one-time bootstrap-ticket signed by the server, TTL 10 min) and `GET /api/cli-bootstrap/redeem/:ticket` (CLI fetches keypair via this ticket exactly once; ticket invalidated after first read or TTL expiry). Tickets are stored in-memory only (an LRU+TTL map keyed by ticket UUID, similar to existing `pending::PendingBundles`).

**Modified files (webapp side — Decision 7):**

- `webapp/src/components/IdentityPanel.tsx` — add a "Send to CLI" button that calls `/api/cli-bootstrap/issue` and displays the resulting one-time ticket as `mnemonic identity import --ticket <uuid>` for the user to paste in their terminal.

**Modified files (build pipeline):**

- `package.json` (repo root) — convert to npm workspace: `"workspaces": ["packages/*", "webapp"]`. Webapp keeps its own `package.json` and standalone build chain.
- `webapp/scripts/build-wasm.sh` — keep existing `--target web` for webapp; add a parallel `wasm-pack` build for SDK consumption (target chosen during Task 1 investigation per Decision 3).
- `core/Cargo.toml` — add a `golden-fixtures` cargo feature flag (does not exist today) used by Task 4 to emit byte-for-byte CBOR/COSE fixtures the SDK validates against.
- `.github/workflows/node-test.yml` (new) — Node 20 / Node 22 / Bun matrix.

**Unchanged (consumed as-is via the existing public MCP HTTP surface):**

- `mcp/src/mcp.rs`, `tools.rs`, `payment.rs`, `pricing.rs` — server-side tool dispatch, payment, pricing untouched. CLI is a third MCP client (alongside Cursor/VS Code/Claude.ai) and uses the same `/mcp`, `/oauth/*`, `/api/sign-callback` endpoints.
- `core/src/wasm/mod.rs` — existing WASM exports (`generate_keypair`, `sign_challenge`, `sign_cose_payload`, `sign_attestation_bundle`, `export_keypair_json`, `import_keypair_json`) cover everything the SDK needs.
- `core/src/codec/canonical.rs` and `core/src/codec/sign.rs` — canonical CBOR + COSE_Sign1 source of truth in Rust; SDK never re-implements them.
- All existing Rust tests, the rest of the MCP server, webapp source.

### How it works

**Onboarding flow (`mnemonic init`):**

1. CLI checks `~/.mnemonic/`. If `identity.json` exists and `--force` is absent, refuse and print existing pubkey.
2. CLI calls `Keypair.generate()` from SDK, which calls WASM `generate_keypair()` (uses `getrandom` with `js` feature → `crypto.getRandomValues`).
3. CLI writes `~/.mnemonic/identity.json` with mode 0600 (Unix). On Windows, uses `node:fs.fchmod` + sets ACLs via `fs-extra`'s `setReadOnlyForOwner` helper or falls back to a `winston-fs-acl` shim — concrete library choice in Task 1.
4. Prints pubkey + DID to stdout.

**Auth flow (`mnemonic login`):**

Interactive (default):
1. CLI generates PKCE verifier (32 bytes from `crypto.getRandomValues`, base64url) + challenge (`SHA-256(verifier)` base64url) + state (32 bytes random base64url).
2. CLI binds a one-shot HTTP server on a free port via `node:net.createServer().listen(0)`, addressing `127.0.0.1` only (never `0.0.0.0`). Server times out after 5 minutes and accepts exactly one `GET /callback` before shutting down.
3. CLI opens the system browser to `https://mc.mnemonik.xyz/oauth/authorize?response_type=code&client_id=mnemonic-cli&redirect_uri=http://127.0.0.1:<port>/callback&code_challenge=<base64url>&code_challenge_method=S256&state=<random>&scope=mcp`.
4. The browser redirects to `mnemonik.xyz/oauth/consent`. The user clicks "Approve" — webapp signs the OAuth challenge with the browser-stored keypair (existing flow). The webapp POSTs the signature to `/oauth/authorize`, the server verifies, issues an authorization code, redirects browser back to `http://127.0.0.1:<port>/callback?code=<code>&state=<state>`.
5. CLI's loopback server validates `state` matches its stored value (PKCE state-to-verifier binding per RFC 7636 §4.4 — verifier and state are stored together in a single Map keyed by state, so a state mismatch terminates the flow before any code exchange), then exchanges code+verifier for a JWT via `POST /oauth/token` (existing endpoint).
6. CLI writes JWT to `~/.mnemonic/token.json` (mode 0600), shuts down the loopback server, prints "Logged in as `<pubkey>`".

The keypair-mismatch problem (CLI's local keypair ≠ webapp's browser keypair) is solved by the bootstrap-ticket flow described below — `mnemonic init` for a webapp user prompts to import the existing browser keypair via ticket, so the CLI and webapp always share one identity before login.

Headless (`mnemonic login --token <jwt>`):
1. CLI receives the pre-issued JWT.
2. CLI parses the header, asserts `alg=HS256` and `exp` not in past. Does not verify signature client-side; server rejects on first request.
3. CLI writes `~/.mnemonic/token.json`. No browser, no callback server.

**Identity bootstrap (`mnemonic identity import --ticket <uuid>` / `--file <path>`):**

Two paths to align CLI's `~/.mnemonic/identity.json` with the webapp's `localStorage["mnemonic.identity"]`:

1. **Server-mediated ticket** (the typical flow). User clicks "Send to CLI" in webapp's IdentityPanel. Webapp POSTs `/api/cli-bootstrap/issue` with Bearer JWT — server creates a one-time bootstrap ticket (random UUID), stores `{ticket_id, ciphertext_of_keypair_json, jwt_sub, expires_at}` in an in-memory LRU+TTL map (10-minute TTL, per-user limit 3 active tickets). Server returns the ticket_id. Webapp displays `mnemonic identity import --ticket <ticket_id>`. User pastes in terminal. CLI calls `GET /api/cli-bootstrap/redeem/:ticket` with no auth (the ticket UUID itself is the capability, like `correlation_id` in browser-mediated signing). Server returns the keypair_json exactly once (atomic remove on first read). CLI writes `~/.mnemonic/identity.json` mode 0600.

2. **File-based** (offline / advanced users). `mnemonic identity export --file ./keypair.json` writes the local keypair as JSON to the path (mode 0600). `mnemonic identity import --file ./keypair.json` reads and writes to `~/.mnemonic/identity.json`. Webapp also exposes its existing IdentityPanel "Download keypair" button which produces a compatible JSON file. No clipboard option in either direction (security: clipboards leak to all OS apps + clipboard managers).

**Sign flow (`mnemonic sign`):**

Server-side reality (confirmed in code review of `mcp/src/tools.rs::sign_memory`): hosted MCP server **always** returns a pending-bundle response for HTTP+JWT clients. There is no inline-signed code path the CLI can take server-side. The SDK ALWAYS handles the pending-bundle:

1. CLI loads `~/.mnemonic/{identity,token}.json` (or fails with exit 1 / 4 if missing).
2. CLI calls `client.signMemory(content, { tags })`.
3. SDK posts JSON-RPC `tools/call name=mnemonic_sign_memory` to `/mcp` with `Authorization: Bearer <jwt>`.
4. Server returns `{correlation_id, sign_url, payload_cbor_base64, expires_at}`. SDK ignores `sign_url` (that's the webapp browser handoff path), decodes `payload_cbor_base64`, runs WASM `sign_cose_payload(cbor_bytes, keypair_json)` to produce the COSE_Sign1 envelope.
5. SDK POSTs `{correlation_id, signer_pubkey, cose_signed: <base64>}` to `/api/sign-callback` (no Bearer JWT — capability auth via correlation_id, identical to the webapp flow). Server validates the COSE envelope, checks `signer_pubkey` matches the JWT `sub` recorded with the pending bundle, persists the attestation, returns `{attestation_id, signed_at}`.
6. CLI formats output and exits 0.

This is the same `/api/sign-callback` endpoint the webapp uses; the only difference is the SDK does the COSE signing in-process via WASM instead of in a browser tab. No server changes needed for the sign flow itself.

**Recall / verify flows:** straightforward HTTP `tools/call` to `/mcp` with `Authorization: Bearer <jwt>`. Output formatting per `--json`/`--quiet`/TTY auto-detect.

**Whoami / prove flows (client-side):** these commands do not call server tools, because the existing server-side `mnemonic_whoami` and `mnemonic_prove_identity` tools return the **server**'s identity, not the user's. CLI implements them locally:

- `mnemonic whoami` reads `~/.mnemonic/identity.json` and `~/.mnemonic/token.json`, prints `{pubkey, did, signer_match, jwt_pubkey, attestation_count}`. The `attestation_count` is fetched via `recall(query=' ', topK=0)` against the server (returns total count for the user). If `signer_pubkey != jwt_sub_pubkey`, the output flags the mismatch ("identity ≠ logged-in identity — run `mnemonic identity import` to align").
- `mnemonic prove [--challenge=<hex>]` reads identity, calls WASM `sign_challenge(keypair_json, challenge_bytes)`, returns `{pubkey, challenge: hex, signature: hex, did}` for the caller to verify with a stock Ed25519 library.

### Shared Resources

**SDK runtime — none.** Stateless — `MnemonicClient` instances hold `{baseUrl, jwt, signer}` only. Multiple clients share nothing. Native `fetch`, no connection pool managed by SDK. WASM module is loaded once per JS runtime (cached by the loader).

**CLI runtime — none.** One-shot invocation: read config, do one HTTP exchange, print, exit.

**Server runtime — bootstrap-ticket store.** New in-memory `BootstrapTickets` LRU+TTL map in `mcp/src/api.rs` (modeled on existing `pending::PendingBundles`). Single instance per `mnemonic-mcp` process, stored in `McpState`. TTL 10 min, max 100 entries, per-user limit 3 active tickets. Owner: API endpoint handlers. Consumers: only the issue + redeem handlers.

**Build-time — wasm-pack output for SDK.** Produced via a `wasm-pack` invocation alongside the existing `--target web` build. Target choice (`bundler` vs `nodejs` vs custom) is investigated in Task 1 — see Decision 3.

## Decisions

### Decision 1: Two packages, one substrate (`@mnemonik-xyz/sdk` + `@mnemonik-xyz/cli`)

Ships SDK and CLI as separate npm packages, with CLI depending on SDK. Supports user-spec § "Что делаем" — explicit requirement that future Chrome extension and agent frameworks reuse the same substrate without re-implementing OAuth, COSE, or MCP wire format. Two packages cleanly partition runtime concerns: SDK is universal (Web APIs only), CLI is Node-only (filesystem, child_process, OS keychain).

Alternative considered: single combined `@mnemonik-xyz/cli` with internal but unpublished modules. Rejected — user-spec § Зачем cites Chrome extension and agent framework consumers as primary motivation.

### Decision 2: Pure ESM, Web APIs only in SDK; Node-specific code stays in CLI

SDK uses only `fetch`, `crypto.subtle`, `URL`, `TextEncoder`, `TextDecoder`. No `node:fs`, `node:http`, `node:child_process`. The OAuth interactive flow's loopback HTTP server lives in CLI's `commands/login.ts` (uses `node:http`); SDK exposes primitives `oauth.buildAuthorizeUrl({...})` and `oauth.exchangeCodeForToken(...)` so a Chrome extension or other host can do its own redirect-handling (e.g. `chrome.identity.launchWebAuthFlow`) while reusing the same primitives.

Supports user-spec MUST "Pure ESM, runtime-agnostic. No `node:*` imports in `sdk/`."

### Decision 3: SDK consumes `@mnemonic/core` WASM via workspace path; correct wasm-pack target investigated in Task 1

SDK depends on the existing `core/` Rust crate compiled to WASM via `wasm-pack`. Phase 1 does NOT publish `@mnemonic/core` to npm — SDK consumes the WASM artifact via a relative workspace path (`../../core/pkg-{target}/`).

The correct `wasm-pack` target for SDK consumption is **the first deliverable of Task 1**. Three viable options to evaluate:
- `--target web` — the existing build. Uses `import.meta.url` + `fetch` for `.wasm` loading. Works in browsers; works in Node 20 ESM with native `fetch`; needs verification under Bun.
- `--target nodejs` — emits `require('fs').readFileSync`. Works in Node CJS-leaning paths; not pure ESM.
- `--target bundler` — emits ESM with synchronous WASM imports, requires a bundler that understands `.wasm` (Vite, esbuild, webpack 5+). Works in browsers via bundler; **does not load standalone in Node or Bun**.

Task 1's smoke test exercises `import { sign_cose_payload } from '@mnemonik-xyz/core-wasm'` from a Node 20 process AND a Bun process. Whichever target passes both is the target the SDK uses. If none passes both, the fallback is to ship two builds (`pkg-web` for browsers, `pkg-nodejs` for Node/Bun) and use `package.json` conditional exports (`{exports: { ".": { "import": "./web.js", "node": "./node.js" }}}`).

The same canonical CBOR + COSE_Sign1 Rust code that the server uses to verify is what the SDK calls — byte-for-byte identical, eliminating the entire class of bugs around "JS canonical CBOR almost matches Rust canonical CBOR but differs in 0.1% of edge cases".

Cost: ~442KB WASM in SDK bundle. Acceptable for Phase 1; swap to `@noble/curves` + custom CBOR is in backlog if size complaints arrive. Public SDK API does not depend on the COSE backend.

Supports user-spec "COSE round-trip CBOR byte-for-byte without re-encoding".

### Decision 4: `Signer` interface for keypair abstraction

```typescript
interface Signer {
  pubkey: string;                                 // base58 Ed25519 pubkey
  sign(bytes: Uint8Array): Promise<Uint8Array>;   // raw 64-byte Ed25519 sig
}
```

Phase 1 ships `LocalSigner`, which holds the secret in memory and signs by calling WASM `sign_challenge(keypair_json, bytes)` — that's the existing WASM export for raw Ed25519 signatures over arbitrary byte payloads (NOT to be confused with `sign_cose_payload`, which wraps server-canonical-CBOR in a COSE_Sign1 envelope and is used in the sign-flow only).

Future `TurnkeySigner` (Phase 1.5), `WebAuthnSigner` (Phase 2) are drop-in replacements without API change. `MnemonicClient` accepts `signer: Signer` in its constructor; never inspects the secret directly.

### Decision 5: OAuth — create redirect-URI allowlist (it does not exist today) including loopback for CLI

`mcp/src/oauth.rs` currently accepts **any** `redirect_uri` from the authorize request — verified by reviewing the existing code. This is a latent vulnerability (RFC 8252 §7) regardless of CLI: a malicious client could direct the redirect to an attacker-controlled domain. Phase 1 of mnemonic-cli adds a redirect-URI allowlist, with these entries:

- `https://mnemonik.xyz/oauth/consent` (existing webapp consent page).
- `cursor://anysphere.cursor-deeplink/mcp/install`, the VS Code variant, the Claude.ai variant — copied from current `cors_policy.rs` and the OAuth flow's effective redirect-URIs as observed in production.
- A regex matcher for `^http://127\.0\.0\.1:[0-9]+/callback$` and `^http://\[::1\]:[0-9]+/callback$`, gated to `client_id=mnemonic-cli`.

PKCE verifier and state are stored together in the server's PKCE-state map (`{state -> {verifier_challenge, redirect_uri, client_id}}`). Token-exchange validates that `verifier`, `state`, and `redirect_uri` all match the originally-issued tuple; mismatch → 400. Per RFC 7636 §4.4 + RFC 8252 §7.

This is a security improvement over today's behavior — the unauthenticated redirect-URI acceptance was not on the user-spec radar, but is dangerous regardless of CLI shipping. **Documented in Deviation 1.**

### Decision 6: OAuth headless mode = `--token <jwt>` opaque pass-through

`mnemonic login --token <jwt>` writes the user-supplied JWT after parsing header (assert `alg=HS256`, `exp` in future). Signature is not verified client-side — server rejects invalid JWTs on first request, surfaced as exit code 4. Works for CI / serverless / no-display environments. Token's 1-hour TTL applies; refresh tokens are explicitly backlog.

### Decision 7: CLI ↔ webapp identity bootstrap via server-issued one-time tickets

The earlier draft proposed a `--cli-bootstrap` URL with a self-signed pubkey blob. **Security audit flagged this as replayable** — signing a pubkey with the same keypair proves possession of the secret to the holder of either side, but the URL itself is freely replayable: anyone who intercepts it can re-import the keypair into a fresh webapp localStorage. Replaced with a server-mediated bootstrap-ticket flow:

1. **Webapp → CLI direction.** User has a browser-side keypair (`localStorage["mnemonic.identity"]`) and wants to import it on the CLI. In webapp's IdentityPanel, user clicks "Send to CLI". Webapp POSTs the localStorage keypair JSON to `/api/cli-bootstrap/issue` with `Authorization: Bearer <webapp-JWT>`. Server stores the keypair_json in `BootstrapTickets` (new in-memory LRU+TTL map: 10-min TTL, max 100 entries, max 3 per `jwt.sub`), returns `ticket_id` (UUID v4). Webapp displays `mnemonic identity import --ticket <uuid>`. User pastes in terminal. CLI calls `GET /api/cli-bootstrap/redeem/:ticket` with no auth (the UUID is the capability — same pattern as `correlation_id` in browser-mediated signing). Server **atomically** removes-and-returns the entry on first call (subsequent calls return 410 Gone). CLI parses keypair, writes to `~/.mnemonic/identity.json` mode 0600.
2. **CLI → webapp direction.** Less common; user has CLI-generated keypair and wants to import to browser. `mnemonic identity export --file ./keypair.json` writes file. User uploads via webapp IdentityPanel's existing "Import keypair" button (already exists — accepts a JSON file from `<input type=file>`).

**No clipboard option** in either direction (security: clipboards are read by every OS app and most clipboard managers).

The flow eliminates phishing replays: tickets are server-issued, single-use, time-bound, scoped to the issuing user's JWT. The keypair never crosses an unencrypted URL.

Supports user-spec § "MCP-compatible identity portability between webapp and CLI".

### Decision 8: CLI persistence at `~/.mnemonic/{identity,token}.json`, mode 0600 / Windows ACL

Plain JSON files in user's home directory. Unix: mode 0600 via `node:fs.chmodSync`. Windows: explicit ACL setting via `fs.chmod` (no-op) plus `winston-fs-acl` or equivalent shim that calls Windows `icacls` to restrict access to current user only. Concrete library choice in Task 1 — recommend `fs-extra`'s `setReadOnlyForOwner` which wraps the platform-specific calls. If no library covers all targets, use `child_process.execSync('icacls "${file}" /inheritance:r /grant:r "${process.env.USERNAME}:F"')` on Windows specifically.

Plain JSON, not encrypted at rest — matches Cursor's `~/.cursor/`, gh's `~/.config/gh/`, npm's `~/.npmrc`. OS keychain (macOS Keychain / Linux Secret Service / Windows Credential Manager) is in backlog.

XDG support via `XDG_CONFIG_HOME` is in backlog — Phase 1 uses `~/.mnemonic/`.

### Decision 9: npm scope = `@mnemonik-xyz`

User-confirmed: org `mnemonik-xyz` registered on npm; `@mnemonik` and `@mnemonic` were taken. Publishing under `@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli`. Migration is a future deploy task.

### Decision 10: Output format + exit codes + logging redaction

- **TTY default:** ANSI color, human-readable.
- **Pipe / non-TTY default:** plain text, no color.
- **`--json`:** machine-readable JSON to stdout; human messages (progress, hints) to stderr.
- **`--quiet`:** suppress stdout except `--json` payload + exit code.
- **`--no-color`:** force plain text.

Exit codes: `0` success, `1` user error, `2` server/network error, `3` integrity failure (verify=tampered), `4` auth error.

**Logging redaction** (in `packages/{sdk,cli}/src/errors.ts`): never include JWT, identity secret, OAuth code, or PKCE verifier in error messages, exception payloads, or `console.error` output. Errors carry a redacted `safe_message` field (no secrets) and a developer-facing `cause` chain. Tests in Task 8 assert that JWT-shaped strings (`eyJ...`) and 64-byte hex-encoded secrets never appear in `process.stderr.write`.

### Decision 11: Cross-runtime CI matrix on every PR

CI runs unit + integration on **Node 20, Node 22, Bun latest, Deno 1.40+** — Deno is upgraded from "manual smoke" to a CI matrix entry because the test-reviewer flagged Medium-likelihood Deno-specific risks (Ed25519 in `crypto.subtle`, ESM resolution edge cases). Cloudflare Workers smoke remains pre-release manual (no Workers test runner integrates well today; revisit when `workerd` exposes one).

### Decision 12: Test fixture: golden COSE round-trip with enforced lockstep

A new `golden-fixtures` cargo feature flag is added to `core/` (does not exist today). Under this feature, a Rust integration test in `core/tests/golden_fixtures.rs` emits a JSON file of `{input_bytes_hex, expected_canonical_cbor_hex, expected_cose_envelope_hex}` triples (deterministic, ~50 cases covering edge cases of CBOR canonical encoding). The JSON file is committed to `packages/sdk/test/fixtures/golden-cose.json` via a script `cargo run --features golden-fixtures --bin gen-fixtures > packages/sdk/test/fixtures/golden-cose.json`.

**Lockstep enforcement** (test-reviewer requirement): the file's checksum is also written to `packages/sdk/test/fixtures/golden-cose.sha256`. CI workflow (`.github/workflows/node-test.yml`) runs `cargo run --features golden-fixtures --bin gen-fixtures | sha256sum | diff - packages/sdk/test/fixtures/golden-cose.sha256`. If the Rust fixture generator output drifts from the committed checksum, CI fails the SDK test workflow. Forces regeneration whenever Rust core's CBOR/COSE encoder changes.

[TECHNICAL] Justification: user-spec MUST mentions "byte-for-byte" without prescribing how. This is the implementation mechanism with concrete CI enforcement.

### Decision 13: Public surface only — `@mnemonik-xyz/sdk` + `@mnemonik-xyz/cli`

Internal helpers (build scripts, golden fixtures, mock server, etc.) live in the monorepo but are not published. `package.json` of each public package lists only public surface in `exports`. NPM provenance attestations (`npm publish --provenance`) are required on every release — see Task 15.

[TECHNICAL] Justification: prevents accidental publishing of internal helpers + adds supply-chain integrity (sigstore-backed provenance lets consumers verify the package was built from a specific git commit).

### Decision 14: Server tools `mnemonic_whoami` / `mnemonic_prove_identity` are NOT used by CLI

The existing server-side tools `mnemonic_whoami` (returns server keypair pubkey) and `mnemonic_prove_identity` (signs a challenge with server keypair) are **not the user-facing semantics** the CLI's `whoami` / `prove` commands need. The CLI implements them client-side instead: `whoami` reads local identity + JWT and prints user's pubkey/DID; `prove` calls WASM `sign_challenge` locally and prints `{pubkey, challenge, signature}`. No server changes required for these commands.

[TECHNICAL] Justification: completeness validator surfaced this — the existing server tools return server identity, not user identity. Reframing whoami/prove as client-side fixes the semantics without adding new server tools.

## Data Models

**No new SQLite tables.** CLI is a client of the existing MCP server; all writes go through existing `attestations` / `memory_embeddings` / `attestation_costs` tables, scoped by `owner_pubkey`.

**New file formats (CLI-local):**

- `~/.mnemonic/identity.json`: `{secret: number[64], pubkey_base58: string}`. Mode 0600. Identical shape to webapp's `localStorage["mnemonic.identity"]`.
- `~/.mnemonic/token.json`: `{jwt: string, pubkey_base58: string, issued_at: ISO-8601, expires_at: ISO-8601}`. Mode 0600. `pubkey_base58` decoded from JWT `sub` for fast lookup.

**New server data (in-memory only):**

- `BootstrapTickets` map: `{ticket_id: string, keypair_json: string, jwt_sub: string, expires_at: i64}`. LRU+TTL store. Lives in `mcp/src/api.rs`. 10-min TTL, max 100 entries, max 3 active per `jwt_sub`. Cleared on server restart (acceptable — tickets are short-lived).

**SDK public types (TypeScript):**

```typescript
export interface SignerInterface {
  pubkey: string;
  sign(bytes: Uint8Array): Promise<Uint8Array>;
}

export interface MnemonicClientConfig {
  baseUrl: string;
  signer: SignerInterface;
  jwt?: string;
}

export interface SignMemoryOptions {
  tags?: string[];
}

export interface SignMemoryResult {
  attestationId: string;
  signedAt: string;
  status: "signed" | "pending" | "anchored";
  arweaveTx?: string;
  solanaTx?: string;
}

export type VerifyResult =
  | { status: "verified"; signer: string; arweaveTx?: string; solanaTx?: string }
  | { status: "tampered"; signer: string; reason: string }
  | { status: "not_found" };
```

## Dependencies

### New packages (`packages/sdk/package.json`)

- `@mnemonic/core-wasm` (workspace-internal, built from `core/pkg-<target>/`) — COSE / canonical CBOR / Ed25519 via WASM.
- `@noble/ed25519` ≥ 2.1 (≈ 12KB) — fallback signer for runtimes without `crypto.subtle.sign({name:'Ed25519'})`.

### New packages (`packages/cli/package.json`)

- `@mnemonik-xyz/sdk` (workspace dependency).
- `commander` ≥ 12.
- `kleur` (≈ 1KB).
- `open` ≥ 10.
- A Windows-ACL helper (concrete library chosen in Task 1; candidates: `fs-extra` `setReadOnlyForOwner`, or `child_process.execSync` shelling to `icacls`).

### Devdependencies

- `vitest` ≥ 1.6.
- `typescript` ≥ 5.4.
- `@types/node` for CLI.

### Removed packages — None.

### Existing (used as-is)

- `wasm-pack` (already installed on dev + VPS).
- Existing `@mnemonic/core` Rust crate code in `core/`. No Rust source changes for the SDK consumption itself; the only Rust changes are `mcp/src/oauth.rs` (Decision 5), `mcp/src/api.rs` (Decision 7's new endpoints), `core/Cargo.toml` (`golden-fixtures` feature flag), and `core/tests/golden_fixtures.rs` (new test).

## Testing Strategy

Per user-spec size **M**: four layers + redaction tests.

### Unit tests (vitest, every PR)

- **SDK:** mock `fetch`, assert request shapes for each of the 5 tool methods. OAuth `buildAuthorizeUrl` emits correct PKCE+state params; `exchangeCodeForToken` POSTs to `/oauth/token` with correct body. `signMemory` correctly handles the **pending-bundle** response shape (the only shape the server returns); SDK never assumes inline-signed responses. `LocalSigner.sign(bytes)` produces deterministic 64-byte Ed25519 signatures verifiable via `sign_challenge` round-trip.
- **`Signer` contract suite:** `packages/sdk/test/signer-contract.ts` exports a function `runSignerContract(signer: Signer)` that asserts: `pubkey` is non-empty base58, `sign(bytes)` returns 64 bytes, signature verifies via WASM `verify_signature(pubkey_base58, bytes, sig)`, signing identical input twice produces identical signature (Ed25519 deterministic), signing rejects on null/empty input. `LocalSigner` runs through the contract; future `TurnkeySigner` is required to.
- **Golden COSE fixture:** Decision 12. SDK test reads `packages/sdk/test/fixtures/golden-cose.json`, runs each input through WASM `sign_cose_payload` + `to_canonical_cbor`, asserts byte-for-byte equality. CI lockstep gate ensures Rust-side regeneration is in sync.
- **Identity bootstrap (Decision 7):** unit tests for `mnemonic identity import --ticket <uuid>` against a mock `BootstrapTickets` server (success, expired ticket → 410, double-redeem → 410, malformed UUID → 400, server-issued keypair → identity.json written with mode 0600).
- **Logging redaction:** assert no JWT-shape strings (`/^eyJ[A-Za-z0-9_-]+$/`) or 128-hex-char secrets ever appear in captured stderr/stdout during error paths.
- **CLI:** argv parser per command, output formatter for TTY/pipe/json/quiet, exit-code mapping for known errors. Concrete file: `packages/cli/test/{init,login,sign,recall,verify,whoami,prove,identity,output}.test.ts`.

Coverage: SDK ≥85% lines / ≥80% branches. CLI ≥75% lines.

### Integration tests (vitest + in-process mock MCP server, every PR)

`packages/sdk/test/mock-server.ts` exposes:
- `/mcp` JSON-RPC endpoint (handles `tools/list`, `tools/call` with all 5 tools)
- `/oauth/{authorize,token,register}` endpoints (full PKCE round-trip)
- `/api/sign-callback` (validates COSE envelope, emits `attestation_id`)
- `/api/cli-bootstrap/{issue,redeem}` (one-time ticket pattern matching server)
- **Fault-injection toggles** (test-reviewer requirement): `withFault('5xx-on-token-exchange')`, `withFault('malformed-cbor-in-pending')`, `withFault('expired-jwt-after-N-requests')`, `withFault('signer-pubkey-mismatch')`, `withFault('callback-timeout')`. Each integration test exercises at least one fault path.

CLI integration tests via `execa` against the same mock server.

### Manual smoke tests (pre-release checklist in `tasks/`)

`packages/cli/SMOKE.md`:

1. `npm install -g @mnemonik-xyz/cli` from a freshly built `.tgz`.
2. `mnemonic init` → `~/.mnemonic/identity.json` appears, mode 0600 (verify with `stat`).
3. `mnemonic login` → browser opens, OAuth flow completes, token persisted.
4. `mnemonic sign "hello"` → attestation_id returned within 5s.
5. `mnemonic recall "hello"` → finds the attestation.
6. `mnemonic verify <id>` → exit 0.
7. `mnemonic identity export --file /tmp/k.json` → file mode 0600.
8. `mnemonic identity import --ticket <issued-via-webapp>` round-trip works.
9. Cross-tool: same pubkey logged into Claude.ai sees the CLI-signed attestation via `mnemonic_recall`.
10. Negative paths: `mnemonic verify <stranger-attestation-id>` → exit 1 with not_found.

### E2E tests (release pipeline, not PR-gating)

One full scenario: `init → login --token <pre-issued via mint-test-jwt> → sign → recall` against a real `STORAGE_MODE=local` self-hosted MCP on a CI runner.

### Cross-runtime matrix

Unit + integration suites on **Node 20, Node 22, Bun latest, Deno 1.40+** in CI. Cloudflare Workers smoke pre-release manual.

## Agent Verification Plan

### Verification approach

1. **OAuth interactive flow against live `mc.mnemonik.xyz`** — manual pre-release verification (mock can't fully replicate real PKCE with loopback callback against real CertificateAuthority TLS termination).
2. **wasm-pack target choice** — Task 1's smoke test (Node + Bun import works) is the deciding artifact.
3. **Webapp `cli-bootstrap` UI flow** (Decision 7 / Task 7) — Playwright MCP verifies "Send to CLI" button, ticket display, end-to-end import.

### Tools required

- **Bash MCP** — install package, run smoke commands, inspect file modes.
- **Playwright MCP** — verify webapp's IdentityPanel "Send to CLI" UI.
- **None of:** browser/macOS-use, third-party API credentials.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `wasm-pack` target output breaks under one of Node 20 / Node 22 / Bun / Deno | High | High (SDK dead) | Task 1's smoke test is the gate. Fallback: ship two builds (`pkg-web` + `pkg-nodejs`) with `package.json` conditional exports. |
| `crypto.subtle.sign({name:'Ed25519'})` not implemented in older Cloudflare Workers / Deno | Medium | Medium | Lazy-load `@noble/ed25519` (12KB) in `LocalSigner.sign()` if `subtle.sign` rejects with `NotSupportedError`. |
| User runs `mnemonic init` after using webapp → mismatched identities → silent failure on first `sign` | High | Medium | Decision 7's bootstrap-ticket flow makes this an explicit, documented step. `mnemonic init` checks for existing webapp use via a one-line tooltip in `--help` ("Used Mnemonic in a browser? Run `mnemonic identity import --ticket` instead."). |
| OAuth loopback redirect blocked by corporate firewall / strict no-localhost-http policy | Low | High for affected users | Headless `--token` fallback documented. Webapp's `/install` page can show a "copy JWT for CLI" button (longer-lived ticket, ~15 min) for pasting — backlog if reports come in. |
| 442KB WASM bloats SDK install | Medium | Low | CI bundle-size budget (≤500KB SDK, ≤200KB CLI). Swap to `@noble/curves` listed in backlog. |
| Server-side `BootstrapTickets` LRU evicts an in-flight ticket if 100+ users issue simultaneously | Low | Medium | Eviction returns 410 Gone — user retries. If real, raise the cap or move to Redis. Phase 1 in-memory is fine. |
| Server-side OAuth allowlist regex misses an edge case (`http://[::1]` ipv6 vs `http://0.0.0.0`) | Low | High (CLI cannot login) | Task 7's unit tests cover the regex with positive + negative cases; manual smoke validates ipv4 + ipv6 loopback both work. |
| Hackathon judges don't see CLI on stage | High | Medium | Demo flow: `mnemonic sign` in terminal alongside Claude.ai prompted "recall what I signed via terminal" — Claude finds it. Cross-tool demo. |
| Decision 5's allowlist breaks existing webapp/Cursor/VS Code/Claude.ai if entries are misconfigured | Medium | Critical (prod outage) | Task 7's smoke test against `mc.mnemonik.xyz` staging includes login from each existing client before deploy. Rollback plan: revert oauth.rs commit, redeploy. |

## User-Spec Deviations

Each entry is `[PENDING USER APPROVAL]` until you accept it.

### Deviation 1: Server-side OAuth redirect-URI allowlist (CREATES, does not extend)

**User-spec implies:** no server changes beyond loading CLI as another MCP client.
**Tech-spec does:** ADDS a redirect-URI allowlist to `mcp/src/oauth.rs` (today the server accepts any `redirect_uri`). Loopback regex (`http://127.0.0.1:*` / `http://[::1]:*`) is one entry; existing webapp / Cursor / VS Code / Claude.ai schemes are also added. PKCE state is also bound to verifier at the same time (RFC 7636 §4.4).
**Why:** This is a security improvement over today's behavior, not just a CLI accommodation. The latent vulnerability (any redirect_uri accepted) is a finding from the security audit and should be fixed regardless of CLI shipping. **`[PENDING USER APPROVAL]`**

### Deviation 2: Server-side `/api/cli-bootstrap/{issue,redeem}` endpoints + `BootstrapTickets` LRU/TTL store

**User-spec implies:** identity bootstrap is a CLI-local concern.
**Tech-spec adds:** two new server endpoints + an in-memory ticket store in `mcp/src/api.rs`. Required to safely move keypair from webapp to CLI without phishing-replayable URLs (security audit finding).
**Why:** The originally-proposed self-signed bootstrap URL was replayable. Server-issued one-time tickets are the correct pattern. Adds ~½ dev-day of server work, but avoids a critical security flaw. **`[PENDING USER APPROVAL]`**

### Deviation 3: Webapp `IdentityPanel` "Send to CLI" button

**User-spec says:** webapp surface unchanged in Phase 1.
**Tech-spec adds:** one new button in `webapp/src/components/IdentityPanel.tsx` that calls `/api/cli-bootstrap/issue` and displays the resulting ticket.
**Why:** Companion to Deviation 2. Trivial UI change (~½ day) that closes the bootstrap UX loop. **`[PENDING USER APPROVAL]`**

### Deviation 4: New CLI subcommand `mnemonic identity {import,export}`

**User-spec says:** 7 commands.
**Tech-spec adds:** `mnemonic identity import --ticket <uuid>` / `--file <path>` and `mnemonic identity export --file <path>`. **No `--to-clipboard` option** (security audit: clipboard leaks).
**Why:** Closes the bootstrap UX loop. **`[PENDING USER APPROVAL]`**

### Deviation 5: CI matrix expanded — Node 20 + Node 22 + Bun + Deno

**User-spec says:** Node ≥20, Bun mentioned as a target.
**Tech-spec runs CI on:** Node 20, Node 22, Bun latest, Deno 1.40+. Cloudflare Workers manual pre-release.
**Why:** Test-reviewer flagged Medium-likelihood Deno-specific risks (Ed25519 in `crypto.subtle`, ESM resolution). Adding Deno as a CI matrix entry catches regressions early. Trivial cost. **`[PENDING USER APPROVAL]`**

### Deviation 6: New top-level `packages/` directory + npm workspace at repo root

**User-spec implies:** packages live somewhere reasonable.
**Tech-spec specifies:** `packages/sdk/`, `packages/cli/`, repo root `package.json` becomes an npm workspace `"workspaces": ["packages/*", "webapp"]`.
**Why:** Standard JS monorepo pattern; webapp brought into workspace too (its own build chain unchanged). **`[PENDING USER APPROVAL]`**

### Deviation 7: `wasm-pack` target investigated in Task 1 (multiple options possible)

**User-spec implies:** SDK consumes WASM via wasm-pack.
**Tech-spec specifies:** the correct wasm-pack target is empirically determined by Task 1's Node + Bun smoke test. Possible outcomes: existing `--target web` works for SDK too (simplest), or a parallel `--target nodejs` build, or two builds with `package.json` conditional exports. Decision deferred to implementation. **`[PENDING USER APPROVAL]`**

### Deviation 8: New `golden-fixtures` cargo feature flag in `core/`

**User-spec says:** "byte-for-byte" CBOR/COSE round-trip.
**Tech-spec specifies:** new `golden-fixtures` cargo feature in `core/Cargo.toml` + new `core/tests/golden_fixtures.rs` test that emits JSON triples. CI lockstep gate via SHA-256 checksum diff.
**Why:** Implementation mechanism for the user-spec MUST. **`[PENDING USER APPROVAL]`**

### Deviation 9: `whoami` and `prove` are client-side; do NOT use server tools

**User-spec says:** CLI exposes all 5 MCP tools.
**Tech-spec specifies:** `whoami` and `prove` are implemented client-side (read local files, sign with WASM). The existing server tools `mnemonic_whoami` and `mnemonic_prove_identity` return server keypair, not user keypair, so they're not the right semantics for the CLI's user-facing commands.
**Why:** Completeness validator surfaced this — the existing server tools have different semantics. Reframing as client-side fixes the semantics without adding new server tools. **`[PENDING USER APPROVAL]`**

## Acceptance Criteria

(carried through from user-spec § Критерии приёмки; tech-spec adds concrete artifacts)

- [ ] `@mnemonik-xyz/sdk` published to npm (or ready: `npm pack` ≤500KB, `npm publish --dry-run --provenance` clean).
- [ ] `@mnemonik-xyz/cli` published; `npm install -g @mnemonik-xyz/cli` registers `mnemonic` on PATH.
- [ ] **Pure ESM, runtime-agnostic.** `grep -r 'node:' packages/sdk/src/` empty. CI green on Node 20 + Node 22 + Bun + Deno.
- [ ] **CLI commands:** `init`, `login [--token <jwt>]`, `sign`, `recall`, `verify`, `whoami`, `prove` (user-spec 7) + `identity import [--ticket <uuid> | --file <path>]` / `identity export --file <path>` (Deviation 4). All with `--help`.
- [ ] **Output:** TTY-aware default; `--json`, `--quiet`, `--no-color` observable in tests.
- [ ] **Exit codes** per Decision 10, asserted in CLI integration tests.
- [ ] **OAuth interactive** end-to-end against `mc.mnemonik.xyz`: browser opens, callback received via 127.0.0.1 loopback, JWT persisted.
- [ ] **OAuth headless** end-to-end: `--token <jwt>` skips browser.
- [ ] **Identity bootstrap (server-mediated):** `mnemonic identity import --ticket <uuid>` works against `mc.mnemonik.xyz/api/cli-bootstrap/redeem/:ticket`. Webapp IdentityPanel issues tickets via `/api/cli-bootstrap/issue`.
- [ ] **Inline COSE signing in sign flow:** SDK's `signMemory` always uses pending-bundle response, signs locally via WASM, POSTs `/api/sign-callback`. Verified by golden COSE fixture (Decision 12).
- [ ] **`Signer` interface** decoupled — abstract contract suite in SDK passes for `LocalSigner`, ready for future impls.
- [ ] **Logging redaction** asserted in tests — no JWT or secrets in stderr/stdout error paths.
- [ ] **OAuth allowlist** in `mcp/src/oauth.rs` rejects arbitrary redirect URIs (regression test) AND existing clients still work (smoke).
- [ ] **`whoami` / `prove` client-side**: read local files, sign with WASM, no server tool calls for these two commands.
- [ ] **Golden fixture CI gate** fails the workflow if Rust fixture generator output drifts from committed `golden-cose.sha256`.
- [ ] **NPM provenance** attestations on every published version (`npm publish --provenance`).
- [ ] **CI:** unit + integration tests on Node 20 / Node 22 / Bun / Deno. SDK + CLI test suites pass without network.
- [ ] **Documentation:** `packages/sdk/README.md`, `packages/cli/README.md`, JSDoc on public SDK methods, repo-root `README.md` updated.
- [ ] **Demo:** `npm install -g @mnemonik-xyz/cli && mnemonic identity import --ticket <web-issued> && mnemonic login && mnemonic sign "..."` works on a fresh box.

## Implementation Tasks

### Wave 1: Foundation (parallel)

#### Task 1: npm workspace + `packages/` skeleton + wasm-pack target investigation

Convert repo root `package.json` to an npm workspace including `packages/*` and the existing `webapp`. Create empty `packages/sdk/` and `packages/cli/` skeletons. Smoke-test all viable wasm-pack targets (`web`, `nodejs`, `bundler`) under Node 20 + Node 22 + Bun + Deno: import `sign_cose_payload` from each compiled artifact, verify it loads and runs. Pick the target (or pair of targets via conditional exports) that works on all four runtimes. Document the choice in `packages/sdk/README.md`.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor
- Verify-smoke: `for runtime in node bun deno; do $runtime -e "import('@mnemonik-xyz/core-wasm').then(m => console.log(typeof m.sign_cose_payload))"; done` prints `function` four times.
- Files to modify: `package.json` (root), `webapp/scripts/build-wasm.sh`, `packages/sdk/package.json`, `packages/cli/package.json`, `packages/sdk/scripts/build-wasm.sh`.
- Files to read: `webapp/package.json`, `webapp/scripts/build-wasm.sh`, `core/Cargo.toml`, `core/src/wasm/mod.rs`, wasm-pack docs.

#### Task 2: SDK core — `MnemonicClient` + `Signer` interface + `LocalSigner` + `Keypair` + contract suite

Implement the SDK's stateless client surface: `MnemonicClient` class with HTTP-based methods for the 5 MCP tools (sign always handles pending-bundle path, no inline assumption), `Signer` interface, `LocalSigner` impl using WASM `sign_challenge` for raw Ed25519, `Keypair` helpers (generate, fromJSON, toJSON via WASM `export_keypair_json`/`import_keypair_json`), public TS types per § Data Models. Includes the `runSignerContract(signer)` abstract test suite that future signer impls must pass. No OAuth code in this task — that lives in Task 3.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `bun test packages/sdk/test/{client,signer,signer-contract}.test.ts` passes.
- Files to modify: `packages/sdk/src/{client,signer,keypair,types,errors,index}.ts`, `packages/sdk/test/{client,signer,signer-contract}.test.ts`.
- Files to read: `mcp/src/{tools,oauth,api}.rs`, `core/src/wasm/mod.rs`, `webapp/src/pages/Sign.tsx`.

#### Task 3: SDK OAuth — `buildAuthorizeUrl`, `exchangeCodeForToken`, headless mode

Implement OAuth 2.1 + PKCE primitives in `packages/sdk/src/oauth.ts`. PKCE verifier+challenge via `crypto.subtle.digest`. State token via `crypto.getRandomValues`. State+verifier+redirect_uri stored together in a Map for matching during code exchange. Headless mode = `MnemonicClient({jwt})` constructor accepts pre-issued JWT directly. No `node:http` server here — that's CLI's responsibility.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `bun test packages/sdk/test/oauth.test.ts` passes.
- Files to modify: `packages/sdk/src/oauth.ts`, `packages/sdk/test/oauth.test.ts`.
- Files to read: `mcp/src/oauth.rs`, `webapp/src/pages/Consent.tsx`.

### Wave 2: COSE + commands + server changes (parallel)

#### Task 4: SDK COSE wrapper + golden fixture + CI lockstep gate

Implement `packages/sdk/src/cose.ts` as wrapper around WASM `sign_cose_payload`. Add `golden-fixtures` cargo feature flag to `core/Cargo.toml` (does not exist today). Add `core/tests/golden_fixtures.rs` integration test that emits JSON of `{input_bytes_hex, expected_canonical_cbor_hex, expected_cose_envelope_hex}` triples. Generate `packages/sdk/test/fixtures/golden-cose.json` + `golden-cose.sha256` checksum. SDK unit test asserts byte-equality. CI workflow includes a lockstep gate: regenerate fixture, diff checksum, fail if drift.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `cargo test --features golden-fixtures -p mnemonic-core && bun test packages/sdk/test/cose.golden.test.ts` both green; `cargo run --features golden-fixtures --bin gen-fixtures | sha256sum | diff - packages/sdk/test/fixtures/golden-cose.sha256` exits 0.
- Files to modify: `packages/sdk/src/cose.ts`, `packages/sdk/test/cose.golden.test.ts`, `packages/sdk/test/fixtures/golden-cose.{json,sha256}`, `core/Cargo.toml`, `core/tests/golden_fixtures.rs`, `.github/workflows/node-test.yml` (lockstep gate).
- Files to read: `core/src/codec/canonical.rs`, `core/src/codec/sign.rs`, `core/src/wasm/mod.rs`.

#### Task 5: CLI commands — all of them

Implement `packages/cli/bin/mnemonic.ts` (argv routing via `commander`) plus all commands: `init`, `login` (interactive loopback OAuth + `--token` headless), `sign`, `recall`, `verify`, `whoami` (client-side per Decision 14), `prove` (client-side), `identity import [--ticket | --file]`, `identity export --file`. Implements `packages/cli/src/output.ts` (TTY detection + `--json`/`--quiet`/`--no-color`), `packages/cli/src/config.ts` (XDG-respecting `~/.mnemonic/` paths, file-mode 0600 enforcement on Unix + Windows ACL via the library chosen in Task 1), `packages/cli/src/errors.ts` (typed errors + redaction helper for JWTs and secrets). Exit codes per Decision 10.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `bun test packages/cli/test/*.test.ts` passes (mock OAuth/MCP server in fixture).
- Verify-user: on a fresh dev machine: `cd packages/cli && bun link && mnemonic init` — `~/.mnemonic/identity.json` exists, mode 0600, contains `pubkey_base58`. Then `mnemonic --json whoami` prints valid JSON with the pubkey.
- Files to modify: `packages/cli/bin/mnemonic.ts`, `packages/cli/src/commands/{init,login,sign,recall,verify,whoami,prove,identity}.ts`, `packages/cli/src/{config,output,errors}.ts`.
- Files to read: `webapp/src/components/IdentityPanel.tsx` (localStorage shape compatibility), `mcp/src/{oauth,api}.rs`, `webapp/src/pages/Consent.tsx`, `core/src/wasm/mod.rs`.

#### Task 6: Server-side — OAuth redirect-URI allowlist + bootstrap-ticket endpoints + PKCE state binding

In `mcp/src/oauth.rs`: introduce a redirect-URI allowlist (does not exist today). Allowlist entries: webapp consent page, Cursor/VS Code/Claude.ai redirect schemes, regex for `http://127.0.0.1:*/callback` and `http://[::1]:*/callback` gated to `client_id=mnemonic-cli`. PKCE state is bound to verifier + redirect_uri at authorize-time and validated at token-exchange-time. In `mcp/src/api.rs`: implement `POST /api/cli-bootstrap/issue` (auth: Bearer JWT, payload: keypair_json, returns: ticket_id) and `GET /api/cli-bootstrap/redeem/:ticket` (no auth — capability via UUID). Add `BootstrapTickets` LRU+TTL store (10-min TTL, max 100, max 3 per jwt_sub) to `mcp/src/`. Atomic remove-and-return on first redeem, 410 Gone subsequent.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `cargo test -p mnemonic-mcp -- oauth_allowlist bootstrap_tickets` green. Manual: `curl https://staging.mc.mnemonik.xyz/oauth/authorize?...&redirect_uri=https://evil.com` returns 400.
- Files to modify: `mcp/src/oauth.rs`, `mcp/src/api.rs`, `mcp/src/main.rs` (route registration), possibly `mcp/src/cors_policy.rs`.
- Files to read: existing `mcp/src/oauth.rs`, `mcp/src/pending.rs` (LRU+TTL pattern reference), RFC 7636 §4.4, RFC 8252 §7.

#### Task 7: Webapp — `IdentityPanel` "Send to CLI" button + bootstrap-ticket display

Add a "Send to CLI" button to `webapp/src/components/IdentityPanel.tsx`. On click: POSTs the localStorage keypair_json to `/api/cli-bootstrap/issue` with the webapp's JWT in `Authorization`. Server returns ticket_id. UI displays a copyable code block: `mnemonic identity import --ticket <uuid>`. Also add unit tests for the new button and `playwright` e2e test verifying the ticket display + copy-to-clipboard works.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `cd webapp && npx vitest run src/components/IdentityPanel.test.tsx` passes; `cd webapp && npx playwright test e2e/cli-bootstrap.spec.ts` passes against staging.
- Files to modify: `webapp/src/components/IdentityPanel.tsx`, `webapp/src/components/IdentityPanel.test.tsx`, `webapp/e2e/cli-bootstrap.spec.ts` (new).
- Files to read: existing `IdentityPanel.tsx`, server endpoints from Task 6.

### Wave 3: Tests + CI + docs (parallel)

#### Task 8: Integration tests (mock server with fault injection) + CI matrix

Build mock MCP server in `packages/sdk/test/mock-server.ts` covering `/mcp`, `/oauth/{authorize,token,register}`, `/api/sign-callback`, `/api/cli-bootstrap/{issue,redeem}`. Includes `withFault('5xx-on-token-exchange' | 'malformed-cbor-in-pending' | 'expired-jwt-after-N-requests' | 'signer-pubkey-mismatch' | 'callback-timeout')` fault-injection. SDK + CLI integration tests use the mock + at least one fault path each. Add `.github/workflows/node-test.yml` matrix (Node 20 / Node 22 / Bun / Deno). Add bundle-size gate (`<500KB` SDK, `<200KB` CLI).

- Skill: `code-writing`
- Reviewers: code-reviewer, test-reviewer, deploy-reviewer
- Verify-smoke: open a draft PR, observe matrix runs all four runtimes green; bundle-size job reports actual sizes.
- Files to modify: `packages/sdk/test/mock-server.ts`, `packages/sdk/test/integration/*.test.ts`, `packages/cli/test/integration/*.test.ts`, `.github/workflows/node-test.yml`.
- Files to read: SDK + CLI source from Wave 1+2; `mcp/src/{mcp,oauth,api}.rs` for handler shapes.

#### Task 9: Documentation — SDK README, CLI README, JSDoc, repo-root pointer

`packages/sdk/README.md` (5-line quick-start, public API reference, runtime-target table, link to backlog), `packages/cli/README.md` (command reference, examples, exit-code table, smoke checklist link), JSDoc on all `MnemonicClient` methods + `Signer` interface (rendered to `.d.ts`). Repo-root `README.md` adds a "Programmatic access" section linking to both packages. `packages/cli/SMOKE.md` with the manual smoke checklist.

- Skill: `documentation-writing`
- Reviewers: documentation-reviewer
- Verify-smoke: `npx typedoc packages/sdk/src/index.ts --emit none` clean (no missing-doc warnings).
- Files to modify: `packages/sdk/README.md`, `packages/cli/README.md`, `packages/cli/SMOKE.md`, repo-root `README.md`.
- Files to read: SDK + CLI source for accurate API reference.

### Audit Wave (parallel, reviewers: none)

#### Task 10: Code Audit

Holistic code-quality audit across SDK + CLI. Read every file under `packages/sdk/src/` and `packages/cli/src/` and the new server changes in `mcp/src/{oauth,api}.rs`. Look for: maintainability, idiomatic TypeScript and Rust, ESM/Web-API correctness in SDK, error handling coverage, naming consistency.

- Skill: `code-reviewing`
- Reviewers: none

#### Task 11: Security Audit

OWASP Top 10 against SDK + CLI + server changes. Specifically: PKCE state-to-verifier-and-redirect-URI binding, JWT handling + redaction, keypair file mode enforcement on Unix + Windows, bootstrap-ticket replay protection, redirect-URI allowlist regex coverage (ipv4 + ipv6 + edge cases), CLI loopback HTTP server hardening (single-shot, 127.0.0.1-only bind, state validation), supply-chain integrity (npm provenance), no `--to-clipboard` flag exists.

- Skill: `security-auditor`
- Reviewers: none

#### Task 12: Test Audit

Test quality + coverage across SDK + CLI + server-side test additions. Verify: golden COSE fixture lockstep gate fails on drift, mock-server fault-injection coverage, bootstrap-ticket replay test, redaction tests, exit-code coverage, edge cases (expired JWT, mismatched pubkeys, offline, double-redeem). Confirm cross-runtime CI matrix catches Bun/Deno-specific regressions.

- Skill: `test-master`
- Reviewers: none

### Final Wave

#### Task 13: Pre-deploy QA

Run all unit + integration suites on Node 20 / Node 22 / Bun / Deno. Validate every acceptance criterion. Run manual smoke checklist (`packages/cli/SMOKE.md`). Verify `npm pack` outputs are within size budgets. Confirm regression: existing webapp e2e tests still green, existing Cursor / VS Code / Claude.ai OAuth flows still work against staging.

- Skill: `pre-deploy-qa`
- Reviewers: none

#### Task 14: Deploy — npm publish + GitHub release + server config + webapp deploy

`npm publish --access public --provenance` for both `@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli`. Tag `v0.1.0` on git. GitHub Release page with changelog. Deploy server changes (`mcp/src/oauth.rs` allowlist + `mcp/src/api.rs` bootstrap endpoints) and webapp changes (`IdentityPanel`) to the VPS. Update `mnemonik.xyz/install` page with a "Install in terminal" card pointing at `npm install -g @mnemonik-xyz/cli`.

- Skill: `deploy-pipeline`
- Reviewers: none

#### Task 15: Post-deploy verification

On a fresh machine (or container): `npm install -g @mnemonik-xyz/cli`, run full demo flow with bootstrap-ticket: open `mnemonik.xyz/install` in browser → "Send to CLI" → paste ticket command → `mnemonic login` → `mnemonic sign "..."` → `mnemonic recall`. Verify cross-tool: same identity logged into Claude.ai sees the CLI-signed attestation. Verify negative paths: arbitrary `redirect_uri` → 400 (regression), double-redeem of bootstrap ticket → 410, `mnemonic verify <stranger-id>` → not_found.

- Skill: `post-deploy-qa`
- Reviewers: none
- Verify-user: yes, manual flow on a fresh box.

---

**Task count: 15.** Within the 15-task cap (down from 16 in the prior draft after merging the original CLI-lifecycle and CLI-tools tasks per validator feedback).
