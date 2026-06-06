# stateless-auth-rearch — parking lot

Status: **TODO — long-term architectural direction**.

Sibling to `work/refresh-token-rotation/` (the tactical fix that is shipping
first), `work/binary-mode-cleanup/` (the orthogonal mode model cleanup —
removes HTTP-local entirely), and
`work/chrome-extension-client-side-storage/` (the chrome-extension
migration to in-browser IndexedDB).

## Why this exists

During user-spec planning for the JWT-session-death bug on 2026-06-06,
user raised a deeper architectural point:

> "Even if save to db on server — user should sign, not server.
> So why MCP generates JWT? Does it represent MCP server, not client?"

Correct critique. Our current auth has two layers:

| Layer | Who signs | Owns identity? |
|-------|-----------|----------------|
| Memory bytes (COSE_Sign1) | User's Ed25519 keypair (browser or stdio) | Yes — protocol thesis honored |
| Session token (JWT) | Server's HMAC secret (HS256) | No — server vouches for user |

The JWT layer exists because MCP-hosts speak OAuth 2.1 Bearer (per MCP-spec
2025-06-18). The host doesn't sign per-request with the user's keypair —
it forwards a Bearer token the server minted.

This works (Stripe does the same), but it's two layers of identity where
one would suffice, and it inherits all of OAuth's failure modes: token
expiry, refresh rotation, revocation lists, replay windows.

## The alternative

**Per-request Ed25519 signing**, no JWT at all on the hosted HTTP API:

- Every `tools/call` carries a signature over the canonical request body.
- Server verifies with the user's pubkey (stateless math, no DB lookup).
- No session, no expiry, no refresh, no revocation — identity is the
  signature itself.

This is the same model as AWS SigV4, HTTP Message Signatures (RFC 9421),
Sign-In with Ethereum (SIWE), Bitcoin `signmessage`. And it's what our
stdio path already does: keypair signs locally, no JWT.

## Why we're not doing it now

Two reasons:

1. **MCP-hosts don't speak per-request signing.** Cursor, VS Code, Claude
   Desktop are OAuth 2.1 Bearer clients. To make them sign per-request,
   we'd need either (a) MCP-spec evolution (slow, political), or
   (b) a local stdio-proxy that holds the user's keypair and forwards
   signed requests upstream.

2. **Stripe-precedent shows refresh tokens are sufficient for the UX
   problem we hit.** Their model (1h access + 1y rolling refresh) works
   in the same MCP-hosts we serve. Refresh tokens are shipping first as
   `work/refresh-token-rotation/`.

## Sketch of what F0 would look like

### Option A — local stdio-proxy

User installs `mnemonik-mcp proxy --upstream mcp.mnemonik.xyz` via the
existing `mnemonik install` flow (agent-native-distribution v0.2.4).
The proxy:
- Runs as a stdio MCP server (MCP-host sees it as a normal stdio server).
- Holds the user's Ed25519 keypair locally.
- For each `tools/call`, signs canonical request body with the keypair.
- Forwards to upstream HTTPS endpoint with `X-Mnemonic-Signature` +
  `X-Mnemonic-Pubkey` headers (or similar).

Upstream server:
- Validates signature math, no DB lookup, fully stateless on auth.
- Routes tenancy via the validated pubkey.

Pros: stateless auth, structural bug-elimination, aligned with protocol
thesis. Pros for user: no OAuth-pages ever, even on first connect (keypair
either generated locally or imported).

Cons: requires local proxy installation (one more thing for users to do —
though `mnemonik install` is one command). Wire protocol on upstream must
change. Browser-mediated signing flow needs reconciliation (still needed
for webapp clients).

### Option B — MCP-spec change

Propose per-request signing as a transport-level MCP feature. Years of
work, political. Out of scope for any near-term plan.

## What to do next (when ready)

1. Run `/new-user-spec stateless-auth-rearch`.
2. Reference:
   - This README for rationale.
   - `work/refresh-token-rotation/decisions.md` for the journey that
     led here.
   - `docs/WHITEPAPER.md` for the cryptographic-identity thesis.
   - `mcp/src/oauth/mod.rs` (current OAuth surface to be deprecated).
3. Coordinate with chrome-extension owner — both Option A and B affect
   the browser-mediated signing flow.
4. Check whether MCP-spec discussion on per-request signing exists in
   the broader community (https://modelcontextprotocol.io).

## Out of scope of THIS feature (live elsewhere)

- `work/refresh-token-rotation/` — tactical fix shipping first.
- `work/binary-mode-cleanup/` — independent mode model cleanup (no more
  HTTP-local; only stdio-local + hosted-participate).
- `work/chrome-extension-client-side-storage/` — extension migration to
  in-browser IndexedDB (unblocks binary-mode-cleanup on the chrome
  surface).
- Stdio mode improvements — already great, no JWT path.
