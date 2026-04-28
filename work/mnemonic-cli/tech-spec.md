---
created: 2026-04-29
status: draft
size: M
branch: dev
---

# Tech Spec: mnemonic-cli (Phase 1 — SDK + CLI)

## Solution

Ship two pure-ESM npm packages under the `@mnemonik-xyz` scope:

1. **`@mnemonik-xyz/sdk`** — runtime-agnostic JavaScript/TypeScript library that wraps the public Mnemonic MCP HTTP surface. Provides a `MnemonicClient` with the 5 tool methods (`whoami`, `signMemory`, `recall`, `verify`, `proveIdentity`), an OAuth 2.1 + PKCE helper supporting both interactive (browser-spawn) and headless (pre-issued JWT) modes, and a pluggable `Signer` interface (Phase 1 ships only `LocalSigner`; future `TurnkeySigner` / `WebAuthnSigner` are drop-in replacements). Distributed as ESM only. Targets: Node ≥20, Bun, Deno, Cloudflare Workers, modern browsers (for future Chrome extension).

2. **`@mnemonik-xyz/cli`** — Node-only CLI binary built on top of the SDK. Implements 7 commands: `init` (generate keypair to `~/.mnemonic/identity.json`), `login` (interactive OAuth or `--token <jwt>` headless), `sign`, `recall`, `verify`, `whoami`, `prove`. Output: human-readable on TTY (ANSI color), `--json` for machine consumption, `--quiet` for CI. Persistence is the CLI's responsibility — SDK itself is stateless beyond its in-memory client.

Both packages live in a new top-level `packages/` directory (npm workspace, not Cargo workspace). The Rust workspace is unaffected. The existing `core/src/wasm/` build (already producing `pkg/` artifacts via `wasm-pack --target web`) is consumed by the SDK as a private dependency; we add a new `wasm-pack --target bundler` build step alongside the existing `--target web` build so the resulting `.wasm` resolves correctly under Node's ESM loader, Bun's runtime, and bundlers (Vite/esbuild) used by webapp + future Chrome extension.

## Architecture

### What we're building / modifying

**New packages (top-level `packages/` directory, npm workspace):**

- `packages/sdk/` — `@mnemonik-xyz/sdk` source. Modules: `client.ts` (MnemonicClient class + 5 tool methods), `oauth.ts` (PKCE helper, interactive + headless modes), `signer.ts` (`Signer` interface + `LocalSigner` impl), `cose.ts` (thin wrapper around `@mnemonic/core` WASM `sign_cose_payload`), `keypair.ts` (Keypair JSON parse/serialize/generate), `errors.ts` (typed error hierarchy), `types.ts` (public TS types). `index.ts` re-exports the public surface.
- `packages/cli/` — `@mnemonik-xyz/cli` source. Modules: `bin/mnemonic.ts` (the binary entrypoint registered as `mnemonic` in `package.json`'s `bin` field), `commands/{init,login,sign,recall,verify,whoami,prove}.ts`, `output.ts` (TTY-aware formatter with `--json`/`--quiet` modes), `config.ts` (XDG-compliant `~/.mnemonic/` paths + persistence), `errors.ts` (CLI-specific exit code mapping).

**New `packages/core-wasm-bundler/`** — build pipeline only, not a published package. Produces `core/pkg-bundler/` via `wasm-pack build core --target bundler --features wasm` alongside the existing `core/pkg/` (web target). The SDK depends on `pkg-bundler` (works in Node + Bun + bundlers) rather than `pkg` (browser-only ESM with `import.meta.url` resolution).

**Modified files:**

- `package.json` (repo root) — convert to npm workspace root: `"workspaces": ["packages/*", "webapp"]`. Webapp already has its own `package.json`; bringing it into the workspace de-duplicates `node_modules`. Backward-compat: webapp still builds standalone via its existing `npm run build`.
- `webapp/scripts/build-wasm.sh` — additive only: keeps the existing `--target web` build for webapp, adds a parallel `--target bundler` build for SDK consumption.
- `core/Cargo.toml` — already has `[lib] crate-type = ["cdylib", "rlib"]`. No change.
- `.github/workflows/ci.yml` — add `node-test.yml` matrix step (Node 20 + Node 22 + Bun latest) running `cd packages/sdk && bun test` and `cd packages/cli && bun test`.

**Unchanged (consumed as-is via the public MCP HTTP surface):**

- `mcp/src/*` — server-side surface stays exactly as-is. CLI is a third MCP client (alongside Cursor/VS Code/Claude.ai) and uses the same `/mcp`, `/oauth/*`, `/api/sign-callback` endpoints.
- `core/src/wasm/mod.rs` — the existing `sign_cose_payload`, `sign_challenge`, `generate_keypair`, etc. exports are exactly what the SDK needs. No new WASM exports.
- `core/src/codec/canonical.rs` — canonical CBOR encoder is the source of truth; SDK does not re-implement it (calls `to_canonical_cbor` via WASM).
- All Rust tests, MCP server, webapp source.

### How it works

**Onboarding flow (`mnemonic init`):**

1. CLI reads `~/.mnemonic/` — if `identity.json` exists and `--force` is not set, refuse and print existing pubkey.
2. CLI calls `Keypair.generate()` from SDK, which calls WASM `generate_keypair()` (uses `getrandom` with the `js` feature → `crypto.getRandomValues` on Node/Bun).
3. CLI writes `~/.mnemonic/identity.json` with mode 0600 (Unix) / NTFS ACL restricting to current user (Windows).
4. Prints pubkey + DID to stdout.

**Auth flow (`mnemonic login`):**

Interactive (default):
1. CLI generates PKCE verifier (32 random bytes, base64url-encoded) + challenge (`SHA-256(verifier)` base64url).
2. CLI starts a one-shot HTTP server on `127.0.0.1:<random-free-port>` to receive the OAuth callback. Times out after 5 minutes.
3. CLI opens the system browser to `https://mc.mnemonik.xyz/oauth/authorize?response_type=code&client_id=mnemonic-cli&redirect_uri=http://127.0.0.1:<port>/callback&code_challenge=<base64url>&code_challenge_method=S256&state=<random>&scope=mcp`.
4. The browser is redirected to `mnemonik.xyz/oauth/consent` (existing webapp page), which signs the challenge using the localStorage keypair (existing flow). The user clicks "Approve". The webapp POSTs the signature to `/oauth/authorize` (existing endpoint), the server verifies and issues an authorization code, redirects browser back to `http://127.0.0.1:<port>/callback?code=<code>&state=<state>`.
5. CLI's local HTTP server receives the GET, validates `state`, exchanges `code + verifier` for a JWT via `POST /oauth/token` (existing endpoint, accepts both JSON and form-urlencoded — CLI uses JSON).
6. CLI writes JWT to `~/.mnemonic/token.json` (mode 0600), shuts down the loopback server, prints "Logged in as `<pubkey>`".

The wrinkle: the user's webapp identity may not match the CLI's local keypair. Two failure modes the design must handle gracefully:

- **Different pubkeys.** CLI's local `~/.mnemonic/identity.json` may be a fresh keypair, but webapp's `localStorage["mnemonic.identity"]` is a different one. The OAuth flow signs with the **webapp** keypair (because consent runs in the webapp). The CLI gets a JWT bound to the webapp's pubkey via `sub`. Then `mnemonic sign` would attempt to sign locally with the CLI's keypair — server rejects because COSE signer ≠ JWT `sub`. **Solution:** CLI's `mnemonic sign` always reads `~/.mnemonic/identity.json` AND the JWT — if their pubkeys disagree, abort with an error: "CLI keypair does not match logged-in identity. Run `mnemonic login --as <pubkey>` to align, or import your webapp keypair via `mnemonic identity import`." `mnemonic identity import` is in scope (see Decision 7).

- **No webapp keypair yet.** First-time CLI user runs `mnemonic init` → has only a CLI keypair. Calling `mnemonic login` opens the consent page in a fresh browser (no localStorage). The webapp's OAuth flow today depends on an already-existing localStorage keypair. **Solution:** CLI `init` writes a new keypair AND opens `https://mnemonik.xyz/install?cli-bootstrap=1&pubkey=<base58>&signature=<>` — webapp recognizes the query string, prompts "Import this keypair into your browser identity?", user clicks Import, webapp populates localStorage from the URL params (signed with same key to prevent tampering). After that, the same CLI keypair is the webapp keypair, and `mnemonic login` works against a unified identity. See Decision 7 for the full bootstrap protocol.

Headless (`mnemonic login --token <jwt>`):
1. CLI receives the pre-issued JWT via flag.
2. CLI verifies basic shape (HS256 algorithm, has `sub`, `exp` not in past). Does NOT verify signature — server will reject if invalid.
3. CLI writes `~/.mnemonic/token.json`. No browser, no callback server.

**Sign flow (`mnemonic sign`):**

1. CLI loads `~/.mnemonic/identity.json` (or fails with exit 1 if missing) and `~/.mnemonic/token.json` (or fails with exit 4 if missing/expired).
2. CLI calls `client.signMemory(content, { tags })` on the SDK.
3. SDK `client.signMemory` does the following over HTTP:
   - POST `/mcp` JSON-RPC `tools/call name=mnemonic_sign_memory params={content, tags}` with `Authorization: Bearer <jwt>`.
   - Server returns either:
     - **Inline-signed result** (if server is configured for inline-server-signing — current `STORAGE_MODE=local` server path with no browser handoff): SDK gets `attestation_id` directly. Done.
     - **Pending bundle** (if server is configured for browser-mediated signing — current production path): SDK receives `{correlation_id, sign_url, payload_cbor_base64, expires_at}`. SDK then takes the `payload_cbor_base64`, decodes it, runs WASM `sign_cose_payload(cbor_bytes, keypair)` to produce the COSE_Sign1 envelope, then POSTs it to `/api/sign-callback` with the `correlation_id` and `signer_pubkey`. Server validates the envelope (signer_pubkey matches `sub` of the JWT used to create the pending bundle), persists the attestation, returns `attestation_id`.
4. CLI formats the result (human/JSON/quiet) and exits 0.

Crucial: the SDK does NOT use the webapp's `/sign/<id>` browser flow. It uses the same `/api/sign-callback` endpoint that the webapp uses, but bypasses the browser UI because the CLI has the keypair locally. This is the "Inline COSE signing" path described in user-spec § Сценарий 3.

**Recall / verify / whoami / prove flows:** straightforward HTTP POST to `/mcp` `tools/call` with the named tool. Output formatting per `--json`/`--quiet`/TTY auto-detect. No COSE signing, no PKCE — just authenticated JSON-RPC.

### Shared Resources

**SDK runtime — none.** SDK is stateless: each `MnemonicClient` instance holds only `{baseUrl, jwt, signer}` in memory. Multiple clients in one process don't share state. No connection pool (uses native `fetch`), no DB, no model. WASM module is loaded once per Node process (cached by the loader), but that's a runtime concern, not application state.

**CLI runtime — none.** Each CLI invocation is one-shot: read config files, do one HTTP exchange, print result, exit. No daemon, no long-lived state, no shared resources.

**Build-time — `core/pkg-bundler/`.** Produced by `wasm-pack build core --target bundler --features wasm`. Owner: the new `core-wasm-bundler` build script. Consumers: SDK (imports types + WASM module via `import { sign_cose_payload, ... } from '../../../core/pkg-bundler'` — relative path inside the npm workspace, no published package). Single instance per build.

## Decisions

### Decision 1: Two packages, one substrate (`@mnemonik-xyz/sdk` + `@mnemonik-xyz/cli`)

Ships SDK and CLI as separate npm packages, with CLI depending on SDK. Supports user-spec § "Что делаем" — the explicit requirement that future Chrome extension and agent frameworks reuse the same substrate without re-implementing OAuth, COSE, or MCP wire format. CLI alone is not enough; SDK alone has no immediate consumer. Two packages cleanly partition runtime concerns: SDK is universal (Web APIs only), CLI is Node-only (filesystem, child_process, OS keychain).

Alternative considered: single combined `@mnemonik-xyz/cli` with internal but unpublished modules. Rejected because user-spec § Зачем explicitly cites Chrome extension and agent framework consumers as primary motivation.

### Decision 2: Pure ESM, Web APIs only in SDK; Node-specific code stays in CLI

SDK uses only `fetch`, `crypto.subtle`, `URL`, `TextEncoder`, `TextDecoder`. No `node:fs`, `node:http`, `node:child_process` imports. This makes SDK universal across Node ≥20, Bun, Deno, Cloudflare Workers, and modern browsers without bundler-level conditional exports.

Consequence: the OAuth interactive flow's loopback HTTP server cannot live in SDK (uses `node:http`). It lives in CLI's `commands/login.ts`. SDK exposes a primitive: `oauth.buildAuthorizeUrl({...})` returns the URL to open + the verifier+state to remember; CLI's command opens the URL via `open` package, listens on `node:http`, receives callback, calls `oauth.exchangeCodeForToken(...)` from SDK. This split lets a Chrome extension do its own `chrome.identity.launchWebAuthFlow` while reusing the same `buildAuthorizeUrl` + `exchangeCodeForToken`.

Supports user-spec MUST "Pure ESM, runtime-agnostic. No `node:*` imports in `sdk/`."

### Decision 3: COSE backend = `@mnemonic/core` WASM (`pkg-bundler` build target)

SDK consumes the existing Rust → WASM core via a new `wasm-pack build core --target bundler --features wasm` build output (alongside the existing `--target web` output for webapp). The bundler target produces ESM that resolves correctly under Node's native ESM loader (uses `fs.readFile` for the `.wasm` blob), Bun's loader, and esbuild/Vite/webpack bundlers. The web target uses `import.meta.url` + `fetch`, which works in browsers but breaks in Node CJS-leaning environments.

Same canonical CBOR + COSE_Sign1 logic the server uses to verify — byte-for-byte identical, because it IS the same code. Eliminates the entire class of bugs around "JS canonical CBOR almost matches Rust canonical CBOR but differs in 0.1% of edge cases."

Cost: 442KB WASM in SDK bundle. Acceptable for Phase 1; swap to `@noble/curves` + custom CBOR is in backlog if size complaints arrive. Public SDK API does not depend on the COSE backend, so swap is invisible to consumers.

Supports user-spec "COSE round-trip CBOR byte-for-byte without re-encoding".

### Decision 4: `Signer` interface for keypair abstraction

```typescript
interface Signer {
  pubkey: string;                                 // base58 Ed25519 pubkey
  sign(bytes: Uint8Array): Promise<Uint8Array>;   // raw 64-byte Ed25519 sig
}
```

Phase 1 ships `LocalSigner` (in-memory secret, signs via WASM `sign_with_secret`). Future `TurnkeySigner` (Phase 1.5), `WebAuthnSigner` (Phase 2) are drop-in replacements without API change.

`MnemonicClient` accepts `signer: Signer` in its constructor; never inspects the secret directly. This is the architectural opening for the user-spec § "future Turnkey compatibility" requirement.

### Decision 5: OAuth interactive mode uses loopback redirect (`http://127.0.0.1:<random-port>/callback`)

Standard practice for native CLI OAuth (RFC 8252). The CLI binds to a random free port via `node:net.createServer().listen(0)`, then registers `http://127.0.0.1:<port>/callback` as the `redirect_uri` in the authorize URL. PKCE (`S256`) + a server-side `redirect_uri` allowlist are sufficient mitigations against malicious local apps stealing the code (see RFC 8252 §7).

**Server-side change required:** `mcp/src/oauth.rs` currently allows the webapp origin and a static set of client redirect URIs (Cursor, VS Code, Claude.ai). It needs to allow loopback URIs of the form `http://127.0.0.1:*` and `http://[::1]:*` for `client_id=mnemonic-cli`. This is a single regex/predicate addition in the existing redirect-URI allowlist. Documented in Deviation 1 below — the user-spec did not anticipate that server config change.

### Decision 6: OAuth headless mode = `--token <jwt>` opaque pass-through

`mnemonic login --token <jwt>` writes the user-supplied JWT to `~/.mnemonic/token.json` after minimal shape validation (header is HS256, payload has `sub`, `exp` is in the future). Signature is not verified on the client — server rejects invalid JWTs on first request, which the CLI surfaces as exit code 4.

This shape works for: CI environments (token issued via webapp, pasted into env var), serverless functions, headless Docker. The token's 1-hour TTL applies — refresh tokens are explicitly backlog (user-spec ограничения "JWT TTL = 1h").

### Decision 7: CLI ↔ webapp identity bootstrap

Two bootstrap paths to handle the keypair-mismatch problem identified in § "Architecture → How it works":

1. **CLI-first user (typical):** `mnemonic init` generates a keypair locally. On the next `mnemonic login`, CLI detects no `~/.mnemonic/token.json`, opens the browser to `https://mnemonik.xyz/install?cli-bootstrap=1&pubkey=<base58>&signature=<base64url-sig-of-pubkey>`. The webapp's `/install` page checks for the `cli-bootstrap` flag, prompts "Import CLI keypair into browser identity?". Verifies the signature is over the pubkey using that same pubkey (proves possession of secret). On user approval, the user pastes the secret bytes manually (or the CLI supports `mnemonic identity export --to-clipboard` and the webapp reads from clipboard). After import, both sides share the same identity, OAuth flow proceeds normally.

2. **Webapp-first user:** user already has `localStorage["mnemonic.identity"]` set up. They run `mnemonic init --import-from-webapp`, which prints a URL `https://mnemonik.xyz/install?cli-export=1`. Webapp shows "Export keypair to CLI: copy this command" and renders `mnemonic identity import '<base64-of-keypair-json>'` for the user to copy and paste in their terminal. CLI command parses, validates, writes `~/.mnemonic/identity.json`. After this, both sides share identity.

This is the ugly part of the design. Reason: the OAuth challenge-signing happens in the webapp (existing flow), but the COSE-signing for `sign_memory` happens in the CLI (Decision 3 of user-spec). They MUST use the same key, otherwise server rejects the COSE envelope after issuing the JWT against a different pubkey.

Three CLI commands implement this: `mnemonic identity export [--to-clipboard]`, `mnemonic identity import <base64-or-path>`, and the implicit `--cli-bootstrap` URL parameter on `mnemonic init` to pre-fill the import on the webapp side. Adds a small `mnemonic identity` subcommand surface beyond the user-spec's 7 commands.

### Decision 8: CLI persistence at `~/.mnemonic/{identity.json,token.json}`, mode 0600

Plain JSON files in the user's home directory. Mode 0600 on Unix; on Windows, NTFS ACL setting `Restrict to current user` (via `node:fs.chmodSync` is a no-op on Windows; we use `winston` or `acl-windows`-style helper). Plain JSON, not encrypted at rest — same security model as Cursor's `~/.cursor/`, `gh`'s `~/.config/gh/`, `npm`'s `~/.npmrc`. OS keychain integration (macOS Keychain / Linux Secret Service / Windows Credential Manager) is in backlog.

XDG support via `XDG_CONFIG_HOME` env (`~/.config/mnemonic/` if set) is in backlog — Phase 1 just uses `~/.mnemonic/` for simplicity.

### Decision 9: npm scope = `@mnemonik-xyz`

User-confirmed: org `mnemonik-xyz` is registered on npm; the more compact `@mnemonik` and `@mnemonic` scopes were already taken. Publishing under `@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli`. Migration to a shorter scope is a future deploy task if either becomes available.

### Decision 10: Output format details

- **Default (TTY):** ANSI color, human-readable, structured per command.
- **Default (pipe / non-TTY):** plain text, no color.
- **`--json`:** machine-readable JSON to stdout, all human-oriented messages (progress, hints) to stderr.
- **`--quiet`:** suppress all stdout except `--json` payload (still emitted) and exit code.
- **`--no-color`:** force plain text on TTY.

Exit codes: `0` success, `1` user error (bad args, missing files), `2` server/network error (5xx, connection refused), `3` integrity failure (verify=tampered), `4` auth error (no token, expired, invalid signature). These match user-spec § Критерии приёмки.

### Decision 11: Cross-runtime CI matrix

CI runs unit + integration on **Node 20, Node 22, Bun latest**. Deno + Cloudflare Workers smoke is manual pre-release (backlog → automate). Bun is included from day 1 because user-spec § Q10 explicitly chose Bun-included as a primary runtime; treating it as a first-class CI target prevents Bun-specific regressions (e.g. `crypto.subtle` Ed25519 support, ESM resolution edge cases).

### Decision 12: Test fixture: "golden COSE round-trip" against Rust

A test fixture in `packages/sdk/test/fixtures/golden-cose.json` containing pairs `{input_bytes_hex, expected_canonical_cbor_hex, expected_cose_envelope_hex}` generated by running the existing Rust `core` crate and capturing outputs. SDK unit test asserts WASM `sign_cose_payload` produces byte-identical output. If WASM ever drifts from Rust (e.g. wasm-bindgen ABI change), this test catches it before any server-side rejection.

The fixture is regenerated whenever Rust core's CBOR/COSE encoder changes, via a small `cargo test --features golden-fixtures` target that emits JSON. CI runs `cargo test --features golden-fixtures` and the SDK test in a single workflow so they stay in lockstep.

[TECHNICAL] Justification: user-spec MUST mentions "COSE-signed CBOR byte-for-byte" but doesn't prescribe how. This fixture is the implementation mechanism.

### Decision 13: SDK is published, CLI bin is published; no internal packages published

`@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli` are public npm packages. Internal helpers (`packages/core-wasm-bundler/` build script, golden fixtures, etc.) live in the monorepo but are not published. `package.json` of each public package lists only the public surface in `exports`.

[TECHNICAL] Justification: prevents accidental publishing of internal helpers + keeps the npm-published surface small.

## Data Models

**No new SQLite tables.** CLI is a client of the existing MCP server; all writes go through the existing `attestations` / `memory_embeddings` / `attestation_costs` tables, scoped by the existing `owner_pubkey` column.

**New file formats (CLI-local):**

- `~/.mnemonic/identity.json`:
  ```json
  {
    "secret": [/* 64-byte Solana keypair (32 seed + 32 pubkey), as number[64] */],
    "pubkey_base58": "..."
  }
  ```
  Identical shape to `localStorage["mnemonic.identity"]` in webapp. Mode 0600.

- `~/.mnemonic/token.json`:
  ```json
  {
    "jwt": "eyJ...",
    "pubkey_base58": "...",
    "issued_at": "2026-04-29T10:23:45Z",
    "expires_at": "2026-04-29T11:23:45Z"
  }
  ```
  `pubkey_base58` is decoded from the JWT's `sub` claim and stored for fast lookup without parsing JWT every command. Mode 0600.

**SDK public types (TypeScript):**

```typescript
export interface SignerInterface {
  pubkey: string;
  sign(bytes: Uint8Array): Promise<Uint8Array>;
}

export interface MnemonicClientConfig {
  baseUrl: string;                    // e.g. "https://mc.mnemonik.xyz"
  signer: SignerInterface;
  jwt?: string;                       // headless mode
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

- `@mnemonik-xyz/core-wasm` (workspace-internal, built from `core/pkg-bundler/`) — COSE / canonical CBOR / Ed25519 via WASM.
- `@noble/ed25519` ≥ 2.1 (≈ 12KB) — fallback signer for runtimes where `crypto.subtle.sign({name:'Ed25519'})` is unavailable. Loaded lazily.

### New packages (`packages/cli/package.json`)

- `@mnemonik-xyz/sdk` (workspace dependency).
- `commander` ≥ 12 — argv parsing.
- `kleur` (≈ 1KB) — ANSI color, no dependencies. Smaller than `chalk`.
- `open` ≥ 10 — cross-platform browser open (replaces `node:child_process` boilerplate).

### Devdependencies (both packages)

- `vitest` ≥ 1.6 — unit + integration test runner. Already used in webapp.
- `typescript` ≥ 5.4.
- `@types/node` for CLI only.

### Removed packages — None.

### Existing (used as-is)

- `wasm-pack` (already installed on dev + VPS).
- Existing `@mnemonic/core` Rust crate code in `core/`. No Rust source changes.

## Testing Strategy

Per user-spec size **M** and § "Тестирование": four layers.

### Unit tests (vitest, every PR)

- **SDK:** mock `fetch`, assert request shapes for each of the 5 tool methods. Specifically: OAuth `buildAuthorizeUrl` produces correct PKCE+state params; `exchangeCodeForToken` POSTs to `/oauth/token` with correct body; `signMemory` correctly handles both inline-signed and pending-bundle response shapes; `LocalSigner.sign(bytes)` produces deterministic 64-byte Ed25519 signature verifiable via `verify_signature` (round-trip check).
- **`Signer` contract:** abstract test suite that any `Signer` impl must pass. `LocalSigner` passes it; future `TurnkeySigner` is required to pass it.
- **Golden COSE fixture:** see Decision 12.
- **CLI:** argv parser per command, output formatter for TTY/pipe/json/quiet, exit-code mapping for known errors.

Coverage target: SDK ≥85% lines / ≥80% branches. CLI ≥75% lines.

### Integration tests (vitest + mock HTTP server, every PR)

- Spin up a mock MCP server in-process (handlers for `/mcp`, `/oauth/authorize`, `/oauth/token`, `/api/sign-callback`). Assert SDK end-to-end flows: OAuth interactive (skipping browser, calling `exchangeCodeForToken` directly with mocked code), `signMemory` against pending-bundle response, recall, verify.
- CLI through `execa` against the same mock server. Asserts: correct stdout/stderr/exit code per scenario.

### Manual smoke tests (pre-release checklist in `tasks/`)

- `npm install -g @mnemonik-xyz/cli` from a freshly built `.tgz`.
- `mnemonic init` → identity file appears, mode 0600.
- `mnemonic login` → browser opens, OAuth flow completes, token appears.
- `mnemonic sign "hello"` → attestation_id returned within 5s.
- `mnemonic recall "hello"` → finds the just-signed attestation.
- `mnemonic verify <id>` → exit 0.
- Cross-tool check: same pubkey logged into Claude.ai sees the CLI-signed attestation via `mnemonic_recall`.

### E2E tests (release pipeline, not PR-gating)

- One scenario: `init → login --token <pre-issued> → sign → recall` against a real `STORAGE_MODE=local` self-hosted MCP server on the CI runner. Token pre-issued via `mcp/src/bin/mint-test-jwt.rs`. Validates that real network + real WASM + real CBOR + real OAuth actually compose end-to-end.

### Cross-runtime matrix

Unit + integration suites run on **Node 20, Node 22, Bun latest** in CI. Deno + Cloudflare Workers smoke runs manually before each release (automate → backlog).

## Agent Verification Plan

### Verification approach

Most of the spec is internally testable via unit + integration tests against mocks. Two areas need real-environment verification:

1. **OAuth interactive flow against live `mc.mnemonik.xyz`.** Mock can't replicate the full PKCE round-trip including loopback callback. Verified manually pre-release.
2. **WASM bundler-target build resolves correctly under Node 20, Node 22, Bun.** Smoke-tested in CI matrix; manually verified once before merge.
3. **`mnemonic install` deeplink + `cli-bootstrap` URL on webapp** (Decision 7) — webapp-side verification needs Playwright MCP since it's a UI flow.

### Tools required

- **Bash MCP** (basic) — install package, run smoke commands, inspect file modes.
- **Playwright MCP** — verify the webapp `cli-bootstrap` page renders, accepts the URL params, and writes localStorage on user approval.
- **None of:** browser/macOS-use, third-party API credentials. CLI is fully client-side; verification is in CI + one manual smoke pass.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `wasm-pack --target bundler` produces ESM that breaks under Bun's module resolution | Medium | High (SDK dead-on-arrival) | CI matrix runs `bun test` on every PR. Fallback: ship `--target nodejs` build alongside, conditional export based on package consumer. |
| `crypto.subtle.sign({name:'Ed25519'})` not implemented in Cloudflare Workers / Deno older versions | Medium | Medium | Lazy-load `@noble/ed25519` (12KB) as fallback if `subtle.sign` rejects with `NotSupportedError`. Detected in `LocalSigner.sign()`. |
| User runs `mnemonic init` on a machine that already had a webapp identity → mismatch on next `sign` | High | Medium (UX confusion) | `mnemonic init` checks for an existing `~/.mnemonic/identity.json` AND warns "you may want `--import-from-webapp` if you've used Mnemonic in a browser before." Documented in `--help`. |
| OAuth loopback redirect blocked by corporate firewall / strict no-localhost-http policy | Low | High for affected users | Documented headless `--token` fallback; webapp's `/install` page shows a "copy JWT for CLI" button that issues a long-lived token (15 min) for one-shot pasting. Not implemented in Phase 1, in backlog if reports come in. |
| 442KB WASM bloats `@mnemonik-xyz/sdk` to >500KB | Medium | Low (slower install, fine for CLI users; fine for Chrome ext via lazy-load) | Bundle size budget in CI; alert if exceeds 600KB. Swap to `@noble/curves` listed in backlog. |
| Server `mcp/src/oauth.rs` redirect-URI allowlist doesn't allow loopback URIs → CLI OAuth fails on day 1 | High (without the change) | High (CLI cannot login interactively at all) | Decision 5: add the loopback allowlist as one of the implementation tasks. Documented in Deviation 1. |
| Webapp `cli-bootstrap` import flow (Decision 7) introduces a phishing vector — attacker hosts a fake `?cli-bootstrap=1&pubkey=...&signature=...` page | Low | Medium | Webapp validates the signature is over the pubkey using that pubkey before importing. Same as importing any keypair-shaped JSON: the user must trust the source. Documented in webapp UI as "Only import from your own CLI." |
| Hackathon judges don't see CLI on stage — same risk as user-spec | High | Medium | Demo plan: open terminal alongside Claude.ai, do `mnemonic sign` in terminal, switch to Claude.ai, ask "recall what I just signed via terminal" — Claude finds it. Tangible cross-tool demo. |

## User-Spec Deviations

Each entry is `[PENDING USER APPROVAL]` until you accept it.

### Deviation 1: Server-side change to `mcp/src/oauth.rs` (loopback redirect URIs)

**User-spec says:** "no server changes needed beyond loading the CLI as another MCP client."
**Tech-spec does:** adds a single allowlist entry for `http://127.0.0.1:*` and `http://[::1]:*` redirect URIs, gated to `client_id=mnemonic-cli`. This is a ~10 LOC change in `mcp/src/oauth.rs`.
**Why:** OAuth 2.1 requires the server to validate redirect URIs against an allowlist. The existing list contains only the three editor-MCP redirect schemes. CLI's loopback URI doesn't match any of them. RFC 8252 is the standard pattern; the alternative (some other auth mechanism) is much worse architecturally. **`[PENDING USER APPROVAL]`**

### Deviation 2: New CLI subcommand `mnemonic identity export|import`

**User-spec says:** 7 commands (init, login, sign, recall, verify, whoami, prove).
**Tech-spec adds:** `mnemonic identity export [--to-clipboard]` and `mnemonic identity import <base64-or-path>`, plus the implicit `--cli-bootstrap` URL on `mnemonic init`.
**Why:** Decision 7 — the CLI ↔ webapp identity bootstrap problem. Without these, the OAuth flow can issue a JWT bound to a pubkey that the CLI cannot sign for, breaking `mnemonic sign`. Forcing users to manually copy keypair JSON via filesystem is worse UX. **`[PENDING USER APPROVAL]`**

### Deviation 3: Cross-runtime CI matrix includes Node 22 (not just Node 20)

**User-spec says:** Node ≥20.
**Tech-spec runs CI on:** Node 20, Node 22, Bun latest.
**Why:** Node 22 is current LTS-track; we want to catch regressions early. Trivial cost (just another job in the matrix). **`[PENDING USER APPROVAL]`** — could drop to just Node 20 + Bun if CI minutes matter.

### Deviation 4: New top-level `packages/` directory + npm workspaces at repo root

**User-spec implies:** packages live somewhere reasonable.
**Tech-spec specifies:** `packages/sdk/`, `packages/cli/`, repo-root `package.json` becomes an npm workspace with `"workspaces": ["packages/*", "webapp"]`.
**Why:** Standard JS monorepo pattern; lets webapp eventually consume SDK without duplicate `node_modules`. Brings `webapp/` into the workspace too, but its own build chain is unaffected (same `npm run build`). **`[PENDING USER APPROVAL]`** — alternative: put SDK + CLI in webapp/ subfolder. Worse separation of concerns.

### Deviation 5: New `wasm-pack --target bundler` build alongside existing `--target web`

**User-spec implies:** WASM is consumed by SDK.
**Tech-spec specifies:** add a parallel build to produce `core/pkg-bundler/`. The existing `--target web` build for webapp stays.
**Why:** Decision 3. Necessary for SDK to load WASM correctly under Node, Bun, and bundlers. **`[PENDING USER APPROVAL]`**

## Acceptance Criteria

(carried through from user-spec § Критерии приёмки; tech-spec adds concrete artifact/tooling references)

- [ ] **`packages/sdk/dist/`** is published to npm as `@mnemonik-xyz/sdk` (or ready: `npm pack` produces tgz ≤500KB, `npm publish --dry-run` clean).
- [ ] **`packages/cli/dist/bin/mnemonic`** is published as `@mnemonik-xyz/cli`; `npm install -g @mnemonik-xyz/cli` registers `mnemonic` on PATH.
- [ ] **Pure ESM, runtime-agnostic.** `grep -r 'node:' packages/sdk/src/` returns empty. CI green on Node 20 + Node 22 + Bun.
- [ ] **CLI has 7 user-spec commands + 2 deviation commands** (`identity export`, `identity import`), all with `--help`.
- [ ] **Output:** TTY-aware default; `--json`, `--quiet`, `--no-color` all observable in tests.
- [ ] **Exit codes** per Decision 10, asserted in CLI integration tests.
- [ ] **OAuth interactive** end-to-end against `mc.mnemonik.xyz`: browser opens, callback received, JWT in `~/.mnemonic/token.json`.
- [ ] **OAuth headless** end-to-end: `--token <jwt>` skips browser, JWT persisted.
- [ ] **Inline COSE signing:** SDK does WASM `sign_cose_payload` locally, POSTs `/api/sign-callback`. Verified via golden fixture.
- [ ] **`Signer` interface** decoupled from `LocalSigner` — abstract contract test suite passes for `LocalSigner`, ready for future impls.
- [ ] **Webapp browser-mediated signing** continues to work (regression test: existing webapp e2e tests pass unchanged).
- [ ] **CI:** unit + integration tests on Node 20 / Node 22 / Bun. SDK + CLI test suites pass without network.
- [ ] **Documentation:** `packages/sdk/README.md` (quick-start + types), `packages/cli/README.md` (commands + examples), JSDoc on all public SDK methods.
- [ ] **Demo:** `npm install -g @mnemonik-xyz/cli && mnemonic init && mnemonic login && mnemonic sign "..."` works on a fresh macOS / Linux box.

## Implementation Tasks

### Wave 1: Foundation (parallel)

#### Task 1: npm workspace + `packages/` skeleton + Rust bundler-target build

Convert the repo root `package.json` to an npm workspace including `packages/*` and the existing `webapp`. Create empty `packages/sdk/` and `packages/cli/` skeletons with `package.json`, `tsconfig.json`, `vitest.config.ts`. Add `wasm-pack build core --target bundler --features wasm --out-dir core/pkg-bundler` to `webapp/scripts/build-wasm.sh` and a new `packages/sdk/scripts/build-wasm.sh`. Verify the bundler-target build produces `.wasm` + `.js` glue that loads under Node 20 + Bun.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor
- Verify-smoke: `cd packages/sdk && bun -e "import('@mnemonic/core-wasm-bundler').then(m => console.log(typeof m.sign_cose_payload))"` prints `function`.
- Files to modify: `package.json` (root), `webapp/scripts/build-wasm.sh`, `webapp/package.json` (move into workspace).
- Files to read: existing `webapp/package.json`, `webapp/scripts/build-wasm.sh`, `core/Cargo.toml`, `core/src/wasm/mod.rs`.

#### Task 2: SDK core — `MnemonicClient` + `Signer` interface + `LocalSigner` + `Keypair`

Implement the SDK's stateless client surface: `MnemonicClient` class with HTTP-based methods for the 5 MCP tools, `Signer` interface and the `LocalSigner` implementation (signs via WASM `sign_with_secret`), `Keypair` helpers (generate, fromJSON, toJSON), and the public TS types from § Data Models. No OAuth code in this task — that lives in Task 3.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: SDK unit-test file `packages/sdk/test/client.test.ts` runs and `signMemory()` mock-test passes.
- Files to modify: `packages/sdk/src/{client,signer,keypair,types,errors,index}.ts`, `packages/sdk/test/{client,signer}.test.ts`.
- Files to read: `mcp/src/{tools,oauth,api}.rs`, `core/src/wasm/mod.rs`, `webapp/src/pages/Sign.tsx` (for the inline-signing model used by SDK).

#### Task 3: SDK OAuth — `buildAuthorizeUrl`, `exchangeCodeForToken`, headless mode

Implement the OAuth 2.1 + PKCE primitives in `packages/sdk/src/oauth.ts`. PKCE verifier+challenge generation via `crypto.subtle.digest`. State token via `crypto.getRandomValues`. `exchangeCodeForToken(code, verifier, redirectUri)` POSTs `/oauth/token` with JSON body and returns JWT. Headless mode is just the absence of these calls — `MnemonicClient({jwt})` constructor accepts the pre-issued token directly. No `node:http` server here — that lives in CLI's `commands/login.ts`.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `bun test packages/sdk/test/oauth.test.ts` passes.
- Files to modify: `packages/sdk/src/oauth.ts`, `packages/sdk/test/oauth.test.ts`.
- Files to read: `mcp/src/oauth.rs`, `webapp/src/pages/Consent.tsx`.

### Wave 2: COSE round-trip + CLI commands (parallel)

#### Task 4: SDK COSE wrapper + golden fixture

Implement `packages/sdk/src/cose.ts` as a thin wrapper around the WASM `sign_cose_payload` export. Build the golden-fixture pipeline: a `cargo test --features golden-fixtures` target in `core/tests/` that emits `{input, canonical_cbor, cose_envelope}` JSON triples; SDK unit test asserts WASM output matches each triple byte-for-byte. Wire into CI so any CBOR/COSE encoder change in Rust runs both halves of the test in lockstep.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `cargo test --features golden-fixtures -p mnemonic-core && bun test packages/sdk/test/cose.golden.test.ts` both green.
- Files to modify: `packages/sdk/src/cose.ts`, `packages/sdk/test/cose.golden.test.ts`, `core/tests/golden_fixtures.rs`, `core/Cargo.toml` (golden-fixtures feature flag).
- Files to read: `core/src/codec/canonical.rs`, `core/src/codec/cose.rs`, `core/src/wasm/mod.rs`.

#### Task 5: CLI commands — init, login, identity {export, import}

Implement `packages/cli/bin/mnemonic.ts` (argv routing via `commander`) plus the four lifecycle commands: `init` (generates keypair, supports `--cli-bootstrap` URL emission), `login` (interactive OAuth via loopback HTTP server using `node:http` + `open`; `--token <jwt>` headless path), `identity export [--to-clipboard]`, `identity import <base64-or-path>`. Implements the bootstrap protocol of Decision 7. Persistence to `~/.mnemonic/{identity,token}.json` mode 0600 (Unix) / restricted ACL (Windows).

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `bun test packages/cli/test/{init,login,identity}.test.ts` passes (mock OAuth server in fixture).
- Verify-user: on a fresh dev machine: `cd packages/cli && bun link && mnemonic init && cat ~/.mnemonic/identity.json` — file exists, mode 0600, contains `pubkey_base58`.
- Files to modify: `packages/cli/bin/mnemonic.ts`, `packages/cli/src/commands/{init,login,identity}.ts`, `packages/cli/src/{config,output,errors}.ts`.
- Files to read: `webapp/src/components/IdentityPanel.tsx` (for the localStorage shape compatibility), `mcp/src/oauth.rs`, `webapp/src/pages/Consent.tsx`.

#### Task 6: CLI commands — sign, recall, verify, whoami, prove + output formatter

Implement the five MCP-tool-mapped commands: each loads identity + token from `~/.mnemonic/`, instantiates `MnemonicClient`, calls the corresponding SDK method, formats and prints. Implements `packages/cli/src/output.ts` with TTY detection + `--json`/`--quiet`/`--no-color`. Exit codes per Decision 10.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor, test-reviewer
- Verify-smoke: `bun test packages/cli/test/{sign,recall,verify,whoami,prove,output}.test.ts` passes.
- Files to modify: `packages/cli/src/commands/{sign,recall,verify,whoami,prove}.ts`, `packages/cli/src/output.ts`.
- Files to read: SDK from Task 2, `mcp/src/tools.rs`.

#### Task 7: Server-side OAuth loopback allowlist

In `mcp/src/oauth.rs`, extend the redirect-URI allowlist to accept `http://127.0.0.1:<any-port>/callback` and `http://[::1]:<any-port>/callback` for `client_id=mnemonic-cli`. Add unit tests for the new allowlist entries (positive: localhost loopback accepted; negative: arbitrary HTTP redirect rejected). Documented in Deviation 1.

- Skill: `code-writing`
- Reviewers: code-reviewer, security-auditor
- Verify-smoke: `cargo test -p mnemonic-mcp oauth_allowlist` green.
- Files to modify: `mcp/src/oauth.rs`, `mcp/src/cors_policy.rs` if relevant.
- Files to read: existing `mcp/src/oauth.rs`, RFC 8252 § 7.

### Wave 3: Tests + docs + cross-runtime CI (parallel)

#### Task 8: Integration tests against in-process mock MCP server

Build a mock MCP server in `packages/sdk/test/mock-server.ts` that listens on a free port and responds to `/mcp`, `/oauth/authorize`, `/oauth/token`, `/api/sign-callback`. SDK integration tests run end-to-end flows (login, sign-via-pending-bundle, recall, verify) against it. CLI integration tests via `execa`.

- Skill: `code-writing`
- Reviewers: code-reviewer, test-reviewer
- Verify-smoke: `bun test packages/sdk/test/integration/ packages/cli/test/integration/` passes without network access.
- Files to modify: `packages/sdk/test/mock-server.ts`, `packages/sdk/test/integration/*.test.ts`, `packages/cli/test/integration/*.test.ts`.
- Files to read: SDK + CLI source; `mcp/src/{mcp,oauth,api}.rs` to mirror handler shapes.

#### Task 9: CI matrix (Node 20 / Node 22 / Bun) + bundle-size budget

Add `.github/workflows/node-test.yml` that runs `bun install && bun test` per package on Node 20, Node 22, Bun latest. Adds a `bundle-size-check` job that runs `npm pack` on each package, asserts `<500KB` for SDK and `<200KB` for CLI (tighter — CLI is mostly argv parsing). Fails if exceeded.

- Skill: `deploy-pipeline`
- Reviewers: code-reviewer, deploy-reviewer
- Verify-smoke: open a draft PR, observe matrix runs all three runtimes green; bundle-size job reports actual sizes.
- Files to modify: `.github/workflows/node-test.yml` (new), `packages/{sdk,cli}/package.json` (any test scripts).
- Files to read: existing `.github/workflows/ci.yml` for Rust matrix patterns.

#### Task 10: Documentation — SDK README, CLI README, JSDoc, repo-root pointer

`packages/sdk/README.md`: 5-line quick-start, public API reference, runtime-target table, link to backlog. `packages/cli/README.md`: command reference (`init / login / sign / recall / verify / whoami / prove / identity`), examples, exit-code table. JSDoc on all `MnemonicClient` methods + `Signer` interface (rendered to types-only `.d.ts`). Repo-root `README.md` gets a "Programmatic access" section linking to both packages.

- Skill: `documentation-writing`
- Reviewers: documentation-reviewer
- Verify-smoke: `npx typedoc packages/sdk/src/index.ts --emit none` clean (no missing-doc warnings).
- Files to modify: `packages/sdk/README.md`, `packages/cli/README.md`, repo-root `README.md` (one-paragraph addition).
- Files to read: SDK + CLI source for accurate API reference.

### Audit Wave (parallel, reviewers: none)

#### Task 11: Code Audit

Holistic code-quality audit across all SDK + CLI source. Read every file under `packages/sdk/src/` and `packages/cli/src/`. Look for: maintainability, idiomatic TypeScript, correct ESM/Web-API usage (no `node:*` leak in SDK), error handling coverage, naming consistency.

- Skill: `code-reviewing`
- Reviewers: none

#### Task 12: Security Audit

OWASP Top 10 against SDK + CLI. Specifically: PKCE state validation, JWT handling (no leak in logs/errors), keypair file mode enforcement, `--cli-bootstrap` URL phishing surface, redirect-URI canonicalization on the server side, CLI's loopback HTTP server hardening (single-shot, IP filtering, CSRF state, port binding to 127.0.0.1 only).

- Skill: `security-auditor`
- Reviewers: none

#### Task 13: Test Audit

Test quality + coverage across SDK + CLI test suites. Verify the golden-COSE fixture, mock-server fidelity, exit-code coverage, edge cases (expired JWT, mismatched pubkeys, offline scenarios). Confirm the cross-runtime CI matrix actually catches Bun-specific regressions (try one synthetic Bun-only failure to validate the matrix).

- Skill: `test-master`
- Reviewers: none

### Final Wave

#### Task 14: Pre-deploy QA

Run all unit + integration suites on Node 20 / Node 22 / Bun. Validate every acceptance criterion in user-spec + tech-spec. Run the manual smoke checklist. Confirm `npm pack` outputs are within size budgets. Confirm regression: existing webapp e2e tests still green.

- Skill: `pre-deploy-qa`
- Reviewers: none

#### Task 15: Deploy — npm publish + GitHub release

`npm publish --access public` for both `@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli`. Tag `v0.1.0` on git. GitHub Release page with changelog from this tech-spec. Update `mnemonik.xyz/install` page (webapp) with a new "Install in terminal" card pointing at `npm install -g @mnemonik-xyz/cli`.

- Skill: `deploy-pipeline`
- Reviewers: none

#### Task 16: Post-deploy verification

On a fresh machine (or container): `npm install -g @mnemonik-xyz/cli`, run the full demo flow: `init → login → sign "..." → recall → verify`. Verify cross-tool: same identity logged into Claude.ai sees the CLI-signed attestation. Verify negative path: `mnemonic verify <some-other-user-id>` returns `not_found` (cross-tenant isolation holds).

- Skill: `post-deploy-qa`
- Reviewers: none
- Verify-user: yes, manual flow on a fresh box.

---

**Task count: 16.** Above the 15-task soft cap by one. Tasks 5 and 6 could be merged into a single CLI commands task (saves 1 task), but separating them keeps each one atomic enough to parallelize and review independently. **`[PENDING USER APPROVAL]`** to keep at 16, or merge.
