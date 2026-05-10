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
