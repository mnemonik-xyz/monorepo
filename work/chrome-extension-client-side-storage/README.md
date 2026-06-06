# chrome-extension-client-side-storage — parking lot

Status: **TODO — brief, then run user-spec-planning**.

Siblings:
- [`work/binary-mode-cleanup/`](../binary-mode-cleanup/) — needs this
  shipped (or in-flight) before `mcp.mnemonik.xyz` can retire
  `STORAGE_MODE=local`. Tightly coupled in rollout, not in code.
- [`work/refresh-token-rotation/`](../refresh-token-rotation/) — independent.
- [`work/chrome-extension/`](../chrome-extension/) — the current
  chrome-extension feature; this stub is the **next** evolution of it.

## The problem

Chrome extension today (per `patterns.md::Storage modes`) targets
`STORAGE_MODE=local + PAYMENT_MODE=none` on a hosted MCP backend. That
means:
- User installs the extension.
- Extension calls hosted MCP (`api.mnemonik.xyz` or similar
  extension-backend deploy) over HTTP.
- Hosted MCP writes to operator's SQLite with synthetic `local:` tx IDs.
- "Free" only because the operator chose `PAYMENT_MODE=none` on that
  deploy.

Under the binary-mode model (see `work/binary-mode-cleanup/`):
- `local` should mean truly client-side — for an extension, that's
  the browser itself (IndexedDB).
- "Free hosted server SQLite" tier is retired.
- The extension must therefore stop relying on a hosted backend for
  local writes.

## Target model

Chrome extension runs **fully client-side** for local writes:

| Component | Where | What |
|-----------|-------|------|
| Identity | `localStorage["mnemonic.identity"]` | Ed25519 keypair (today's pattern, see webapp `Sign.tsx`) |
| Embeddings | WASM-compiled `core/` in extension's service-worker / content-script | `fastembed` or similar — local model already in webapp build |
| Storage | IndexedDB via `wa-sqlite` or `sql.js` | full text + uncompressed f32 embedding + metadata |
| COSE signing | WASM `sign_cose_payload` export | user's keypair, never leaves browser |

For `participate` writes (paid + anchored), extension continues to call
external MCP (`mcp.mnemonik.xyz` or operator's choice) — same path as
the webapp uses today (`correlation_id` → webapp consent → COSE sign
in browser → `/api/sign-callback`). Could be integrated directly into
extension's UI (no separate webapp tab) — TBD design decision.

## Affected surfaces

### Extension code (in `work/chrome-extension/` repo or wherever lives)

- Service worker / background script: replace HTTP calls to hosted MCP
  with WASM-core invocations.
- IndexedDB schema migration: extension needs to ship a migration from
  "no local storage" → IndexedDB-backed.
- UI: settings panel for "free local (in this browser) vs participate
  (paid, anchored)". Currently invisible because everything goes to
  hosted; needs to be visible because user now has a real choice.
- Onboarding: clarify that local data lives in this browser only.
  Add export/import flow (user might want to move between browsers).

### Existing user data on hosted backend

- Extension users today have rows in operator's SQLite. Migration
  options:
  - **(a) Voluntary export-import**: webapp page that lets user export
    their hosted-local rows and re-import into extension's IndexedDB.
  - **(b) Auto-sync on first open**: extension fetches user's hosted-local
    history via authenticated recall, stores locally. Hosted rows
    eventually deleted per retention.
  - **(c) Leave behind**: hosted rows stay readable via webapp; new
    writes go local-only.

### Operator costs

Operator running the hosted backend for the extension today bears
storage cost. Once extension migrates, this load drops to zero for
local writes. Participate writes continue (and are paid).

## Why this exists — context from 2026-06-06

User crystallized the model:
> "About browser extension — not free saving on external server. Local
> means saving in local storage for example. Onchain — pay and use
> external MCP."

This is the structural fix for chrome-extension target. Once shipped:
- `binary-mode-cleanup/` can retire `STORAGE_MODE=local` on
  `mcp.mnemonik.xyz` without breaking the extension.
- Extension stops being the "anti-pattern preservation" reason for
  HTTP-local mode.
- Whitepaper §5.7.1 invariant holds for extension users by
  construction (local = browser, free, no server involvement).

## What to do next

1. When ready: run `/new-user-spec chrome-extension-client-side-storage`.
2. Reference this README, `work/binary-mode-cleanup/README.md`,
   `work/chrome-extension/` (existing feature for context).
3. Coordinate rollout with `work/binary-mode-cleanup/` — both shipping
   close in time, extension first if possible (otherwise
   `mcp.mnemonik.xyz` retiring HTTP-local breaks the extension).
4. Code-research focus:
   - `webapp/src/wasm/` — pattern for using WASM-core in browser.
   - `webapp/src/Sign.tsx` — pattern for browser-mediated COSE signing.
   - `core/` WASM build configuration (`#[cfg(target_arch = "wasm32")]`
     gates).
   - IndexedDB Rust SDK options: `wa-sqlite`, `sql.js`, `idb-keyval`.
   - Chrome extension manifest v3 service-worker constraints (long-lived
     WASM might need offscreen documents).

## Out of scope

- The binary-mode-cleanup itself — separate feature.
- Refresh-token rotation — separate feature.
- Free trial / onboarding tweaks for hosted MCP — separate feature.
- Mobile / non-Chrome extensions — separate features when there's appetite.
