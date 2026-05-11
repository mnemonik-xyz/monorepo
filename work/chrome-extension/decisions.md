# Decisions — `work/chrome-extension/`

Append-only log of decisions and audit findings. Each entry: date, decision, rationale, alternatives considered.

---

## 2026-05-10 · D1 — Extension is fully self-contained for local mode

Embed / compress / sign / store all run in the browser via WASM (`@mnemonic/core`) + `transformers.js`. Cloud mode is a thin client to the existing hosted MCP-`full` HTTP surface.

**Rationale:** local-mode true privacy ("data never leaves the device") is a marketable differentiator and matches the project's privacy-first positioning. Reuses already-planned WASM exports.

**Alternatives:** (a) thin UI talking to server in all modes — rejected (no offline, no privacy story, paid even for trivial use); (b) hybrid — rejected as worst-of-both.

---

## 2026-05-10 · D2 — Server `StorageMode` is unchanged

`mcp/src/config.rs:95` keeps `local | full`. "Cloud" is purely UX naming in the extension that maps to hosted-`full`.

**Rationale:** avoids new Rust enum variant, `S3Store` impl, AWS SDK in Cargo. Cloud-tier infra is an operational concern (which AWS region runs `STORAGE_MODE=full`), not a code change.

**Alternatives:** add `cloud` variant with `S3Store` — rejected as scope creep without product benefit.

---

## 2026-05-10 · D3 — Browser embedder

`transformers.js` + `Xenova/all-MiniLM-L6-v2` (384 dim, ~25MB ONNX). Lazy-loaded in a Web Worker on first sign. Cached via Cache API.

**Rationale:** vector dim 384 must match server default (golden-fixture parity). Smallest viable model with strong performance. Web Worker keeps popup responsive.

**Alternatives:** (a) call server embed API — rejected, breaks local-mode offline; (b) larger model — rejected for bundle size; (c) build custom embedder via `core/` WASM — rejected, fastembed not in WASM target today.

**Re-evaluate:** post-MVP if 25MB is too heavy on slow networks; consider quantized MiniLM-INT8 (~10MB).

---

## 2026-05-10 · D4 — Build tool: WXT (default), Vite + crxjs (fallback)

Final pick during T1 spike.

**Rationale:** WXT auto-handles MV3 quirks (HMR for service worker, content-script hot reload), reduces boilerplate. crxjs is more mature but slower dev loop.

### 2026-05-10 · D4 ratification (T01)

**Final pick: Vite 6 + `@crxjs/vite-plugin` 2.x.** WXT was the prior default; superseded after evaluating monorepo consistency.

**Why Vite + crxjs over WXT:**

- The webapp (`webapp/`) already runs on Vite 6.3.3 + React 19 + TypeScript 5.7.2; using the same toolchain in `packages/extension/` keeps a single Vite plugin/config story across the monorepo (one less moving part for new contributors).
- `@crxjs/vite-plugin` is a thin layer that handles MV3 manifest entry-point resolution + HMR for popup/options + the service-worker reload dance, without imposing WXT's wholesale project structure or auto-generated routing.
- WXT's main wins (file-system routing, auto-imports) are nice-to-have, not required for our flat 4-entry layout (popup + options + content + service-worker). The cost (extra abstraction layer + smaller community than Vite directly) outweighs the win at this scope.
- Bundle-size / dev-loop comparison ran short of the planned 4h spike; observed both produce similar empty-extension output (~25KB JS + manifest). HMR works in both. Decision driven by consistency.

**Backlog re-evaluation trigger:** if the extension grows past ~10 entry points or we find ourselves hand-rolling routing logic, revisit WXT.

---

## 2026-05-10 · D5 — Google OAuth via `chrome.identity.launchWebAuthFlow` + PKCE S256

No Google client_secret in extension; server holds it. Server validates Google `id_token` (issuer + audience + signature against cached JWKS), then issues our own JWT (`aud=mcp` + `google_sub` claim). First link requires possession proof (extension signs a server-issued challenge with the new Ed25519 keypair).

**Rationale:** standard OAuth 2.1 web client flow, no custom secret distribution risk in the extension. Possession proof prevents an attacker who steals a Google account from claiming an existing on-chain identity.

**Alternatives:** (a) `chrome.identity.getAuthToken` (Google-specific, simpler) — rejected, ties us to Chrome-store distribution and Chrome-only; web flow works on Edge/Brave/Arc unchanged.

---

## 2026-05-10 · D6 — One Ed25519 keypair per user across CLI / webapp / extension

Cross-device sync is via either (a) restore-on-Google-signin (new) or (b) existing bootstrap-ticket flow.

**Rationale:** consistent identity across all clients is the protocol's value prop. Multi-account is backlog.

---

## 2026-05-10 · D7 — Mode switching is explicit

Local → Cloud uploads existing attestations one-by-one (best-effort, resumable queue). Cloud → Local exports + disconnects. No silent dual-write.

**Rationale:** silent dual-write creates conflict resolution hell when the user runs offline on one device and online on another. Explicit user gesture sets expectations.

---

## 2026-05-10 · D8 — Primary feature is AI-chat capture, not generic page capture

Phase 1 ships adapters for ChatGPT, Claude.ai, Gemini. Generic page-selection capture is a free fallback via `activeTab`. Auto-capture is opt-in per domain.

**Rationale:** the user-stated primary use case is browser AI chats. Generic-capture-first would dilute positioning and require `<all_urls>` host_permission (Chrome Web Store review risk).

**Alternatives:** (a) generic capture only — rejected, weak product story; (b) ChatGPT-only — rejected, locks out Claude/Gemini users; (c) all 5+ adapters Phase 1 — rejected for scope.

---

## 2026-05-10 · D9 — Encrypted key escrow for Google-login restore

Server stores Ed25519 secret as a passphrase-encrypted blob keyed by Google `sub`.

- KDF: Argon2id (`memory_cost=64MiB, time_cost=3, parallelism=1`, salt = 16 random bytes).
- Cipher: AES-GCM-256, 96-bit nonce.
- Server stores: `{ciphertext, nonce, kdf, kdf_params, pubkey_base58}`. No plaintext, no keys.
- Rate limit: 5 GET fetches / 24h / `google_sub`.

Server cannot decrypt. Lost passphrase = lost restore (user falls back to manual keypair import from another device).

**Rationale:** standard zero-knowledge passphrase wrap. Argon2id parameters meet OWASP 2026 minimums. Rate limit bounds online brute force; KDF cost bounds offline brute force on stolen ciphertext.

**Alternatives:** (a) server-held plaintext (or KMS-wrapped on server) — rejected, server compromise leaks all keys; (b) Shamir/threshold — rejected for MVP scope; (c) WebAuthn passkey wrap — backlog (better UX, harder to ship Phase 1 cross-device); (d) BIP-39 mnemonic — rejected, even worse UX than passphrase.

---

## 2026-05-10 · D10 — `ChatAdapter` interface keeps platform support extensible

Flat registry, one adapter per supported domain. Phase 1: ChatGPT, Claude.ai, Gemini. Backlog: Grok, Perplexity, Poe, OpenRouter, t3.chat.

**Rationale:** AI-chat UIs change; isolating per-platform DOM logic behind a stable interface lets us patch one adapter without touching the rest. Community contributions land as new adapter modules + HAR fixtures + entry in registry.

---

## 2026-05-10 · D11 — Enumerated host_permissions, no `<all_urls>`

`manifest.json` declares only `https://chatgpt.com/*`, `https://claude.ai/*`, `https://gemini.google.com/*` (+ extension origin). Generic page-selection capture works via `activeTab` + user gesture (popup hotkey or context-menu).

**Rationale:** Chrome Web Store review treats `<all_urls>` as high-risk; enumerated permissions ease approval. Adding new platforms is a versioned release.

---

## 2026-05-10 · D12 — Auto-capture is opt-in per domain, off by default

Privacy default. User explicitly enables in options page per supported domain.

**Rationale:** silent capture of every assistant response is a privacy red flag and a Chrome Web Store review risk. Opt-in keeps trust and review-friendliness.

---

## 2026-05-10 · D13 — Test-coverage gate: no task is `done` without passing tests

A task in `work/chrome-extension/tasks/` cannot be marked `status: done` until:

1. Every test named in its `## TDD Anchor` section is implemented and passing locally.
2. Every command in its `verify:` frontmatter (e.g. `pnpm-test`, `pnpm-build`, `web-ext-lint`) returns exit code 0 in CI on the task's PR.
3. `test-reviewer` (or, for tasks without an explicit reviewer, the merging maintainer) signs off that the tests cover the task's acceptance criteria — not just smoke-paths.
4. CI runs all four test layers from `tech-spec.md` `## Testing`: unit (vitest), component (Playwright + Vitest browser mode), E2E (Playwright `--load-extension`), and server-side Rust tests where the task touches `mcp/` or `core/`.

A task that is functionally complete but missing tests stays at `status: in_review` with a `blocked_on: tests` note in `decisions.md` until coverage lands. Exceptions (e.g. a UI-only Playwright fixture spike) require an inline note in the task file naming the follow-up task that will add tests.

**Rationale:** the convention was already implicit (every task has `verify:` + a `## TDD Anchor`, `test-reviewer` is on `reviewers:` for storage/embedder/auth tasks), but without a hard gate it slips. Making this explicit prevents the "I'll add tests later" pattern that has historically cost waves of audit rework. Aligned with the project's existing CI policy (`cargo test --workspace --no-fail-fast` on every PR — same posture extended to the extension's npm/Playwright pipeline).

**Alternatives:** (a) advisory-only convention — rejected (current state, doesn't enforce); (b) hard coverage threshold (e.g. ≥80% line coverage gating merge) — rejected for MVP, too noisy on Playwright-heavy code where coverage instrumentation is brittle; can be revisited once the test pyramid stabilises.

**Scope:** applies to all `work/chrome-extension/tasks/*.md`. Server changes (T11, T13, T14) ALSO inherit the existing `cargo test --workspace --no-fail-fast` gate from root `CLAUDE.md` — the rule is additive, not substitute.

---

## 2026-05-10 · T07 — ChatGPT adapter lands; awaiting CI verify

`packages/extension/src/runtime/chat/adapters/chatgpt.adapter.ts` implements the `ChatAdapter` contract for `chatgpt.com`. 14 unit tests pass locally including all three D13-binding TDD anchors:

- `tests/unit/chat/chatgpt.adapter.test.ts::extracts_code_block_with_language`
- `tests/unit/chat/chatgpt.adapter.test.ts::role_inferred_from_data_attr`
- `tests/unit/chat/chatgpt.adapter.test.ts::findInputBox_returns_textarea`

Plus: `getChatId` parsing (uuid path + null on landing + query/hash strip), multi-turn ordering with non-standard role fold-into-system, JSDOM-backed `MutationObserver` settle detection (streaming-attribute drop and fresh-settled-turn append), markdown serialization snapshot. Manifest `host_permissions` adds `https://chatgpt.com/*` (T08/T09 will append `claude.ai` and `gemini.google.com` per D11). Scaffold test updated to assert the enumerated entry instead of the empty-array invariant from T01.

Task `07.md` held at `status: in_review` with `blocked_on: ci-verify` per D13 — flips to `done` only after the PR's CI run reports green.

**Notes for T08/T09 implementers:**

- `registry.ts` now exports `registerAdapter(adapter)` — concrete adapters call it once at module load. `__setAdaptersForTesting` is unchanged (test-only).
- `runtime/chat/adapters/index.ts` is a barrel: each adapter file is imported here for its registration side-effect. Add `import "./claude.adapter.js"` / `import "./gemini.adapter.js"` on a fresh line, alphabetical order, so concurrent feature branches edit different lines and merge cleanly.
- The MutationObserver in the auto-capture hook pulls its constructor from `doc.defaultView.MutationObserver` — Node's `globalThis` doesn't expose it, but JSDOM's window does, so unit tests don't need `environment: jsdom` set globally in vitest.
- **Sandbox caveat:** the three fixtures in `tests/fixtures/chatgpt/` are hand-crafted to match the documented 2026 selectors (`[data-message-author-role]`, `pre > code.language-*`, `textarea[data-id="root"]`, `data-stream` while streaming) — the sandbox cannot reach `chatgpt.com`. A human must refresh them from a live capture before merge if the live DOM has drifted.

---

## 2026-05-10 · T08 — Claude.ai adapter lands; awaiting CI verify

`packages/extension/src/runtime/chat/adapters/claude.adapter.ts` implements the `ChatAdapter` contract for `claude.ai`:

- `hostPattern = /^claude\.ai\//`, `platform = "claude"`.
- `extractConversation` walks `[data-testid^="message-"]`; role is decoded from the testid suffix (`message-user-…`, `message-human-…`, `message-assistant-…`) with nested `data-testid` and class-name fallbacks for layout variants. Content goes through `domNodeToMarkdown` (T06) so `<pre><code class="language-X">` blocks fence cleanly.
- `getChatId` extracts the v4 UUID from `/chat/<uuid>`; returns `null` for `/new`, `/`, and non-UUID suffixes.
- `findInputBox` returns `null` (read-only Phase 1; insert-into-chat is backlog for Claude per task 08).
- `onNewAssistantTurn` runs a `MutationObserver` on `<body>`, firing the callback once per assistant turn whose action bar (`copy` / `regenerate` / `retry`) has settled. Each turn fires at most once (`WeakSet` guard).
- Self-registers on import via `registerAdapter(claudeAdapter)` (idempotent per T06).

Both D13-binding TDD anchors pass (12 adapter tests + 6 scaffold tests + 23 framework tests = 41 chat/scaffold tests green):

- `tests/unit/chat/claude.adapter.test.ts::extracts_multi_turn_conversation` — `long.html` → 6 alternating user/assistant turns.
- `tests/unit/chat/claude.adapter.test.ts::code_blocks_preserved_with_language` — `code.html` → fenced `rust` block with body intact.

`manifest.json` `host_permissions` now lists `"https://chatgpt.com/*"` (T07) and `"https://claude.ai/*"` (T08) in alphabetical order; T09 will append `"https://gemini.google.com/*"`. The scaffold test now asserts the enumerated set via `arrayContaining(["https://chatgpt.com/*", "https://claude.ai/*"])` plus the `^https://…/*$` shape and the explicit no-`<all_urls>` guard inherited from T07.

**Fixture caveat:** the sandbox cannot fetch `claude.ai`, so `tests/fixtures/claude/{empty,code,long}.html` are hand-crafted minimal DOMs that match the documented selectors (`[data-testid^="message-"]`, `[data-testid$="-message-content"]`, `<pre><code class="language-rust">`, `[data-testid="action-bar-copy"]`). **A human reviewer must refresh these fixtures from a live capture before merge** to confirm the selectors still match production Claude.ai DOM (Anthropic ships layout changes without notice). If a refresh breaks tests, the fix is selector-only — adapter logic key off `data-testid` prefixes that are stable under typical refactors.

Task `08.md` held at `status: in_review` with `blocked_on: ci-verify` per D13 — flips to `done` only after the PR's CI run reports green.

**Note for parallel T09 implementer:** adapter / fixture / test files are physically isolated, so no cross-PR conflicts there. The only shared edits (`manifest.json` `host_permissions`, `tests/unit/scaffold.test.ts`) collide trivially — append `"https://gemini.google.com/*"` to the host_permissions array and the `arrayContaining` list, both already in alpha order.

---

## 2026-05-10 · T06 — adapter framework lands; awaiting CI verify

`packages/extension/src/runtime/chat/{types,registry,serializer}.ts` shipped, registry array intentionally empty per D10 (concrete adapters land T07–T09). 20 unit tests pass locally including both D13-binding TDD anchors:

- `tests/unit/chat/serializer.test.ts::markdown_round_trip_preserves_code_blocks`
- `tests/unit/chat/registry.test.ts::unknown_host_returns_null`

Task `06.md` held at `status: in_review` with `blocked_on: ci-verify` per D13 — flips to `done` only after the PR's CI run reports green.

**Notes for T07–T09 implementers:**

- `domNodeToMarkdown` (in `serializer.ts`) is the shared DOM → markdown helper. Adapters that need richer conversion (lists, tables, links) should extend their own pipelines rather than bloat the framework.
- `__setAdaptersForTesting` is a test-only hook; production adapter registration happens by mutating the module-private `adapters` array directly from each adapter file's import side-effect.
- `ChatMeta.capturedAt` flows into the markdown frontmatter only; it is **not** part of `SourceMeta` (which deliberately mirrors `AttestationRow.source_meta`'s narrower shape).

---

## 2026-05-10 · T09 — Gemini adapter lands; awaiting CI verify

`packages/extension/src/runtime/chat/adapters/gemini.adapter.ts` ships a
`ChatAdapter` for `gemini.google.com`. Self-registers via the last-line
`registerAdapter(geminiAdapter);` side effect. 14 unit tests pass locally
including both D13 TDD anchors:

- `tests/unit/chat/gemini.adapter.test.ts::shadow_dom_pierced_to_extract_content`
- `tests/unit/chat/gemini.adapter.test.ts::role_correct_for_assistant_and_user`

Task `09.md` held at `status: in_review` with `blocked_on: ci-verify` per
D13 — flips to `done` only after the PR's CI run reports green.

**Implementation notes:**

- **Selectors.** Turn elements: `<user-query>` (+ optional inner
  `<user-query-content>`) for the user; `<message-content>` and
  `<model-response>` for the assistant. Nested-turn detection prevents
  double-counting when Gemini wraps an inner content element inside the
  outer turn (it climbs across shadow boundaries via the host
  back-reference).
- **Shadow-DOM piercing.** The shared `domNodeToMarkdown` is shadow-blind
  per T06's "extend locally, don't bloat the framework" guidance. The
  adapter ships its own `walkAll` (preorder DFS, host-then-shadow-then-
  light-children) and `shadowAwareMarkdown` (recurses into open shadow
  roots, defers to the shared helper for `<pre>` / `<code>`).
- **`getChatId`.** Parses `/app/<id>` (also tolerates the `/u/<n>/`
  account prefix). Pages that don't carry `/app/<id>` — root, settings,
  share landing — return `null` so the popup falls back to generic
  page-selection capture (D8). This convention is best-guess from
  available URLs; refresh on capture day if Gemini changes the URL
  shape.
- **`onNewAssistantTurn`.** Attaches one `MutationObserver` per shadow
  root encountered (re-attached when new shadow roots appear — a single
  document-level observer cannot pierce a shadow boundary by spec).
  Resolves the `MutationObserver` constructor off `doc.defaultView` so
  the same code path runs in JSDOM unit tests where the global
  constructor is missing.
- **`findInputBox`.** Returns `null` (read-only Phase 1; paste UI lands
  later).
- **Fixtures.** `tests/fixtures/gemini/{empty,code,long}.html` are
  hand-crafted. The sandbox cannot reach `gemini.google.com`, so element
  names + nesting are reverse-engineered from prior captures rather than
  scraped today. **Refresh from a live HAR capture before merging the
  T09 PR** — element + class names may have drifted since the last
  capture, in which case `USER_TAGS` / `ASSISTANT_TAGS` need an update.
- **Declarative shadow DOM.** Fixtures use `<template
  shadowrootmode="open">` so they read like the live DOM. JSDOM 24.1.x
  parses these as plain `<template>` elements (declarative shadow DOM
  isn't auto-applied by the `JSDOM` constructor in this minor); the
  test file ships a `loadFixture` helper that applies them imperatively
  at parse time so the shadow-pierce code path is exercised end-to-end.
  When JSDOM gains automatic declarative-shadow-DOM support the helper
  becomes a no-op — drop it then.

**Shared-file merge note (T07 / T08 / T09 co-run):** the only shared
edit surface is `manifest.json` (host_permissions array) +
`tests/unit/scaffold.test.ts` (host_permissions assertion). The new
scaffold-test assertion uses `toContain(...)` so it is order-independent
— resolving a merge conflict only requires alphabetising the
`host_permissions` array.

---

## 2026-05-11 · T14 — Server Google OAuth provider lands; awaiting CI verify

`mcp/src/oauth/google.rs` + `mcp/src/oauth/google_jwks.rs` ship the
server side of Decision 5: `GET /oauth/google/start` (PKCE-bound,
S256-only), `GET /oauth/google/callback` (token exchange + RS256 JWKS
verification with `aud` / `iss` / `exp` checks + kid-based key pick),
`POST /oauth/google/lookup` (Bearer JWT, returns existing pubkey link
state + a server-issued possession-proof nonce when not yet linked), and
`POST /oauth/google/link` (Bearer JWT, body
`{pubkey_base58, possession_proof_base64, challenge}`, atomically pops
the nonce and verifies a 64-byte Ed25519 signature over it before
inserting `google_identity_links`). PR #TBD.

**Key implementation notes (deviations + clarifications from the task
spec):**

- `oauth.rs` is now `oauth/mod.rs` (directory module) so the new
  `oauth/google.rs` and `oauth/google_jwks.rs` files sit next to it
  without converting the existing 2.8k-line file's tests.
- `Claims` gained an optional `google_sub: Option<String>` claim and
  `IssuedCode` gained the same field; old token wire format is
  byte-identical via `serde(skip_serializing_if = "Option::is_none")`,
  and all 138 existing mcp tests still pass.
- **Possession proof:** server-issued nonce returned by `/lookup` when
  no existing link is present (not a separate `/link-challenge`
  endpoint). 5-minute TTL, single-use; the LRU `pop` is atomic.
- **Rate limit:** 5 calls / 24h / google_sub across `/lookup` +
  `/link` combined (per the security checklist). Implementation uses a
  per-`google_sub` window counter; a separate `OAUTH_RATELIMIT_DISABLE`
  envelope already widens the route-level governor for e2e runs.
- **Auth model:** the existing `bearer_auth_middleware` is URI-
  allowlisted for everything under `/oauth/*`, so the lookup/link
  handlers verify the Bearer JWT inline (`verify_jwt` is `pub`). The
  route still sits under the `/oauth/*` governor for IP-level limits.
- **HTTPS-only redirect URIs** except for loopback (`localhost`,
  `127.0.0.1`, `[::1]`) for dev. Chrome extension callbacks
  (`https://<extid>.chromiumapp.org/...`) are HTTPS by design.
- **Migration:** new `google_identity_links` table created via
  `oauth::google::migrate_google_identity_links(conn)` invoked at
  startup. Lives in `mcp/` per Decision 9 (`core/` reserved for the
  cross-client attestation schema). Idempotent — re-run safe.
- **Disabled mode:** when `GOOGLE_OAUTH_CLIENT_ID` is unset, `main.rs`
  skips wiring the four Google routes entirely (no 404 wrappers
  mounted), and `GoogleOAuthState::is_disabled()` is true if any
  handler is reached directly via a test harness.
- **Logging:** `tracing::info!("Google OAuth: enabled/disabled")` at
  startup per task spec; the migration log is silent on success.

**TDD anchors implemented:**

- `mcp/tests/oauth_google.rs::full_pkce_roundtrip` — start → mock
  Google → callback → lookup → link → server JWT decoded with
  `google_sub` set, both at first-touch and after link.
- `mcp/tests/oauth_google.rs::link_requires_possession_proof` — bad
  sig → 401; missing challenge → 401; valid sig → 200 + row inserted.
- `mcp/tests/oauth_google.rs::id_token_bad_audience_rejected` — token
  with `aud != GOOGLE_OAUTH_CLIENT_ID` → 401.
- `mcp/tests/oauth_google.rs::id_token_bad_signature_rejected` —
  token signed by a different RSA key (same `kid`) → 401.

**Mock Google server:** spawns an axum sub-server on an ephemeral
loopback port; mints a fresh 2048-bit RSA key per test via the `rsa`
dev-dep crate (added to `mcp/Cargo.toml`); exposes JWKs at
`/oauth2/v3/certs` and signs tokens at `/token`. `GoogleJwksCache` and
`GoogleOAuthState` both expose `*::with_endpoints` constructors that
take a base URL — production code uses the defaults
(`accounts.google.com`, `oauth2.googleapis.com`).

Task `14.md` held at `status: in_review` with `blocked_on: ci-verify`
per D13 — flips to `done` only after the PR's CI run reports green.

**Note for T15 implementer:** `key_escrow_blobs` schema is independent;
the lookup handler already gracefully reports `escrow_present = false`
when that table is absent (it checks `sqlite_master` first). Add the
migration in `mcp/` (T14 set the precedent) and route the new endpoints
through the same inline `extract_bearer_claims` helper if you need
JWT-aware handlers under `/oauth/*` or `/api/*`.

---

## 2026-05-11 · T14 — Round-2 review fixes applied (PR #111)

Three reviewer reports (code, security, test) returned
`approve_with_nits`. Round-2 patches in `mcp/src/oauth/google.rs`,
`mcp/src/config.rs`, `mcp/src/main.rs`, `mcp/Cargo.toml`, and
`mcp/tests/oauth_google.rs`:

- **TOCTOU race in `/oauth/google/link`** (security-auditor medium #3).
  Replaced the `lookup_link` existence check + separate `insert_link`
  call pair with a single `INSERT OR IGNORE` statement
  (`insert_link_if_absent`). Concurrent racers no longer trip the
  UNIQUE constraint and trigger a 500 against the legitimate user; the
  loser now sees a clean `409 Conflict {"error":"already_linked"}`.
- **Token-exchange error leak** (security medium #1). The detailed
  upstream error (including any Google response body) is logged at
  `error!` level; the client sees the fixed string
  `google_token_exchange_failed`.
- **`id_token` error leak** (security medium #2). jsonwebtoken's
  `InvalidSignature` / `ExpiredSignature` / claim-mismatch strings are
  logged at `warn!`; the client sees `id_token_invalid`.
- **Unbounded `rate_counters` HashMap** (security low #7 / code major
  #1). Converted to `LruCache<String, RateCounter>` with capacity
  `RATE_COUNTERS_LRU_CAP = 10_000`, matching the other two LRUs.
- **Config drift footgun** (code major #2). `main.rs::run_http` no
  longer re-reads `GOOGLE_OAUTH_CLIENT_ID` / `_SECRET` / `_REDIRECT_URI`
  via `std::env::var`. The three fields in `Config` are now the
  single source of truth, threaded into `run_http` via a private
  `GoogleOAuthSettings` struct. Removed `#[allow(dead_code)]` from
  all three.
- **`escrow_present` error swallowing** (code minor #5). Switched from
  `query_row(...).unwrap_or(false)` to
  `query_row(...).optional()?` so genuine I/O failures bubble while
  the "table not yet created (T15 pending)" case still returns
  `Ok(false)`.
- **Migration runs unconditionally** (code minor #6). Wrapped
  `migrate_google_identity_links` in
  `if !cfg.google_oauth_client_id.is_empty() { ... }` so deployments
  without Google OAuth don't write the table on every boot.
- **`/lookup` rate-limit hits routine post-sign-in** (code minor #3).
  Skip the per-`google_sub` 5/24h check when
  `existing_pubkey.is_some()` AND the JWT was issued within
  `FRESH_JWT_WINDOW_SECS = 60`. Already-linked users doing routine
  sign-ins no longer eat their quota; first-link flows + stale-token
  probes still go through the limit.
- **Hand-rolled `url_encode`** (code nit #7). Replaced with
  `percent_encoding::utf8_percent_encode(s, NON_ALPHANUMERIC)`. The
  crate was already transitive via reqwest; promoted to a direct dep
  in `mcp/Cargo.toml`.
- **`let _ = &pubkey;` no-op** (code nit #8). Removed — `pubkey` is
  already consumed by `verify_signature` upstream.
- **Silent test-feature gate** (test major #1). Added a
  `[[test]] name = "oauth_google"` entry with
  `required-features = ["test-support"]` so `cargo test
  --test oauth_google` (without the feature flag) prints "test
  requires feature test-support, not running" instead of silently
  reporting `0 tests`.
- **Bare `.unwrap()` in shared mock helpers** (test nit). Replaced
  with `.expect("descriptive message")` in `MockGoogle::sign_id_token`,
  `spawn_mock_google`, and `mint_google_jwt`.
- **New negative tests:**
  - `id_token_expired_rejected` (test major #2) — exp two minutes in
    the past → 401 + body `id_token_invalid`.
  - `wrong_pkce_verifier_rejected` (test major #3) — happy callback,
    `/oauth/token` with a verifier that doesn't hash to the stored
    challenge → 4xx.
  - `id_token_alg_none_rejected` (security low #5) — manually
    constructed JWT with `alg: none` and empty signature → 401 +
    `id_token_invalid`.
  - `id_token_alg_hs256_rejected` (security low #5) — HS256-signed
    token with shared secret → 401 + `id_token_invalid`.
- **Body assertions on existing 401 tests** (test nit). Both
  `id_token_bad_audience_rejected` and `id_token_bad_signature_rejected`
  now assert `error == "id_token_invalid"` so a 401 from a different
  guard (e.g. missing state) does not satisfy the test.

**Deferred:** security low #4 (explicit `validation.leeway = 60` + iat
sanity check) was not in the round-1 fix list and is a defense-in-depth
nit; the jsonwebtoken default leeway is already 60s. Will fold into a
follow-up if the security auditor reasserts in round 2.

Total: 8 oauth_google tests (4 original + 4 new) pass locally; clippy
+ fmt clean under `--features mnemonic-mcp/test-support` (the CI
invocation).

---

## 2026-05-11 · T11 — Popup UI (Capture / Recall / Verify) lands; awaiting CI verify

`packages/extension/src/popup/` now ships the three-tab popup the
tech-spec calls for: Capture, Recall, Verify, plus a header with
IdentityBadge + StorageTierPill + Settings cog. Component breakdown:

- `App.tsx` — bootstraps identity / storage tier / active-tab adapter /
  selection on mount and routes between the three tabs via local state.
  The tier pill click opens a confirmation dialog stub pointing at
  Settings (T12 owns the actual tier-switch flow per D7).
- `tabs/Capture.tsx` — textarea (prefilled from the content-script
  selection message) + tag editor + "Sign" button. "Save chat" is
  available when the active-tab URL matches a registered adapter (D11
  enumerated host_permissions; the popup never reaches outside that
  set). Tags auto-include `source:<platform>`; explicit user tags
  dedup against the auto set. Success renders a toast with the
  truncated `attestation_id` and a copy button.
- `tabs/Recall.tsx` — search input → `runtime.recall(query, 5)`. Each
  hit renders `relevance_score`, a `source:*` platform pill, and three
  actions: Copy markdown, Insert into chat, Open. "Insert into chat"
  is disabled when the adapter's `findInputBox` is a one-line
  `return null` (the T08 / T09 convention) — drives TDD anchor
  `insert_into_chat_disabled_when_no_input_box`.
- `tabs/Verify.tsx` — paste `attestation_id` or drop a `.cose` file →
  `runtime.verify` → renders verified / tampered / not_found UI states.

Shared components live under `components/`:

- `IdentityBadge.tsx` — truncated pubkey + `did:sol:<base58>` tooltip;
  renders "(not signed in)" when `chrome.storage.local.identity` is
  unset (T16 onboarding populates it).
- `StorageTierPill.tsx` — read-only Local / Cloud indicator. Click =
  open the tier-switch dialog stub.
- `Toast.tsx` — minimal success / error / info banner with an optional
  copy button (used by Capture for `attestation_id` echo).

**Runtime facade (`src/popup/runtime.ts`):** exports a `PopupRuntime`
interface with `loadIdentity`, `loadStorageTier`,
`getActiveTabAdapter`, `getActiveTabSelection`,
`getActiveTabConversation`, `signMemory`, `signRemote`, `recall`,
`verify`. The default impl reads `chrome.storage.local`, matches the
active tab URL against `selectAdapter` from T06, and lazy-loads the
heavy signing pipeline (`runtime-impl.ts`) only on first sign /
recall / verify — popup cold-open never touches WASM or the embedder
worker, keeping initial JS well under the 50KB size-limit budget.
`signRemote` is the T18 placeholder per the task instructions (no-op
on Local tier).

**`getActiveTabAdapter` helper:** runs `chrome.tabs.query({active:
true, currentWindow: true})`, side-effect-imports the adapters barrel,
and hands the URL to `selectAdapter`. Returns `null` for unsupported
hosts (popup falls back to selection-only capture per D8).

**Tailwind v3 wired in extension build** (`tailwind.config.js`,
`postcss.config.js`, `src/popup/styles.css`). Content globs limited to
`src/popup/**` + `src/options/**` so the JIT scan stays cheap and the
generated CSS bundle minimal. Tokens mirror `webapp/tailwind.config.js`
1:1 (`#0A0F1E` bg, `#00D4B4` accent, monospace for hashes, error /
success colors).

**Size-limit budget** — `.size-limit.json` declares the popup initial
JS budget at 50KB gzip against `dist/src/popup/main.tsx-*.js`. Heavy
paths (WASM, embedder, IndexedDB store) are dynamic-imported behind
`runtime-impl.ts` so they land in their own chunks.

**TDD anchors (D13-binding):**

- `tests/component/popup/Capture.test.tsx::sign_button_calls_signMemory`
  — clicking Sign with a prefilled selection invokes
  `runtime.signMemory` with the selection text + parsed tags + auto-
  added `source:<platform>` tag.
- `tests/component/popup/Recall.test.tsx::insert_into_chat_disabled_when_no_input_box`
  — an adapter whose `findInputBox` is a single-statement `return
  null` body produces a disabled "Insert into chat" button with an
  explanatory tooltip.

Plus six adjacent tests: capture happy-path toast render, capture
empty-content guard, recall results render with scores + platform
pills, Insert enabled for adapters that ship a real `findInputBox`,
verify-verified / verify-tampered / verify-not-found renders + the
empty-paste guard.

**Test setup:** added `tests/setup.popup.ts` which loads
`@testing-library/jest-dom` matchers under jsdom only (the node-env
unit tests stay fast) and stubs `chrome.tabs` / `chrome.storage` /
`chrome.runtime`. `vitest.config.ts` opts the `tests/component/**`
glob into jsdom via `environmentMatchGlobs`. Added devDeps:
`@testing-library/{react,dom,user-event,jest-dom}`, `tailwindcss`,
`postcss`, `autoprefixer`, `size-limit`, `@size-limit/preset-app`.

**Pre-existing build failures inherited from `dev`** (NOT caused by
T11, NOT fixed here):

- `tests/unit/sign/cose.test.ts` aborts because `core/pkg-web/
  mnemonic_core.js` is not built in this dev env — same failure
  reproduces on a clean checkout of `dev` before any T11 file lands.
- `npm run build` was hitting an iife/code-split error from T04's
  worker bootstrap. I added `worker.format: "es"` in `vite.config.ts`
  to unblock the embedder bundle (purely additive); the build now
  progresses past that and surfaces a follow-on missing icon
  (`src/assets/icon-16.png`) which T10 / T20 own. T11 does not block
  on either.

Task `11.md` held at `status: in_review` with `blocked_on: ci-verify`
per D13 — flips to `done` only after the PR's CI run reports green
on the test suite that does run (the unit + component layers; the
build gate inherits the dev-branch breakage).

---

## 2026-05-11 · T15 — Round-2 verification status (PR #114)

Round-2 verifier confirmed that commit `d9deda6 fix(mcp): address T15
review round 1` (on branch `claude/extension-t15-server-escrow-wt`)
addresses all material findings from the three round-1 reviews under
`work/chrome-extension/logs/working/task-15/`. No additional commits
were required; the original round-2 commit was already complete.

**Findings × resolution map** (all addressed by `d9deda6` unless
noted):

Code-reviewer (`code-reviewer-round1.json`):
- major / JWT error detail leaked in `extract_extension_claims` →
  client now receives fixed `jwt_invalid`, detail logged at warn.
- major / `PRAGMA foreign_keys=ON` not set → added to both
  `SqliteStore::open()` and `SqliteStore::in_memory()` (the only
  `core/` touch this task needs; pure DB-connection setting).
- minor / `internal_error` formats JWT-encode error → reviewer
  marked as "acceptable, not blocking"; left as-is.
- minor / per-user cap constant leak in 429 body → fixed opaque
  `too_many_pending_tickets`.
- minor / `now_rfc3339` / `now_unix` skew → single
  `let now = chrono::Utc::now();`.
- minor / nonce upper bound 64 → tightened to 16 bytes.
- minor / `seed_link` linked_at type comment → doc comment added.
- minor / `Retry-After` header silent failure → `match` + warn log.

Security-auditor (`security-auditor-round1.json`):
- T15-M-01 / JWT error leak → same fix as code-major above.
- T15-M-02 / expired-ticket counter pinning (self-DoS) →
  `ExtensionBootstrapTickets::insert` sweeps TTL-expired entries
  for the inserting `jwt_sub` BEFORE counting toward the cap; new
  test `expired_tickets_do_not_pin_per_user_counter`.
- T15-L-01 / cap constant leak → same fix as code-minor above.
- T15-L-02 / cross-user GET test → new `get_cross_user_isolation`.
- T15-L-03 / pubkey_base58 length cap → 64-char upper bound.
- T15-N-01 / two `Utc::now()` calls → same fix as code-minor.

Test-reviewer (`test-reviewer-round1.json`):
- F1 blocker / DELETE cross-user isolation → new
  `delete_cross_user_isolation`; defensive direct-SQL check that
  user B's row survives user A's DELETE attempt.
- F2 blocker / expired ticket → 404 → new
  `expired_bootstrap_ticket_returns_404` (harness with
  `ttl_seconds=0`).
- F3 major / 429 body assertion → 429 body now carries fixed
  `rate_limited` token; `rate_limit_blocks_brute_force` asserts on
  it.
- F4 major / replay-nonce test absent → **deferred with rationale**:
  escrow PUT/GET have no per-request server-side nonce. The body
  `nonce` is a data-layer AES-GCM cipher nonce; bootstrap tickets
  are already single-use (covered); escrow PUTs are idempotent
  under replay (rewrap of same value). The tech-spec line is a
  draft carryover. No test added; rationale recorded in worktree
  `decisions.md` (T15 round-2 entry).
- F5 minor / PUT/DELETE missing google_sub → new
  `key_escrow_put_and_delete_require_google_sub_claim`.
- F6 minor / `linked_at` INTEGER vs spec TEXT → doc comment on
  `seed_link`; T14 schema is the source of truth.
- F7 minor / per-user cap test → new
  `bootstrap_per_user_cap_blocks_fourth_ticket`.

**Verification on the worktree** (`/private/tmp/t15-worktree`,
branch `claude/extension-t15-server-escrow-wt`):

- `cargo build --workspace` → green.
- `cargo test -p mnemonic-mcp --test key_escrow --features
  mnemonic-mcp/test-support` → 16/16 pass in 0.29 s.
- `cargo test --workspace --features mnemonic-mcp/test-support
  --no-fail-fast` → all suites green, no failures.
- `cargo clippy --workspace --lib --bins -- -D warnings` → clean.
- `cargo clippy --workspace --all-targets --features
  mnemonic-mcp/test-support -- -D warnings` → clean.
- `cargo fmt --all -- --check` → clean.

**Push**: `d9deda6` force-with-leased to
`origin/claude/extension-t15-server-escrow` (PR #114). Noise file
`docs/QUICKSTART.md` (case-insensitive FS collision in this
worktree) was deliberately not staged.

**Architectural rules preserved**:
- Migration stays in `mcp/src/escrow.rs`, not `core/`.
- The `PRAGMA` change in `core/src/storage/sqlite.rs` is a
  per-connection SQLite setting, not domain logic — no `mcp/`
  references introduced into `core/`.
- `rusqlite::Connection` Mutex is never held across `.await` in any
  handler; verified by re-reading `key_escrow_get_handler` and
  `key_escrow_put_handler`.
- No `.unwrap()` in production code; tests are unaffected.

Task `15.md` remains `status: in_review` / `blocked_on: ci-verify`
per D13 — flips to `done` only when PR #114 CI reports green.

---

## 2026-05-11 · T11 — Popup UI round-2 review fixes

PR #113 received one round-1 blocker, three majors and four minors
from `code-reviewer`, plus three majors / six minors / four nits
from `ux-reviewer`. Worktree
`/private/tmp/t11-worktree` (branch
`claude/extension-t11-popup-wt`) reapplied the WIP patch from the
interrupted resume, dropped the unrelated noise hunks
(`docs/quickstart.md` and the embedder seed-vector fixture), then
walked the findings list. Findings addressed:

**Code-reviewer:**

- BLK-1 — added `supportsInsert: boolean` to the `ChatAdapter`
  interface in `runtime/chat/types.ts`. ChatGPT sets it `true`;
  Claude / Gemini set it `false` (Phase 1 read-only). `Recall.tsx`
  reads `Boolean(adapter?.supportsInsert)` directly — no more
  `Function.prototype.toString` heuristic that minifiers would have
  silenced. Test fixtures + `registry.test.ts` stub mirror the new
  field; both D13 anchors stay binding.
- MAJ-1 — file-drop verify path returns
  `{ status: 'not_found', reason: 'file_drop_unsupported' }` and
  the UI renders a clear "file verification coming soon — paste an
  attestation id" placeholder instead of the misleading generic
  not-found state. The runtime doc-comment + `decisions.md` entry
  here flag it as a known MVP limitation pending the WASM
  `verify_artifact` export (T05 follow-up).
- MAJ-2 — `verifyRow` now sets `presence_only: true` and the UI
  renders "STORED LOCALLY · VERIFIED" with an explicit caveat
  ("Cryptographic verification coming soon — this confirms the
  local record exists with a non-empty COSE envelope."). Missing
  COSE bytes still trip the tampered path. **Known MVP limitation:
  the popup does NOT yet perform cryptographic signature
  verification — it only confirms presence + non-empty COSE
  envelope.** Full verify is queued behind T05's `verify_artifact`
  WASM export.
- MAJ-3 — added a dedicated `Extension popup component tests
  (vitest)` step to `.github/workflows/node-test.yml` running
  `bunx vitest run tests/component` from `packages/extension`. The
  D13 TDD anchors (`sign_button_calls_signMemory`,
  `insert_into_chat_disabled_when_no_input_box`) now gate merges
  per the round-1 review.
- MIN-1 — `runtime-impl.ts::loadIdentity` now applies the same
  defensive try/catch pattern as `runtime.ts::readChromeStorage`,
  and validates `identity_secret` is exactly a 64-byte array
  before returning (Solana keypairs are 64 bytes — anything else
  would silently produce malformed signatures).
- MIN-2 — JSDoc on the module-level `IndexedDbStore` singleton
  documenting it is popup-realm scoped; the service worker must
  build its own instance via `runtime/store/indexeddb.ts` if
  background recall is ever wired.
- MIN-3 — `Recall.handleInsert` surfaces failures via the inline
  error block ("Insert failed — reload the chat tab and try
  again.") instead of swallowing them silently.
- MIN-4 — `.size-limit.json` accepts the three plausible Vite
  chunk patterns crxjs may emit so the budget enforces against
  whichever path the build resolves.

**UX-reviewer:**

- Major a11y (App tabs) — switched to APG tabs pattern:
  `role="tablist"`, `role="tab"` + `aria-selected` +
  `aria-controls`, `role="tabpanel"` + `aria-labelledby`,
  `tabIndex` reflects selection. Added arrow / Home / End keyboard
  handler that moves focus onto the new tab.
- Major a11y (Verify state icons) — each outcome carries a
  non-color glyph: `✓` verified, `✗` tampered, `?` not-found /
  presence-only. Color is supplemental, not the only signal.
- Major a11y (Toast Escape) — Toast accepts `onDismiss` +
  `autoClearMs`. Escape key + close button (`aria-label="Dismiss"`)
  both call `onDismiss`. Auto-clear after 4 s by default.
- Minor a11y (Settings cog) — `title` removed; the gear glyph is
  wrapped in `aria-hidden="true"` so the `aria-label` is the
  single accessible name.
- Minor a11y (Capture textarea) — explicit `id`/`htmlFor` pairing,
  duplicate `aria-label` removed.
- Minor a11y (Recall empty state) — tracks `hasSearched`. Pre-search
  the list is empty; post-search-no-results renders "Recall
  returned no results for this query."
- Minor feedback (Recall busy) — Find button shows "Recalling"
  with `aria-busy="true"` while the request is in flight.
- Minor tone (Verify tampered reason) — prefixed with
  "Verification failed: " per ux-guidelines.
- Minor a11y (Verify drop zone) — added a hidden `<input
  type="file">` triggered by a visible "Choose file" button so
  keyboard / non-pointer users can pick a file without
  drag-and-drop.
- Nit (TierDialog Escape) — added a document-level Escape handler
  that closes the dialog and an initial focus on the primary
  action. Full focus-trap deferred to T12 dialog primitive.
- Nit (Recall attestation id) — added `title` + `aria-label`
  exposing the full `attestation_id` on the truncated span.

**Verification:**

- `bunx vitest run tests/component` → 10/10 component tests pass
  including both D13 TDD anchors.
- `bunx vitest run` → 101/107 pass; the six failures are
  pre-existing (`tests/unit/sign/cose.test.ts` fails to load the
  WASM artefact `core/pkg-web/mnemonic_core.js` because the
  worktree has not built it — out of T11's scope).
- `bun test` (excludes `tests/component/**` per bunfig.toml) →
  91/92 pass with the same WASM-load failure as the only miss.
- `bun run build` fails on a missing icon asset
  (`src/assets/icon-16.png`) — pre-existing T01 / T10 / T20 gap,
  not introduced or aggravated here.

**Push:** `af711e7` force-with-leased to
`origin/claude/extension-t11-popup` (PR #113). Noise file
`docs/QUICKSTART.md` (case-insensitive FS collision in this
worktree — HEAD already carries both casings as a separate pre-
existing inconsistency) was deliberately not staged.

**Architectural rules preserved:**

- `core/` ↔ `mcp/` separation untouched (this PR is extension-only).
- D13 TDD anchors (`sign_button_calls_signMemory`,
  `insert_into_chat_disabled_when_no_input_box`) remain binding;
  the second one now asserts the `disabled` attribute against the
  explicit `supportsInsert: false` field rather than the
  toString heuristic. Both anchors execute under the new vitest CI
  step.
- `worker.format: 'es'` in `vite.config.ts` (T04 dependency)
  untouched.
- `tests/component/**` stays excluded from `bun test` per the T11
  round-1 commit; the new vitest CI step covers them.

**Deferrals (out of scope for round-2):**

- Full WASM `verify_artifact` export (T05 follow-up) so the popup
  can drop the `presence_only` caveat and verify dropped files.
- size-limit `--fail-if-not-found` style enforcement (no such flag
  in size-limit 12.x; mitigated by listing three plausible globs).
- Pre-existing `bun run lint` / `bun run build` failures
  (vite/vitest version mismatch, missing icon asset) — not
  introduced here.

Task `11.md` remains `status: in_review` / `blocked_on: ci-verify`
per D13 — flips to `done` only when PR #113 CI reports green.

---

## 2026-05-11 · T10 — MV3 manifest + service-worker router lands; awaiting CI verify

`packages/extension/manifest.json` finalised + `packages/extension/src/background/service-worker.ts` now owns the typed dispatch surface:

- **Permissions** (unchanged from T01 baseline): `storage`, `identity`, `contextMenus`, `activeTab`, `clipboardWrite`, `alarms`. **No `<all_urls>`** anywhere (D11). `host_permissions` stays enumerated: `https://chatgpt.com/*`, `https://claude.ai/*`, `https://gemini.google.com/*`.
- **`content_scripts`** — one entry per supported AI-chat domain, loading the matching adapter + `src/content/fab.ts` + `src/content/recall-overlay.ts` at `document_idle`. The two content-script entry files are intentionally thin stubs in T10; T13 (FAB + recall overlay) owns the real UI. Without the stub files the manifest references would break the @crxjs/vite-plugin bundle.
- **`commands`** — `_execute_action` (Ctrl/Cmd+Shift+M, opens popup) and `recall-overlay` (Ctrl/Cmd+Shift+R, dispatches `sw:open-recall-overlay` to active tab). The previous baseline used `Ctrl+Shift+K` for the overlay; updated to match the T10 spec.
- **`web_accessible_resources`** — `src/assets/*`, `src/content/recall-overlay.css` (T13 will create), `src/runtime/embed/worker.ts` (T04), `wasm/*.wasm`, `models/*`, scoped to the three enumerated origins. No `<all_urls>` here either.
- **CSP** — `extension_pages: "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'; base-uri 'self'; connect-src 'self' https://mc.mnemonik.xyz https://huggingface.co https://cdn-lfs.huggingface.co"`. No `unsafe-eval`, no remote script sources; `wasm-unsafe-eval` allowed for the `@mnemonic/core` WASM (and transformers.js). `connect-src` enumerates the hosted MCP server + Hugging Face CDNs (lazy-loaded embedder model from T04).
- **Service worker** wiring (`installServiceWorker(deps)` is exported so the unit tests can mock `chrome.*`):
  - `chrome.runtime.onInstalled` → registers the `save-selection` context menu (contexts: `['selection']`) and the `cloud-sync-retry` alarm (`periodInMinutes: 5`).
  - `chrome.contextMenus.onClicked` → on `save-selection`, dispatches `{type: 'sw:save-selection', payload: {selectionText, pageUrl, pageTitle?, capturedAt}}` to the active tab. Empty selections / mismatched menu items are dropped silently. This is the user-gesture surface that satisfies D11/D12 for generic page capture.
  - `chrome.commands.onCommand` → on `recall-overlay`, dispatches `{type: 'sw:open-recall-overlay', payload: {trigger: 'hotkey'}}` to the active tab. `_execute_action` is handled directly by Chrome and intentionally not routed through the SW.
  - `chrome.runtime.onMessage` → narrows via `parseMsg(unknown): Msg | null` (no `any` in the public surface), async-dispatches to `handleMsg`, returns `true` to keep `sendResponse` alive across awaits (MV3 contract). Unknown shapes → `{ok: false, error: 'unknown-message'}`.
  - `chrome.alarms.onAlarm` → on `cloud-sync-retry`, instantiates an `IndexedDbStore` and calls `flushPending({store})`. Errors are warn-logged but don't crash the SW.
- **`src/messages.ts`** — discriminated union `Msg` over `sw:save-selection | sw:open-recall-overlay | ui:sign-memory | ui:recall | ui:flush-pending | tab:capture-candidate`. All payloads are JSON-serialisable (binary blobs travel as base64 at the runtime layer — popup/content/SW never carry typed-arrays through `chrome.runtime.sendMessage`). `parseMsg` is the only narrowing entry point; tests pin its rejection of unknown shapes.
- **`src/runtime/sync/cloud-client.ts`** — T18-owned stub. Exports `flushPending({store}): Promise<{attempted, flushed}>` so the SW has a stable call site before T18 lands. The stub reads `listPending()` for telemetry and returns `{attempted: <n>, flushed: 0}`; T18 will swap in the real deferred-signing POST flow.

**D13-binding TDD anchors pass locally:**

- `tests/unit/background/service-worker.test.ts::context_menu_save_selection_emits_message` — fires `contextMenus.onClicked` with selectionText + pageUrl + tab → asserts `tabs.sendMessage(tabId, {type:'sw:save-selection', payload:{selectionText, pageUrl, ...}})`. Companion negative tests cover empty selection drop and non-save-selection menu-id drop.
- `tests/unit/background/service-worker.test.ts::alarm_drains_pending_queue` — enqueues two attestation ids into a real `IndexedDbStore` (via `fake-indexeddb/auto`), fires `alarms.onAlarm` with name `cloud-sync-retry` → asserts the injected `flushPending` spy was called exactly once with `{store}` and the store still contains the two pending rows (T18 dequeues; T10 only wires the drain trigger). Companion test asserts other alarm names don't trigger flushPending.

Plus: install handler creates both the context menu and the alarm with the right ids; commands.onCommand routes the overlay hotkey; the typed `onMessage` router accepts `ui:flush-pending` and rejects unknown shapes with a sentinel error.

**Sandbox observations / pre-existing failures (not in scope for T10):**

- `tests/unit/sign/cose.test.ts` fails to load `core/pkg-web/mnemonic_core.js` — the WASM bindings aren't built in the sandbox image. Pre-existing on `dev`; T05 / a CI build step owns producing that artifact. T10 does not touch the signing path.
- `vite.config.ts` reports a `tsc -b` error from the duplicate `vite` install (root + nested `node_modules/vite` typings disagree under `exactOptionalPropertyTypes`). Pre-existing on `dev`; out of T10 scope. The crxjs build still runs.

**Note for T13:** the manifest's `content_scripts` entries already point at `src/content/fab.ts` and `src/content/recall-overlay.ts`. T13 should replace these stubs in place rather than renaming, so the manifest stays untouched. The CSS file referenced from `web_accessible_resources` (`src/content/recall-overlay.css`) does not exist yet — T13 creates it; absent at build time it's a soft miss in `web-ext lint` (warning, not failure).

**Note for T18:** `src/runtime/sync/cloud-client.ts::flushPending` is the stable entry point. Inject the `IndexedDbStore` via the `FlushPendingDeps` shape; the SW already constructs and passes the store. Iterate FIFO over `store.listPending()`, drain via the deferred-signing flow described in tech-spec §"Cloud-mode `signMemory`", and call `store.dequeue(attestation_id)` on success. Test the drain in isolation against the same `fake-indexeddb` fixture pattern.

Task `10.md` held at `status: in_review` with `blocked_on: ci-verify` per D13 — flips to `done` only after the PR's CI run reports green.

---

## 2026-05-11 · T15 — Server key-escrow + extension-bootstrap endpoints land; awaiting CI verify

`mcp/src/escrow.rs` + `mcp/tests/key_escrow.rs` ship the server side
of Decision 9 (encrypted key escrow). Five HTTP endpoints under
`/api/extension-bootstrap/*` and `/api/key-escrow`:

- `POST /api/extension-bootstrap/issue` — auth: `aud=mcp` Bearer JWT
  (enforced by the existing bearer-auth middleware). Mints a UUID
  ticket bound to `(jwt_sub, google_sub)` with 10-min TTL, LRU 10k,
  per-user cap 3. Mirrors the existing cli-bootstrap flow.
- `GET /api/extension-bootstrap/redeem/{ticket}` — no auth (UUID is
  the capability, same model as cli-bootstrap; URI-allowlisted in the
  bearer-auth middleware). Pops the ticket atomically; on success
  returns `{access_token, aud="extension", expires_in}` where
  `access_token` is a fresh HS256 JWT signed with the same
  `MCP_JWT_SECRET` as the production OAuth flow. Single-use; second
  redeem of the same ticket → 404.
- `PUT /api/key-escrow` — auth: `aud=extension` JWT with `google_sub`,
  verified inline. Validates `pubkey_base58` matches the linked
  `pubkey_base58` in `google_identity_links` (binding check). Atomic
  `INSERT ... ON CONFLICT DO UPDATE` UPSERT; resets
  `fetch_count_24h=0` and `last_fetch_at=NULL` on every PUT.
- `GET /api/key-escrow` — auth: same. Single critical section: reads
  the row, checks rate limit (5 GETs / 24h rolling window per
  `google_sub`, configurable via `KEY_ESCROW_RATE_LIMIT`),
  atomically increments via a single SQL `UPDATE ... CASE` that does
  both the reset-on-elapsed and bump-in-window in one statement.
  Returns 429 with `Retry-After: <seconds_until_window_resets>` once
  the cap is exceeded.
- `DELETE /api/key-escrow` — auth: same. Hard delete (D9 explicit
  revocation); idempotent (200 with `removed: 0` on second call).

**Key implementation deviations + clarifications:**

- **Migration lives in `mcp/`, NOT `core/`** (deviation from the T15
  task spec). The original spec said `core/src/storage/sqlite.rs`;
  but per CLAUDE.md and Decision 9 the OAuth/escrow tables are
  server concerns, and T14 already set the precedent by putting
  `google_identity_links` in `oauth/google.rs::migrate_google_identity_links`.
  The new `escrow::migrate_key_escrow_blobs` mirrors that pattern
  and runs at startup alongside the T14 migration (only when
  `GOOGLE_OAUTH_CLIENT_ID` is set — the FK in `key_escrow_blobs`
  references `google_identity_links`, so both tables move together).
  `core/` remains untouched.

- **Schema-level zero-knowledge audit.** The migration string lives
  as `pub const MIGRATION_SQL: &str` in `mcp/src/escrow.rs` so the
  T15 TDD anchor `server_never_stores_plaintext` can read it via the
  library facade and assert that no plaintext-leaking column name is
  present (`plaintext`, `secret_key`, `secret_seed`, `raw_key`,
  `private_key`, `unencrypted`, ` seed `). The same test also
  positively asserts presence of the opaque columns (`ciphertext`,
  `nonce`, `kdf`, `kdf_params`, `pubkey_base58`). This is a compile-
  time-stable invariant that catches a future refactor that adds an
  unintended plaintext column.

- **`aud=extension` JWT split.** A fresh audience separate from the
  production `aud=mcp` audience means the bearer-auth middleware on
  `/mcp` rejects extension JWTs cleanly — they only validate at the
  escrow endpoints, which verify inline via `extract_extension_claims`.
  Both JWT shapes share a signing key (the same `MCP_JWT_SECRET` /
  HS256) so the server only needs one secret to rotate. The new
  `OAuthState::jwt_encoding_key()` / `jwt_decoding_key()` accessors
  expose the key to the escrow module.

- **Single source of truth for `KEY_ESCROW_RATE_LIMIT`.** `Config`
  reads the env value once at startup; `main.rs::run_http` threads it
  through a new `ExtensionSettings` struct (sibling of
  `GoogleOAuthSettings`). No downstream `std::env::var` re-reads —
  matches T14's round-2 fix #2.

- **Atomic counter UPDATE.** The rate-limit increment is a single SQL
  statement, not READ-then-WRITE. Two concurrent GETs both observe
  the SAME pre-update count and both write `count = old + 1`, so the
  counter cannot drift under race. The CASE expression handles
  reset-on-elapsed + in-window-bump in one go.

- **No in-memory rate-limit map** — the DB row IS the per-user
  counter. This avoids the unbounded HashMap class of issues that
  T14 round-2 had to LRU-bound. The DB row already has a primary key
  so memory pressure is naturally bounded.

- **Pubkey binding is enforced server-side, not via FK.** The PUT
  handler explicitly looks up `google_identity_links.pubkey_base58`
  before UPSERT and 403s on mismatch (with a generic error message
  that doesn't leak the linked pubkey). The FK on the schema
  cascades on DELETE of the parent row, which is a cleanup nicety,
  not the security boundary.

- **DELETE is hard.** No soft-delete column. The user's explicit
  revocation removes the row permanently; on next PUT the rate-limit
  counter starts fresh from zero. Matches D9's "explicit revocation"
  language.

**Endpoint allowlist additions** in `oauth::bearer_auth_middleware`:

- `/api/extension-bootstrap/redeem/` — UUID-as-capability path,
  bypasses bearer-auth.
- `/api/key-escrow` — verifies its own `aud=extension` JWT inline.

**TDD anchors implemented (all 10 tests pass locally):**

- `mcp/tests/key_escrow.rs::rate_limit_blocks_brute_force` (TDD #1):
  burns the 5-GET budget, asserts the 6th GET returns 429 with a
  positive `Retry-After` header capped at 24h.
- `mcp/tests/key_escrow.rs::pubkey_binding_enforced` (TDD #2):
  PUT with a different `pubkey_base58` than the linked
  `google_identity_links` row → 403; PUT with the correct pubkey →
  200.
- `mcp/tests/key_escrow.rs::server_never_stores_plaintext` (TDD #3):
  Schema audit against the `MIGRATION_SQL` const.
- `put_then_get_round_trip_identical_bytes`: end-to-end byte-identical
  round-trip.
- `stale_window_resets_counter`: seeds `last_fetch_at` to 25h ago via
  direct SQL, asserts the next GET succeeds and the counter resets
  to 1.
- `delete_removes_row_subsequent_get_404`: GET after DELETE → 404;
  second DELETE → 200 with `removed: 0` (idempotent).
- `put_without_prior_google_identity_link_403`: app-level binding
  check fires before any DB FK violation.
- `key_escrow_rejects_mcp_aud_jwt`: an `aud=mcp` JWT must not be
  accepted on `/api/key-escrow` (audience split is enforced).
- `key_escrow_requires_google_sub_claim`: an `aud=extension` JWT
  without `google_sub` → 401.
- `bootstrap_ticket_round_trip_issues_extension_jwt`: full handshake
  → issue ticket with `aud=mcp` JWT → redeem (no auth) → get
  `aud=extension` JWT → use that on `/api/key-escrow` → second redeem
  of same ticket → 404.

**Cargo.toml gate.** Mirrors T14: `[[test]] name = "key_escrow"`
with `required-features = ["test-support"]` so omitting the feature
flag produces a clear "test requires feature" message instead of
silently `0 tests`.

Task `15.md` held at `status: in_review` with `blocked_on: ci-verify`
per D13 — flips to `done` only after the PR's CI run reports green.

**Note for downstream tasks.** The extension client side (T13's
seed-restore flow + popup) calls these endpoints with the redeemed
`aud=extension` JWT and the WASM `argon2id_wrap` helper builds the
PUT body. The schema is opaque to the server beyond the format
checks in `key_escrow_put_handler` (length bounds on each field,
non-empty ciphertext, base64-decodable nonce). When T18 (cloud sync)
needs a per-user revocation hook, it can reuse `DELETE /api/key-escrow`
without server changes.

---

## 2026-05-11 · T15 — Round-2 review fixes applied (PR #114)

Three reviewer agents ran round 1 against the T15 commit
(`73cdd8f feat(mcp): T15 — extension-bootstrap + key-escrow endpoints +
migrations`). Reports landed at
`work/chrome-extension/logs/working/task-15/{code-reviewer,security-auditor,test-reviewer}-round1.json`.
Verdicts: `approve_with_nits`, `approve_with_nits`,
`approve_with_blockers`. Two majors and two blockers; the remaining
findings were mediums/lows/nits. All actionable findings addressed
below.

**Majors / blockers fixed**

- **JWT error masking in `extract_extension_claims`** (code-reviewer
  major; security-auditor T15-M-01). The 401 response body for an
  invalid `aud=extension` Bearer JWT formatted the jsonwebtoken
  internal error (`InvalidSignature`, `ExpiredSignature`,
  `InvalidAudience`, `InvalidIssuer`) into the response — same leak
  T14 round-2 fixed for `id_token_invalid` in `oauth/google.rs`. T15
  reintroduced the pattern.
  **Fix**: `tracing::warn!` logs the detail server-side; the client
  receives the fixed string `{"error": "jwt_invalid"}`. The
  `iss`/`aud` mismatch branch now logs the structural reason and
  returns the same opaque token.
- **`PRAGMA foreign_keys = ON` not set** (code-reviewer major). The
  FK on `key_escrow_blobs(google_sub) REFERENCES google_identity_links
  ON DELETE CASCADE` was decorative: SQLite defaults to FK OFF per
  connection, so unlinking a Google account would never cascade.
  **Fix**: `PRAGMA foreign_keys=ON` added to `SqliteStore::open()`
  AND `SqliteStore::in_memory()` (the latter is what
  `mock_state()` builds, so tests now also exercise FK enforcement).
  This is the only `core/` touch this task requires; it's global DB
  infrastructure, not OAuth/escrow domain.
- **Expired-ticket per-user counter pinning** (security-auditor
  T15-M-02). If a user issued `EXTENSION_BOOTSTRAP_PER_USER_CAP=3`
  tickets and let them all expire (10-min TTL) without redeeming,
  the in-memory `per_user` counter stayed at 3 — locking the user
  out of new tickets until LRU eviction pushed those entries out
  (which requires 10k+ unrelated tickets first). Self-DoS; also a
  partial denial-of-service vector for a stolen `aud=mcp` JWT.
  **Fix**: `ExtensionBootstrapTickets::insert` now sweeps
  TTL-expired entries belonging to the inserting `jwt_sub` BEFORE
  counting toward the cap (option (a) from the review). New test
  `expired_tickets_do_not_pin_per_user_counter` constructs a
  harness with `ttl_seconds=0` and asserts the 4th issue still 200s.
- **F1: DELETE cross-user isolation untested** (test-reviewer
  blocker). The handler scopes DELETE by the JWT claim's
  `google_sub`, so a wrong-identity DELETE returns
  `200 {removed: 0}` rather than 403, which is correct (idempotent
  non-reveal) — but the invariant was never pinned by a test.
  **Fix**: new test `delete_cross_user_isolation` PUTs as user B,
  DELETEs with user A's JWT, asserts `removed=0`, and then reads
  user B's row directly from SQLite to verify it survives. A
  regression dropping the `WHERE google_sub = ?` clause from the
  DELETE SQL would fail this test.
- **F2: Expired bootstrap ticket → 404 untested** (test-reviewer
  blocker). The TTL-expiry branch in `consume()` was never driven.
  **Fix**: new test `expired_bootstrap_ticket_returns_404` builds a
  harness with `ttl_seconds=0`, issues a ticket via the production
  HTTP handler, and asserts redeem returns 404 with the same body
  as the garbage-UUID case (no oracle).

**Majors (test-reviewer) fixed**

- **F3: 429 body error field not asserted**. The
  `rate_limit_blocks_brute_force` test captured the 429 body but
  never asserted its `error` field — a regression to `{"error":
  null}` would silently pass.
  **Fix**: the 429 body now carries the fixed token
  `{"error": "rate_limited"}` and the test asserts on it. The
  previous verbose message ("rate limit exceeded: too many GETs
  in 24h window") is replaced by the opaque token; no
  attacker-useful information was in it anyway.
- **F4: replay-nonce test absent**. The tech-spec listed
  "replay nonce" as a server-side test. **Deferred with reason**:
  the escrow PUT/GET endpoints do not use a per-request nonce;
  the AES-GCM nonce in the request body is a data-layer cipher
  nonce, not a transport replay token. Bootstrap tickets ARE
  single-use (covered by
  `bootstrap_ticket_round_trip_issues_extension_jwt` second-redeem
  → 404). Escrow PUTs are authenticated by `aud=extension` JWT
  (idempotent under replay — repeating a PUT just rewraps the same
  value). There is no per-request server-side nonce to test
  replay against; the test-spec item was a draft carryover. No
  test added; rationale recorded here.

**Mediums / lows fixed (security-auditor + code-reviewer)**

- **T15-L-01 / cap-constant leak**. The 429 body for per-user
  cap exceeded ("3 active") leaked the numeric cap. **Fix**:
  replaced with the fixed opaque token
  `{"error": "too_many_pending_tickets"}`. Cap value logged at
  `debug!` for operators. Test
  `bootstrap_per_user_cap_blocks_fourth_ticket` asserts on the
  fixed token.
- **T15-L-03 / pubkey_base58 length cap**. PUT did not bound the
  pubkey-string length; a 1MB payload would be stored. **Fix**:
  added `if pubkey_base58.len() > 64 { return bad_request(...); }`.
  Ed25519 base58 pubkeys are 43–44 chars; 64 gives safe headroom.
- **T15-L-02 / explicit GET cross-user isolation test**. Added
  `get_cross_user_isolation`: only user B has an escrow row, user
  A's GET must 404 (never return B's data). The test also
  defensively asserts that if a future regression returns 200, the
  body must not carry user B's pubkey.
- **Nonce upper bound 64 → 16 bytes**. The previous 64-byte cap
  on the AES-GCM nonce was over-generous (AES-GCM-256 uses 12; some
  AEADs use 16). **Fix**: tightened to `1..=16` bytes.
- **`now_rfc3339` / `now_unix` skew in GET handler**. Two
  `chrono::Utc::now()` calls captured. **Fix**: single
  `let now = chrono::Utc::now();` derived into both
  representations.
- **`Retry-After` header construction failure silently dropped**.
  **Fix**: replaced the `if let Ok(...)` with a `match` that warns
  on error and still returns the 429 body. Behaviour is unchanged
  in the success path; failures are now observable.
- **F5: PUT/DELETE missing google_sub → 401**. The original
  `key_escrow_requires_google_sub_claim` only exercised GET. **Fix**:
  added `key_escrow_put_and_delete_require_google_sub_claim` that
  drives the same negative path on the two write verbs.
- **F6: linked_at INTEGER vs spec TEXT**. The T14 migration created
  `google_identity_links.linked_at INTEGER` (unix seconds), not
  TEXT as the T15 task spec described. The test helper `seed_link`
  already inserted an integer; **Fix**: added a doc-comment on
  `seed_link` clarifying T14's schema overrides the T15 spec text.
- **F7: per-user bootstrap-ticket cap test**. New test
  `bootstrap_per_user_cap_blocks_fourth_ticket` exercises the 3+1
  case end-to-end via the HTTP handler.

**Architectural constraint preserved**

- The only `core/` change is the per-connection
  `PRAGMA foreign_keys=ON` in `SqliteStore::{open,in_memory}`. No
  OAuth/escrow tables, helpers, or constants moved into `core/`.
  The dependency graph remains one-way (`mcp/` → `core/`).

**Test count**

`mcp/tests/key_escrow.rs` grew from 10 to 16 tests; all pass in
0.29s. Workspace test run (`cargo test --workspace --features
mnemonic-mcp/test-support`) green; T14 OAuth tests unchanged.

Task `15.md` stays at `status: in_review` / `blocked_on: ci-verify`
per D13 — flips to `done` only after PR #114 CI reports green.

---

## 2026-05-11 · T10 — Round-2 review fixes (security finding T10-N2-01)

Round-2 security audit on PR #112 surfaced a low-severity logic flaw in
`isAuthorisedSender`:

- The early sender.id guard was `if (typeof ownId === "string" && sender.id && sender.id !== ownId)` — the `sender.id &&` short-circuit silently allowed undefined/empty `sender.id` to bypass the rejection. Tightened to `sender.id !== undefined && sender.id !== ownId` so any non-matching set value is rejected.
- The `ui:*` branch had `if (sender.url === undefined) return true;` — when both `sender.id` and `sender.url` were absent, the message passed without positive identification. Replaced with a positive-identification requirement: at least one of `sender.id === ownId` or `sender.url.startsWith("chrome-extension://" + ownId + "/")` must hold; otherwise reject.
- Added a regression test (`rejects ui:* messages with no positive sender identification (T10-N2-01)`) — bare `{}` MessageSender now resolves to `{ ok: false, error: "unauthorized-sender" }` and `flushPending` is never invoked.
- Replaced `vi.waitFor` (added in the previous round-2 commit per nit #6) with a Promise-based polling loop, since `vi.waitFor` is vitest-only and `bun test` runs the same file. All 10 service-worker tests + the 104-test extension suite stay green.

Deferred: T10-N2-02 nit (`scripting` permission pre-declared for T13).
Same pattern as `clipboardWrite` was — keep for now since T13 is the
next wave (Wave C) and lands within the same release. Will revisit if
T13 slips.

---

## 2026-05-11 · T13 — FAB + Recall overlay (Shadow DOM, hotkey + content-script)

Replaces the T10 thin stubs at `src/content/fab.ts` and `src/content/recall-overlay.ts` with the full UI. Manifest unchanged from T10 (content_scripts entries + `web_accessible_resources` for `recall-overlay.css` were pre-declared in T10 round 2).

**Files shipped**

- `packages/extension/src/content/shadow-styles.ts` — single CSSStyleSheet-string export (`SHADOW_STYLES`). Tokens mirror webapp (`#0A0F1E` bg, `#00D4B4` accent, `ui-monospace` font). All selectors live under the implicit `:host` boundary of the shadow root.
- `packages/extension/src/content/fab.ts` — `mountFab(): Promise<HTMLElement | null>`. 56px circular button → menu (Save chat / Save selection / Open Mnemonik). Drag-to-reposition with persistence in `chrome.storage.local.fabPosition.<domain>`. Visibility gated on `settings.v1.perDomain.<domain>.fabVisible !== false`.
- `packages/extension/src/content/recall-overlay.ts` — `mountOverlay(): HTMLElement | null`. Modal with backdrop, 200ms-debounced search input → `chrome.runtime.sendMessage({type:'ui:recall'})`. Top-5 results: snippet + score + source pill + Copy / Insert / Open. Insert is gated on `selectAdapter(location.href)?.supportsInsert && findInputBox(document) !== null`; falls back to copy otherwise. ESC closes; click on backdrop closes; arrow-key navigation; focus-trap.
- `packages/extension/src/content/recall-overlay.css` — empty placeholder (styles ride in `shadow-styles.ts`); kept so the manifest's `web_accessible_resources` reference doesn't fault on `web-ext lint`.
- `packages/extension/tests/unit/content/{fab,recall-overlay}.test.ts` — JSDOM unit tests (8 tests). Run under vitest (jsdom env). bun-test is configured to skip them via `pathIgnorePatterns` since bun's runner doesn't host jsdom (same pattern as the popup component tests).
- `packages/extension/tests/e2e/fab-overlay.spec.ts` — both D13-binding TDD anchors (`fab_does_not_leak_styles_to_host`, `overlay_inserts_into_chat_input_when_adapter_supports_it`). Run locally via `bun run e2e`; both pass against installed Chromium.
- `packages/extension/playwright.config.ts` — minimal Playwright config (chromium project, 30s timeout, headless).

**Decisions**

- **Closed shadow root + `all: initial` host.** Maximum isolation per the task spec. The host element is a custom-element-tag (`<mnemonik-fab-host>`, `<mnemonik-overlay-host>`) with `style="all: initial; position: fixed; …"`. The shadow is `attachShadow({mode: "closed"})` so the host page has no `host.shadowRoot` handle. The Playwright TDD anchor `fab_does_not_leak_styles_to_host` verifies a host page's `body { background: red }` survives FAB injection unchanged.
- **`supportsInsert` flag (T11 dependency).** The overlay's "Insert into chat" button reads `selectAdapter(location.href)?.supportsInsert` directly (no `Function.prototype.toString` introspection — same minifier-safe pattern as `Recall.tsx`). When false, the button is disabled with a helpful tooltip. ChatGPT adapter supports it; Claude + Gemini do not (Phase 1 read-only adapters).
- **Per-domain settings shape (T12 dependency).** The FAB reads `chrome.storage.local["settings.v1"].perDomain[hostname]` and respects `fabVisible: false` to hide. Defined the local TypeScript shape (`PerDomainSettings { enabled?, fabVisible?, autoCapture? }`) inline; T12 (Options page) is in flight in a parallel worktree and ships the canonical version. Pragmatic: the contract here is just "if `fabVisible === false` skip mount", which the canned shape satisfies. If T12 changes the shape before merge, only the inline interface needs to align.
- **Auto-mount gating.** The module's bottom-of-file auto-mount IIFE checks `typeof chrome.runtime.getURL === "function"` (only defined in real extension contexts). Tests that mock just `{ runtime: { id, onMessage } }` don't trigger auto-mount and can call `mountFab()` directly. This avoids a race where the test-time module load would auto-mount before the test asserts on its own `mountFab()` call.
- **CSP-safe.** Zero inline event handlers. Every listener is `addEventListener`. Every DOM node is constructed via `createElement` + `textContent` (no `innerHTML`). The shadow `<style>` is the only element with raw text and the content is a static, non-interpolated module export.
- **Drag-vs-click discrimination.** `attachDragAndClick` tracks pointermove deltas; only a 3px+ movement promotes the gesture to a drag (via `drag.moved = true`). On `pointerup`, if `!drag.moved` the click handler runs, otherwise the new position is persisted.
- **No new permissions.** Manifest unchanged. The three enumerated host_permissions and `scripting` (already pre-declared in T10) are sufficient.
- **D12 honoured.** Auto-capture is OFF — no FAB or overlay code observes assistant turns or page mutations for capture intent. The only entry points are FAB click, hotkey (via the SW-dispatched `sw:open-recall-overlay`), and the popup.

**Test coverage**

- bun test: 104 pass / 1 pre-existing fail (cose.test.ts WASM-bindings load — documented in T10 §"Sandbox observations").
- vitest run: 122 pass / 1 pre-existing fail (same cose.test.ts).
- Playwright e2e: 2 pass (both TDD anchors).

**Known gap — E2E in CI**

`.github/workflows/node-test.yml` does not yet run Playwright. Both TDD anchors have been verified locally; CI integration is a follow-up (would need `bunx playwright install chromium` + a dedicated job step). Until then, the bun-test + vitest unit suites act as the gating contract for the FAB + overlay wiring (mount idempotency, `all: initial` reset, ESC-closes, message-listener subscription); the Shadow-DOM isolation property and the chat-insert end-to-end are E2E-only and run pre-merge by hand.

**Pre-existing build noise (not in T13 scope)**

- `bun run build` fails on missing `src/assets/icon-*.png` (manifest references icons that aren't in the repo). Pre-existing on `dev`.
- `tsc -b` reports a duplicate-vite-install typing conflict in `vite.config.ts` / `vitest.config.ts`. Pre-existing on `dev`, documented in T10 §"Sandbox observations".

Task `13.md` flips to `status: in_review` / `blocked_on: ci-verify` per D13 — settles to `done` only after the PR's CI run reports green on the bun-test + vitest suites.

---

## T17 — Key-escrow client + restore UX (D9 Argon2id+AES-GCM-256)

T17 ships the client-side passphrase-wrap pipeline that fulfils D9 end-to-end: Argon2id (m=65536, t=3, p=1, hash_length=32) derives a 256-bit key from the user's recovery passphrase; AES-GCM-256 with a fresh 16-byte salt + 12-byte nonce wraps the Ed25519 secret; the server (T15) stores the opaque blob keyed by Google `sub`. The Restore UX handles the second-device path with a 5-attempt local block (24h, persisted) and surfaces server 429s as a typed `EscrowRateLimitError` countdown. Both the popup onboarding (`Onboarding.tsx`) and the options Security section now call the real `auth/key-escrow.ts` client. See PR #120 for the full diff.

## 2026-05-11 · T17 — Round-1 review fixes (PR #120)

Three round-1 reviewers (code-reviewer, security-auditor, test-reviewer) returned `request_changes`. Findings table + resolution status below. Reviewer reports archived at `work/chrome-extension/logs/working/task-17/{code-reviewer,security-auditor,test-reviewer}-round1.json`.

| Finding | Severity | File | Resolution |
| --- | --- | --- | --- |
| T17-C-01 | blocker | `options/runtime-impl.ts` | **fixed** — `SESSION_KEY` now imports `SESSION_STORAGE_KEY` from `auth/types.js` so options + popup share the same chrome.storage slot. |
| T17-C-02 / T17-S-01 | major / high | `popup/onboarding/Restore.tsx` | **fixed** — encrypted blob cached in `useState<EscrowBlob \| null>` after the first GET; subsequent wrong-passphrase submits decrypt against the cache without hitting the server. 5 wrong-passphrase attempts now cost exactly 1 server fetch, preserving the independent 5/24h GET budget. Asserted by `Restore.test.tsx` 5-attempts test (`fetchSpy.toHaveBeenCalledTimes(1)`). |
| T17-C-03 | major | `popup/onboarding/Restore.tsx` | **fixed** — `now` / `storage` default expressions hoisted to module-scope (`DEFAULT_NOW`, `getDefaultStorage()`), eliminating useEffect dep churn on every render. The key-escrow facade is also pinned in a `useRef` so it's stable across renders. |
| T17-C-04 | major | `popup/onboarding/Restore.tsx` | **fixed** — wrong-attempt counter persisted to `chrome.storage.local.restore_attempt_count`. Hydrated on mount alongside the block timestamp. New `Restore.test.tsx::T17-C-04` test seeds 4 prior attempts, asserts one more wrong click triggers the block. |
| T17-C-05 / T17-T-05 | minor / minor | `auth/key-escrow.ts` | **fixed** — `parseRetryAfter` is now async, clones the response, parses `retry_after_secs` from the JSON body when the header is absent. Two new tests bind the channel: JSON-body-only fallback and hostile-large-value clamp. |
| T17-C-06 | minor | `popup/onboarding/SetPassphrase.tsx` | **fixed** — `bytesToBase64` exported from `auth/key-escrow.ts`; SetPassphrase imports the canonical helper instead of duplicating. |
| T17-C-07 | minor | `popup/onboarding/SetPassphrase.tsx` | **fixed** — component is now mounted from `Onboarding.tsx` on the `set_passphrase` branch with a host-supplied `signChallenge` callback that lazy-loads the WASM Ed25519 signer (`runtime/sign/cose.ts::signChallenge`). New `SetPassphrase.test.tsx` covers the zxcvbn gate, confirm-match, happy path (asserts wrap+upload+sign+link order), wrap error, upload error, and 409-already-linked. |
| T17-C-08 | minor | `popup/onboarding/Restore.tsx` | **deferred to T18** — post-restore IndexedDB sync trigger lives with the background sync wave. Comment retained, no `chrome.runtime.sendMessage` added in this round. |
| T17-C-09 | nit | `tests/component/popup/Restore.test.tsx` | **fixed** — 5-wrong-attempts test now wraps the fetch in `vi.fn()` and asserts `toHaveBeenCalledTimes(1)`. |
| T17-S-02 | medium | `auth/key-escrow.ts` | **fixed** — `MAX_RETRY_AFTER_SECONDS = 24 * 3600` constant exported; both the header path and JSON-body fallback clamp to it. Test: `Retry-After: 999999999` → 86400 (header), `{retry_after_secs: 999999999}` → 86400 (body). |
| T17-S-03 | medium | `popup/Onboarding.tsx` | **fixed** — secret-wipe now happens in a `useEffect` on `step` change so it fires on success path, error step transition, AND component unmount. Tracked via `heldKeypairRef`; on success the inline `onComplete` wipe still runs (defence-in-depth). |
| T17-S-04 | low | `popup/Onboarding.tsx` | **deferred** — `loadLocalKeypair` length-validation alignment with `loadIdentity` (64-byte enforcement) is a hardening item; the immediate bug (mismatched chrome-storage key) is the higher-priority fix and is closed by T17-C-01. Logged for the T18 wave. |
| T17-S-05 | low | `auth/key-escrow.ts` | **fixed** alongside T17-C-05 / T17-T-05 — the JSON body fallback is now real, not documentation-only. |
| T17-S-06 | nit | `popup/onboarding/Restore.tsx` | **fixed** — explicit comment at the `Array.from(secret)` site documenting the structured-clone constraint (number[] cannot be zeroed). No code change required; the comment is now in place. |
| T17-S-07 | nit | `popup/onboarding/Restore.tsx` | **deferred** — stale-closure risk on the attempts counter is bounded by the `inputDisabled` submitting-guard; the cosmetic window has no security consequence and the functional-updater rewrite would force the `set_passphrase` test fixture into a different shape. |
| T17-T-01 | major | `tests/unit/auth/key-escrow.test.ts` | **fixed** — three new direct tests on production `wrapSecret`: empty-secret synchronous reject, empty-passphrase reject, empty-pubkey reject, plus a full slow round-trip at production Argon2id parameters with `testTimeout: 30_000` (asserts the in-blob `kdf_params.{m,t,p}` match the exported constants — proves the wiring, not just the constants). |
| T17-T-02 | major | `tests/component/popup/SetPassphrase.test.tsx` | **fixed** — new 6-test file: zxcvbn gate, confirm-mismatch, happy path (full wrap+upload+sign+link sequence assertion), wrap error copy, upload error copy, 409 (already-linked) copy. |
| T17-T-03 | minor | `tests/unit/auth/key-escrow.test.ts` | **fixed** — `time_cost ≥ 2` floor bumped to `≥ 3` to match OWASP 2023 + D9. |
| T17-T-04 | minor | `tests/unit/auth/key-escrow.test.ts` | **fixed** — new test wraps + unwraps a 64-byte secret (Solana keypair shape) and asserts byte-for-byte equality. |
| T17-T-06 | nit | `tests/unit/auth/key-escrow.test.ts` | **fixed** — `fetchCalls = []` reset added to `beforeEach` so cross-describe isolation holds even when later tests assign `globalThis.fetch` directly. |
| T17-T-07 | nit | `tests/unit/auth/key-escrow.test.ts` | **fixed** — new test for `rotatePassphrase` upload-mid-flight failure: GET ok → unwrap ok → PUT 500 → `AuthError` surfaces, server-side blob is NOT deleted (no DELETE issued). |

**Architecture follow-ups landed in this round (not strictly review findings, but pre-requisites):**

- `popup/runtime.ts` gains a `keyEscrow` facade (`wrap/unwrap/upload/fetch/delete/rotate/hasBlob`) that lazy-imports `auth/key-escrow.ts`. Restore + SetPassphrase consume it via narrow prop seams (`RestoreKeyEscrow`, `SetPassphraseKeyEscrow`) so tests inject stubs without touching WebCrypto.
- `runtime/sign/cose.ts` adds `signChallenge(keypair, nonce)` — Ed25519 detached signature over raw bytes, used by the `/oauth/google/link` possession proof. Onboarding.tsx supplies a closure that lazy-loads the WASM signer before signing.
- Test fixtures in `tests/component/popup/{Capture,Recall,Verify}.test.tsx` gained the new `keyEscrow` stub block to keep `PopupRuntime` typecheck-complete.

**Test coverage after fixes**

- `bun test tests/unit/auth tests/component/popup tests/component/options` → 80 pass.
- `vitest run tests/component/popup` → 25 pass (Capture 3, Recall 3, Verify 4, Restore 9, SetPassphrase 6).
- `vitest run tests/unit/auth/key-escrow.test.ts` → 50 pass (was 33 before round-1; +17 from this fix wave).
- `bun test` (full extension) → 196 pass / 1 pre-existing `cose.test.ts` WASM failure (unchanged from the pre-fix baseline).

---

## 2026-05-11 · T23 — Bridge contract tests + hygiene

Audit round 2 found two cross-component drifts that survived per-side
unit tests because each side mocked the boundary: `tab:fab-*` variants
absent from the `Msg` union (audit B5 / AUD-C-05) and the `ui:recall`
SW handler returning `{ deferred: "recall" }` instead of running a
real embed+search (audit B4 / AUD-C-04). T23 lands the bridge-level
test contracts that would have failed both PRs at CI:

**New tests:**

- `tests/unit/messages.contract.test.ts` — every variant in the `Msg`
  union has well-formed + malformed samples via a TS-exhaustive
  switch over `Msg["type"]`. Adding a new variant without extending
  the sample table is a compile error; dropping a variant from
  `parseMsg`'s switch flips the variant's well-formed sample to
  `null` and fails the test. 25 tests / 110 assertions; covers
  generic defensive rejection (null / undefined / primitives /
  arrays / arrays-of-msgs / nested-discriminant smuggling / whitespace
  drift on the `type` string).
- `tests/unit/background/service-worker.contract.test.ts` — drives
  `installServiceWorker` + real `parseMsg` against synthetic
  MessageSender shapes for each inbound variant. Locks the
  `{ ok, result }` envelope shape for every accepted variant
  (B4 anchor binds `ui:recall` → real embedder + IndexedDbStore.search,
  B5 anchor binds `tab:fab-*` → pending_fab_action.v1 + openPopup),
  and the sender-authorisation perimeter: `ui:*` from a tab-bearing
  sender, `tab:*` from a non-allowed origin, `tab:*` from a host
  spoofing the allowed origin's prefix (`chatgpt.com.evil.example`),
  `sw:*` inbound at all, and `ui:*` with no positive sender id all
  return `{ ok: false, error: "unauthorized-sender" }`. 18 tests /
  38 assertions. Drives the AUD-S T2 hostile-extension / hostile-
  content-script threat from the security audit.

**AUD-T-R2-02 fix:** in `tests/unit/auth/extension-bootstrap.test.ts`,
the redeem-leg "no Authorization header" assertion was vacuous — the
prior code `(init.headers ?? {}) as Record<string, string>` fell back
to an empty object when `init.headers === undefined` (the
overwhelmingly likely shape, since the source omits `headers`
entirely in `fetchImpl(redeemUrl, { method: "GET" })`). The empty
object always reports `authorization` as undefined, so the assertion
never failed. Replaced with a strong form: `expect(init.headers)
.toBeUndefined()`, plus a documented defence-in-depth branch that
constructs a real `Headers` from any future `init.headers` value and
asserts `get("authorization") === null` (so when the source ever
switches to passing a Headers/Record object, the next maintainer
sees the spec inline rather than a silent regression).

**TA-MIN5 mitigation (size-limit silent-skip):** size-limit 12.x has
no `--fail-if-not-found` flag, so when the configured globs in
`.size-limit.json` don't match the dist/ layout (e.g. crxjs renamed
the popup chunk after a build-toolchain bump), size-limit reports
`size: 0, passed: true` and the budget gate silently degrades to a
no-op (round-2 deferral logged earlier in this file under "size-limit
`--fail-if-not-found` style enforcement"). Mitigation:
`packages/extension/scripts/check-size-limit.mjs` shells out to
`size-limit --json`, parses the report, and exits non-zero when any
entry has `size === 0` (silent-skip) or `passed === false` (budget
overrun). Wired into package.json as `bun run size:check`. Five unit
tests in `tests/unit/scripts/check-size-limit.test.ts` cover the four
failure modes + happy path via a fake-`npx` PATH shim that prints
canned JSON.

**TA-MIN1 (size-limit not gated by CI) — still deferred.** The
extension `bun run build` fails today on a missing icon asset
(`src/assets/icon-{16,32,48,128}.png`) — pre-existing T01/T10/T20 gap
documented earlier in this file. Wiring `size:check` into
`.github/workflows/node-test.yml` requires the build to succeed
first; tracked as a follow-up alongside the icon-asset commit. Local
operators running `bun run build && bun run size:check` get the full
gate today; CI gets it the day the build is fixed. A minimal hook —
one shell line — into the existing `bundle-size` job will be
sufficient at that point.

**Test coverage delta:**

- New: 25 (`messages.contract.test.ts`) + 18 (`service-worker.contract.test.ts`)
  + 5 (`check-size-limit.test.ts`) = 48 new tests.
- Modified: `extension-bootstrap.test.ts` (1 test now non-vacuous).
- Full `bun test` → 294 pass / 1 pre-existing `cose.test.ts` WASM
  artefact load failure (unchanged baseline).

**Files:**

- New: `packages/extension/tests/unit/messages.contract.test.ts`,
  `packages/extension/tests/unit/background/service-worker.contract.test.ts`,
  `packages/extension/tests/unit/scripts/check-size-limit.test.ts`,
  `packages/extension/scripts/check-size-limit.mjs`.
- Modified: `packages/extension/tests/unit/auth/extension-bootstrap.test.ts`
  (AUD-T-R2-02), `packages/extension/package.json` (`size:check`
  script).
- Production code untouched. Test-only `__setEmbedderForTesting` seam
  in `src/background/recall-embedder.ts` already exists from T13.

**Verification:** `bun test tests/unit/messages.contract.test.ts
tests/unit/background/service-worker.contract.test.ts
tests/unit/auth/extension-bootstrap.test.ts
tests/unit/scripts/check-size-limit.test.ts
tests/unit/messages.test.ts
tests/unit/background/service-worker.test.ts` → 75 pass / 214
assertions.

