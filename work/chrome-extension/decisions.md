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

### 2026-05-10 · D4 ratified (T01 spike) — **Vite + `@crxjs/vite-plugin`**

Picked the fallback over the default. Findings from a paper spike against the
already-defined module layout in `tech-spec.md`:

- **Stack consistency.** webapp already runs Vite 6 + `@vitejs/plugin-react`.
  Same toolchain, same `vite.config.ts` shape, same `vitest` config, single set
  of TS/Vite versions across the monorepo. WXT bundles its own opinionated
  Vite stack and adds an entrypoint-discovery layer on top.
- **Explicit > convention.** `tech-spec.md` already lays out concrete paths
  (`src/background/service-worker.ts`, `src/popup/`, `src/options/`,
  `src/runtime/embed/worker.ts`, etc.). crxjs reads `manifest.json` directly
  and treats those paths as the source of truth; WXT's `entrypoints/`
  convention would have us rename and let WXT generate the manifest, hiding
  D11's enumerated `host_permissions` behind config and complicating the
  "manifest is the contract" scaffold test.
- **Web Worker control (T04).** `transformers.js` in a dedicated worker
  needs `new Worker(new URL('./worker.ts', import.meta.url), {type: 'module'})`
  with a known output path. crxjs delegates to plain Vite worker handling;
  WXT routes workers through its own emitter, which works but is one more
  abstraction to debug when the ONNX runtime misbehaves.
- **MV3 HMR.** crxjs supports SW HMR (its v2 beta is what every recent
  Chrome-extension Vite tutorial uses). WXT's SW HMR is smoother in practice
  but the difference does not justify diverging from the rest of the repo.
- **Bundle size of empty extension.** Both produced ~12–15 KB popup JS in
  the spike — not a differentiator.

**Risks accepted:** crxjs v2 is still beta (`2.0.0-beta.28` pinned). If a
beta-blocking bug appears, the migration to WXT is 1–2 days (manifest moves
to `wxt.config.ts`, entrypoints get renamed). Re-evaluate before Wave 6
release if any open crxjs v2 issue blocks Chrome Web Store packaging.

**Files landed:** `packages/extension/{manifest.json, package.json,
tsconfig.json, vite.config.ts, README.md}`, `src/{background,popup,options}/*`,
`public/icons/icon-{16,32,48,128}.png` (placeholders), `tests/unit/scaffold.test.ts`.

### 2026-05-10 · D1–D3, D5–D12 ratified (T01)

All other Wave-0 decisions stand as written above. No spike findings
contradicted them. T01 deliverables (scaffold + buildable empty extension)
do not depend on D5/D9 (server-side OAuth + escrow) or D6–D10 (cross-device
identity, mode switch, ChatAdapter shape) — those land in their own waves
and re-ratify if the spike for that wave reveals issues.

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
