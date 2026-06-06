# binary-mode-cleanup — parking lot

Status: **TODO — brief, then run user-spec-planning**.

Renamed from `hosted-mode-rename-and-pricing/` on 2026-06-06 after the
binary-mode pivot (see History below).

Siblings:
- [`work/refresh-token-rotation/`](../refresh-token-rotation/) — ships
  first; fixes JWT mid-session expiry.
- [`work/chrome-extension-client-side-storage/`](../chrome-extension-client-side-storage/)
  — chrome extension migration to in-browser IndexedDB storage. Needed
  to unblock the chrome-extension target before `STORAGE_MODE=local`
  on a hosted deploy can be retired.
- [`work/stateless-auth-rearch/`](../stateless-auth-rearch/) — long-term
  direction; per-request signing.

## The model

User-facing mode model becomes **binary**:

| Mode | Where it lives | Cost | Anchored? | Who signs |
|------|----------------|------|-----------|-----------|
| `local` | client-side: stdio→SQLite file, browser-extension→IndexedDB | free | no | user's keypair (locally) |
| `participate` | server SQLite (full text for recall) + Arweave (compressed embedding) + Solana (timestamp anchor) | x402/balance via external MCP | yes | user's keypair (browser-mediated via webapp) |

**No third "free hosted server SQLite without anchor" tier exists**. Today
it does — via `STORAGE_MODE=local + PAYMENT_MODE=none` on operator
deploys (chrome-extension's production target, see `patterns.md::Storage
modes`). That config becomes a deploy-anti-pattern and is retired.

Whitepaper §5.7.1 invariant "Личная память бесплатна всегда" holds
structurally:
- Free = client-side. Either user owns the machine (stdio) or the browser
  IndexedDB (extension).
- Paid = external MCP + anchor. User pays operator for storage + chain
  fees + service.

## Scope

### Server side — remove `local` from HTTP surface

- `tools::resolve_write_mode` (`mcp/src/tools.rs:101-135`): on HTTP
  transport (jwt_sub.is_some() OR any HTTP request), `mode:"local"`
  returns `-32010 UnsupportedMode` with hint: "this server does not
  serve local mode; install mnemonic-mcp locally for free, or use
  mode:'participate' to anchor on-chain (paid)."
- Stdio path unchanged: `mode:"local"` works as today.
- `tools::resolve_visibility` AC14 stops being a paradox (local mode
  no longer exists on HTTP, so the "visibility=public+mode=local"
  combo can't arise from HTTP requests).

### Envelope and discovery

- `mnemonic_whoami` envelope on HTTP deploys advertises
  `supported_modes: ["participate"]` only.
- Discovery metadata unchanged (OAuth-side, not mode-side).

### Pricing

- Hosted MCP charges for `participate` writes — already implemented
  via `mcp/src/payment.rs` x402/balance gate.
- Chrome-extension's hosted deploy continues to operate `PAYMENT_MODE=none`
  ONLY DURING the transition window. Once extension migrates to in-browser
  storage (sibling feature), the hosted backend for the extension can be
  retired or repurposed for participate-only.

### Migration of existing rows

- Existing `mcp.mnemonik.xyz` rows with `write_mode='local'` (synthetic
  `local:` tx IDs): pick one strategy:
  - **(a) Read-only legacy**: keep them in SQLite; recall returns them
    normally; writes are no longer accepted. Banner in webapp explaining.
  - **(b) Voluntary promote**: surface a tool / webapp action
    "promote to participate" — user signs the existing content with their
    keypair, server anchors, replaces row. NB: blake3-hash changes
    because signer changes, so `attestation_id` shifts; track via
    `legacy_id` link column.
  - **(c) Retention sweep**: announce N-month retention, then drop.
- Recommend (a) — lowest cost, preserves recall continuity.

### Agent skill updates

- Update `.claude/skills/` agent guidance: when asked to "save locally
  for free", suggest `mnemonik install` (stdio path). When connected
  to a hosted MCP, only `participate` is offered.

## Why this exists — context from 2026-06-06

During planning of `session-reauth-recovery` (now
`refresh-token-rotation`), user asked:
> "Why do we need auth for local mode at all? Aren't we saving locally
> on client side?"

Investigation revealed `mode:"local"` on HTTP means **operator's SQLite,
scoped to your OAuth sub**, billed-or-not depending on `PAYMENT_MODE`.
Server-signed COSE_Sign1 (not user-signed). Not truly local. Not
obviously free.

User then proposed (and confirmed): drop the middle tier entirely. Two
options: install MCP locally (truly local, your machine), or pay for
external MCP with on-chain anchor.

Cleanly removes:
1. "Who signs HTTP-hosted local" architectural question (no HTTP-hosted
   local exists).
2. "Local upgrade to participate" UX question (only local→participate
   is across-transport, handled by user re-saving via the new path).
3. Naming confusion (`local` on HTTP meant something different from
   `local` on stdio).
4. Whitepaper invariant tension (free is now strictly client-side).

## History

- Originally opened as `work/hosted-mode-rename-and-pricing/` 2026-06-06
  with scope "rename HTTP-local → hosted + charge for it". User pivoted
  to "drop HTTP-local entirely" later same day.
- Renamed to `work/binary-mode-cleanup/` to reflect the new model.

## What to do next

1. When ready: run `/new-user-spec binary-mode-cleanup`.
2. Reference this README and
   `work/refresh-token-rotation/decisions.md` for rationale.
3. Coordinate with
   `work/chrome-extension-client-side-storage/` —
   binary-mode-cleanup can't ship on `mcp.mnemonik.xyz` until extension
   migrates (otherwise extension breaks).
4. Code-research focus areas:
   - `mcp/src/tools.rs::resolve_write_mode` (mode parsing, gating).
   - `mcp/src/tools.rs::sign_memory` (routing — explicit-local branch
     gets a guard).
   - `mcp/src/mcp.rs::handle_request_with_resolved_mode` (paywall gate).
   - `core/src/storage/sqlite.rs::write_mode` column (migration of
     existing rows).
   - `mcp/src/tools.rs::resolve_envelope` or whoami builder
     (`supported_modes` advertise).

## Out of scope of THIS feature

- Refresh-token rotation — see `work/refresh-token-rotation/`. Independent.
- Chrome extension client-side migration — see
  `work/chrome-extension-client-side-storage/`. Sibling, but separate
  surface (in-browser code, not server-side).
- Stateless per-request signing — see `work/stateless-auth-rearch/`.
  Different transport-level concern.
- Free trial / x402-credits for new hosted users — separate concern;
  raised as part of migration discussion but mechanically independent.
