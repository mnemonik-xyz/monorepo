# Economics — STORAGE_MODE=full Switch Analysis

Status: **draft notes for future deliberation**. Not a decision document. Captures the cost picture of flipping `STORAGE_MODE=local` → `STORAGE_MODE=full` on hosted `mc.mnemonik.xyz`, plus the inseparable billing question (`PAYMENT_MODE`).

---

## Engineering cost (low)

Flipping the storage switch is config + restart, not code:

```
# /home/claude/mcp.env — three lines
STORAGE_MODE=full
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com   # or Helius for stability
ARWEAVE_URL=https://uploader.irys.xyz
```

`sudo systemctl restart mnemonic-mcp` — done. The CLI / SDK / MCP-clients see no API change; only `mnemonic_verify` start returning real `arweave_tx` / `solana_tx` instead of the synthetic `local:` IDs, and `mnemonic_sign_memory` latency rises (see below).

What's already in place: `core/src/{arweave,solana}/` modules, `attestation_costs` table, ANS-104 bundle builder with deep-hash + Avro encoding, SPL Memo writer, idempotency keys for tx submission. Implementation done in pre-integrations Phase 0. No further code work for the flip itself.

Effort to flip: **~½ dev-day** (config + smoke test).

---

## One-time funding (single capital outlay)

| Resource | Where | Amount | Covers |
|---|---|---|---|
| Solana keypair `DYVu4Bry3BzGVsR3Hj2iGVT5fNdWFoHw2zRxsdTmrG25` (file at `/home/claude/monorepo/keypair/id.json`) | Send SOL to that address | ~0.1 SOL (~$15–20 at $200/SOL) | ~10–20K SPL Memo txs |
| Irys credits | irys.xyz dashboard, top up via SOL or USDC | $20–50 | 5–20K attestations of typical ~1KB payload |

**Total upfront:** **~$50** for a comfortable hackathon-grade demo window.

---

## Per-attestation marginal cost

| Component | Cost | Bears | Notes |
|---|---|---|---|
| Solana SPL Memo tx fee | ~5,000 lamports ≈ **$0.001** | Operator (sender keypair) | Fixed minimum fee. Anchors blake3(payload) + arweave_tx_id on-chain. |
| Irys upload (~1KB COSE-signed CBOR) | **$0.001–0.003** | Operator (Irys credit) | Variable with payload size. Bigger embeddings or larger content scale linearly. |
| **Turnkey user-sig** (Phase 1.x onwards) | **~$0.001** | TBD — see pricing model | Per signing op. User-side custody adds ~$0.001 per `sign_memory` + per OAuth login (1h TTL). |
| Server compute (embed + DB) | ~$0.0001 amortized | Operator (VPS fixed cost) | Constant regardless of attestation rate at the current scale. |
| **Total per `sign_memory`** | **$0.002–0.004 (LocalSigner)** | | |
| **Total per `sign_memory`** | **$0.003–0.005 (Turnkey)** | | |

`recall` and `verify` stay free — they read SQLite locally + optionally re-fetch from Arweave (free GET) and Solana RPC (free reads).

Plus per-OAuth-login (every 1h JWT TTL): 1× challenge sig. With `LocalSigner` it's free (WASM in-browser); with Turnkey it's $0.001 per login.

---

## Cost projections at scale

Assumes Turnkey adoption rate of 50% (LocalSigner free, Turnkey opt-in for custody). Avg signs/user/month grows with engagement.

| Monthly active users | Avg signs/user | Total signs | Operator burn (free for all) | Operator burn (free LocalSigner + paid Turnkey at 50% adoption pass-through) |
|---|---|---|---|---|
| 100 (early) | 30 | 3,000 | ~$15/mo | ~$8/mo |
| 1,000 (beta) | 50 | 50,000 | ~$250/mo | ~$125/mo |
| 10,000 (launch) | 80 | 800,000 | **~$4,000/mo** | ~$2,000/mo (with 50% paying users) |
| 100,000 (scale) | 100 | 10,000,000 | **~$50,000/mo** | ~$25,000/mo |

**At 10K users, "free for everyone" = $4K/mo operator burn.** Unsustainable past beta. Pricing must kick in before then.

---

## Pricing model options

| Option | UX surface | User-side predictability | Operator risk | Implementation cost |
|---|---|---|---|---|
| **A. Per-call USDC top-up** | $0.01/sign visible per call | Low (cognitive friction every action) | None — pay-as-you-go | Phase 1.5 `PAYMENT_MODE=balance` (already wired in `mcp/src/payment.rs`) |
| **B. Subscription** ($5/mo for 500 signs) | "I subscribed, just works" | High | Medium (over-usage by power users absorbs margin) | Stripe + usage-tracking middleware (~3 days) |
| **C. Free tier + paid upgrade** (50 free/mo, then $5/mo) | "Try free, pay when needed" | Medium | Medium | Both A + B + tier-tracking (~5 days) |
| **D. Self-sovereign** (user pays Turnkey directly, Mnemonic charges only compute) | Cleaner separation | Variable per user | Low (no usage cost on operator) | Webapp UX to walk user through Turnkey signup + own Sub-Org (~4 days) |
| **E. LocalSigner free + Turnkey opt-in metered** | Default = free, custody-want users pay extra | High | Low | Phase 1.x ships LocalSigner+Turnkey side-by-side; Turnkey signs gated through metered endpoint |
| **F. Operator absorbs everything** | "It's free!" | High | **Catastrophic at scale** | Trivial (do nothing) — only viable in beta/hackathon |

---

## Recommended pricing model (locked candidate, 2026-04-30)

**Cleaner split: free tier = SQL-only "try it" mode, paid tier = real protocol value (anchor + permanence + recovery).** Aligns with existing `STORAGE_MODE` infrastructure — same code path as today's `local` mode, just per-user instead of global.

```
Free tier — "Try it"  ($0/month)
  - LocalSigner (browser localStorage / CLI ~/.mnemonic/identity.json)
  - sign_memory persists to mcp-server's SQLite ONLY
  - synthetic local: tx IDs (no Solana memo, no Arweave upload)
  - recall works (semantic search over user's own attestations)
  - verify returns not_found (no on-chain anchor to validate)
  - NOT portable across MCP servers
  - NO recovery — if operator's DB dies or evicts, attestations are gone
  - Up to 1000 signs/month (high cap because zero marginal cost)
  - Rate limit: 10/min, 500/day

Paid tier — "Verifiable"  ($5/month)
  - LocalSigner OR Turnkey custody (Phase 1.x)
  - Full STORAGE_MODE=full pipeline: Solana SPL Memo + Arweave/Irys upload
  - verify works end-to-end (third party can independently re-hash + check)
  - Portable across any MCP server with same identity
  - Recovery: Turnkey email/passkey if Turnkey-managed; otherwise self-custody backup
  - Up to 1000 signs/month
  - Project absorbs ~$3-4/user/month operating cost; ~$1-2/user/mo margin

Enterprise (custom)
  - Self-hosted MCP option (zero operator cost, license-based pricing)
  - Hosted with custom quotas / SLAs / dedicated VPS / dedicated Turnkey Org
  - Bring-your-own Turnkey Sub-Org / Irys account / Solana keypair
  - Pricing: per-seat or per-attestation contract
```

### Why this design works

- **Free tier marginal cost is essentially zero.** SQL row storage at ~5KB (content + 1.5KB f32 embedding) means 1000 signs/user/mo = ~6MB/user/year. 10K free users = 60GB/year — trivial on VPS. No Solana fees, no Irys credits consumed.
- **Free tier "no guarantee" disclaimer is honest** — we keep rows as long as operator is healthy, no eviction policy is the default, but no contractual durability promise. If user wants durability → pay.
- **Paid tier value prop is unambiguous:** the on-chain anchor is the protocol's headline feature. Without it, `verify` is theatrical. Free tier explicitly opts out of verifiability.
- **No customer cannibalization:** free user who needs verifiability has a clear forced upgrade. No middle-ground "kinda anchored" tier to confuse the message.
- **Implementation alignment:** existing `mcp/src/tools.rs::sign_memory` already branches on `STORAGE_MODE=local` vs `full`. Per-user version of the same branch — read user's tier from `api_keys` table, choose path. Small refactor, same primitives.

### Free tier abuse handling

Free tier abuse vector is low because:
- No on-chain cost to pump. Storage is cheap (~$0.000001/row at SSD prices). Adversary spamming 1M rows costs operator ~$1.
- Rate limit per JWT (10/min, 500/day) catches trivial spam.
- LocalSigner-only — adversary loses key on device wipe, can't accumulate identities easily.
- DB capacity monitoring: alert if a single `owner_pubkey` exceeds, say, 10K rows. Manual review.
- Worst case: prune oldest free-tier rows when DB hits cap. Document the eviction policy in ToS.

### Free → Paid migration UX

When a free user upgrades, two questions:

1. **Backfill historical free-tier attestations to on-chain?** Cost: ~$0.003 × N rows. At 100 free attestations = $0.30 — absorb-able as upgrade incentive. At 1000 free = $3 — break-even on first month. **Recommended: backfill last N rows up to a cap (e.g., last 30 days), older rows stay SQL-only with explicit "from free tier, not anchored" badge.**
2. **What happens on downgrade?** Existing on-chain rows stay verifiable forever (Arweave is paid). Future signs revert to SQL-only. Clean.

### Cost projections (free + paid hybrid, refined)

| MAU | Free ratio | Paid ratio | Free signs/user/mo | Paid signs/user/mo | Operator marginal cost | Subscription revenue | Net |
|---|---|---|---|---|---|---|---|
| 100 | 80% | 20% | 100 | 50 | $20 (paid only — Solana+Irys) | $100 | **+$80** |
| 1,000 | 70% | 30% | 100 | 100 | $300 | $1,500 | **+$1,200** |
| 10,000 | 60% | 40% | 80 | 150 | $4,500 | $20,000 | **+$15,500** |
| 100,000 | 50% | 50% | 80 | 200 | $50,000 | $250,000 | **+$200,000** |

Free-tier cost is negligible (just SQL storage) — operator burn comes from paid users (~$3-4/user/mo Solana+Irys+Turnkey). Subscription revenue covers it with comfortable margin.

---

## Alternatives considered (kept open for re-evaluation)

The recommended free/paid split is the **current candidate**, not a frozen decision. Other viable models, with the trade-offs they imply:

### Pure pay-as-you-go (Option A from earlier)

`PAYMENT_MODE=balance` only — every `sign_memory` deducts USDC. No subscription tier. No free tier (or trivial $1 free credit on signup).

**Why we didn't pick:** cognitive friction every action; user reads "this will cost $0.003" on every `mnemonic sign` — kills demo flow. But: cleanest economic alignment (user pays exactly cost + margin, no over/under-use risk).

**When to revisit:** if subscription tier abusers consistently exceed quota and the support load grows. Per-call billing scales linearly without quota enforcement complexity.

### Subscription-only (no free tier)

$5/mo entry, no $0 tier. Stripe + waitlist before launch.

**Why we didn't pick:** kills viral / demo / "try it" path. Hackathon judges and curious developers won't pay $5 just to read the README.

**When to revisit:** if the free tier turns out to be 99% of users and 0% of revenue (no upgrade path landing). Forced subscription is a stronger signal of actual demand.

### Operator absorbs everything (Option F from earlier)

"It's free for everyone, forever." Project pays for all signs, all anchors, all Turnkey ops.

**Why we didn't pick:** unsustainable past beta. At 10K users that's $4K/mo burn with no revenue path.

**When to revisit:** if a sponsor / grant / VC underwrites the operator burn for a defined period (e.g., "first 12 months free, then we flip"). Common open-source-protocol playbook.

### Self-sovereign (Option D from earlier)

User brings own Turnkey Sub-Org, own Irys account, own Solana keypair. Mnemonic operator only charges for compute (server / RPC / monitoring) flat-fee.

**Why we didn't pick as default:** UX is rough — multi-vendor signup, three account creations before first `sign`. Most users want hosted convenience.

**When to revisit:** for enterprise deals (see below — already in recommended hybrid as the enterprise tier).

### Capacity-based pricing (storage GB / month)

Charge by total stored memory size, not by signing count. $5/mo for 100MB equivalent, $20 for 1GB.

**Why we didn't pick:** doesn't track the real costs. A 1KB-memory and a 100KB-memory cost roughly the same on Solana (fixed fee) but very different on Irys (linear). Storage-based billing under-charges power users and over-charges casual users.

**When to revisit:** if attestation sizes start varying wildly (e.g., users start signing PDFs / images). Right now they're all small text + 1.5KB embedding.

### Token-based (memory units, like Anthropic API tokens)

$0.001 per "memory unit", where unit = ~1KB content + embedding. Bills predictably regardless of underlying tx fees.

**Why we didn't pick:** abstracts away the Solana/Irys cost — when fees spike, operator eats it. When fees drop, operator pockets margin. Adds opacity vs the cleaner "cost + 30% margin" model.

**When to revisit:** if Solana/Irys pricing becomes too volatile to pass through directly. Stable-coin pricing layer is industry-standard for this.

### "Crypto-native" pricing (token / NFT gated)

Hold $MNEMONIC token to access paid tier. Or: NFT-gated tiers ("Founder Pass" = lifetime free).

**Why we didn't pick:** out of scope until/if there's a token. Premature crypto-economics distraction. Subscription via USDC top-up is already crypto-native enough.

**When to revisit:** if a token launch makes sense for protocol governance / decentralization. Not before.

---

## Enterprise self-host (locked candidate, expanded)

User flagged interest in this — expanding to make it concrete.

### Use cases (who buys this)

- **Privacy-sensitive companies** — law firms, healthcare orgs, financial services. Their data doesn't leave their VPC. Compliance: HIPAA, SOC2, GDPR data-residency.
- **Research labs / universities** — IRB-protected research data, internal-only attestations.
- **AI agent platforms** — companies building agentic products on top of Mnemonic want dedicated capacity, custom rate limits, no shared multi-tenancy noise.
- **Crypto-native builders** — DAOs, web3 protocols. Want their own Solana keypair, treasury, no operator dependency.
- **Sovereign deploys** — governments, NGOs. Independence from any single hosted operator.

Enterprise self-host removes Mnemonic operator from the per-sign cost loop entirely. The customer runs the binary on their infra, brings their own Solana keypair / Irys account / optional Turnkey Org.

### What's already built (Phase 1 ready)

- `mnemonic-mcp` is a Rust binary — runs anywhere with Linux + libssl. Nothing operator-specific in the code.
- `STORAGE_MODE`, `PAYMENT_MODE`, `MCP_JWT_SECRET`, `MNEMONIC_KEYPAIR_PATH`, all anchor URLs are env vars — fully configurable.
- Database: SQLite by default (single file, trivial backup); could swap to Postgres in Phase 2 for multi-instance.
- Smithery / Cursor / VS Code / Claude.ai connectors work against any deployed `mcp.<customer-domain>/mcp` — not tied to `mcp.mnemonik.xyz`.

### What's needed to ship enterprise self-host

| Item | Effort | Notes |
|---|---|---|
| Docker image (multi-arch: linux/amd64, linux/arm64) | 1 day | Existing release.yml builds binaries; add docker step |
| GHCR + Docker Hub publish | 0.5 day | `ghcr.io/mnemonik-xyz/mnemonic-mcp:v0.1.0` |
| Helm chart (K8s) | 1–2 days | Optional; many enterprises run K8s |
| Quickstart docs (`docker run` → `init` → first sign) | 0.5 day | |
| Configuration reference (every env var, threat model) | 1 day | |
| License gate or telemetry (anonymous opt-in usage stats) | 1–2 days | Allows tracking adoption without violating privacy |
| Compliance documentation (SOC2 readiness checklist, HIPAA disclaimer, GDPR data flow diagram) | 2–4 days | Required for enterprise sales |
| Support tier infra (Slack channel / email / on-call) | Operational, not engineering | |
| **Total Phase 2 self-host MVP** | **~6–10 dev-days** | |

### Pricing models for self-host

| Model | Description | Pros | Cons |
|---|---|---|---|
| **Open-source free** | Apache-2.0, anyone runs it | Maximum adoption, signals commitment to open protocol | Zero direct revenue; relies on hosted tier funnel |
| **Per-seat license** | $X/seat/month for commercial use | Predictable revenue per customer | Per-seat metering is hard if customer isn't honest |
| **Per-attestation license** | $X per million attestations / month | Aligns with usage | Telemetry / honor-system challenges; hard to enforce |
| **Tiered support** | Open-source binary free; paid support / SLA / migration help | Common open-core pattern | Revenue depends on customer volume × support price |
| **Feature-gate** | Self-host has subset of features; paid tier unlocks (e.g., advanced auth, multi-region replication) | Clean upsell path | Forks the codebase psychologically; risks "open" reputation |
| **Dual-license** | Apache-2.0 for non-commercial; commercial license required for revenue use | Standard for protocols (MongoDB AGPL → SSPL pattern) | Legal complexity; needs licensing infra |

**Recommended for Phase 2:** **Apache-2.0 binary + paid support tier**. Customers can run free indefinitely. Mnemonic earns from:
- **Hosted tier** — for customers who don't want to run infra (the $5/mo "Verifiable" tier)
- **Enterprise support contracts** — $500–5K/mo per customer for SLA, custom integrations, migration assistance, security audits
- **Custom development** — bespoke features funded by customer

This pattern works for: Postgres (community + enterprise support vendors), HashiCorp pre-relicense, Elastic pre-relicense, Mattermost. Apache-2.0 keeps "real open-source" credibility.

### Operator → enterprise customer transition

Some Mnemonic operators today (running `mnemonic-mcp` for their own AI agents) might want to graduate from "casual self-host" to "supported deployment". Phase 2 self-host doc + commercial license offer should make this explicit.

### Open questions for enterprise self-host

1. **License choice** — Apache-2.0 (permissive, fork-friendly) vs MIT (similar) vs AGPL (network-use copyleft, forces SaaS forks back). Tech-spec already says Apache-2.0; reaffirm.
2. **Trademark policy** — can a fork call itself "Mnemonic"? Recommended: trademark the name, allow forks but require renaming. Standard for protocols.
3. **Data export** — enterprise customer must be able to export all attestations as JSONL / SQL dump for audit / migration. Trivial via SQLite backup; should be one CLI command (`mnemonic-mcp dump`).
4. **Multi-tenant within enterprise** — does a single enterprise deploy serve multiple internal teams? Already supported via `owner_pubkey` scoping; just needs documentation.
5. **Federation** — can enterprise A's MCP server verify attestations issued by enterprise B's MCP server? YES — Arweave + Solana are global; cross-tenant verification is the protocol's strength. Needs explicit Phase 2 docs to highlight this.
6. **Update / patch policy** — security updates: do customers auto-pull new docker tags? Do we backport critical fixes to LTS branches? Operational decision once customers exist.

---

## Cost layer separation

Three orthogonal cost layers should be modeled separately in `attestation_costs` and any future billing report:

1. **Operator-fixed** — server compute, RPC subscription, monitoring. Monthly OpEx (~$50–200/mo at current VPS scale; scales to ~$500–1500/mo at 10K-user scale with Helius RPC).
2. **On-chain anchor** — Solana fee + Irys per attestation. Linear in `sign_memory` count. Already tracked in `attestation_costs` table.
3. **Custody fee** — Turnkey per signing op (Phase 1.x onwards). Currently NOT tracked. Need new column `turnkey_lamports_or_usdc_cents` or similar when Phase 1.x lands.

---

## Latency cost

| Mode | `sign_memory` typical | `recall` | `verify` |
|---|---|---|---|
| local | <500ms | <300ms | <100ms (re-hash only) |
| full (sync) | **3–5s** (Solana confirmation block + Irys upload) | <300ms | 1–3s (Solana RPC + Arweave fetch) |
| full (async, **not yet implemented**) | <500ms (write to SQLite immediately, on-chain in background, status via `verify` later) | <300ms | 1–3s |

**Optional optimization:** async write path. Server returns `attestation_id` + `status: pending` immediately, queues Arweave + Solana writes, exposes `/api/attestations/{id}/anchor-status` for clients to poll. Estimated cost: **~1 dev-day**. Worth it if demo UX is important, otherwise users tolerate 3–5s.

---

## The `PAYMENT_MODE` question — inseparable from this switch

The economic cliff: flipping storage to `full` without flipping payment to `balance` means **the project pays for every user's writes**. On a hackathon demo with controlled traffic, fine. On an open public service, that's an attack surface — adversary scripts `mnemonic_sign_memory` in a loop and burns operator funds at $0.003 × N attempts.

Three protective options, in order of effort:

### Option A — `PAYMENT_MODE=balance` (proper economic moat)

Users top up USDC balance, each `sign_memory` deducts. Existing `payment.rs` infrastructure already supports this — what's missing is the **user-facing surface**: webapp top-up flow, balance display, low-balance warnings, refund-on-error UX, CLI `mnemonic balance` / `mnemonic top-up`. Estimated **3–5 dev-days** for full UX.

Pros: actual sustainable economics; users self-fund their own writes; project takes a margin (margin set in `pricing.rs`).
Cons: real money flow → KYC implications → terms-of-service → support load. Big jump from hackathon to real product.

### Option B — Rate limit `sign_memory` per JWT

Extend `tower_governor` (already on `/oauth/*`) to `/mcp` for `tools/call` with `name=mnemonic_sign_memory`. Cap at, say, 10 / min / user, 200 / day / user. Estimated **~½ dev-day**.

Pros: cheap, sufficient for hackathon-window. Doesn't introduce billing complexity.
Cons: doesn't solve the long-term economics — still operator-funded, just bounded burn rate.

### Option C — Captcha / waitlist on signup

Limit OAuth `/oauth/register` and `/oauth/authorize` to known users via captcha or invite codes. **~½ dev-day**.

Pros: keeps total user count bounded.
Cons: kills the "open MCP server anyone can connect" pitch.

**My (assistant's) read:** for hackathon → **A=local + B=ratelimit prep**, OR **A=full + B=ratelimit** if the demo story benefits from real Arweave + Solana proof. For real product launch → **C=full + balance billing**, no exceptions.

---

## Risks of operating in full mode

- **Funding alerts:** monitor SOL balance + Irys balance. Without alerts, the server silently starts returning errors when one runs out. ~1 hour of work for cron + Telegram bot.
- **Graceful degradation:** if SOL or Irys runs dry, `sign_memory` should return a structured `503` with retry-after header, not a 500. Backlog item.
- **RPC reliability:** mainnet-beta is rate-limited and flaky under load. Helius (paid) gives ~100K req/day for $50/mo, much better tail latency. Worth budgeting.
- **Tx fee volatility:** Solana fees can spike during congestion (rare for SPL Memo, but possible). Cap by setting `solana::priority_fee_micro_lamports = 0` (already default) and accept variable confirmation time.
- **Irys vs Arweave directly:** currently using Irys (fastest UX, ~5s confirmation). Direct Arweave (`arweave.net`) is cheaper (~30%) but slower (~30 min confirmation). Stay on Irys for hackathon, evaluate native Arweave for cost reduction post-launch.

---

## Open questions for future thinking

These are the things that need proper deliberation before flipping the switch in production:

1. **Pricing surface to user.** When `mnemonic sign "..."` costs $0.003 (LocalSigner) or $0.004 (Turnkey), do we surface that or hide it under a flat-rate tier? Per-call pricing has cognitive friction; flat-rate ($5/mo for 1000 signs) is cleaner UX but exposes the operator to abuse. **Recommended hybrid above** picks flat-rate for paid tier + free LocalSigner tier.
2. **Margin & sustainability.** Free tier with rate limits absorbs ~$0.30/user/mo. Paid tier $5/mo charges ~$4 in costs (Solana + Irys + Turnkey + margin) → ~$1/user/mo margin. At 10K paying users → $10K/mo margin. At 100K → $100K/mo. Need to build to ~5K paying users before sustainability.
3. **Free tier abuse.** LocalSigner+rate-limit gates casual abuse (5/min, 100/day, 100/month). But coordinated multi-account abuse possible. Need: per-IP rate limit on signup; CAPTCHA on `/oauth/register`; KYC-lite for identities crossing thresholds (>$10/mo equivalent).
4. **Free tier shape.** 100 signs/mo, 5/min rate, 100/day cap — first cut. Tunable per usage telemetry. Recall stays free (read-only, no on-chain cost). Verify free.
5. **Refund-on-error semantics.** If Solana confirmation times out but Arweave succeeds, do we refund? Charge half? Retry async? `payment.rs::refund_balance` exists but is currently called only on full failure.
6. **KYC threshold.** Spending >$50/mo or >5000 signs/mo → require email verification + stronger identity. Current Turnkey email-passkey flow naturally gates this.
7. **Cross-tenant cost attribution.** `attestation_costs` row has `irys_lamports`, `sol_tx_fee_lamports`, `sol_price_usdc`, `charge_micro_usdc`. Need new `turnkey_micro_usdc` column for Phase 1.x. Then per-user invoicing dashboard. Backlog.
8. **Treasury management.** Where do collected USDC go? Operator multisig (recommended for protocol legitimacy). Auto-swap to fiat or stable? Stripe payouts vs direct USDC retention? Decision deferred until Phase 1.5 billing UX lands.
9. **Demo vs product mode.** Hackathon demo: `STORAGE_MODE=full + PAYMENT_MODE=none + RATE_LIMIT=on` works for ~hour-long demo, $50 budget. Real product: requires PAYMENT_MODE=balance + treasury + monitoring + support docs.
10. **Turnkey vendor cost passthrough vs absorption.** Even at "free LocalSigner" tier, if a free user opts to use Turnkey for recovery without paying, who eats the $0.001/sig? Current recommended hybrid forces paid tier for Turnkey use — locks the vendor cost into the paid line item. Alternative: free Turnkey for first N signs, then forced upgrade.
11. **Self-sovereign escape hatch.** Can a user export their Turnkey-managed key to a `LocalSigner` profile and switch tiers retroactively? Turnkey supports export — need UX flow + tier-downgrade logic.
12. **Enterprise self-host.** Companies running their own MCP server pay zero per-sign costs (their own keypair, their own Irys account, their own Turnkey Sub-Org). Mnemonic charges enterprise license per seat or contract-based. UX flow: docker-compose + config docs + support tier.

---

## Decision framework

When the question of flipping comes up, the answer depends on three orthogonal axes:

| Axis | Hackathon | Beta launch | Real product |
|---|---|---|---|
| `STORAGE_MODE` | local OR full | full | full |
| `PAYMENT_MODE` | none | none + ratelimit | balance |
| Funding source | $50 personal | $200 + ratelimit cap | self-funding via balance |
| User UX | "sign just works" | "sign just works, free for now" | "balance: $4.20, sign: -$0.006" |
| Risk surface | low (controlled traffic) | medium (open but capped) | high (real money) |

**Status today (2026-04-29):** local + none. Stable for current traffic.

---

## Pointers

- Implementation: `core/src/arweave/`, `core/src/solana/`, `mcp/src/payment.rs`, `mcp/src/pricing.rs`.
- Schema: `attestation_costs` table — see [architecture.md → Data Model](architecture.md#data-model).
- Existing decision-record: Decisions 4 + 8 of `work/completed/mnemonic-integrations/tech-spec.md` constrain canonical CBOR encoding (relevant for cross-mode integrity).
- Trace of why payment surface was deferred: search `work/completed/mnemonic-integrations/decisions.md` for "PAYMENT_MODE".
