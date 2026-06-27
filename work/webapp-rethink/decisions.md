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
`visibility = public` rows (reuse `SEARCH_SQL_CROSS_OWNER_VIS` at
`core/src/storage/sqlite.rs:116`). Never owner-private.

### Q3 — SEO depth
Decision: **real crawl coverage required** — not just head hoisting. Add
server-rendered / prerendered HTML for crawlers: build-time prerender for static
routes (`/`, `/ledger`, `/analytics`, `/blog`) plus **server-rendered
`/blog/:slug`** (dynamic, agent-published) emitting full HTML + meta + JSON-LD +
post content from the Rust server. Tracked as its own decision + wave below.

## 2026-06-27 — Verified codebase reality (recalled context was stale)

- **Payments:** the non-custodial paradigm landed (commit `a762d87`). `PAYMENT_MODE`
  is now ONLY `none` | `x402`; the custodial `balance`/`both` modes were removed
  (`mcp/src/payment.rs`, "Wave 4 — non-custodial; custodial balance/api-keys
  removed"). `check_payment` fail-closes on unknown modes. **x402 is the only
  billable rail**, and it gates the `participate` (Arweave + Solana on-chain) write
  path. Default is `none` (free). Bearer auth lives in
  `mcp/src/oauth/mod.rs` (`bearer_auth_middleware`), not `payment.rs`.
- **Storage:** SQLite is STILL the server-side attestation store
  (`core/src/storage/sqlite.rs`, `attestations.content TEXT NOT NULL`;
  `mcp/src/tools.rs` persists via `SqliteStore`). `ruvector.db` / `rag_chunks` are
  RAG chat seeding, not the memory store. The cross-owner public-only query
  constant is **`SEARCH_SQL_CROSS_OWNER_VIS`** (`sqlite.rs:116`) — there is no
  `SEARCH_SQL_PUBLIC` (earlier spec text corrected).

### Decision 9 — Blog publishing is a free `local` public write for V1
A blog post is a signed PUBLIC attestation (Decision 8). For V1 it is written as a
**`local` public** attestation: free, server-stored in SQLite, recallable, and
listed at `/blog`. It is NOT a `participate` on-chain write, so it does NOT incur
x402. Optional on-chain anchoring of a post (a `participate` write, x402-charged)
is deferred to a later iteration. This resolves the reality-checker's open
question on Task 9 (local vs participate). Auth on publish = the OAuth2/Ed25519
bearer path (`oauth/mod.rs`), independent of payment.

## 2026-06-27 — Validation round 1 (task-validator + reality-checker)

Ran both validators on-branch (single agents, not the failed parallel fan-out).
Reports: `logs/tasks/template-batch1-review.json`, `logs/tasks/reality-batch1-review.json`.

Fixes applied (this commit):
- **Critical:** `SEARCH_SQL_PUBLIC` → `SEARCH_SQL_CROSS_OWNER_VIS` (task 7, tech-spec
  Decision 6, decisions Q2) — hallucinated constant removed.
- **Re-waved backend** so every `depends_on` crosses a wave boundary:
  T7→w5, T8→w6, T9→w7, T10/T11→w8, T12→w6, T13→w9, T14→w10, T15→w11.
- **Task 9 context:** bearer auth is in `oauth/mod.rs`, not `payment.rs`.
- **Task 12:** routes are added by T5 (not T9); `<Seo>` from T1 (not T5).
- **Task 4:** `MARKDOWN_COMPONENTS` lives in `Roadmap.tsx`.
- **Tech-spec Decision 1:** CSP is `font-src 'self' data:`.

Remaining minors accepted (stylistic plain-text paths in some tasks); no stale
`balance`/`both` references remain. Note: the tech-spec "Implementation Tasks"
section keeps its coarse wave grouping; the authoritative execution waves are the
per-task frontmatter `wave:` values updated above.

## 2026-06-27 — Architecture: separate webapp from mcp server (user request)

User asked whether webapp can be deployed separately from the mcp server. Finding:
**it already is.** Webapp consumes the API via `VITE_MCP_BASE` (`webapp/src/lib/api.ts`),
mcp does not serve the webapp (no ServeDir in `main.rs`), and `cors_policy.rs` already
allows `https://mnemonik.xyz`. Data + publish plane is cleanly headless JSON.

Decisions recorded in tech-spec:
- **Decision 9 (new):** webapp = standalone static deploy; mcp = headless JSON+OAuth API
  on its own origin. All new routes are JSON only; mcp renders NO HTML. New read routes
  must be covered by the existing webapp-origin CORS allowance.
- **Decision 4 (revised):** crawlable `/blog/:slug` is now produced **webapp-side at build
  time** (prerender each post by fetching `GET /blog`), not via Rust SSR. Publish webhook
  (`BLOG_REBUILD_HOOK`, mcp-side, optional) triggers a webapp rebuild for freshness.
  Sitemap generated at webapp build. Supersedes the original Rust-SSR plan.
- **Task 13 rewritten** accordingly (webapp prerender + build-time sitemap + publish-hook
  ping); the mcp server only pings the deploy hook, no HTML.
