# Deployment & Operations

## Purpose
Deployment process, infrastructure, and production operations for AI agents.

---

## Deployment Platform

| Component | Platform | Type |
|---|---|---|
| `core/` native | crates.io | Rust library crate |
| `core/` WASM | npm (`@mnemonic/core`) | WASM npm package |
| `mcp/` | GitHub Releases + Docker (GHCR) | Binary (x86_64, aarch64, linux/macos) |
| `webapp/` | Cloudflare Pages | Static site |
| `docs/` | Cloudflare Pages | Static docs |

---

## Access Information

No server access needed. `mcp/` runs locally on user's machine. `webapp/` is static.

**CI/CD:** GitHub Actions.

**GitHub Actions secrets:**

| Secret name | Purpose |
|---|---|
| `CRATES_IO_TOKEN` | Publish `mnemonic-core` to crates.io |
| `NPM_TOKEN` | Publish `@mnemonic/core` WASM package to npm (future) |
| `CLOUDFLARE_API_TOKEN` | Deploy `webapp/` and `docs/` to Cloudflare Pages |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account identifier |

**Cloudflare Pages projects:** `mnemonic-webapp` (webapp/) and `mnemonic-docs` (docs/) — TBD, confirm names after first deploy.

---

## Environment Variables

**See:** `.env.example` in repo root.

Variables for `mcp/` (set by user):

| Variable | Default | Purpose |
|---|---|---|
| `MNEMONIC_KEYPAIR_PATH` | `~/.mnemonic/id.json` | Ed25519 keypair |
| `DATABASE_PATH` | `~/.mnemonic/attestations.db` | SQLite |
| `STORAGE_MODE` | `local` | `local` or `full` |
| `EMBED_PROVIDER` | `fastembed` | `fastembed`, `openai`, `hash` |
| `OPENAI_API_KEY` | — | If `EMBED_PROVIDER=openai` |
| `TURBO_BITS` | `4` | 2, 3, or 4 |
| `ARWEAVE_URL` | `https://uploader.irys.xyz` | Arweave/Irys endpoint |
| `SOLANA_RPC_URL` | `https://api.mainnet-beta.solana.com` | Solana RPC |
| `MCP_TRANSPORT` | `http` | `stdio` or `http` |
| `MCP_HTTP_PORT` | `3000` | HTTP transport port |
| `PAYMENT_MODE` | `none` | `none`, `balance`, `x402`, `both` |

---

## Deployment Triggers

**crates.io + npm:** Manual on git tag `v*`. CI publishes both.

**mcp/ binary:** Manual on git tag. CI cross-compiles and attaches to GitHub Release. Docker image pushed to GHCR.

**webapp/ + docs/:** Auto-deploy on push to `main`. Preview on every PR (Cloudflare Pages).

**CI tests:** Every push to `main` and `dev`, every PR.

---

## Pre-Deploy Checklist

- [ ] Bump versions: `core/Cargo.toml`, `mcp/Cargo.toml`, `webapp/package.json`
- [ ] Update `CHANGELOG.md`
- [ ] `cargo test --workspace` + `wasm-pack test --headless --chrome` pass locally
- [ ] `git tag v0.x.y && git push origin v0.x.y`

---

## Rollback Procedure

**crates.io:** `cargo yank --vers 0.x.y`, publish patch.
**npm:** `npm deprecate @mnemonic/core@0.x.y`, publish patch.
**Cloudflare Pages:** Instant rollback in dashboard.
**GitHub Release:** Edit release, replace binary attachments.
Time: ~5–10 minutes.

---

## Environments

**Production:** crates.io + npm + GitHub Releases + Cloudflare Pages — from `main`.
**Preview:** Cloudflare Pages preview URLs — from PRs.
**Local dev:** Prerequisites: Rust toolchain, wasm-pack, Node.js, Ollama running with Qwen2.5-7B-Instruct pulled.
1. `cd core && wasm-pack build --target web` — build WASM package (required before webapp dev server)
2. `cd mcp && STORAGE_MODE=local cargo run` — start MCP server in local mode (no funded keypair needed)
3. `cd webapp && npm install && npm run dev` — start webapp dev server

---

## Monitoring & Observability

**Logging:** `tracing` crate in `mcp/`, level via `RUST_LOG`. Browser console in webapp.

**Health check:** `GET /health` on `mcp/` HTTP transport — returns server version and storage mode.

**Metrics:** crates.io + npm download stats only. No app-level metrics for MVP.

**Error tracking:** Not configured. Errors surface in MCP client (Cursor/Claude Desktop) via JSON-RPC error responses.
