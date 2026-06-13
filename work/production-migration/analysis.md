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

### The one fact that reframes everything

The live deploy runs `STORAGE_MODE=local`, `PAYMENT_MODE=none`
(see `deployment.md` mcp.env block). That means production today is a **free,
SQLite-only demo** — no on-chain anchoring, no revenue. The actual product value
(verifiable on-chain "participate" writes, paid) has *code* (`payment.rs`,
`pricing.rs`, the `modes-user-choice` feature) but is **not proven live**, and
issue #165 shows the pricing engine fails at boot and the `whoami` envelope then
lies that everything is "free".

So there are two distinct "productions", and they should be sequenced, not merged:

- **P-A — a reliable free service.** What's live now, made durable. Cheap, do first.
- **P-B — the paid on-chain product.** Flip `participate`/`full` on, with payments.
  Higher risk; gate it behind P-A + a pricing fix + a funded keypair.

---

## 2. Real gaps (grounded, not aspirational)

| # | Gap | Impact | Effort |
|---|-----|--------|--------|
| G1 | **No off-box backups.** SQLite + keypair + secrets live only on one VPS disk. | Disk loss = total, unrecoverable loss of every attestation + identity. **Highest risk.** | ~½ day |
| G2 | **Pricing fetch fails → "free" lie (#165).** Can't safely turn on paid mode; even the free envelope is ambiguous. | Blocks P-B; misleads clients. | ~½–1 day |
| G3 | **Build-on-box deploy.** `cargo build --release` on a 4 GB RAM box is slow and can OOM. No deploy script. | Deploys are fragile and irreproducible. | ~1 day |
| G4 | **No uptime/error signal.** `/health` exists but nothing watches it. | Outages discovered by users, not us. | ~1 hr |
| G5 | **claude-code OAuth interop broken (#163).** | One client class can't use hosted MCP. | unbounded — investigate, don't gate on it |
| G6 | **Secrets durability.** `MCP_REFRESH_SALT` loss invalidates all refresh tokens; keypair loss = identity loss. Off-box copy missing. | Recoverability. | folded into G1 |

#164 (macOS keychain prompts) is **dev-loop only** — not a production concern. Skip.

---

## 3. Minimum-effort plan (do these, in order)

### P0 — make it survivable (do before anything else)
1. **Off-box backup cron (G1, G6).** Nightly `sqlite3 attestations.db .backup` +
   copy of `keypair/id.json` and `mcp.env` (secrets), pushed with `restic`/`rclone`
   to Cloudflare R2 or S3. One script + one cron line. This is the single highest
   value / lowest effort item — do it first.
2. **Fix pricing #165 (G2).** Make the `whoami` envelope tell the truth: add a
   `pricing_status` flag (or null `amount_cents`) when the CoinGecko fetch fails,
   and **drop `participate` from `supported_modes` while pricing is unavailable**
   rather than offering on-chain writes at a fake $0. Background retry already
   exists; just surface its state.

### P1 — make it reproducible & observed
3. **Ship the prebuilt binary, stop building on the box (G3).** `release.yml`
   already cross-compiles the MCP binary. Have the VPS *pull the artifact* (binary
   + `webapp/dist`) instead of compiling. Wrap the steps in a committed `deploy.sh`.
   Removes OOM risk; cuts deploy from minutes-of-compile to seconds.
4. **External uptime monitor (G4).** UptimeRobot / Cloudflare health check on
   `/health` → Discord/email alert. ~15 minutes.

### P2 — turn on the actual product (only after P0/P1)
5. **Enable paid `participate` (P-B).** Requires: #165 fixed, a **funded** Ed25519
   keypair for Arweave+Solana, `PAYMENT_MODE=balance` (or `both`), and one
   end-to-end `participate` write verified on-chain. Keep `local` as the free
   default tier so existing clients are unaffected.
6. **Investigate #163** opportunistically. Don't let it gate the launch — browser
   clients (the majority) already work.

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

Production is mostly an **operations** problem, not a code problem. The smallest
set that gets us to a defensible production posture is **P0 (backups + pricing
truth)** — roughly one day of work — after which the service is durable and honest.
P1 makes deploys safe; P2 is the deliberate, gated step to actually charge money.
Everything else on the roadmap is post-production growth, not a launch blocker.
