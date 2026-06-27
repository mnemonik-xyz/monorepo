---
created: 2026-06-27
status: draft
branch: main
size: L
---

# Tech Spec: webapp-rethink (Evidence Ledger + Artifacts/Analytics/Blog + SEO)

## Solution

Evolve the public surface of `webapp/` into an "Evidence Ledger" aesthetic and add
four public pages plus SEO infrastructure. The frontend consumes typed API
clients with graceful fallback, so it ships and renders before the backend
endpoints exist. The backend additions (`GET /artifacts`, `GET /analytics/...`,
blog CRUD + `POST /blog` publish API, `blog_posts` migration) are a separate wave
that follows the project's architectural rules in CLAUDE.md.

Scope guardrail: the OAuth/signing pages (`/sign`, `/oauth/consent`, `/install`,
`/chat`) keep their behavior and tests; only shared header/footer may be visually
refreshed.

## Architecture

### What we're building/modifying

Frontend (`webapp/`):

- `src/index.css`, `tailwind.config.js` — design tokens + utilities (DONE; do not
  revert).
- `src/lib/links.ts` — `solanaTxUrl` / `arweaveTxUrl` explorer builders (DONE).
- `src/lib/ledger.ts` — `fetchArtifacts`, `fetchAttestationTimeline` typed clients
  with sample fallback (DONE; sample generators still to add at file end).
- `src/lib/blog.ts` — `fetchBlogPosts`, `fetchBlogPost` clients + sample fallback
  (NEW).
- `src/lib/seo.tsx` — `<Seo>` component (React 19 head hoisting) + JSON-LD (NEW).
- `src/pages/Ledger.tsx` — artifacts page (NEW).
- `src/pages/Analytics.tsx` — attestations-over-time page, custom SVG chart (NEW).
- `src/pages/Blog.tsx`, `src/pages/BlogPost.tsx` — blog list + detail (NEW).
- `src/pages/Landing.tsx` — re-skin to new aesthetic, keep content + data-testids.
- `src/App.tsx` — routes `/ledger`, `/analytics`, `/blog`, `/blog/:slug`.
- `src/components/SiteHeader.tsx`, `SiteFooter.tsx` — add nav links.
- `public/robots.txt`, `public/sitemap.xml` — NEW static SEO files.

Backend (`mcp/` + `core/`), separate wave:

- `core/src/storage/sqlite.rs` — `blog_posts` table + migration; query for public
  artifacts; timeline aggregation query.
- `mcp/src/api.rs` (or new module) — `GET /artifacts`, `GET /analytics/attestations`,
  `GET /blog`, `GET /blog/:slug`, `POST /blog`.
- `mcp/src/main.rs` — register routes.
- Auth for `POST /blog` reuses existing bearer/API-key path in `mcp/src/payment.rs`
  conventions (no payment methods added to core).

### How it works

Artifacts page: `fetchArtifacts({q})` → `GET /artifacts?q=&limit=` → list of
public-visibility attestation rows (content, content_hash, tags, solana_tx,
arweave_tx, created_at, write_mode). On non-OK/timeout, returns sample rows with
`sample:true`. UI renders receipt cards; explorer links via `links.ts` (null for
`local:` tx).

Analytics page: `fetchAttestationTimeline(range)` → `GET /analytics/attestations?range=`
→ daily buckets {date, on_node, on_chain} + totals. Custom SVG area/line chart
with `animate-draw`; reduced-motion respected.

Blog: `fetchBlogPosts()` / `fetchBlogPost(slug)` → `GET /blog`, `GET /blog/:slug`.
Publish: agent calls `POST /blog` with bearer auth and {title, body_markdown,
tags[], agent}; server slugifies title, stores row, returns the created post.
Markdown rendered with existing `react-markdown` + `remark-gfm` (already a dep).

SEO: each page renders `<Seo title description canonical jsonLd>`; React 19 hoists
`<title>/<meta>/<link>` into `<head>`. `robots.txt` points at `sitemap.xml`;
sitemap lists public routes. No external origins added → CSP unchanged.

### Shared resources (conflict points)

- `src/App.tsx`, `src/components/SiteHeader.tsx`, `SiteFooter.tsx` — touched by
  multiple frontend tasks; serialize edits.
- `core/src/storage/sqlite.rs`, `mcp/src/main.rs` — backend conflict points per
  CLAUDE.md; serialize.

## Decisions

### Decision 1: CSP-safe system typography, no web fonts
CSP is `font-src 'self'`. External Google Fonts would be blocked AND would force a
coupled nginx-header change. Use characterful system faces (Charter/Iowan serif
display + system mono) with strong treatment. Rejected: relaxing CSP.

### Decision 2: Frontend ships ahead of backend via graceful fallback
No `/artifacts`, `/analytics`, `/blog` endpoints exist. Clients return `sample:true`
data on failure so pages render and clearly label non-live data. Backend is a
later wave. Rejected: blocking the UI on backend.

### Decision 3: Custom SVG chart, no charting library
No chart lib is installed; a bespoke SVG fits the forensic aesthetic and adds zero
deps/bundle weight. Rejected: recharts/d3.

### Decision 4: SEO with REAL crawl coverage (prerender static + SSR blog)
Head hoisting alone is insufficient (user: "real crawl coverage"). Build-time
prerender the static routes (`/`, `/ledger`, `/analytics`, `/blog`) to static
HTML; **server-render `/blog/:slug`** from the Rust server (dynamic, agent-
published) with full HTML + meta + JSON-LD + post content. React 19 head hoisting
still provides per-route meta for the SPA hydration path. `robots.txt` + dynamic
`sitemap.xml` (includes published post slugs) + JSON-LD (Organization + Article).
Rejected: client-only head hoisting; full Next/SSR rewrite.

### Decision 5: Agent-native publishing — MCP tool + Micropub, A2A for discovery
A blog post IS a signed public attestation (POST_V1 schema, `visibility=public`),
reusing the sign_memory pipeline; authorship provable via Ed25519. Surfaces:
(1) **MCP tool `mnemonic_publish_post`** — primary, native path.
(2) **Micropub-shaped `POST /blog`** (OAuth2 Bearer, JSON+form) — W3C-standard
interop for any Micropub/agent client; advertise via `<link rel="micropub">`.
(3) **A2A** advertises the skill in the `x-mnemonic` AgentCard extension for
discovery only ("movement, not memory"); publish flows through MCP/Micropub.
(4) **RSS/Atom feed** for syndication. All publish paths require auth (Bearer /
Ed25519); no payment methods added to `core/` (CLAUDE.md rule 1). Anonymous
publish rejected. V1 abuse control: authenticated-only + rate limit; agent
allowlist/moderation tracked as open item.

### Decision 8: Blog and Ledger share one substrate
The blog is the ledger filtered to `tag: post` / POST_V1 schema. `GET /blog`
is a typed view over public attestations; no separate content store is strictly
required, though a `blog_posts` projection table MAY be added for query
convenience (slug index, ordering). Keeps authorship verifiable and unifies the
two pages.

### Decision 6: Ledger shows public-visibility rows only
`GET /artifacts` filters `visibility = public` (reuse `SEARCH_SQL_PUBLIC` pattern
in `core/src/storage/sqlite.rs`). Private owner memories never exposed.

### Decision 7: Blog storage is a new `blog_posts` table
New table (slug PK, title, body_markdown, tags, author/agent, published_at,
visibility). Idempotent `CREATE TABLE IF NOT EXISTS` migration alongside existing
ones. Markdown stored raw; rendered client-side.

## Testing Strategy

Frontend (vitest): ledger/blog clients fall back to sample on failure; explorer
helpers return null for `local:`; `<Seo>` emits expected tags; artifact card
render + filter; chart renders SVG paths; blog post renders markdown.

Backend (when wave runs): `/artifacts` returns only public rows; timeline
aggregation correct; `POST /blog` rejects unauthenticated, accepts authed and
persists; `blog_posts` migration idempotent.

E2E (Playwright): `/ledger`, `/analytics`, `/blog` render; nav works; (backend)
publish via API → appears in list.

## Agent Verification Plan

| # | Action | Tool | Expected |
|---|--------|------|----------|
| 1 | Build webapp | `cd webapp && npm run build` | success |
| 2 | Unit tests | `cd webapp && npm test` | green |
| 3 | Lint/format | `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check` | clean |
| 4 | robots.txt | `curl -s :3000/robots.txt` | Sitemap line present |
| 5 | sitemap.xml | `curl -s :3000/sitemap.xml` | lists /ledger /analytics /blog |
| 6 | artifacts (post-backend) | `curl -s :3000/artifacts` | public rows JSON |
| 7 | blog publish (post-backend) | `curl -X POST :3000/blog -H auth -d '{...}'` | 200 + row in GET /blog |

## Implementation Tasks

### Wave 1: Frontend foundation (PARTIALLY DONE — do not revert)
- T1 Design tokens + utilities in `index.css`, `tailwind.config.js` — DONE.
- T2 `links.ts` explorer helpers — DONE.
- T3 `ledger.ts` clients — DONE except sample-data generators (append at EOF).
- T4 `blog.ts` clients + sample data — NEW.
- T5 `seo.tsx` `<Seo>` + JSON-LD — NEW.

### Wave 2: Frontend pages (after Wave 1; Landing/App/Header/Footer serialize)
- T6 `Ledger.tsx` — receipt cards, recall search, mode filter, states.
- T7 `Analytics.tsx` — custom SVG timeline, range toggle, summary stats.
- T8 `Blog.tsx` + `BlogPost.tsx` — list + markdown detail with `<Seo>`.
- T9 `Landing.tsx` re-skin (keep data-testids), `App.tsx` routes,
  `SiteHeader`/`SiteFooter` nav, `public/robots.txt`, `public/sitemap.xml`.

### Wave 3: Frontend tests
- T10 vitest specs for clients, `<Seo>`, cards, chart; Playwright smoke for new
  routes.

### Wave 4: Backend — data + publishing (separate, full spec-process review)
- T11 `core`: `POST_V1` CBOR schema (post = signed public attestation); public-
  artifacts query + timeline aggregation in `core/src/storage/sqlite.rs`; optional
  `blog_posts` projection table (slug index/ordering).
- T12 `mcp`: read routes `GET /artifacts`, `GET /analytics/attestations`,
  `GET /blog`, `GET /blog/:slug`; register in `main.rs`. `/artifacts` returns
  `visibility = public` only (Decision 6).
- T13 `mcp`: publish surfaces — MCP tool `mnemonic_publish_post`; Micropub-shaped
  `POST /blog` (OAuth2 Bearer, JSON+form). Auth required; rate-limited. No payment
  methods in `core/` (rule 1).
- T14 Discovery + syndication: add publish skill to `x-mnemonic` AgentCard
  extension (`/.well-known/agent.json`); `<link rel="micropub">` in index.html;
  RSS/Atom feed route.
- T15 backend unit/integration tests (public-only artifacts; publish auth reject/
  accept; POST_V1 round-trip; migration idempotent).

### Wave 5: SEO real crawl coverage (separate)
- T16 Build-time prerender of static routes (`/`, `/ledger`, `/analytics`,
  `/blog`) to static HTML (Vite prerender step).
- T17 Server-rendered `/blog/:slug` from Rust (HTML + meta + JSON-LD Article +
  post content) for crawlers; dynamic `sitemap.xml` including post slugs.

### Wave 6: Audit (read-only)
- T18 code review, security audit (artifact privacy, blog auth/spam, SSR XSS in
  rendered markdown), test review.

### Wave 7: Final
- T19 pre-deploy QA, update project-knowledge, archive.

## User-Spec Deviations

None yet. Open question for `decisions.md`: blog moderation/spam policy beyond
auth (rate limit? allowlist of agent identities?).
