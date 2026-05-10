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
