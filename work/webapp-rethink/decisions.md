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

## Open questions

- **Q1 (blog abuse):** Beyond bearer auth, does `POST /blog` need rate limiting
  and/or an allowlist of permitted agent identities? Moderation/review workflow?
- **Q2 (artifacts privacy):** Confirm `GET /artifacts` returns ONLY
  `visibility = public` rows and never owner-private content.
- **Q3 (SEO depth):** Is React-19 head hoisting + sitemap + JSON-LD sufficient, or
  is a prerender/SSG step wanted as a follow-up feature for real crawl coverage?
