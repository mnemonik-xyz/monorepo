---
created: 2026-09-23
status: draft
type: feature
size: L
priority: P0
related:
  - work/presentable-mvp/user-spec.md
  - docs/USE_CASES.md
  - work/x402-v2-conformance/scope.md
---

# Plan: make Mnemonic presentable and testable

Goal: a stranger goes from zero to "my agent remembered something, shared
it, and someone else verified it" in under 5 minutes, without a wallet.

## Where we are (verified 2026-09-23)

| Area | State | Evidence |
|---|---|---|
| Keychain prompt on every session | Fixed on this branch | `core/src/identity/lazy.rs` |
| Local memory across Cursor / Claude Desktop / Claude Code | Works: one DB per machine | `mcp/src/config.rs:256` (`~/.mnemonic/attestations.db`) |
| On-chain write for a new user | Blocked: funded USDC wallet, no free tier | `mcp/src/config.rs:270`, `mcp/src/payment.rs` |
| 402 / 428 in SDK + webapp | Not rendered: generic error | `packages/sdk/src/client.ts:236`, `webapp/src/pages/Sign.tsx:268` |
| Rail mismatch | HTTP 500 "unexpected payment rail" | `mcp/src/api.rs:386` |
| Share one memory with someone | Does not exist | no `/m/<id>`, no share tool |
| Authenticated recall of others' public memories | Not possible | `mcp/src/mcp.rs:1768` |
| Third-party verify without account | Not possible: `mnemonic_verify` needs JWT + owner match | `mcp/src/oauth/mod.rs:2294`, `core/src/storage/sqlite.rs:993` |
| `mnemonic_publish_post` signer | Server key, not the author's | `mcp/src/publish.rs` |
| Smithery config | Stale: wrong URL (no `/mcp`), lists 5 tools | `smithery.yaml:40` |

## Design principle for sharing

**The signature is the login.** A shared memory is a COSE_Sign1 bundle signed
by the author's own key. Anyone can verify it offline. Uploading a signed
bundle needs no OAuth account: the server checks the signature, rate-limits
per public key, and stores it as public. On-chain anchoring is an optional
upgrade, not a precondition.

This also fits the keychain fix: the key is unlocked only when the user
shares or anchors, never for private local work.

## Phases

### P0 — "It works for a first-time tester" (1 week)

1. **Free anchoring quota.** First N participate writes per identity are
   operator-paid (quota counted per `owner_pubkey` in SQLite). Price shown in
   `mnemonic_whoami` as `free_anchors_left`. Payment only after the quota.
2. **Render the payment step.** SDK: typed `PaymentRequiredError` carrying
   `approval_url` + amount; accept `mode` in `signMemory`. Webapp `Sign.tsx`:
   show the link and price. Replace the HTTP 500 with a clear 402.
3. **Human error text.** Every payment/identity error carries a
   `next_step` sentence (e.g. "Open <url> to approve $0.001").
4. **Install in one line.** `npx @mnemonik-xyz/mcp install` without a `gh`
   dependency for verification (use the published checksum file instead).
   Fix `smithery.yaml` URL and tool list.
5. **Demo seed.** A public "welcome" memory set so `recall` on a fresh
   install returns something meaningful.

### P1 — "Share a memory" (2 weeks)

New tools (stdio + hosted):

| Tool | What it does |
|---|---|
| `mnemonic_share` | Sign a memory (by `attestation_id` or new `content`) with the user's key, upload to hosted `/share`, return `{url, share_code, content_hash}`. Optional `anchor: true` uses the free quota. |
| `mnemonic_import` | Fetch by URL or `share_code`, verify signature + hash locally, store as a local row tagged `received_from: did:sol:…`. Recallable like own memories, shown with provenance. |
| `mnemonic_verify` (extended) | Accept `url`, `share_code` or `content_hash`; works anonymously; returns author DID, signature validity, anchor status. |

New HTTP routes (no auth, rate-limited):

- `POST /share` — body: COSE_Sign1 bytes. Verify, store Public, return id.
- `GET /m/{hash}` — JSON bundle + verification result.
- Webapp `/m/:hash` — page: content, author DID, "Signature valid" badge
  verified **in the browser** (SDK WASM), anchor links, and a copy button
  "Import into your agent: `mnemonic_import <code>`".

Data model: new `received_from` column on `attestations` (nullable) and a
`shares` table `(content_hash PK, cose_bytes, author_pubkey, created_at,
anchor_tx NULL)`. No change to TurboQuant width or existing rows.

Fix alongside: `mnemonic_publish_post` signs with the author's key via the
same share path.

### P2 — "Shared spaces" (later, after P1 feedback)

Named spaces (`space: "team-alpha"`) with a member list of public keys;
`recall` gains `space`. Spec only until P1 usage tells us it is needed.
A2A (agent-to-agent) bridge stays in `work/a2a-bridge` as spec.

## Demo script (what we show)

1. Install (1 command) → agent calls `mnemonic_whoami`: no prompt.
2. "Remember that our API uses snake_case" → local memory, free, instant.
3. Switch to another tool (Cursor → Claude Desktop): recall works.
4. "Share that with Anna" → keychain prompt once → link `mnemonik.xyz/m/…`.
5. Anna opens the link: sees author + "Signature valid", no account.
6. Anna's agent: `mnemonic_import <code>` → her agent now recalls it,
   labelled "from did:sol:…".
7. Optional: "Anchor it" → free quota → Solana + Arweave links on the page.

## Testing

- Unit: share bundle round-trip, import rejects tampered bytes, quota
  decrement is atomic, anonymous verify never leaks private rows.
- Integration: stdio A shares → hosted → stdio B imports → B recalls.
- E2E (Playwright): `/m/:hash` shows valid badge; tampered bundle shows
  invalid.
- Security audit wave: `/share` abuse (size cap, per-key rate limit,
  content-type), privacy (share is explicit opt-in per memory).

## Open questions (owner decides)

1. Free quota size: one-time N, monthly, or both? (proposal: 100 one-time)
2. Is a shared memory public-by-link (unlisted) or listed in `/ledger`?
   (proposal: unlisted by default, `listed: true` to list)
3. Can a sharer delete/revoke? (proposal: hide from `/m` page; signature
   copies already imported stay valid — say so in UI)
4. Keep `mnemonic_publish_post` or fold it into `mnemonic_share`?
