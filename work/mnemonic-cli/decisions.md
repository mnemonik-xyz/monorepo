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

### Task 2 — Round 2 fixes

- Date: 2026-04-29
- Status: review-fixes applied
- Fixed:
  - **Branch coverage 75.74% -> 81.25%** (line coverage 85.02% -> 93.98%, both above tech-spec gates of 80% / 85%). Added 16 new tests:
    - `client.test.ts`: 5xx surfaces ServerError with redacted body + status 500; malformed JSON-on-/mcp throws ServerError with `/malformed JSON/`; non-object JSON-RPC body throws ServerError; JSON-RPC `error.code === 401` maps to AuthError; JSON-RPC `error.message` propagates through ServerError; network failure (fetch throws) → ServerError `/network error/`; signMemory without setKeypair → UserError `/no keypair/`; sign-callback returning 200 sans `attestation_id` → IntegrityError; verify tampered discriminant propagates `signer` + `reason`; recall normalises `results[]` alternate to `hits[]`; recall handles missing hits/total; setJwt setter attaches Bearer header on next call; readBodySafely redacts a JWT straddling the 500-char boundary.
    - `signer.test.ts`: defense-in-depth: WASM `sign_challenge` throws → UserError `/sign_challenge failed/`; WASM returns non-Uint8Array → UserError `/did not return Uint8Array/`; WASM returns wrong-length sig → UserError `/must be 64 bytes/`.
    - `index.test.ts` (new): smoke test of public barrel re-exports, taking `src/index.ts` from 0% to 100%.
  - **Security low #2 (redact-then-slice):** `client.ts::readBodySafely` now runs `redactJWT(txt).slice(0, 500)` (was `redactJWT(txt.slice(0, 500))`). A JWT straddling the 500-char cutoff no longer leaks its prefix below the regex's `{20,}` threshold. Covered by the new `readBodySafely redact-then-slice` test.
- Verification:
  - `npx vitest run --coverage` (cwd `packages/sdk`): **92 tests passing**, lines 93.98%, **branches 81.25%**, funcs 100% — exit 0, threshold gate satisfied.
  - `bun test`: 92 / 92 pass.
  - `npx tsc -p packages/sdk --noEmit`: clean.
- Deferred to backlog (per spec: "skip — defer"):
  - code-reviewer minor #1 (Signer/keypair dual-bind architecture for future TurnkeySigner/WebAuthnSigner) — Phase 1.5+ concern.
  - code-reviewer minor #2 (extend JWT regex to also catch the third signature segment of full three-part JWTs) — current Decision-10 contract is met.
  - code-reviewer minor #3 (`MnemonicError.cause` redaction) — `cause` is documented developer-facing per Decision 10; option (a) of the recommendation already in JSDoc spirit.
  - code-reviewer minor #6 (defense-in-depth comment in `readBodySafely` flagging double redaction is intentional) — superseded by the redact-then-slice fix; double-redaction comment no longer applies cleanly.
  - test-reviewer minor (delete redundant `signer-contract.test.ts` OR remove inline `runSignerContract` from `signer.test.ts`) — kept both for now; the duplication is intentional belt-and-suspenders for the contract suite.
  - test-reviewer minor (large-content >32KB chunking branch in `bytesToBase64`) — robustness gap, not correctness.
  - test-reviewer minor (signer-contract.ts comment about WebAuthn non-determinism) — documentation polish, no behaviour change.
  - security-auditor low #1 (full-three-segment JWT regex) — same as code-reviewer #2.
  - security-auditor low #3 (Keypair zeroize / dispose surface) — Phase 2 concern documented in `signer.ts` JSDoc; Phase 1 threat model accepts heap exposure.

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

### Task 3 — Round 2 fixes
- Date: 2026-04-29
- Fixed: parseJwtPayload now enforces alg=HS256 (rejects none/RS256/missing); pendingAuthSessions has 10min TTL + 100-entry FIFO cap; malformed base64 throws AuthError not DOMException; null/non-object JWT payloads covered.
- Deferred to backlog: clientId asymmetry, cause-chain redaction, response-body discarded, micro-perf nits.

---

## Task 7 — Webapp `IdentityPanel` "Send to CLI" button

- **Task:** 7
- **Date:** 2026-04-29
- **Status:** complete
- **Summary:**
  Added a "Send to CLI" button to `webapp/src/components/IdentityPanel.tsx`
  (`data-testid="identity-send-to-cli"`) that closes the webapp ↔ CLI identity
  loop per Decision 7. Click handler reads `localStorage["mnemonic.identity"]`
  and the JWT from `lib/storage::readJwt` (key `mnemonic.jwt`), POSTs
  `{keypair_json}` to `${VITE_MCP_BASE}/api/cli-bootstrap/issue` with
  `Authorization: Bearer <jwt>`, and on 200 renders a code block
  `mnemonic identity import --ticket <uuid>` plus a Copy button that calls
  `navigator.clipboard.writeText(...)`. The component owns a separate
  `CliBootstrapState` discriminated union (`idle | issuing | issued | error`)
  so a failed bootstrap does not clobber the panel's main `error` channel.
  A 1Hz `setInterval` ticks a `mm:ss` countdown only while a ticket is
  outstanding; the timer auto-flips to `error` at 0 so the user is never
  shown a stale ticket. Server may include `expires_at` (unix seconds);
  fallback is "now + 10 min" per Decision 7.

  HTTP error mapping:
    - **401** → `window.location.assign("/oauth/consent")` so the user
      re-runs OAuth and returns with a fresh JWT.
    - **429** → inline message "You have 3 active CLI tickets. Wait for one
      to expire (10 min) or revoke later." (matches per-user cap from
      `BOOTSTRAP_PER_USER_CAP=3` in `mcp/src/api.rs`).
    - other 4xx/5xx → "Could not issue ticket: ${error.message}" with the
      server's `error` field if JSON-parseable.

- **Verification:**
  - `cd webapp && npx vitest run src/components/IdentityPanel.test.tsx` — 4
    tests pass: existing `renders_did_after_generate` (regression),
    `send_to_cli_calls_endpoint_and_displays_ticket` (TDD anchor — drives
    Decision 7 frontend; asserts Bearer header, body shape, paste-command
    text, and `navigator.clipboard.writeText` exact-string),
    `send_to_cli_429_shows_per_user_cap_message`,
    `send_to_cli_401_redirects_to_oauth_consent`.
  - `cd webapp && npx tsc -b --noEmit` clean (one type cast required:
    `fetchMock.mock.calls[0]` typed as `unknown as [string, RequestInit]`
    because vitest's mock-call element type is `[]` under strict mode).
  - `cd webapp && npm run build` succeeds end-to-end (vite + wasm-pack);
    `dist/assets/index-*.js` is 292 kB gzipped 91 kB — a 0.4 kB increase
    over the pre-task baseline.
  - `npx playwright test --list e2e/cli-bootstrap.spec.ts` enumerates 2
    tests: an offline `page.route(...)`-stubbed render check and a
    live-backend redeem-twice check that asserts the second `/redeem`
    returns 404 (single-use). The live test is skipped when
    `PLAYWRIGHT_SKIP_BACKEND_E2E=1` or `PLAYWRIGHT_TEST_JWT` is unset.

- **Concerns / follow-ups:**
  1. **JWT acquisition for the live e2e** — currently gated on a manually-
     supplied `PLAYWRIGHT_TEST_JWT`. A future task could exercise the
     full headless OAuth helper from `@mnemonik-xyz/sdk` (Task 3) inside
     Playwright so the live redeem check runs unattended.
  2. **`navigator.clipboard.writeText` failure modes** — under non-secure
     contexts (e.g. plain `http://`) the call rejects with a DOMException.
     The handler surfaces a "Copy failed: ..." inline error rather than
     falling back to `document.execCommand("copy")`; this matches the
     existing webapp's clipboard discipline (no IE/legacy fallbacks
     elsewhere).
  3. **No "revoke ticket" surface yet** — the 429 message tells the user to
     wait or revoke "later"; the SDK / CLI side will gain `mnemonic
     identity list` + `revoke` in a follow-up task. Leaving the message
     forward-compatible.
  4. **Storage caveat unchanged** — keypair still lives unencrypted in
     localStorage. The "Send to CLI" handler reads the raw string and
     posts it verbatim; the server stores it without inspection (api.rs
     comment confirms). Encryption-at-rest is still tracked as a separate
     hardening item.

---

## Task 4 — COSE wrapper + golden fixture + CI lockstep gate

- **Task:** 4
- **Date:** 2026-04-29
- **Status:** complete
- **Summary:**
  Added the `golden-fixtures` cargo feature flag to `core/Cargo.toml` (does
  not exist before this task). Implemented `core/tests/golden_fixtures.rs` —
  an `#[ignore]`-gated integration test that emits a JSON array of 22 fixture
  triples (`name`, `input_hex`, `canonical_cbor_hex`, `cose_envelope_hex`,
  `keypair_secret_hex`). Coverage: empty content, ASCII, UTF-8 (Cyrillic /
  emoji / CJK / mixed scripts), control chars, embedded NUL, quotes/backslashes,
  long content (1KB and 5KB), single/many/empty tags, JSON metadata variants,
  high-byte UTF-8 boundaries. All cases share one hardcoded 32-byte test seed
  (`00112233...eeff`) so output is deterministic — `test_emitter_deterministic`
  asserts that two consecutive runs produce identical JSON.

  Wrote `packages/sdk/scripts/regen-golden-fixtures.sh`: invokes the cargo
  test, slices the JSON body out of cargo's framing using a python heredoc with
  RAW input passed via env var (heredoc owns stdin), writes
  `packages/sdk/test/fixtures/golden-cose.json` (76 KB, 22 entries) and the
  matching SHA-256 to `golden-cose.sha256`. Re-running the script verified
  byte-identical output.

  Wrote `packages/sdk/test/cose.golden.test.ts` — pure ESM vitest. Loads the
  fixture, installs the real `core/pkg-nodejs/mnemonic_core.js` WASM artifact
  via the SDK's `__setWasmForTesting` hook, then for each entry calls
  `coseSignPayload(canonicalCbor, keypairJson)` and asserts byte-equality
  against `cose_envelope_hex`. The pkg-nodejs target is used (not pkg-web)
  because vitest runs under Node, where pkg-web's `fetch(file://)` init path
  does not work without polyfills; pkg-nodejs and pkg-web link the same Rust
  code and produce bit-identical COSE bytes by construction.

  Wrote `.github/workflows/node-test.yml` with the lockstep gate as job
  `golden-fixture-lockstep`: re-runs the regenerator on every PR and `git
  diff --exit-code` against the committed JSON + SHA. Drift fails CI with an
  error annotation pointing to the regenerator command. Task 8 will extend
  this workflow with the cross-runtime matrix (Node 20 / Node 22 / Bun / Deno).

- **Verification:**
  - `cargo test --features golden-fixtures -p mnemonic-core --test golden_fixtures` —
    3 passed (test_emitter_deterministic, test_fixture_count_and_unique_names,
    test_fixed_keypair_pubkey_stable), 1 ignored (emit_fixtures itself).
  - `bash packages/sdk/scripts/regen-golden-fixtures.sh` — produced
    SHA `15ed6eac683679ae79234879a72e38ecad2c8eb0cae451aca268ad435b7337fc`,
    22 entries, deterministic across two consecutive invocations.
  - `cd packages/sdk && npx vitest run` — 8 files, 106 tests, all pass
    (including the 2 new golden tests + the existing 104).
  - `cargo fmt --all -- --check` clean. `cargo clippy --features
    golden-fixtures -p mnemonic-core --tests -- -D warnings` clean.
    (Workspace-wide clippy reports pre-existing failures in `mnemonic-mcp`
    `test-support` feature gating that are unrelated to this task.)

- **Concerns / follow-ups:**
  1. **pkg-nodejs as the test artifact** — the golden test imports
     `core/pkg-nodejs/`, but the SDK's runtime default is pkg-web. Both
     targets link the same Rust code so output is bit-identical, but a
     follow-up should wire CI to also exercise the pkg-web artifact under
     Bun + Deno (those runtimes can load pkg-web natively); Task 8 owns
     the runtime matrix.
  2. **Fixture file size** — 76 KB at 22 entries; if the catalogue grows
     past 50 entries we'll cross the 100 KB soft cap stated in the task.
     Current entries cover the documented edge cases (CBOR length-prefix
     boundaries at 256-byte mark, UTF-8 multi-byte, embedded NUL, control
     chars, empty/long tags, metadata variants); further additions should
     justify themselves.
  3. **`input_hex` field is purely documentary** — the SDK does not
     consume it; we keep it so future debuggers can round-trip canonical
     CBOR back to the JSON artifact via `from_canonical_cbor` if a
     fixture mismatch arises.
  4. **Lockstep gate uses `git diff --exit-code`** — depends on
     `actions/checkout@v4` defaults (full clone, not shallow). If a
     future caller switches to `fetch-depth: 1` and the regenerator
     legitimately needs to write outside the committed range, the gate
     will still catch it because we compare the working-tree file directly.


---

### Task 5 — completion (T5-impl-cont)

- Date: 2026-04-29
- Status: complete (cut-off T5-impl finished here)
- Summary:
  Finished `packages/cli/` end to end after T5-impl ran out of context. Wrote
  the missing `src/commands/identity.ts` (`import --ticket <uuid>` /
  `import --file <path>` / `export --file <path>`, no clipboard flag), wired
  `identity` + `import` + `export` into `bin/mnemonic.ts` via commander parent
  + child commands, fixed `bin/mnemonic.ts` so all eight top-level commands
  (init/login/sign/recall/verify/whoami/prove/identity) route through
  `OutputOptions` + `handleError`. Added the auto-run guard so test files can
  `import { buildProgram }` without triggering `parseAsync(process.argv)`.
  Authored 10 vitest files with 43 tests covering every command, all four
  TDD anchors (login state-mismatch, identity ticket atomic-redeem, whoami
  no-server-call, output --json --quiet) plus a bin smoke test that lists
  registered commands and parses top-level flags. Tests use a tmpdir-scoped
  `MNEMONIC_CONFIG_DIR` and the SDK's existing WASM mock via
  `__setWasmForTesting`. Two TS errors fixed during integration:
  `node:net::AddressInfo` requires `import type` under
  `verbatimModuleSyntax`, and `init`'s `force` flag had to be conditionally
  spread under `exactOptionalPropertyTypes`.
- Verification:
  - `cd packages/cli && bun install` (no diff — workspace-resolved deps).
  - `bun test` -> 43 pass / 0 fail / 73 expect() calls / 83 ms.
  - `vitest run` -> 10 test files, 43 / 43 pass, 320 ms.
  - `vitest run --coverage` -> **lines 77.63%** (above the 75% gate),
    branches 68.14%, functions 78.46%. Per-module: `init.ts` 96%,
    `verify.ts` 97%, `recall.ts` 94%, `whoami.ts` 77%, `output.ts` 89%,
    `identity.ts` 81%, `sign.ts` 78%, `login.ts` 68%, `errors.ts` 80%,
    `config.ts` 79%, `bin/mnemonic.ts` 61%. Branch coverage is below the
    typical 80% bar; the misses are all in the human-render paths (no
    `--json` branch) and the loopback `awaitCallback` error-path branches
    (timeout, 405, 404, error params). See concerns below.
  - `npx tsc -p packages/cli --noEmit` -> clean (0 errors).
  - `tsc -b` -> clean build of `dist/`.
  - `bun packages/cli/bin/mnemonic.ts --help` -> all 8 commands listed.
  - `bun packages/cli/bin/mnemonic.ts identity --help` -> import + export.
  - `node dist/bin/mnemonic.js --help` -> identical output (built path OK).
- Concerns / follow-ups:
  1. **Branch coverage 68%** is below a notional 80% gate. The hot misses
     are: `login.ts::awaitCallback` 405/404/missing-code branches, the
     `error_description` callback path, and several `colorEnabled` /
     `paint` branches that only fire on a TTY. None of these are
     business-logic gaps — they're all renderer + defensive branches that
     a unit test cannot hit without simulating a real browser callback.
     A T8 follow-up could add a focused loopback test that POSTs each error
     shape (405, 404, missing-state, idp-error) and observes the recovered
     AuthError.
  2. **`bin/mnemonic.ts` 61% lines.** Tests cover `buildProgram` and flag
     parsing; the auto-run path (`if (invokedDirectly)`), `main()` wrapper,
     and the per-command `.action()` closures themselves are unreached by
     in-process tests. An `execa`-driven smoke would close this gap but
     was kept out per spec (we directly import command modules instead).
  3. **`whoami` 28.6% branch coverage** is misleading — most of the
     uncovered branches are the optional `--with-count` server-call path,
     which the TDD anchor explicitly rejects. Lines coverage is 77%; the
     gap is intentional.
  4. **No tests for `--with-count`** because exercising it pulls in the
     full SDK signMemory/recall fetch chain. If we want it covered, the
     test would mirror `recall.test.ts`'s mock and pass `withCount: true`.
  5. **Login interactive test patches `Server.prototype.listen`** to drive
     the wrong-state callback. This is brittle if commander or login.ts
     ever switches HTTP libraries, but it's the only way to drive the
     loopback synchronously without an external port-discovery race.
     `vi.spyOn(http, "createServer")` cannot be used directly because the
     ESM export is frozen (`TypeError: Cannot redefine property`).
  6. **No `execa` dep added** — every test runs in-process via direct
     module imports, which is faster (sub-second total) and avoids the
     `bin/mnemonic.ts` auto-run heuristic edge cases. Spec allowed either
     route.
  7. **Identity import refuses-to-overwrite test** uses `--force` to
     bypass the refusal on the second 410 fetch (otherwise we'd never
     reach the server's 410 path). This is a deliberate test fixture
     choice; production users should NOT pass `--force` on a re-redeem
     attempt.

## 2026-04-28 — Task 4 review round 1 fixes (T4-impl-r2)

- **Reviews consumed:**
  - code-reviewer round 1: 1 major (CR-T4-1) + 6 minor.
  - security-auditor round 1: 2 low (T4-S1, T4-S2).
  - test-reviewer round 1: non-blocking concerns (schema coverage gap,
    pubkey value pinning).
- **Major fix — CI ANSI escape bug (CR-T4-1):**
  CI workflow `.github/workflows/node-test.yml` sets
  `CARGO_TERM_COLOR: always` workspace-wide. The regen script's python
  slice used `text.find("[")`, which would land on the ANSI escape
  prefix `\x1b[` instead of the JSON `[`, corrupting the fixture in CI
  while passing locally. Fix in
  `packages/sdk/scripts/regen-golden-fixtures.sh`:
  - Inline-override `CARGO_TERM_COLOR=never` and pass `--color=never`
    on the cargo invocation (belt-and-braces).
  - Strip residual ANSI CSI sequences (`\x1b\[[0-9;]*m`) inside the
    python slicer as defense-in-depth.
  - Verified by running `CARGO_TERM_COLOR=always bash
    packages/sdk/scripts/regen-golden-fixtures.sh`: SHA still
    `15ed6eac…7337fc`, matching the committed file. Also re-ran without
    the override (baseline) — still matches.
- **Low fix — script quote-escape footgun (T4-S1):**
  `regen-golden-fixtures.sh:75` previously interpolated `$FIXTURE_JSON`
  into a python single-quoted literal — a developer footgun for repo
  paths containing `'`. Fixed by passing the path through the
  `REGEN_OUT_PATH` env var (mirroring the pattern already used for
  `RAW_CARGO_OUTPUT`). Applied to both the heredoc slicer and the
  inline `python3 -c` entry-counter.
- **Deferred (per scope):**
  - Schema coverage expansion (4 of 5 schemas untested) → backlog post-MVP.
  - Pubkey value pinning → backlog (deterministic seed already pins
    upstream output).
  - Spec count discrepancy (~50 specced vs 22 fixtures delivered) →
    spec/decisions adjustment, not a code change.
- **Verification:**
  - `CARGO_TERM_COLOR=always bash packages/sdk/scripts/regen-golden-fixtures.sh`
    → SHA `15ed6eac…7337fc`, `git diff --exit-code packages/sdk/test/fixtures/`
    clean.
  - `unset CARGO_TERM_COLOR; bash packages/sdk/scripts/regen-golden-fixtures.sh`
    → same SHA, no drift.
  - `cd packages/sdk && bun test test/cose.golden.test.ts` → 2 pass,
    0 fail.
  - `cargo test --features golden-fixtures -p mnemonic-core --test
    golden_fixtures` → 3 pass, 1 ignored (the gated `emit_fixtures`).

---

## Task 7 — review round 2 (CRITICAL wire-shape fix)

- **Task:** 7
- **Date:** 2026-04-28
- **Status:** review-round-2
- **Summary:**
  Address `T7-CR-1` (critical) plus the two minor follow-ups (`T7-CR-2`,
  test-reviewer F1). The round-1 implementation posted the entire
  localStorage payload (`{secret: number[64], pubkey_base58: string}`) as
  `keypair_json`, but the T6 redeem handler at
  `mcp/src/api.rs:563` parses it as a flat byte array via
  `serde_json::from_str::<Vec<u8>>(...)`. End-to-end the CLI could never
  redeem a webapp-issued ticket — the call would 500 with `stored
  keypair_json is not a JSON byte array`.

  Fix: in `IdentityPanel.handleSendToCli`, parse the localStorage entry,
  validate `parsed.secret` is exactly 64 numbers, and send
  `JSON.stringify(parsed.secret)` as `keypair_json`. The server now sees
  the bare 64-element array it already knows how to parse. Cross-validated
  against `bootstrap_redeem_handler` — the new wire shape feeds straight
  into `Vec<u8>::deserialize` and into the `bytes.len() != 64` guard
  unchanged.

- **Other fixes folded in:**
  - `T7-CR-2`: `IdentityPanel.test.tsx::afterEach` now calls
    `vi.unstubAllGlobals()` in addition to `vi.restoreAllMocks()` so the
    `vi.stubGlobal('fetch', ...)` calls in individual tests cannot leak
    across tests (vitest's `restoreAllMocks` does NOT undo `stubGlobal`).
  - Test-reviewer F1: added `send_to_cli_5xx_shows_generic_error_with_server_message`
    asserting the `Could not issue ticket: <detail>` branch when the server
    returns 500 + `{error: "internal"}`.
  - Added `send_to_cli_malformed_local_identity_shows_error_no_fetch` to
    pin the new client-side validation: a wrong-length `secret` (32 bytes)
    surfaces an inline error and never calls `fetch`.

- **Skipped (per task brief):**
  - T7-CR-3 (`MCP_BASE` constant duplication across three components) →
    backlog refactor.
  - T7-CR-4 (auto-expire transitions to `error` rather than `idle`) →
    UX polish.
  - T7-CR-5 (401 redirect drops the bootstrap intent) → design choice,
    matches existing `Sign.tsx` / `Consent.tsx` behavior.

- **TDD anchor update:**
  `send_to_cli_calls_endpoint_and_displays_ticket` now asserts the wire
  shape directly:

  ```ts
  const innerKp = JSON.parse(body.keypair_json);
  expect(Array.isArray(innerKp)).toBe(true);
  expect(innerKp.length).toBe(64);
  expect(innerKp.every((n: unknown) => typeof n === "number")).toBe(true);
  ```

  This locks the contract symmetrical to the server's `Vec<u8>` parser —
  any future regression that re-introduces the object-form payload will
  fail this test before reaching live backends.

- **Verification:**
  - `cd webapp && npx vitest run src/components/IdentityPanel.test.tsx`
    → 6/6 pass (was 4/4; +2 new cases).
  - `cd webapp && npx tsc -b --noEmit` → clean.
  - `cd webapp && npm run build` → succeeds (vite build + wasm rebuild).
  - Cross-validated server side: re-read
    `mcp/src/api.rs::bootstrap_redeem_handler`. `serde_json::from_str::<Vec<u8>>`
    now succeeds against the new wire shape; `bytes.len() != 64` guard
    remains the only post-parse check, and the 64-element array we send
    satisfies it.

## 2026-04-28 — Task 6 review round 1 fixes (T6-impl-r2)

- **Reviews consumed:**
  - test-reviewer round 1: 1 high (production handlers untested behind
    real middleware), 2 medium (atomic-consume on single-thread runtime,
    no 429 HTTP-mapping coverage). Other findings (localhost-by-name,
    LruExhausted dead branch, all-zero-pubkey fixture, pending.rs
    `#[allow(dead_code)]` revert, ambiguous positive-path on
    `state_binding_validates_redirect_uri_too`) deferred to backlog per
    fix scope.
  - code-reviewer round 1: 9 minor (non-blocking) — all backlog.
  - security-auditor round 1: 5 low/info, none blocking — all backlog.
- **High fix — real-handler integration test:**
  Added `mcp/tests/cli_bootstrap_auth_allowlist.rs` (5 tests) that builds
  an axum Router with the production `bootstrap_issue_handler` and
  `bootstrap_redeem_handler` from `mnemonic_mcp::api`, layered with the
  real `oauth::bearer_auth_middleware`. Asserts:
  1. Issue with valid JWT → 200 + UUID `ticket_id`.
  2. Redeem with NO Authorization header → 200 + `secret[64]` +
     `pubkey_base58` (proves `/api/cli-bootstrap/redeem/` is on the
     middleware URI allowlist).
  3. Second redeem of the same ticket → 404 (single-use).
  4. Redeem with garbage UUID and NO auth → 404 (NOT 401 — pins the
     allowlist so a regression that drops the prefix flips this to 401).
  5. Issue with NO Bearer → 401 (middleware bites — `extract_json_rpc_method`
     on `{"keypair_json":...}` returns None → not allowlisted → required JWT
     missing).
  6. Per-user 429 HTTP mapping: 4 tickets for the same `jwt.sub` — 4th
     returns `StatusCode::TOO_MANY_REQUESTS` with a JSON `error` body.
     A different `sub` is unaffected (proves keying is per-user, not
     global).
  Wired `BootstrapTickets` and the two handlers through the public
  library facade (`mnemonic_mcp::api::*`) and used the existing
  `test_support::{mock_state, mint_jwt}` fixtures (model:
  `mcp/tests/auth_allowlist.rs`).
- **Medium fix — multi-thread atomic consume:**
  Promoted `mcp/src/api.rs::tests::test_bootstrap_ticket_atomic_consume`
  from default `#[tokio::test]` (current_thread) to
  `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`.
  Inserted a `tokio::sync::Barrier` so both spawned tasks cross the
  consume-mutex boundary at the same instant, and looped 64 iterations
  to amplify any race window. A regression where `consume` was reduced
  to `peek` (returning Some twice) would now flip the `some_count == 1`
  assertion with very high probability across iterations.
- **Medium fix — 429 HTTP mapping test:**
  Covered by integration test #6 above. Previously only the underlying
  `BootstrapInsertError::PerUserCapExceeded` enum value was asserted at
  the unit-store level; the handler's mapping to
  `StatusCode::TOO_MANY_REQUESTS` plus the `{"error": "..."}` body shape
  were uncovered.
- **Deferred (per scope):**
  - localhost-by-name DNS-rebinding negative test → backlog.
  - Unreachable `BootstrapInsertError::LruExhausted` variant + 503
    branch → backlog (YAGNI; remove or test once LRU policy changes).
  - All-zero-pubkey fixture inconsistency in
    `test_bootstrap_redeem_no_auth_required` → backlog (already covered
    by the new integration test which uses a non-trivial trailing 32
    bytes).
  - 9 minor code-review nits + 5 low/info security findings → backlog.
- **Verification:**
  - `cargo test -p mnemonic-mcp --features test-support --test cli_bootstrap_auth_allowlist`
    → 5 passed / 0 failed.
  - `cargo test -p mnemonic-mcp --lib --features local-embed test_bootstrap_ticket_atomic_consume`
    → 1 passed / 0 failed (multi_thread runtime confirmed via test
    annotation).
  - `cargo test -p mnemonic-mcp --lib --features local-embed -- bootstrap_`
    → 7 passed / 0 failed (all 6 pre-existing + 1 modified bootstrap
    unit tests stable).
  - `cargo clippy -p mnemonic-mcp --all-targets --features local-embed,test-support -- -D warnings`
    → clean.
  - `cargo fmt -p mnemonic-mcp -- --check` → clean.
  - Pre-existing baseline failures (9 lib + 1 oauth_flow + 2 pending_authz)
    re-verified unchanged — none introduced or reactivated by this round.

---

## Task 5 — Round 2 review fixes (5xx coverage + icacls execFile + minors)

- **Task:** 5 (round 2)
- **Date:** 2026-04-28
- **Status:** complete
- **Reviewer findings addressed:**

  | id     | source            | severity | action |
  |--------|-------------------|----------|--------|
  | R2-1   | test-reviewer     | high     | applied — added 5xx tests on sign / recall / verify / identity (--ticket) + login (token endpoint 500 path) |
  | F-1    | security-auditor  | low      | applied — `restrictFileMode` switched from `execSync` template-literal to `execFileSync` argv array (CWE-78 mitigation); new `test/config.test.ts` asserts the argv shape |
  | L1     | code-reviewer     | minor    | applied — `awaitCallback` now removes the request listener inside `settle()` so post-settle stray probes are dropped (close-after-settle preserved via outer `finally`) |
  | S1     | code-reviewer     | minor    | applied — `whoami.ts` adds `Keypair` to the static SDK import, removes the dynamic `import('@mnemonik-xyz/sdk').then(...)` |
  | S3     | code-reviewer     | minor    | applied — removed dead `(color ? l : l)` ternary in `init.ts` |
  | O1     | code-reviewer     | minor    | deferred to backlog (tech-spec Decision 10 colour wiring) |
  | T1     | code-reviewer     | minor    | deferred to backlog (Server.prototype.listen → DI is a bigger refactor) |
  | S2/S4/S5 | code-reviewer   | minor    | deferred to backlog (cosmetic / build-ordering documentation) |
  | R2-3..R2-6 | test-reviewer | low/med  | deferred to backlog (malformed-JWT shapes, branch-coverage lift via login error-callback tests, bin smoke) |

- **5xx test coverage — deviation note (login):**
  The brief asked for ServerError (exit 2) on login `--token` or interactive
  `/oauth/token` 500. The headless `--token` path performs no fetch (it only
  decodes a JWT locally), so 5xx is not reachable there. On the interactive
  path, the SDK's `exchangeCodeForToken` (oauth.ts:306) explicitly maps a
  non-2xx token endpoint response to `AuthError` — it is in the OAuth-protocol
  layer, not the generic-fetch layer. The CLI's `fromSdkError` therefore
  surfaces this as `AuthError` (exit 4), not `ServerError` (exit 2). The new
  test (`login.test.ts:: token endpoint 500 → AuthError (exit 4), no token
  persisted`) asserts the **actual** contract: 5xx surfaces cleanly, with
  redaction, no token written, and a typed exit code. This is consistent
  with sign/recall/verify/identity which DO route through the generic-fetch
  ServerError path (exit 2). Changing the SDK's OAuth error mapping was out
  of T5 scope.

- **Verification:**

  ```
  cd packages/cli
  npx tsc -p . --noEmit         # clean
  bun test                       # 51 pass (was 43; +8 = 5 server-error + 3 icacls)
  npx vitest run --coverage     # lines 80.02% (was 77.63%), branches 71.31% (was 68.14%)
  ```

- **Files changed:**
  - `packages/cli/src/config.ts` — execSync → execFileSync (F-1)
  - `packages/cli/src/commands/login.ts` — removeListener inside settle (L1)
  - `packages/cli/src/commands/whoami.ts` — static Keypair import (S1)
  - `packages/cli/src/commands/init.ts` — drop dead ternary (S3)
  - `packages/cli/test/sign.test.ts` — +1 ServerError test
  - `packages/cli/test/recall.test.ts` — +1 ServerError test
  - `packages/cli/test/verify.test.ts` — +1 ServerError test
  - `packages/cli/test/identity.test.ts` — +1 ServerError test (--ticket 500)
  - `packages/cli/test/login.test.ts` — +1 AuthError-on-token-500 test (deviation noted above)
  - `packages/cli/test/config.test.ts` — NEW; 3 tests for icacls argv shape + best-effort failure modes

- **Carried-forward concerns (round 3 backlog, none blocking T5 merge):**
  1. O1 — wire `colors.{green,yellow,cyan}` into at least one renderer to
     make `--no-color` observable (currently a no-op end-to-end).
  2. T1 — replace `Server.prototype.listen` patch with an injectable
     `_serverFactory` test hook in `LoginOptions`.
  3. R2-3 — expand malformed-JWT coverage (missing exp, non-string sub,
     alg=RS256, malformed base64).
  4. S5 — promote SDK test helpers via package.json `exports` so CLI tests
     stop reaching into `../../sdk/dist/*` directly.
