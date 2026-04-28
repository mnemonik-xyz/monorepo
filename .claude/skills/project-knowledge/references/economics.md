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

| Component | Cost | Notes |
|---|---|---|
| Solana SPL Memo tx fee | ~5,000 lamports ≈ **$0.001** | Fixed minimum fee. Anchors blake3(payload) + arweave_tx_id on-chain. |
| Irys upload (~1KB COSE-signed CBOR) | **$0.001–0.003** | Variable with payload size. Bigger embeddings or larger content scale linearly. |
| **Total per `sign_memory`** | **~$0.002–0.004** | |

`recall` and `verify` stay free — they read SQLite locally + optionally re-fetch from Arweave (free GET) and Solana RPC (free reads).

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

1. **Pricing surface to user.** When `mnemonic sign "..."` costs $0.003, do we surface that or hide it under a flat-rate tier? Per-call pricing has cognitive friction; flat-rate ($X/mo for unlimited) is cleaner UX but exposes the operator to abuse.
2. **Margin & sustainability.** What's the `pricing.rs` markup? At $0.003 marginal cost + $0.001 ops + $0.002 margin = $0.006 charged. Is that competitive against alternatives (storage on Notion / Obsidian Sync / Mem.ai)?
3. **Free tier shape.** First N attestations free? Free recall always? Free if private? Free if signed by specific identity tier?
4. **Refund-on-error semantics.** If Solana confirmation times out but Arweave succeeds, do we refund? Charge half? Retry async? `payment.rs::refund_balance` exists but is currently called only on full failure.
5. **KYC threshold.** Spending limits per identity before requiring stronger identity proof. None today.
6. **Cross-tenant cost attribution.** Each `attestation_costs` row already has `irys_lamports`, `sol_tx_fee_lamports`, `sol_price_usdc`, `charge_micro_usdc`. Reporting / dashboard / per-user invoicing — backlog.
7. **Treasury management.** Where do collected USDC go? Multisig? Single-sig? Auto-swap to USDC stable? Operator's responsibility — no in-product UX yet.
8. **Demo vs product mode.** Hackathon demo: `STORAGE_MODE=full + PAYMENT_MODE=none + RATE_LIMIT=on` works for ~hour-long demo, $50 budget. Real product: requires PAYMENT_MODE=balance + treasury + monitoring + support docs.

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
