# Mnemonic → Production: minimum-effort analysis

**Date:** 2026-06-13
**Question:** What is the *minimum-effort* path to move Mnemonic to production?
**Philosophy:** the bicycle, not the spaceship. Stabilise what exists; don't rearchitect.

---

## 1. Where we actually are (the surprising part)

Mnemonic is **already deployed and reachable**:

- `https://mnemonik.xyz` (webapp) + hosted MCP at `https://mcp.mnemonik.xyz/mcp`.
- OAuth 2.1 + PKCE works for browser clients (Claude.ai, Cursor, VS Code).
- CLI (`@mnemonik-xyz/cli`) and SDK (`@mnemonik-xyz/sdk`) published to npm; Chrome
  extension shipped.
- Runs on one VPS (justhost.asia, 4 vCPU / 4 GB) under `systemd` + `nginx`, native
  MCP binary + Docker Ollama.

So "go to production" is **not** a greenfield launch. It is **hardening a live
soft-launch** into something that won't lose data, won't silently mischarge, and
can be redeployed reliably.

### The launch model (corrected)

Mnemonic ships **two modes**, and *both* are the product — there is no separate
"free demo" vs "real product" sequencing:

- **Self-host local.** The user runs the MCP binary themselves. SQLite-only, free,
  no funded keypair, no payment. (`STORAGE_MODE=local`, `PAYMENT_MODE=none`.)
- **Hosted x402.** The user points an agent at `https://mcp.mnemonik.xyz/mcp` and
  issues a `participate` write. The memory is anchored on-chain (Arweave durable
  bytes + Solana SPL Memo) and the agent **pays per-write in USDC via x402**: it
  sends a USDC transfer to the operator treasury and presents the tx signature in
  the `X-Payment` header on retry; the server verifies it with
  `verify_usdc_transfer` before anchoring. (`PAYMENT_MODE=x402`,
  `STORAGE_MODE=full`, `mode: "participate"` from `modes-user-choice`.)

This is genuinely **minimum-effort by design**: x402 is autonomous per-call payment
— no balance accounts, no Stripe, no user billing system, no subscription plumbing.
The `modes-user-choice` delivery-confirmation logic already demotes a row to `local`
and *does not consume the x402 nonce* if anchoring fails, so the failure path is
already production-shaped.

### What this means for "production"

The paid hosted tier is **the launch**, not a later gated step. So the work is to
(a) configure the hosted deploy for x402 correctly, (b) fix the one bug that breaks
x402 pricing (#165), and (c) keep the box durable. The deployment doc is **stale** —
its `mcp.env` still shows `PAYMENT_MODE=none` and never mentions `TREASURY_PUBKEY`
or x402; that doc must be updated as part of the cutover.

---

## 2. Real gaps (grounded, not aspirational)

| # | Gap | Impact | Effort |
|---|-----|--------|--------|
| G1 | **x402 pricing fails → $0 (#165).** x402 challenge price is derived from the SOL/USDC pricing engine; on fetch failure it floors to ~$0 and the `whoami` envelope reports "free". | **Launch blocker** — the hosted paid tier either gives on-chain writes away or quotes a price that doesn't match what it charges. | ~½–1 day |
| G2 | **Hosted x402 not configured for prod.** `TREASURY_PUBKEY` defaults to `""`; deploy still runs `PAYMENT_MODE=none`, `STORAGE_MODE=local`; operator keypair must be **funded** (Arweave Irys + Solana fees) to anchor. | Without this the hosted tier physically cannot do a paid on-chain write. | ~½ day + funding |
| G3 | **No off-box backups.** SQLite + keypair + secrets live only on one VPS disk. | Disk loss = loss of local-mode rows, the recall embeddings index, identity, and `MCP_REFRESH_SALT`. *Participate* rows are recoverable from Arweave, so blast radius is smaller — but still serious. **Highest ops risk.** | ~½ day |
| G4 | **Build-on-box deploy.** `cargo build --release` on a 4 GB RAM box is slow and can OOM. No deploy script. | Deploys are fragile and irreproducible. | ~1 day |
| G5 | **No uptime/error signal.** `/health` exists but nothing watches it. | Outages (incl. a treasury/pricing outage that silently disables paid writes) discovered by users, not us. | ~1 hr |
| G6 | **Stale deployment doc.** `mcp.env` example shows `PAYMENT_MODE=none`, no `TREASURY_PUBKEY`/x402. | Next deploy reproduces the non-revenue config. | folded into G2 |
| G7 | **claude-code OAuth interop broken (#163).** | One client class can't use hosted MCP. | unbounded — investigate, don't gate launch on it |

`MCP_REFRESH_SALT` / `MCP_JWT_SECRET` / keypair durability folds into G3 (back them
up off-box). #164 (macOS keychain prompts) is **dev-loop only** — skip for prod.

---

## 3. Minimum-effort plan (do these, in order)

### P0 — turn the launch model on, correctly
1. **Fix x402 pricing #165 (G1).** Make pricing failure explicit instead of silent
   $0: surface a `pricing_status` (or null `amount_cents`) in the `whoami` envelope
   and **drop `participate` from `supported_modes` while pricing is unavailable**,
   so the hosted tier refuses to quote/anchor at a fake free price. Background
   refresh already exists — just gate on its state. This is the launch blocker.
2. **Configure + verify the hosted x402 tier (G2, G6).** Set `PAYMENT_MODE=x402`,
   `STORAGE_MODE=full`, a real `TREASURY_PUBKEY`, confirm `USDC_MINT` (mainnet
   default is correct), and **fund the operator keypair** for Arweave Irys + Solana
   fees. Then run **one end-to-end paid `participate` write** and verify it on-chain
   (Arweave bytes + Solana memo + USDC landed in treasury). Update `deployment.md`'s
   `mcp.env` so the config is reproducible.
3. **Off-box backup cron (G3).** Nightly `sqlite3 attestations.db .backup` + copy of
   `keypair/id.json` and the secrets (`MCP_REFRESH_SALT`, `MCP_JWT_SECRET`,
   `GOOGLE_OAUTH_CLIENT_SECRET`), pushed with `restic`/`rclone` to Cloudflare R2 / S3.
   One script + one cron line.

### P1 — make it reproducible & observed
4. **Ship the prebuilt binary, stop building on the box (G4).** `release.yml`
   already cross-compiles the MCP binary. Have the VPS *pull the artifact* (binary
   + `webapp/dist`) instead of compiling. Wrap the steps in a committed `deploy.sh`.
   Removes OOM risk; cuts deploy from minutes-of-compile to seconds.
5. **External uptime monitor (G5).** UptimeRobot / Cloudflare health check on
   `/health` → Discord/email alert. ~15 minutes. Ideally also alert on the
   pricing/treasury degraded state from step 1.

### P2 — opportunistic
6. **Investigate #163** (claude-code OAuth). Don't gate launch on it — browser
   clients (Claude.ai, Cursor, VS Code) and self-host local already work.

---

## 4. Explicitly NOT doing (spaceships to refuse)

These appear in the backlog/roadmap and are real future work, but **none are
required for production** at current scale and all are large:

- Concurrent-writers event-sourcing / CRDT rearchitecture
  (`docs/problems/CONCURRENT_WRITERS.md`) — single-writer per owner is fine for MVP.
- ERC-8004 / Ethereum anchor (issues #69–#75).
- Postgres migration, Kubernetes/HA, multi-region. SQLite + one VPS + nightly
  backups comfortably covers current load. Revisit only when a concrete capacity
  or concurrency limit is actually hit.
- Upgrading the VPS for Ollama speed: Ollama only powers the webapp **chat demo**,
  not the core memory path, so its 30–60 s latency does not block the product.
  If snappiness matters, point the demo at a hosted LLM — cheaper than a bigger box.

---

## 5. Bottom line

The launch model — self-host local (free) **or** hosted x402 (pay-per-write,
on-chain) — is deliberately lean: x402 means no billing system to build. Production
is therefore mostly **config + one bug fix + ops durability**, not new architecture.

The smallest set that gets us to a real, revenue-capable production is **P0**:
fix x402 pricing (#165), configure + fund the hosted x402 tier and prove one paid
write on-chain, and add off-box backups — roughly one to two days. P1 (prebuilt-binary
deploy + uptime alerting) makes it safe to operate. Everything else on the roadmap
(concurrent writers, ERC-8004, Postgres/HA, balance/Stripe tiers) is post-launch
growth, not a blocker.
