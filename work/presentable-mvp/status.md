---
updated: 2026-09-23
branch: claude/mnemonic-implementation-iser5h
purpose: single source of truth for this feature; read this first in a new session
---

# Status and backlog: presentable MVP

Read order for a new session: this file → `plan.md` (design + decisions)
→ `user-spec.md` → `docs/USE_CASES.md`.

## Done (pushed on the branch)

| # | What | Commit | Notes |
|---|---|---|---|
| D1 | Keychain read deferred to first signing operation | `feat(mcp): defer OS keychain unlock…` | `core/src/identity/lazy.rs`, `ensure_lazy()`. Local write / recall / whoami never touch keychain. Locked keychain → `-32094 IdentityBootstrapFailed`. |
| D2 | Plan, user-spec, use cases | `4f68eb5` | `plan.md`, `user-spec.md`, `docs/USE_CASES.md` |
| D3 | Server never signs user posts | `974fd47` | `mnemonic_publish_post` / `POST /blog` need `signed_post` (hex COSE_Sign1 by the author). Operator identity only may send plain fields. Slug and artifact_id overwrite holes closed. |
| D4 | Owner decisions + sealed-memory design | `4973417` | `plan.md` §Decisions, §Design: sealed memory |
| D5 | CI moved to nightly | this commit | `ci.yml`, `node-test.yml`, `docs-link-check.yml`, `ext-e2e.yml`: schedule + manual only. CLAUDE.md §CI updated. |

Known test failures that also fail on `main` (not caused by this branch):
`identity::ensure::tests::ensure_rolls_back_on_partial_failure` (fails as
root), `public_read_routes::analytics_buckets_and_totals_by_write_mode`
(date-dependent).

## Owner decisions (2026-09-23) — do not re-ask

1. Server never signs user content.
2. P0/P1 plan accepted.
3. Shared memories are sealed: ciphertext on Arweave, hash on Solana,
   only granted readers can open. Reader may be unknown at write time →
   envelope encryption + later grants (see `plan.md`).
4. Free anchoring quota: 100 per identity per week.
5. Public memories: plaintext, signed, anyone can read.
6. Listed / unlisted: not needed.
7. No revocation (Arweave is permanent storage).
8. CI does not gate PRs; runs nightly.

## Backlog (in order)

### Documents (next)

- [ ] **B1 Simplified whitepaper** in ASD-STE100 style → `docs/WHITEPAPER-SIMPLE.md`.
  Source: `docs/WHITEPAPER.md`. Rules to apply: one topic per sentence,
  max 20 words per procedural sentence / 25 per descriptive, active voice,
  approved simple verbs, no idioms, define every abbreviation on first use.
  Must reflect decisions above (3 modes: local / sealed / public; server
  never signs; quota 100/week). Write in passes of ≤150 lines.
- [ ] **B2 Presentation** (slide deck, via Artifact quickstart `slides`):
  problem → what Mnemonic is → 3 modes → demo script (plan.md) → use cases →
  trust model (who signs, what is on-chain) → roadmap P0/P1/P2 → how to try.

### P0 code — "works for a first-time tester"

- [ ] **C1 Free quota 100/week** per `owner_pubkey` (SQLite counter, rolling
  7 days). Expose `free_anchors_left` in `mnemonic_whoami`. Gate in
  `mcp/src/mcp.rs` participate path before payment.
- [ ] **C2 SDK payment handling**: typed `PaymentRequiredError` with
  `approval_url` + amount (`packages/sdk/src/client.ts:236`); `mode` option
  in `signMemory` (`client.ts:170`).
- [ ] **C3 Webapp**: render approval link + price on 402/428
  (`webapp/src/pages/Sign.tsx:268`).
- [ ] **C4** Replace HTTP 500 "unexpected payment rail" (`mcp/src/api.rs:386`)
  with a clear 402. Add `next_step` text to payment / identity errors.
- [ ] **C5** Install without `gh` (checksum file); fix `smithery.yaml` URL
  (`/mcp`) and tool list.
- [ ] **C6** SDK + CLI helper to build and sign a POST_V1 (`signed_post`),
  since D3 made it mandatory. Webapp publish UI if any.

### P1 code — sharing

- [ ] **S1** `public` mode: plaintext signed memory anchored; readable by all.
- [ ] **S2** `sealed` mode: per-memory key K, XChaCha20-Poly1305, hash of
  ciphertext anchored, K wrapped to author's X25519 (from Ed25519).
- [ ] **S3** `mnemonic_grant` (wrap K to reader pubkey; signed grant record)
  and link grants (`/m/<hash>#k=…`, key in URL fragment).
- [ ] **S4** `mnemonic_import` (verify, decrypt, store locally with
  `received_from`), anonymous `mnemonic_verify` by hash / link.
- [ ] **S5** Webapp `/m/:hash` page with in-browser signature check.

### Ops

- [ ] **O1 Owner action:** in GitHub → Settings → Branches (or Rulesets)
  for `main`, remove "required status checks" that came from CI jobs
  (e.g. `fmt`, `clippy`, `test`, `cross-lang-build (gate)`). Otherwise PRs
  wait forever for checks that no longer run.
- [ ] **O2** Check the first nightly run (02:00 UTC) and fix anything red.
