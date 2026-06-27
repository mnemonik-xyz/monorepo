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

## 2026-06-27 — Task 1 done (frontend data + SEO libs)

Teammate t1-libs. The lib implementations (ledger.ts, blog.ts, seo.tsx, links.ts) were
already complete from prior uncommitted work; verified against all acceptance criteria,
no changes needed. Added the 3 TDD-anchor test files the task mandates:
- ledger.test.ts (13), blog.test.ts (5), seo.test.tsx (4) — 22 tests, all green.
- <Seo> JSON-LD asserted inline same-origin (CSP-safe); fetchers degrade to sample:true.
Verify: `npm run build` success; `npm test` 49/50 — the sole failure is a pre-existing
`Sign.test.tsx` waitFor timeout (baseline, outside scope; QA wave to confirm not a regression).
Committed 6e9429f. For pages: every fetcher returns `sample` flag (must surface non-live
indicator); pages must use MCP_BASE (no hardcoded origins); tx helpers return null for local:.

## 2026-06-27 — Task 7 done (core schema + queries)

Teammate t7-core. POST_V1 schema (codec::schema, body in standard `content` slot so
content_hash commits to markdown); blog_posts table (idempotent CREATE IF NOT EXISTS,
slug PK); list_public_artifacts(limit), attestation_timeline(since), upsert/list/get
blog_post on SqliteStore. Reused SEARCH_SQL_CROSS_OWNER_VIS public predicate (no
SEARCH_SQL_PUBLIC). Exports via mnemonic_core::codec::schema::POST_V1 and
mnemonic_core::storage::{PublicArtifact, TimelineBucket, BlogPost}. No payment in core,
no HashEmbedder, all SQL parameterized. Verify: build/clippy/fmt clean, 162 tests pass
(9 new green). Committed. T8/T9 call signatures recorded in the teammate report.

## 2026-06-27 — Task 2 done (Ledger page)

Teammate t2-ledger. webapp/src/pages/Ledger.tsx + Ledger.test.tsx (additive). Receipt
cards (content, blake3 hash w/ copy, tags, write_mode badge, Solana/Arweave anchors via
links.ts — local:/null render as plain text), recall-by-meaning search, write_mode filter
chips, explicit loading/empty/error states + "Sample · not live" banner (Decision 2). A11y
+ reduced-motion handled. No hardcoded origins (MCP_BASE only). Verify: Ledger.test.tsx 9/9
green; suite green except known pre-existing Sign.test.tsx. Committed 8dc102e. T5: add
/ledger route + nav; page is a prerender candidate (canonical /ledger set).

## 2026-06-27 — OPEN SECURITY ITEM (from T3 security review): seo.tsx JSON-LD escaping

seo.tsx injects JSON-LD via dangerouslySetInnerHTML={{__html: JSON.stringify(block)}}
WITHOUT escaping `</script>`. Safe for static callers (Landing/Ledger/Analytics) but a
STORED-XSS breakout once Blog renders real agent-published posts via articleJsonLd(post)
(attacker-controllable title/author/body). MUST harden before the blog serves live posts
(before T13 prerender / T9 publish go live): escape `<` to `<` (and ideally `>`/`&`)
in the JSON-LD string, with a regression test. Tracked as ad-hoc fix in Wave 2 closeout.

## 2026-06-27 — Tasks 3 & 4 done (Analytics, Blog) + seo.tsx XSS hardened

T3 (t3-analytics): Analytics.tsx + bespoke TimelineChart.tsx (zero deps, custom SVG,
animate-draw + reduced-motion). 13 specs (chart edge cases empty/single/all-zero no-NaN,
loading/error, last-write-wins cancelled guard, sample both-ways). Fixed yTicks
duplicate-key collision on all-zero counts (key by index). Committed d83219c.

T4 (t4-blog): Blog.tsx + BlogPost.tsx; markdown body via react-markdown, NO rehype-raw,
no dangerouslySetInnerHTML for body, default urlTransform strips javascript:. Coverage:
formatDate fallback, empty body, loading/error, strengthened XSS. Committed 33a7c6d.

SECURITY ITEM RESOLVED: seo.tsx JSON-LD sink hardened via safeJsonLd() — escapes < > &
as JSON UNICODE escapes (< etc.), NOT HTML entities. Decision: unicode escapes are
the correct fix (script content is raw text; HTML entities wouldn't decode and would
corrupt JSON for crawlers / break JSON.parse). Same anti-breakout guarantee, valid JSON
preserved. Regression test added. Closes the open security item from the T3 review.

## 2026-06-27 — Task 8 done (MCP public read routes) + integration-mapping notes

T8 (t8-routes): mcp/src/api.rs (4 handlers + structs), main.rs (routes on base app router
with global CORS layer), mcp/tests/public_read_routes.rs (7 tests). Public-only on list AND
search paths (Decision 6, seeded public+private same-embedding test). JSON only, no HTML
(Decision 9). limit clamp [1,200]; transient DB errors → 200 empty (never 5xx the public page);
unknown slug → 404. await-free Mutex sections. Committed (see git).
Verify: build/clippy/fmt clean; `cargo test -p mnemonic-mcp --features test-support` → 574 pass.
CI NOTE: plain `cargo test -p mnemonic-mcp` does NOT compile — test-support-gated suites need
`--features test-support` (matches CI). QA wave must use this invocation.

INTEGRATION DEBT to resolve when wiring frontend → live backend (own in T9/T12/T13):
- /artifacts row key is `attestation_id`; lib/ledger.ts Artifact uses `id` → remap in the
  fetch client (ledger.ts) when backend goes live.
- solana_tx/arweave_tx are ALWAYS strings (`local:`-prefixed when not anchored), never JSON
  null → treat `local:` prefix as "not anchored" (links.ts already does).
- /blog post shape: core omits summary/agent/reading_minutes (optional in lib/blog.ts, frontend
  already tolerates absence) and adds attestation_id/content_hash. T9 publish: blog_posts has no
  summary/agent/reading_minutes columns — `author` carries agent name. If we want those fields
  surfaced, T9 needs a core schema extension; else frontend derives (summary=first para,
  reading_minutes=client-computed). Decision deferred to T9.
- /analytics.unique_users is ALL-TIME (core lacks range-scoped distinct-owner query). Accepted.

## 2026-06-27 — Task 5 done (frontend integration)

T5 (t5-integration): routes (/ledger /analytics /blog /blog/:slug) + header/footer nav +
Landing re-skin (additive <Seo> + ExploreStrip, all data-testids preserved, OAuth pages
byte-for-byte untouched) + robots.txt (Allow / ; Disallow /sign/ /oauth/) + static
sitemap.xml (private routes omitted). 3 reviewers: code approved, security approved (robots
Disallow added), test needs_improvement -> fixed (added /sign//oauth/consent//chat
route-preservation tests guarding the scope guardrail). INTEGRATED GATE: `npm run build`
SUCCESS; `npm test` 105 pass / 1 fail (+15 net-new tests). Committed.

TRACKED FOR T6/QA (pre-existing, NOT regressions):
- src/pages/Sign.test.tsx > countdown_displays_mm_ss: flaky waitFor timeout, failing since
  before this feature. T6/QA to stabilize or quarantine.
- e2e/app.spec.ts expects a heading named "Mnemonic Protocol" on "/", but Landing h1 is
  "Verifiable memory for AI agents." ("Mnemonic Protocol" is the eyebrow/wordmark). Pre-existing
  mismatch; e2e not in `npm test`. T6 to reconcile assertion vs markup.

## 2026-06-27 — Task 9 done (agent-native publish surfaces)

T9 (t9-publish): mcp/src/publish.rs (shared pipeline) + blog_publish.rs (4 tests); MCP tool
mnemonic_publish_post (mcp.rs, tool count 7->8) + Micropub POST /blog (api.rs, JSON h-entry +
form-urlencoded) on the bearer-authed api_subrouter. Auth via oauth::bearer_auth_middleware +
handler-level Option<Claims> 401 (anonymous publish rejected); per-pubkey governor rate limit
10/min. Pipeline reuses Task 7 core verbatim: POST_V1 build -> validate_artifact -> sign_artifact
-> save_attestation(Public, Local) -> upsert_blog_post. ZERO core changes (rule 1 honored).
Decisions: slug=slugify(title), re-publish replaces the blog_posts row (upsert), prior public
attestation stays in the immutable append-only ledger (both public, not a leak); owner=server
keypair as publisher-of-record; V1 = free local public write (no x402/on-chain); author carries
agent name (no new columns). Rebuild-hook (BLOG_REBUILD_HOOK) seam left commented in publish.rs
for T13. Verify: build/clippy/fmt clean; `cargo test -p mnemonic-mcp --features test-support`
586 pass. Committed.
AUDIT NOTE (T14): stdio_backward_compat.rs tool-count assert loosened 5->=8 but that test is
#[ignore]d (unverified by the teammate) — audit should confirm it's correct.
FOR T10: advertise <link rel="micropub" href="{mcp_base}/blog">; x-mnemonic AgentCard publish
skill -> both mnemonic_publish_post (native) + POST /blog (interop); RSS reads GET /blog.

## 2026-06-27 — Task 11 done (backend integration tests)

T11 (t11-betests): mcp/tests/privacy_publish_e2e.rs (5 tests, test-support gated). Cross-cutting,
non-duplicative of T8/T9: (1) private memory never leaks across /artifacts plain+?q= search, /blog,
/blog/:slug (Decision 6, sentinel-string assertion over full body — stronger than T8's length check);
(2) re-publish same title replaces row across MCP-tool + POST /blog surfaces (slug-PK upsert);
(3) anon 401 on both surfaces then one bearer authorises both; (4) rate-limit trips 429 after 10/min
quota on the live POST path; (5) live publish pipeline output reconstructs a verifiable POST_V1
attestation (content_hash reproducible, verify_artifact valid, signer == server pubkey). No source
touched, no bugs found. blog_posts migration idempotency already covered by T7 (not duplicated).
Verify: build/clippy/fmt clean; cargo test -p mnemonic-mcp --features test-support all pass. Committed.

## 2026-06-27 — Task 12 done (build-time SEO prerender, webapp-side)

T12 (t12-prerender): Vite SSG. entry-server.tsx (PRERENDER_ROUTES + render(url) via StaticRouter/
renderToString) + scripts/prerender.mjs (builds SSR bundle in node_modules/.prerender temp, splits
<Seo> head tags into the index.html seo:start/seo:end markers, writes dist/<route>/index.html).
index.html reordered head with markers; package.json build = tsc -b && vite build && node
scripts/prerender.mjs. ZERO new deps (react-dom/server + react-router/server already present); no
headless browser (deterministic, VPS-safe). Client uses createRoot (not hydrate) → no mismatch.
CSP unchanged. Evidence: dist/{index,ledger,analytics,blog}/index.html each carry route-specific
title/canonical/OG (+ JSON-LD on / and /analytics) in raw HTML. npm test 109/1 (only known Sign).
Committed. Implements revised Decision 4 (static-route half). T13 extends prerender.mjs to per-post
/blog/:slug from GET /blog + regenerates dist/sitemap.xml with slugs (hook points documented).

## 2026-06-27 — Task 12 fix (build repair, caught by test-reviewer)

The committed T12 (4421c8f) shipped a RED build: prerender.test.ts failed tsc -b under
noUncheckedIndexedAccess (ROUTE_FILES[route] string|undefined -> readFileSync; m[1] possibly
undefined), so `npm run build` exited 1 and vitest's 4 green were reading a STALE dist/.
Test-reviewer (failed verdict) caught it. Fixed forward (t12-fix, commit follows 4421c8f):
typed ROUTES table + guarded captures (tsc EXIT 0); gate hardened to HARD-fail on missing dist
+ mtime freshness guard + per-route og:url/title + Set(4) uniqueness + no-JS shell all 4 routes;
prerender.mjs string->function replacers (T13 live-content safety). Proven: rm -rf dist && build
EXIT 0, gate demonstrated to fail-when-broken then restored; vitest 112/1 (only known Sign).
CARRIED TO T13 (T12 code-review minors): (a) prerender.mjs head-extraction regex `[^>]*?>`
truncates if a Seo meta/og value contains a raw `>` (React doesn't escape > in attr values) —
escape `>` in Seo meta/og output OR assert no raw > survives, BEFORE feeding live post
title/description into meta. (b) route list duplicated in App.tsx / entry-server / test — consider
one keyed source; at minimum keep entry-server PRERENDER_ROUTES in sync when T13 adds blog slugs.

## 2026-06-27 — Task 10 done (discovery + syndication)

T10 (t10-discovery): agent.json publish skill served BOTH as the static webapp canonical card
AND a NEW mcp GET /.well-known/agent.json handler (the referenced mcp handler did not pre-exist;
well_known only had OAuth metadata) — mcp handler makes the skill cargo-testable + mirrors OAuth
well-known, with a `canonical` pointer to the webapp card. x-mnemonic.publish advertises both
mnemonic_publish_post (native) + Micropub POST /blog (interop) + oauth2-bearer + syndication feed.
Atom 1.0 feed GET /blog/feed.xml: public-only (SQL-filtered), entity-escaped all agent text +
URLs (no CDATA so ]]> is moot), entry links -> webapp origin (Decision 9), self-link -> mcp;
attestation_id/content_hash withheld. Lock-safe (pure render). Micropub <link> via Vite native
%VITE_MCP_BASE% substitution in index.html (same base as the SPA). 6 tests; cargo build/test/
clippy/fmt clean; xmllint-validated. Committed.
QA NOTE: %VITE_MCP_BASE% is left literal by Vite if unset at build — prod sets it to
https://mcp.mnemonik.xyz; T6/QA should build with it set (or accept literal in default build).
NOTE: index.html changed after the T12 prerender build -> dist is stale vs index.html; T13/T6
rebuild regenerates it (new rel=micropub/alternate are global tags outside the seo markers).

## 2026-06-27 — Task 13 done (blog prerender + dynamic sitemap + rebuild hook)

T13 (t13b-blogseo; first agent t13-blogseo collided concurrently in the shared tree and was
killed — t13b reconciled both writers and fixed the compile-breaking gaps the other left:
McpState.blog_rebuild_hook added but 5 ctor sites unupdated E0063, deprecated httpmock call).
Delivered: (A) webapp prerender of /blog/<slug> from GET $VITE_MCP_BASE/blog (or
PRERENDER_BLOG_FIXTURE) -> dist/blog/<slug>/index.html with title/canonical/OG/Article JSON-LD +
body; graceful static-only fallback when backend unreachable. (B) dynamic dist/sitemap.xml =
static routes + every /blog/<slug>, private routes excluded. (C) mcp publish.rs fire_rebuild_hook:
best-effort non-blocking SSRF-safe ping to BLOG_REBUILD_HOOK (redirect Policy::none, http(s)-only,
detached tokio task, no-op when unset), 4 tests.
XSS DEVIATION (accepted, better than instructed): did NOT escape in seo.tsx (React 19 already
escapes <>&" in attrs+text; pre-escaping would double-escape). Instead made prerender head
extraction quote-aware (splitHead) so it never depends on React escaping; prerender-blog.test.ts
proves </script>/<img onerror>/raw-> escaped, head not truncated, on the real built artifact.
Lead INDEPENDENTLY verified post-concurrency: cargo build clean, npm run build EXIT 0
(4 static routes + graceful blog fallback), clippy clean. t13b report: vitest 119/1 (known Sign),
cargo test --features test-support all pass. Committed e8c1c7b.

## 2026-06-27 — Audit wave (T14): TEST audit (audit-test) — needs_improvement (2H/2M/2L)

Implemented unit+integration suite is strong (privacy e2e, publish auth both surfaces, rate-limit
429, multi-layer XSS, attestation verifiability, migration idempotency, fail-closed prerender gate).
Gaps = the deferred E2E layer (T6 scope):
- F1 (HIGH): Playwright e2e smoke for /ledger /analytics /blog /blog/:slug ABSENT -> T6.
- F2 (HIGH): pre-existing e2e heading asserts RED against re-skinned Landing in TWO files —
  app.spec.ts:6 AND chat.spec.ts:62,237 ("Mnemonic Protocol" vs new h1 "Verifiable memory for AI
  agents"). e2e not in npm test so CI falsely green. -> T6 fix BOTH files.
- F3 (MED): ledger.ts fetchArtifacts/fetchAttestationTimeline have no direct fallback test
  (asymmetric w/ blog.ts) -> T6 mirror blog.test.ts.
- F4 (MED): attestation_id vs ledger.ts Artifact.id integration debt unguarded -> tracked for live wiring.
- F5/F6 (LOW): Sign countdown flaky (known, pre-existing); stdio_backward_compat tool-count assert is
  correct but the test is #[ignore]d (only runs in --ignored lane).
ACTION: T6 addresses F1/F2/F3 + stabilize Sign; F4 deferred to live backend wiring.

## 2026-06-27 — Audit wave (T14): CODE audit (audit-code) — approve_with_comments (0C/2Maj/3Min/2nit)

No CLAUDE.md hard-rule violations (no payment in core, core->mcp zero refs, Mutex await-free, no
unwrap outside tests, no HashEmbedder). Security posture good.
TWO MAJORS = live-path integration seams (masked by sample fallback; since scope is full end-to-end,
FIX THEM in T6 frontend sweep):
- M1: core/mcp emit attestation_id; webapp Artifact uses id; fetchArtifacts never remaps -> live a.id
  undefined, key={a.id} (Ledger.tsx:96) breaks. Fix: remap attestation_id->id in ledger.ts.
- M2: core BlogPost has no summary; webapp BlogPost.summary feeds meta description + Article JSON-LD ->
  live/prerendered posts ship EMPTY descriptions (undercuts Decision 4 crawl coverage). Fix: derive
  summary client-side (first paragraph of body_markdown) when omitted — in blog.ts so SPA AND prerender
  both get it. (Also derive reading_minutes/agent client-side -> resolves T8/F4 debt.)
MINORS (lower priority): /artifacts `total` = page length not true count; analytics unique_users
all-time (documented); public /analytics exposes daily COUNTS of private writes as metadata (counts
only, consistent with existing /stats — see security audit verdict). NITS: stale axum routing comment
in main.rs; duplicated ORIGIN constant. -> small cleanup pass.

## 2026-06-27 — Audit wave (T14): SECURITY audit (audit-security) — PASS_WITH_MINOR (0C/0H/0M/3L/1I)

Three load-bearing guarantees hold under adversarial review:
- PRIVACY: list_public_artifacts + SEARCH_SQL_CROSS_OWNER_VIS both bind owner_pubkey IS NOT NULL AND
  visibility=Public; search (None,None) returns empty defensively; no private/NULL-owner leak across any
  surface (/artifacts, ?q=, /blog, /blog/:slug, /blog/feed.xml, /analytics).
- AUTH: anonymous publish impossible on both POST /blog and the MCP tool (bearer middleware + handler 401).
- XSS: react-markdown no rehype-raw; safeJsonLd; quote-aware prerender splitHead; feed xml_escape (]]>);
  CSP script-src 'self'. No breakout constructible. SSRF: rebuild hook env-only + Policy::none + detached.

FIX NOW: SEC-T14-04 (info) enforce http(s) scheme allowlist for BLOG_REBUILD_HOOK in CODE (doc/code gap) +
main.rs routing-comment nit -> mcp cleanup pass.

DEFERRED — pre-public-launch open items (spec-acknowledged; tracked, NOT blocking merge):
- SEC-T14-01 (low): per-pubkey publish rate limit Sybil-bypassable (free keypair + open OAuth reg) ->
  public-blog spam/DB growth. Needs agent allowlist/moderation before public scale (Decision 5 open item).
- SEC-T14-02 (low): post `author` is arbitrary caller text; COSE signer is the SERVER key, claims.sub not
  persisted -> any authed agent can impersonate ("Agent-authored" badge). CONTRADICTS Decision 5
  "provable authorship" — needs a product decision (persist + verify claims.sub, or relabel the badge).
- SEC-T14-03 (low): GET /artifacts?q= runs ONNX embedding unauth/uncached/unrated -> CPU DoS. Add a /stats-
  style cache or rate-limit before public launch.
- code-minor: /artifacts `total` = page length, not true count.

## 2026-06-27 — SEC-T14-04 closed (mcp cleanup)

mcp-cleanup: hook_url_allowed() (url::Url scheme parse, http/https only) enforced at both main.rs
env-read AND fire_rebuild_hook firing point (defense-in-depth — direct McpState callers can no longer
bypass via main.rs). Replaces weak starts_with prefix check. +2 tests (scheme allowlist + runtime
rejection). main.rs routing comment corrected. cargo build/test --features test-support/clippy/fmt clean.
Committed. (SEC-T14-01/02/03 remain tracked pre-public-launch items.)

## 2026-06-27 — Task 6 done (frontend sweep + live-path integration fixes)

T6 (t6-fesweep): M1 ledger.ts attestation_id->id remap (live branch); M2 blog.ts deriveBlogPost
(summary=first prose para ~160c, reading_minutes=ceil(words/200), agent=author only if automated-
looking) wired into SPA fetch* AND entry-server prerender so live/prerendered posts get real meta +
Article JSON-LD; F3 ledger client fallback tests; F1 new e2e/ledger.spec.ts (5/5 pass) cross-route
smoke; F2 heading asserts fixed in app.spec + chat.spec; F5 Sign countdown ROOT-CAUSED (test never
seeded mnemonic.identity -> first effect redirected to /install and unmounted before countdown) +
stabilized. Verify: npm run build EXIT 0; vitest 135/135 (Sign now GREEN); e2e ledger 5/5, app 1/1.
Committed 6ab0c0c. ORIGIN-dedupe nit skipped (seo.tsx client vs prerender-lib.mjs Node build script —
different module systems; unifying couples bundle to build script).

INVESTIGATED — chat.spec.ts 6 failures are PRE-EXISTING, NOT a webapp-rethink regression:
origin/main already routes "/" to pages/Landing.tsx; "Start chat" exists NOWHERE in main's webapp/src,
and "Download protocol knowledge" lives only in the LEGACY components/LandingPage.tsx which main does
NOT route to "/". So chat.spec.ts golden-path (Start-chat button + download link) was already broken on
main (stale e2e referencing retired UI). This feature only changed the Landing heading (T6 fixed that
assertion). The chat.spec golden-path/nav rewrite to the current /chat-entry UX is separate pre-existing
tech-debt, OUT OF SCOPE for webapp-rethink. e2e isn't in npm test so CI was already green despite it.
