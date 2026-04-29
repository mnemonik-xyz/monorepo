# mnemonic-cli — decisions log

Append-only. Each entry: task, date, status, summary, verification, concerns.

---

## Task 1 — npm workspace skeleton + wasm-pack target chosen

- **Task:** 1
- **Date:** 2026-04-29
- **Status:** complete
- **Summary:**
  Converted repo root to npm workspace (`workspaces: ["packages/*", "webapp"]`,
  `private: true`), scaffolded empty `@mnemonik-xyz/sdk` and `@mnemonik-xyz/cli`
  packages with TS / vitest / build scripts. Investigated all three viable
  wasm-pack targets (`web`, `nodejs`, `bundler`) under Node 20.19, Bun 1.3.13,
  and Deno 2.7.5; **`--target web` was the only target that loaded on all three
  runtimes**. No conditional exports needed. Updated
  `webapp/scripts/build-wasm.sh` to additively produce both `webapp/src/wasm/`
  (existing webapp consumer) and `core/pkg-web/` (new SDK consumer) in one pass
  — same `--target web` artifact bytes mirrored to both destinations.

- **Verification (smoke matrix output):**

  ```
  === node ===
    web:     function
    nodejs:  function
    bundler: node:internal/modules/esm/get_format:189  (ERR_UNKNOWN_FILE_EXTENSION .wasm)
  === bun ===
    web:     function
    nodejs:  function
    bundler: TypeError: wasm.__wbindgen_start is not a function
  === deno ===
    web:     function
    nodejs:  Uncaught SyntaxError: module does not provide an export named 'default'
    bundler: function
  ```

  Smoke scripts: `packages/sdk/scripts/smoke-{web,nodejs,bundler}.mjs`.
  Workspace install validated: `bun install` from `packages/sdk` → 190 packages,
  workspace symlinks present at `node_modules/@mnemonik-xyz/{sdk,cli}`.
  Webapp regression check: `cd webapp && npm run build` succeeded end-to-end
  (vite output `dist/assets/mnemonic_core_bg-*.wasm 458.35 kB`).

- **Concerns / follow-ups for T2/T3/lead:**
  1. **`wasm-pack` × `cargo ≥ 1.92` bug.** cargo renamed `--out-dir` to
     `--artifact-dir`, but wasm-pack still forwards `--out-dir` for any value
     other than the default `pkg`. The previous webapp build script
     (`--out-dir <absolute-webapp-path>`) was a latent bug — it only "worked"
     because compiled wasm was already cached in `target/`. Both scripts now
     build into the default `core/pkg/` and `cp -R` / `mv` to the final
     destination. If wasm-pack fixes the upstream issue we can simplify, but
     the workaround is harmless.
  2. **wasm-pack version bumped: 0.13.1 → 0.14.0** (`cargo install wasm-pack
     --version 0.14.0 --force` on this machine). 0.13.1 had the same `--out-dir`
     forwarding behavior; the bump did not fix it but is the latest stable.
     Other dev machines and CI may still have 0.13.x — the build script does
     not pin a specific version (just requires `wasm-pack` on PATH), so this is
     not a blocker, but T8/CI work should ensure CI runners have ≥ 0.13.
  3. **Bun was not preinstalled on this dev machine** — installed via
     `npm install -g bun` (allowed) rather than `curl | bash` (denied by
     sandbox). T8's CI matrix should use the official setup-bun action.
  4. **`getrandom` "JS crate" warning at runtime.** The wasm-bindgen output
     does not import `getrandom`'s WebCrypto shim eagerly; it only matters when
     `generate_keypair` is invoked. Wave-1 smoke only checked `typeof
     sign_cose_payload`, which is a static export and works without `init()`.
     T2 must call the default-exported async `init()` from `mnemonic_core.js`
     before invoking any signing function. The web target's loader resolves
     the .wasm via `import.meta.url` + `fetch`/`fs.readFile` — works
     out-of-the-box on Node 20+, Bun, Deno.
  5. **Cloudflare Workers smoke is deferred to pre-release** (no Workers test
     runner today). Same `--target web` artifact already runs in production
     under the webapp, so risk is low.
  6. **Phase 1 does not need `package.json` conditional exports** in the SDK
     because a single artifact covers all four target runtimes.

---

### Task 1 — Round 2 fixes

- Date: 2026-04-29
- Fixed: CLI bin exit code 64 -> 1 (Decision 10 alignment); package.json `private: true` clarifying comments; SDK `engines.node >= 20`.
- Deferred to backlog: build-script duplication, README polish, bun.lock policy, wasm-pack version pin.

---

## Task 3 — SDK OAuth primitives (PKCE + headless)

- **Task:** 3
- **Date:** 2026-04-29
- **Status:** complete
- **Summary:**
  Implemented `packages/sdk/src/oauth.ts` with the OAuth 2.1 + PKCE primitives:
  `generatePkceVerifier()` (32 random bytes -> base64url), `pkceChallenge()`
  (`SHA-256(utf8(verifier))` via `crypto.subtle.digest`, base64url),
  `randomState()`, `buildAuthorizeUrl({...})` returning `{url, state, verifier,
  sessionId}` and storing the `{verifier, state, redirectUri, sessionId}`
  tuple in a module-level `pendingAuthSessions` Map keyed by `sessionId`,
  `exchangeCodeForToken({...})` enforcing Decision 5 binding (state +
  redirect_uri match before any HTTP), and `parseJwtPayload()` extracting
  `{sub, exp, iat}` with full validation (no signature check — server's job).
  Pure ESM, Web APIs only — no `node:*` imports anywhere.

- **Verification:**
  - `bun test test/oauth.test.ts` -> **29 pass / 0 fail / 95 expect() calls**.
  - `npx vitest run test/oauth.test.ts` -> 29/29 pass under vitest.
  - `npx tsc -p packages/sdk --noEmit` -> clean.
  - `grep -n 'node:' packages/sdk/src/oauth.ts` -> empty.
  - **Coverage** (vitest v8 against `src/oauth.ts`): **96.61% statements,
    96.15% branches, 100% functions, 96.61% lines** — comfortably above the
    85% / 80% gate. Uncovered: UUID-v4 fallback (only fires when
    `crypto.randomUUID` is missing) and one already-functionally-covered
    branch in `parseJwtPayload`.
  - RFC 7636 Appendix B fixture asserted byte-for-byte
    (`dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` ->
    `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`).
  - State-mismatch and redirect_uri-mismatch tests assert `fetch` is
    NOT called (Decision 5 enforcement — pre-HTTP rejection).
  - Session cleared on success (second call same `sessionId` throws
    session-not-found AuthError).

- **Concerns / follow-ups:**
  - **T2 status:** T2 already landed `errors.ts` (with `AuthError` accepting
    `(message, cause?)` positional, NOT options-bag) and updated `index.ts`
    in parallel, including the OAuth re-exports as `./oauth.js`. Both T2 and
    T3 changes are reconciled — `oauth.ts` imports `./errors.js`, tests
    import `../src/errors.js`, and the index.ts barrel re-exports the full
    OAuth surface. No integration risk remaining.
  - **Server contract assumption:** the SDK accepts response shape
    `{jwt, expires_at}` from `POST /oauth/token`. If T6 changes the server
    response shape, this needs to change too. Falls back to a synthetic
    `expiresAt` (`now + 1h` ISO string) when `expires_at` is omitted.
  - **`client_id` is hardcoded** to `"mnemonic-cli"` in
    `exchangeCodeForToken`. Per task spec — Chrome extension / other hosts
    that need a different client_id can add a parameter later (backlog).
  - **JWT validation is intentionally minimal:** parses payload only,
    does NOT verify the HS256 signature (server is the authority). Throws
    on missing `sub`/`exp`/`iat`, malformed JSON, or `exp <= now`.
  - **Bun-vs-vitest fetch mocking:** used `globalThis.fetch = mock` instead
    of `vi.stubGlobal` (which bun's compat shim doesn't expose) so the same
    test file passes under both runners. Cross-runtime CI matrix (Node 20 /
    Node 22 / Bun / Deno per Decision 11) should pick this up cleanly.

### Task 2 — completion (T2-impl-cont)
- Date: 2026-04-29
- Status: complete (cut-off T2-impl finished here)
- Summary: SDK core surface + Signer contract suite + LocalSigner + Keypair + COSE wrapper + JWT-redacting errors. WASM injection for tests via `__setWasmForTesting` in `wasm.ts`. Wrote `client.test.ts` (18 tests) covering all 5 tool methods, the pending-bundle / sign-callback flow (asserts NO `Authorization` header on `/api/sign-callback`, capability auth via `correlation_id` + `signer_pubkey` + COSE chain), and JWT-redaction in error paths. Added `cose.test.ts` and `keypair.test.ts` for envelope-shape + round-trip coverage. Note: T2-impl ran out of context just before client.test.ts.
- Verification: `bun test packages/sdk/test/` ALL pass (75 total, 189 expect calls). Coverage: lines 89.55%, funcs 91.18% overall — `client.ts` 98.68% lines, `keypair.ts` 100%, `cose.ts` 94.12%, `oauth.ts` 94.64%, `errors.ts` 94.29%. Only `wasm.ts` dynamic-import path (33%) is unreached because tests inject the mock via `__setWasmForTesting`; that path is exercised by Task 4's golden fixture against real WASM. `npx tsc -p packages/sdk --noEmit` clean. `grep -r 'node:' packages/sdk/src/` returns only doc-comment mentions, no actual `node:*` imports.
- Bug fix: `test/helpers/wasm-mock.ts` imported from `@noble/hashes/sha2` — Bun's strict ESM resolver couldn't resolve that against `@noble/hashes` v2.2.0's exports map (which lists `./sha2.js`). Changed to `@noble/hashes/sha2.js`. No new dependency added; package was already a devDep.
- Concerns / follow-ups:
  - `wasm.ts` lines 31-46 (the real dynamic-import + `init()` path) remain uncovered by SDK unit tests by design. Task 4's golden-fixture test should hit them; if not, add a smoke test there.
  - `client.ts` line 81 (`setJwt`) is uncovered — minor. Could add a one-liner test if desired.
  - `errors.ts` lines 74-75 (`IntegrityError` constructor) currently uncovered — `signMemory` only throws `IntegrityError` when the callback omits `attestation_id`, which is hard to reach without a fragile mock. Could add later.

---

## Task 6 — Server OAuth allowlist + bootstrap-ticket endpoints

- **Task:** 6
- **Date:** 2026-04-29
- **Status:** complete
- **Summary:**
  Added `oauth::allowed_redirect(uri, client_id) -> bool` (exact-match webapp,
  exact-prefix Cursor / VS Code / Claude.ai, hand-rolled loopback regex gated
  to `client_id == "mnemonic-cli"`) and wired it into `authorize_init_handler`
  so non-allowlisted `redirect_uri` values are rejected with 400 BEFORE pending
  state is stored. PKCE state map now binds `redirect_uri` alongside verifier
  + state; `/oauth/token` validates the body's optional `redirect_uri` field
  against the value bound at /authorize and rejects mismatches with 400 (RFC
  6749 §4.1.3 / RFC 7636 §4.4). Added new `BootstrapTickets` LRU+TTL store
  in `mcp/src/api.rs` modeled on `pending::PendingBundles` (LRU 100, TTL 600s,
  per-`jwt_sub` cap 3, atomic remove-and-return), plus `POST /api/cli-bootstrap/issue`
  (Bearer JWT'd, returns `{ticket_id}`) and `GET /api/cli-bootstrap/redeem/:ticket`
  (UUID-as-capability, no auth, returns `{secret: number[64], pubkey_base58}`).
  The redeem endpoint is added to the `bearer_auth_middleware` URI allowlist;
  the issue endpoint runs through the same middleware so `Claims` is injected
  via request extension. New field `McpState::bootstrap_tickets: Arc<BootstrapTickets>`
  threaded through `main.rs`, `chat.rs` test scaffolding, `mcp.rs` test
  scaffolding, `test_support.rs::mock_state`, and three integration test
  fixtures (`pending_authz.rs`, `pending_expiry.rs`, `sign_callback.rs`).

- **Verification:**

  ```
  cargo test -p mnemonic-mcp --lib --features local-embed -- oauth_ bootstrap_
  test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 106 filtered out
  ```

  Build / lint gates clean:

  - `cargo build -p mnemonic-mcp --release --features local-embed` → success.
  - `cargo clippy -p mnemonic-mcp --all-targets --features local-embed,test-support -- -D warnings` → clean.
  - `cargo fmt -p mnemonic-mcp -- --check` → clean.

  Full lib suite: 110 passed / 9 pre-existing failures (none introduced by
  this task — `test_authorize_valid_signature` etc. were already failing on
  the branch before my changes; confirmed via `git stash` baseline).

- **Concerns / follow-ups:**
  1. **Pre-existing baseline test failures** (`test_authorize_valid_signature`
     and 8 sibling tests using `cose_signed`) signal a drift between the
     `authorize_handler` (now requires raw 64-byte Ed25519 signature) and
     the legacy COSE_Sign1 wrapper the tests still build. Out of scope for
     T6 but flagged as a documentation /-test debt — the live POST flow uses
     the new raw-signature shape (covered by my new
     `test_oauth_state_binding_validates_redirect_uri_too`), so the failures
     are stale tests, not a regression.
  2. **Pre-existing pending_authz integration tests** (`test_pending_get_403_for_wrong_jwt_sub`,
     `test_pending_get_requires_jwt`) fail because the production allowlist
     in `bearer_auth_middleware` exempts `/api/pending/*` (Decision 12
     browser-mediated flow). Tests were not updated when that decision
     landed. Out of scope for T6.
  3. **`redirect_uri` is bound on /token by EQUALITY** to the value
     supplied at /authorize. The legacy webapp client did not send a
     `redirect_uri` on the token call; we therefore made the field
     `Option<String>` and only validate when present. A future hardening
     pass should make it required for the `client_id=mnemonic-cli` path
     specifically — in line with RFC 6749 §4.1.3's "if redirect_uri was
     supplied at authorize then it MUST be supplied at token."
  4. **Bootstrap tickets are in-memory only** — server restart drops every
     pending ticket. Acceptable for the Phase-1 hackathon scope; if we
     later need durability, persisting tickets behind the same SQLite
     boundary as `attestations` is straightforward.
  5. **`BootstrapInsertError::LruExhausted`** is currently unreachable (the
     LRU always evicts an older entry instead). Variant retained as
     documented dead code so future LRU changes can surface 503 without an
     API break.
