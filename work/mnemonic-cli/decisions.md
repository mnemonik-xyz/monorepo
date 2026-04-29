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
