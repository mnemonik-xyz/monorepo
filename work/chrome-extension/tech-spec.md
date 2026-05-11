---
created: 2026-05-10
status: draft
size: L
branch: claude/chrome-extension-storage-modes-ukS8l
---

# Tech Spec: Chrome Extension `Mnemonik` (Phase 1)

## Solution

New npm package `packages/extension/` (`@mnemonik-xyz/extension`), Manifest V3, built with **WXT** (decision in T1, fallback Vite + `@crxjs/vite-plugin`). Pure ESM. Reuses `@mnemonik-xyz/sdk` (browser-bundle from T2) and `@mnemonic/core` WASM. Distributed as `.zip` via Chrome Web Store; dev `pnpm -C packages/extension dev` produces `dist/` for unpacked load.

The extension is a third public MCP-consumer (alongside CLI from `work/mnemonic-cli/` and webapp). Its **primary use case** is capturing AI-chat context (selections, messages, full conversations) from supported web AI-chat platforms — ChatGPT, Claude.ai, Gemini in Phase 1; Grok / Perplexity / Poe / OpenRouter in backlog. Secondary use case is generic page-selection capture on any site.

Two user-visible storage tiers:
- **Local** — fully self-contained in the browser (IndexedDB + WASM signing). Free, offline, no account.
- **Cloud** — same client-side pipeline, plus push to hosted MCP server (existing `STORAGE_MODE=full` deployment) for Arweave + Solana anchoring and cross-device sync. Requires Google sign-in. Paid (existing `PAYMENT_MODE` flow).

Server-side `StorageMode` enum in `mcp/src/config.rs` is **unchanged** (still `local | full`). "Cloud" is purely UX naming in the extension that maps to the existing hosted-`full` HTTP surface.

Identity model: one Ed25519 keypair per user across all clients. Cloud-tier signs the keypair's secret with a user passphrase (Argon2id+AES-GCM-256) and escrows the ciphertext on the server keyed by Google `sub`. New-device sign-in fetches and decrypts client-side. Server cannot decrypt.

## Architecture

### What we build / modify

**New (`packages/extension/`):**

- `manifest.json` — MV3. Permissions: `storage`, `identity`, `contextMenus`, `activeTab`, `clipboardWrite`, `alarms`. host_permissions enumerated: `https://chatgpt.com/*`, `https://claude.ai/*`, `https://gemini.google.com/*` (+ extension origin for OAuth callback). Avoids `<all_urls>` to keep Chrome Web Store review simple. Generic page-selection capture works via `activeTab` + user gesture.
- `src/background/service-worker.ts` — message router (popup ↔ content ↔ runtime), `chrome.commands` hotkey handler, `chrome.alarms` for cloud-sync retries, OAuth bootstrap.
- `src/popup/` — React + Tailwind, dark theme, reuses webapp tokens (`#0A0F1E` bg, `#00D4B4` accent, monospace for hashes). Tabs: Capture · Recall · Verify · Settings-link.
- `src/options/` — Settings, storage-mode toggle, identity management, security (rotate passphrase, export keypair), per-domain auto-capture toggles, telemetry opt-in.
- `src/content/` — Per-domain content scripts:
  - `chatgpt.adapter.ts`, `claude.adapter.ts`, `gemini.adapter.ts` — implement the `ChatAdapter` interface (see below).
  - `selection.ts` — generic selection capture (works on any page; injected via `activeTab`).
  - `fab.ts` — floating action button injected on supported domains.
  - `recall-overlay.ts` — modal opened by hotkey on any page.
- `src/runtime/` — domain logic, runtime-agnostic (no chrome.* APIs):
  - `store/indexeddb.ts` — `IndexedDbStore implements AttestationStore` (TS interface mirroring `core/src/storage/traits.rs::AttestationStore`). Schema: object stores `attestations` (keyed by `attestation_id`), `lineage_edges`, `pending_uploads` (queue for cloud sync). Indexes on `signer_pubkey`, `created_at`, `tags` (multi-entry). Versioned schema (`v1`).
  - `embed/transformers-embedder.ts` — `transformers.js` with `Xenova/all-MiniLM-L6-v2` (384 dim, ~25MB). Lazy-loaded in a Web Worker on first sign. Implements TS `Embedder` shape mirroring `core/src/embed/mod.rs::Embedder`. Vector dim 384 must match `mcp/src/config.rs` default — golden-fixture test enforces.
  - `compress/turboquant.ts` — calls `@mnemonic/core` WASM `compress_embedding(emb, bits=4)`.
  - `sign/cose.ts` — calls WASM `to_canonical_cbor`, `blake3_hash`, `sign_cose_payload`. Identical bytes to server (golden-fixture parity).
  - `sync/cloud-client.ts` — talks to existing MCP HTTP server using `MnemonicClient` from SDK. Cloud-mode sign uses deferred-signing flow (`POST /mcp` `mnemonic_sign_memory` returns `correlation_id` → `POST /api/sign-callback` with COSE bytes).
  - `chat/` — `ChatAdapter` interface (see below) + adapter registry + serializer (chat → `{messages: [{role, content, ts}], meta: {model, source, url, chat_id}}`).
- `src/auth/`:
  - `google-oauth.ts` — `chrome.identity.launchWebAuthFlow` against `GET /oauth/google/start`. PKCE S256. Returns server-issued JWT (existing `aud=mcp` shape, plus optional `google_sub` claim).
  - `bootstrap-ticket.ts` — wraps `/api/extension-bootstrap/{issue,redeem}` (server change).
  - `key-escrow.ts` — Argon2id KDF (via `hash-wasm`), AES-GCM-256 wrap/unwrap of Ed25519 secret using WebCrypto. Talks to `PUT/GET/DELETE /api/key-escrow`.
- `src/types/` — `ChatAdapter`, `ChatTurn`, `Memory`, `StorageTier`, etc.
- `tests/` — vitest unit + Playwright E2E. Adapter HAR fixtures in `tests/fixtures/`.

**`ChatAdapter` interface (the framework that makes new platforms easy to add):**

```ts
export interface ChatAdapter {
  readonly hostPattern: RegExp;            // matched against location.hostname
  readonly platform: string;               // 'chatgpt' | 'claude' | 'gemini' | ...
  /** Extract the current visible conversation as ordered turns. */
  extractConversation(doc: Document): ChatTurn[];
  /** Locate the chat input box for "insert into chat" recall. Phase 1 only ChatGPT must support; others can return null. */
  findInputBox(doc: Document): HTMLElement | null;
  /** Optional: return a stable per-chat id if the platform exposes one (URL slug, data attr). */
  getChatId(doc: Document, location: Location): string | null;
  /** Optional: detect when a new assistant turn finishes (for auto-capture mode). */
  onNewAssistantTurn?(doc: Document, cb: (turn: ChatTurn) => void): () => void;
}
export type ChatTurn = { role: 'user' | 'assistant' | 'system'; content: string; ts?: string; modelHint?: string };
```

Adapter registry is a flat array exported from `src/content/adapters.ts`. Service worker selects adapter by `hostPattern` match against the active tab URL.

**Modified (server, `mcp/`):**

- `mcp/src/oauth.rs` — add Google OAuth provider:
  - `GET /oauth/google/start?client_id=mnemonic-extension&redirect_uri=...&code_challenge=...&state=...` — redirects to Google consent.
  - `GET /oauth/google/callback?code=...&state=...` — exchanges code for Google tokens, validates `id_token` (issuer = `accounts.google.com`, audience = `GOOGLE_OAUTH_CLIENT_ID`, signature via cached Google JWKS), then redirects to `redirect_uri` with our own server-issued auth code (existing `oauth/token` flow).
  - On first link: requires extension to call `POST /oauth/google/link` with a possession-proof challenge signed by the new Ed25519 keypair, server inserts a row into `google_identity_links`.
  - On subsequent sign-in: `POST /oauth/google/lookup` returns `{existing_pubkey, escrow_present: bool}`.
- `mcp/src/api.rs` — new endpoints:
  - `POST /api/extension-bootstrap/issue` (auth: server JWT) → one-time ticket.
  - `GET /api/extension-bootstrap/redeem/:ticket` → JWT with `aud=extension`.
  - `PUT /api/key-escrow` (auth: server JWT bound to a `google_sub`) — body `{ciphertext_b64, nonce_b64, kdf, kdf_params, pubkey_base58}`. Replaces existing blob for that `google_sub`. Server validates `pubkey_base58` matches the linked pubkey.
  - `GET /api/key-escrow` (auth: server JWT) — returns blob + KDF params. Rate-limited: 5 fetches per 24h per `google_sub`. Counter persisted in `key_escrow_blobs` row.
  - `DELETE /api/key-escrow` (auth: server JWT) — deletes blob (user explicit revocation).
- `mcp/src/config.rs` — new env vars (all optional; if unset, Google login + escrow are disabled and server still boots):
  - `GOOGLE_OAUTH_CLIENT_ID`
  - `GOOGLE_OAUTH_CLIENT_SECRET`
  - `GOOGLE_OAUTH_REDIRECT_URI` (default `https://mcp.mnemonik.xyz/oauth/google/callback`)
  - `KEY_ESCROW_RATE_LIMIT` (default 5, per 24h, per `google_sub`)
- `mcp/src/main.rs` — wire Google OAuth router and key-escrow router conditionally (only if `GOOGLE_OAUTH_CLIENT_ID` set).
- `core/src/storage/sqlite.rs` — migrations:
  - `google_identity_links(google_sub TEXT PK, owner_pubkey TEXT NOT NULL, linked_at TEXT NOT NULL, last_seen TEXT)`. Index on `owner_pubkey`.
  - `key_escrow_blobs(google_sub TEXT PK, ciphertext BLOB NOT NULL, nonce BLOB NOT NULL, kdf TEXT NOT NULL, kdf_params TEXT NOT NULL, pubkey_base58 TEXT NOT NULL, updated_at TEXT NOT NULL, fetch_count_24h INTEGER NOT NULL DEFAULT 0, last_fetch_at TEXT)`. Server only ever stores opaque ciphertext — secret never sent in plaintext.
- `mcp/tests/oauth_google.rs` — mock Google JWKS server, full PKCE round-trip, link + lookup flows.
- `mcp/tests/key_escrow.rs` — PUT/GET/DELETE, rate-limit, pubkey-binding validation.

**Modified (build / repo):**

- Root `package.json` — add `packages/extension` to workspaces.
- `packages/sdk/package.json` — add `browser` export field for ESM browser bundle (already in T2 from `work/mnemonic-cli/backlog.md`).
- `core/src/wasm/mod.rs` — verify `compress_embedding`, `decompress_embedding`, `to_canonical_cbor`, `blake3_hash`, `sign_cose_payload`, `generate_keypair`, `import_keypair_json`, `export_keypair_json` are all `#[wasm_bindgen]`-exported. Add any missing (T5 task).
- `webapp/src/components/IdentityPanel.tsx` — add "Send to Extension" button mirroring "Send to CLI" pattern.
- `.github/workflows/node-test.yml` — add `packages/extension` to matrix (Node 20, Bun).
- New `.github/workflows/ext-e2e.yml` — Playwright E2E with extension loaded.

**Unchanged (consumed as-is):**

- `core/src/storage/traits.rs`, `mcp/src/tools.rs`, `mcp/src/payment.rs`, `mcp/src/pricing.rs`, all Arweave/Solana code, all existing OAuth (Solana wallet) flow.
- **No new `cloud` enum variant in Rust.** "Cloud" is extension-side UX naming for hosted-`full`.

## Data flow

### Local-mode `signMemory` (browser-only, zero network)

```
chat capture (content script) ──┐
selection capture ──────────────┴─→ ChatAdapter or selection.ts
                                       │
                                       ▼  {content, tags, source_meta}
                                runtime/embed.ts  (transformers.js, Web Worker)
                                       │  f32[384]
                                       ▼
                                runtime/compress.ts (WASM compress_embedding)
                                       │  bytes (TurboQuant 4-bit)
                                       ▼
                                runtime/sign.ts
                                       │  to_canonical_cbor → blake3 → sign_cose_payload
                                       ▼  COSE_Sign1 envelope
                                runtime/store/indexeddb.ts
                                       │  put attestation
                                       ▼
                                popup updated via chrome.runtime.sendMessage
```

Synthetic `local:<truncated_hash>` tx ids identical to server-`local`.

### Cloud-mode `signMemory`

Same client-side pipeline up through COSE_Sign1 envelope. Then `sync/cloud-client.ts`:

1. `POST /mcp` `mnemonic_sign_memory` (Bearer JWT) → server returns `{correlation_id, expires_in}` (existing deferred-signing flow).
2. `POST /api/sign-callback` with `{correlation_id, cose_signed_bytes (base64), signer_pubkey (base58)}`. Server verifies, persists, anchors to Arweave + Solana.
3. Response `{attestation_id, solana_tx, arweave_tx}`.
4. `IndexedDbStore.put` with the same row plus real `solana_tx` / `arweave_tx`.
5. Offline: enqueue in `pending_uploads` store; alarm-triggered retry.

### Recall

Always client-side cosine over IndexedDB embeddings. Cloud-tier additionally calls `mnemonic_recall` JSON-RPC and merges results, deduping by `attestation_id`.

### Restore on new device

```
Install extension (fresh) ─→ first-run popup
   │
   ▼ user clicks "Sign in with Google"
chrome.identity.launchWebAuthFlow → /oauth/google/start → Google consent → callback
   │
   ▼ server-issued JWT
POST /oauth/google/lookup → { existing_pubkey: "H8x...", escrow_present: true }
   │
   ▼ popup shows "Welcome back, identity H8x... detected"
user enters recovery passphrase
   │
   ▼ GET /api/key-escrow → { ciphertext, nonce, kdf, kdf_params, pubkey }
key-escrow.ts: derive key via Argon2id(passphrase, salt) → AES-GCM-256 unwrap
   │
   ▼ Ed25519 secret recovered
chrome.storage.local.set({ keypair }) — restored
   │
   ▼ background: sync mnemonic_recall (all) → IndexedDB cache
ready: cloud-history visible in popup
```

Wrong passphrase: client retries decryption locally (no server roundtrip until `GET /api/key-escrow` is called again). Server-side rate-limit prevents brute force on the ciphertext fetch (5 per 24h per `google_sub`). After 5 failed client-side decrypts, popup blocks input for 24h (UX-level; server enforces fetch limit independently).

## Decisions (seed for `decisions.md`)

1. Extension is fully self-contained for local mode (browser-only). Cloud mode = thin client to hosted MCP-`full`.
2. Server `StorageMode` is unchanged: still `local | full`. "Cloud" is purely UX naming in the extension.
3. Browser embedder = `transformers.js` + `Xenova/all-MiniLM-L6-v2` (lazy-loaded, 384 dim). Vector dim MUST match server default — golden-fixture test enforces.
4. Bundle: WXT (default) — fastest MV3 dev loop. Fallback Vite + crxjs. Final pick in T1.
5. Google OAuth via `chrome.identity.launchWebAuthFlow` + PKCE S256. No Google client_secret in extension; server holds it. Server validates Google id_token + binds Google `sub` to Ed25519 pubkey via possession-proof.
6. Identity: 1 Ed25519 keypair per user across CLI / webapp / extension. Cross-device = restore-on-google-signin OR existing bootstrap-ticket.
7. Mode switching is explicit: local → cloud uploads existing attestations one-by-one (best-effort, resumable queue); cloud → local exports + disconnects. No silent dual-write.
8. **Primary feature is AI-chat capture, not generic page capture.** Generic capture is a free fallback. Phase 1 ships adapters for ChatGPT, Claude.ai, Gemini.
9. **Key escrow / restore via Google login** — server stores Ed25519 secret as a passphrase-encrypted blob keyed by Google `sub`. Wrap = AES-GCM-256 with key derived via Argon2id (`memory_cost=64MiB, time_cost=3, parallelism=1`, salt = 16 random bytes stored alongside ciphertext). Server only ever sees opaque ciphertext + KDF params + pubkey. Server cannot decrypt — passphrase never leaves client. Lost passphrase = lost restore (user falls back to manual keypair import). Rate limit: 5 GET fetches / 24h / `google_sub`.
10. `ChatAdapter` interface keeps platform support extensible. Phase 1 ships 3 adapters (ChatGPT, Claude.ai, Gemini); contributions land via PRs adding new adapter modules + HAR fixtures + entry in registry.
11. host_permissions enumerated explicitly per supported AI-chat domain (no `<all_urls>`) to ease Chrome Web Store review. Generic page-selection capture relies on `activeTab` + user gesture (popup hotkey or context-menu).
12. Auto-capture is opt-in per domain (off by default). Privacy-first.

## Testing

### Unit (vitest, in `packages/extension/tests/unit/`)

- `store/indexeddb.test.ts` — CRUD, search-by-tags, lineage edges, schema migration.
- `embed/transformers.test.ts` — deterministic seed → expected vector (slack 1e-4).
- `compress/turboquant.test.ts` — round-trip via WASM, compare against `core/` golden bytes.
- `sign/cose.test.ts` — same plaintext + same keypair → byte-identical to server `to_canonical_cbor` + `sign_cose_payload`. Reuses `--features golden-fixtures` from `core/`.
- `auth/key-escrow.test.ts` — wrap then unwrap → original bytes; wrong passphrase → fail; tampered ciphertext → fail.
- `chat/chatgpt.adapter.test.ts`, `claude.adapter.test.ts`, `gemini.adapter.test.ts` — parse fixed HTML snapshot → expected `ChatTurn[]`.

### Component (Playwright + Vitest browser mode)

- Popup: capture, recall, verify, settings panel.
- Onboarding: Local path; Cloud path with mocked Google + server.
- Restore: fresh storage → mocked Google → passphrase prompt → identity restored.

### E2E (Playwright with `--load-extension=dist/`)

- `e2e/chatgpt-capture.spec.ts` — load HAR fixture of ChatGPT page → click "Save chat" → assert IndexedDB row.
- `e2e/claude-capture.spec.ts` — same for Claude.ai.
- `e2e/gemini-capture.spec.ts` — same for Gemini.
- `e2e/recall-overlay.spec.ts` — open hotkey overlay on arbitrary page → search → copy.
- `e2e/mode-switch.spec.ts` — Local → Cloud migration with mocked server.
- `e2e/restore-on-second-profile.spec.ts` — install in second Chrome profile → Google → passphrase → assert keypair restored + memories synced.

### Server (Rust)

- `mcp/tests/oauth_google.rs` — mock Google JWKS, PKCE round-trip, link + lookup, possession-proof validation, signature-fail rejection.
- `mcp/tests/key_escrow.rs` — PUT then GET round-trip, rate-limit (6th fetch within 24h fails), pubkey-binding mismatch rejection, DELETE, replay nonce.

### CI

- Existing `ci.yml` unchanged (Rust workspace).
- Extend `node-test.yml`: matrix step for `packages/extension` (Node 20, Bun) running unit + component.
- New `ext-e2e.yml`: Playwright with extension loaded, runs on PRs touching `packages/extension/**` or `packages/sdk/**`.
- Bundle-size budget enforcement: fails CI if popup initial JS >50KB or total package >2MB (via `size-limit`).

## Tasks (waves)

Each task file in `tasks/` follows the existing pattern (`work/mnemonic-cli/tasks/3.md`): frontmatter (`status`, `depends_on`, `wave`, `skills`, `verify`, `reviewers`), description, what-to-do, TDD anchor, files (create/modify/read).

- **Wave 1 (foundation, parallel):** T01 (scaffolding + WXT decision), T02 (SDK browser bundle, unblocks `work/mnemonic-cli` Phase 1.5).
- **Wave 2 (storage + crypto pipeline, parallel after W1):** T03 (IndexedDB store), T04 (transformers.js embedder in worker), T05 (WASM bindings parity tests).
- **Wave 3 (chat capture — primary feature):** T06 (ChatAdapter framework + registry), T07 (ChatGPT adapter + HAR fixture), T08 (Claude.ai adapter), T09 (Gemini adapter).
- **Wave 4 (extension shell, parallel after W3):** T10 (MV3 manifest + service worker), T11 (popup UI), T12 (options page), T13 (FAB + recall overlay + hotkeys).
- **Wave 5 (auth + cloud + escrow, sequential):** T14 (server: Google OAuth provider) → T15 (server: extension-bootstrap + key-escrow endpoints + DB migrations) → T16 (extension: Google sign-in client) → T17 (extension: key-escrow client + restore UX) → T18 (extension: cloud-tier sync via deferred signing).
- **Wave 6 (release):** T19 (E2E suite + recorded HAR fixtures), T20 (Chrome Web Store packaging + privacy policy + listing assets).

## Critical files (read-only context for implementer)

- `core/src/storage/traits.rs` — `AttestationStore` trait shape to mirror in TS.
- `core/src/storage/sqlite.rs:332+` — only existing impl; reference for IndexedDB schema.
- `core/src/wasm/mod.rs` — verify exports cover signing pipeline + escrow primitives.
- `mcp/src/tools.rs:380, 467` — storage-mode dispatch; unchanged but referenced.
- `mcp/src/oauth.rs` — extend with Google provider.
- `mcp/src/api.rs` — extend bootstrap-ticket flow + add escrow endpoints.
- `mcp/src/config.rs:95` — env-driven config; add Google + escrow vars.
- `webapp/src/components/IdentityPanel.tsx` — pattern for "Send to Extension" button.
- `work/mnemonic-cli/user-spec.md`, `work/mnemonic-cli/tech-spec.md`, `work/mnemonic-cli/tasks/3.md` — format templates.
- `work/mnemonic-cli/backlog.md` — Phase 1.5 SDK browser bundle (T02 fulfills).

## Verification

1. `cargo test --workspace --no-fail-fast` green; new `oauth_google.rs`, `key_escrow.rs` included.
2. `pnpm -C packages/extension test` green (vitest unit + component).
3. `pnpm -C packages/extension build` produces `dist/` with valid MV3 manifest; `web-ext lint dist/` clean.
4. `pnpm -C packages/extension e2e` Playwright suite green (extension loaded, HAR fixtures replayed).
5. Bundle size budget: popup ≤50KB initial, total ≤2MB (excl. lazy embedder model).
6. Manual flow A (Local): install unpacked → choose Local → on `chatgpt.com` save chat → recall finds it → verify returns verified.
7. Manual flow B (Cloud + restore): install → choose Cloud → Google sign-in → set passphrase → save chat on `claude.ai` → in webapp under same identity see attestation. Then second Chrome profile: install → Google sign-in (same account) → enter passphrase → identity restored, attestation visible. Wrong-passphrase 5 attempts → 6th blocked 24h.
8. Server boot with `STORAGE_MODE=full` + `GOOGLE_OAUTH_CLIENT_ID` set: `GET /oauth/google/start` redirects; mock id_token tests in suite green.
9. Privacy policy reviewed and linked from manifest + Chrome Web Store listing.
