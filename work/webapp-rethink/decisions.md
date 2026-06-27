# Decisions — webapp-rethink

Append-only log of decisions and audit findings.

## 2026-06-27 — Spec drafted

- Scope confirmed with user (via clarifying questions):
  - Redesign scope: **Landing + new pages**; leave OAuth/signing pages functional.
  - Data source: **frontend + graceful states** (no backend blocking).
  - Aesthetic: **evolve the ledger** (dark + mint + Solana-purple, forensic).
- Three later asks folded into this feature: SEO-friendliness, Analytics page
  (attestations over time), Blog with an AI-agent publish API.
- Frontend foundation already implemented and MUST NOT be reverted (per user):
  `webapp/src/index.css`, `webapp/tailwind.config.js`, `webapp/src/lib/links.ts`
  (explorer helpers), `webapp/src/lib/ledger.ts` (typed clients).

## 2026-06-27 — Open questions resolved by user

### Q1 — Agent-native blog publishing approach (researched)

Decision: **layered publishing native to this protocol**, where a blog post IS a
signed public attestation (reuse the sign_memory / CBOR / COSE_Sign1 / blake3
pipeline with a `POST_V1` schema + `visibility = public`). Authorship is provable
via the agent's Ed25519 identity; Blog and Ledger share one substrate (blog = the
ledger filtered to `tag: post`).

Surfaces, by role:
1. **Native publish — MCP tool `mnemonic_publish_post`.** Primary path; agents are
   already connected via MCP + OAuth2 + Ed25519. Signs a public POST attestation.
2. **Interop — Micropub-shaped `POST /blog`.** W3C Micropub is the established
   web standard for programmatic create/edit/delete of posts (OAuth2 Bearer,
   JSON + form). Lets any Micropub/IndieWeb/agent client publish with no bespoke
   glue. Advertise endpoint via `<link rel="micropub">` + agent card.
3. **A2A — discovery/identity only, NOT the publishing transport.** A2A is task
   delegation ("movement, not memory" per work/a2a-bridge). Advertise the publish
   capability as a skill in the existing `x-mnemonic` AgentCard extension; A2A
   agents discover it, then publish via the MCP tool or Micropub endpoint.
4. **Distribution — RSS/Atom feed** for the read/syndication side (+ SEO).
   ActivityPub / WebSub deferred.

Research basis: A2A v1.0 (Linux Foundation, 150+ orgs) is JSON-RPC/SSE task
delegation with AgentCard skills — no content model. Micropub is a W3C
Recommendation purpose-built for post publishing over OAuth2 Bearer. MCP (2026
spec: Tasks + OIDC-aligned auth) is this project's native transport. Sources
captured in the session research note.

Sub-decision (blog abuse): `POST /blog` and the MCP tool require auth (OAuth2
Bearer / Ed25519). Open: rate limit + allowlist of agent identities + moderation
— fold into backend wave; default to authenticated-only + rate limit for V1.

### Q2 — Artifacts privacy
Decision: **return public only.** `GET /artifacts` returns strictly
`visibility = public` rows (reuse `SEARCH_SQL_PUBLIC` pattern). Never owner-private.

### Q3 — SEO depth
Decision: **real crawl coverage required** — not just head hoisting. Add
server-rendered / prerendered HTML for crawlers: build-time prerender for static
routes (`/`, `/ledger`, `/analytics`, `/blog`) plus **server-rendered
`/blog/:slug`** (dynamic, agent-published) emitting full HTML + meta + JSON-LD +
post content from the Rust server. Tracked as its own decision + wave below.
