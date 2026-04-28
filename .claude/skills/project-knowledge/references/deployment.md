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
| `MCP_JWT_SECRET` | — | HS256 secret for OAuth Bearer JWTs (required in hosted mode; ≥32 random bytes — `openssl rand -base64 32`) |
| `MCP_PUBLIC_BASE_URL` | — | Public origin advertised in OAuth metadata + `/sign/{id}` redirect (e.g. `https://mcp.mnemonik.xyz`) |
| `OAUTH_RATELIMIT_DISABLE` | `0` | Set to `1` only in CI / Playwright runs to bypass the `tower_governor` per-IP limiter on `/oauth/*` |

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

**Production VPS (mnemonik.xyz):** justhost.asia, 4 vCPU / 4GB RAM / 120GB NVMe, user `claude`.
**Preview:** Cloudflare Pages preview URLs — from PRs (future).
**Local dev:** Prerequisites: Rust toolchain, Node.js, Ollama with qwen2.5:3b pulled.
1. `cd mcp && STORAGE_MODE=local OLLAMA_URL=http://localhost:11434 OLLAMA_MODEL=qwen2.5:3b cargo run --features local-embed` — start MCP server
2. `cd webapp && npm install && npm run dev` — start webapp dev server (Vite proxies /api to localhost:3000)

---

## VPS Deploy Process (150.251.147.215 / mnemonik.xyz)

### Architecture

Hybrid deploy: native MCP binary + Docker Ollama + native nginx.

```
Internet → nginx (:80/:443) → proxy to MCP (:3000) → Ollama (:11434, Docker)
                             → static files (webapp/dist/)
```

### Server Setup (one-time)

```bash
# SSH access
ssh claude@150.251.147.215

# 1. Swap (4GB on NVMe — required for 4GB RAM server)
sudo fallocate -l 4G /swapfile && sudo chmod 600 /swapfile
sudo mkswap /swapfile && sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab

# 2. Docker (for Ollama)
sudo apt-get update
sudo apt-get install -y ca-certificates curl
sudo install -m 0755 -d /etc/apt/keyrings
sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo $VERSION_CODENAME) stable" | sudo tee /etc/apt/sources.list.d/docker.list
sudo apt-get update && sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo usermod -aG docker claude

# 3. Rust toolchain (for building MCP binary)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
sudo apt-get install -y pkg-config libssl-dev g++

# 4. Node.js (for building webapp)
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
sudo apt-get install -y nodejs

# 5. nginx
sudo apt-get install -y nginx certbot python3-certbot-nginx
```

### Deploy Steps

```bash
ssh claude@150.251.147.215

# 1. Clone/pull repo
cd /home/claude
git clone https://github.com/mnemonik-xyz/monorepo.git  # first time
# or: cd monorepo && git pull origin main                # updates

cd /home/claude/monorepo

# 2. Build MCP binary (native, with fastembed)
source ~/.cargo/env
cargo build --release -p mnemonic-mcp --features local-embed

# 3. Build webapp
cd webapp && npm install && npm run build && cd ..

# 4. Build Ollama Docker image (pre-pulls model)
# Note: edit ollama/Dockerfile to set correct model (qwen2.5:1.5b or qwen2.5:3b)
docker build -t monorepo-ollama ollama/

# 5. Start Ollama container
docker run -d --name ollama --restart unless-stopped \
  -p 11434:11434 -v ollama-data:/root/.ollama monorepo-ollama

# 6. Generate keypair (first deploy only)
mkdir -p keypair
docker run --rm --entrypoint /bin/sh \
  -v $(pwd)/keypair:/run/secrets/keypair \
  -e MNEMONIC_KEYPAIR_PATH=/run/secrets/keypair/id.json \
  -e DATABASE_PATH=/tmp/test.db -e MCP_TRANSPORT=http \
  -e STORAGE_MODE=local -e EMBED_PROVIDER=fastembed \
  -e OLLAMA_URL=http://localhost:11434 \
  monorepo-mcp -c 'timeout 10 mnemonic-mcp --transport http 2>&1 || true'

# 7. Create env file
cat > /home/claude/mcp.env << 'EOF'
MNEMONIC_KEYPAIR_PATH=/home/claude/monorepo/keypair/id.json
DATABASE_PATH=/home/claude/data/attestations.db
STORAGE_MODE=local
EMBED_PROVIDER=fastembed
TURBO_BITS=4
MCP_TRANSPORT=http
MCP_HTTP_PORT=3000
PAYMENT_MODE=none
OLLAMA_URL=http://localhost:11434
OLLAMA_MODEL=qwen2.5:1.5b
RAG_CHUNK_DIR=/home/claude/data/rag_chunks
RUST_LOG=info
EOF
mkdir -p /home/claude/data

# 8. Create systemd service (first deploy only)
sudo tee /etc/systemd/system/mnemonic-mcp.service << 'EOF'
[Unit]
Description=Mnemonic MCP Server
After=network.target docker.service
Wants=docker.service
[Service]
Type=simple
User=claude
WorkingDirectory=/home/claude/monorepo
EnvironmentFile=/home/claude/mcp.env
ExecStart=/home/claude/monorepo/target/release/mnemonic-mcp --transport http --port 3000
Restart=on-failure
RestartSec=5
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload && sudo systemctl enable mnemonic-mcp

# 9. Start/restart MCP
sudo systemctl restart mnemonic-mcp

# 10. Configure nginx (first deploy only)
sudo tee /etc/nginx/sites-available/mnemonic << 'NGINX'
server {
    listen 80;
    server_name mnemonik.xyz;
    root /home/claude/monorepo/webapp/dist;
    index index.html;
    location / { try_files $uri $uri/ /index.html; }
    location /mcp { proxy_pass http://127.0.0.1:3000; proxy_set_header Host $host; proxy_set_header X-Real-IP $remote_addr; }
    location /chat { proxy_pass http://127.0.0.1:3000; proxy_set_header Host $host; proxy_set_header X-Real-IP $remote_addr; proxy_read_timeout 120s; }
    location /download-knowledge { proxy_pass http://127.0.0.1:3000; proxy_set_header Host $host; }
    location /health { proxy_pass http://127.0.0.1:3000; }
    location /admin { return 403; }
}
NGINX
sudo ln -sf /etc/nginx/sites-available/mnemonic /etc/nginx/sites-enabled/mnemonic
sudo rm -f /etc/nginx/sites-enabled/default
chmod 755 /home/claude /home/claude/monorepo /home/claude/monorepo/webapp/dist
sudo nginx -t && sudo systemctl reload nginx

# 11. SSL (after DNS points to VPS)
sudo certbot --nginx -d mnemonik.xyz --non-interactive --agree-tos --email bogdan.sivochkin@gmail.com
```

### Update Deploy (after code changes)

```bash
ssh claude@150.251.147.215
cd /home/claude/monorepo && git pull origin main
source ~/.cargo/env && cargo build --release -p mnemonic-mcp --features local-embed
cd webapp && npm install && npm run build && cd ..
sudo systemctl restart mnemonic-mcp
```

#### Hosted MCP subdomain — `mcp.mnemonik.xyz` (T14, 2026-04-26)

The hosted MCP for the Smithery / Cursor / Claude.ai OAuth flow lives on the
subdomain `mcp.mnemonik.xyz` (DNS A → 150.251.147.215). nginx config:
`/etc/nginx/sites-available/mnemonic-mcp` (in-tree source:
`mcp/deploy/nginx-mcp-subdomain.conf`); LetsEncrypt cert at
`/etc/letsencrypt/live/mcp.mnemonik.xyz/`. The same `mnemonic-mcp.service`
serves both `mnemonik.xyz/mcp` and `mcp.mnemonik.xyz/mcp` — only nginx
routing differs.

Hosted-mode env vars in `/home/claude/mcp.env` now require **`MCP_JWT_SECRET`**
(Decision 11 of the `mnemonic-integrations` tech-spec: HS256-signed JWT, 1h
TTL, claims bound to user pubkey). Generate once on the VPS:

```bash
echo "MCP_JWT_SECRET=$(openssl rand -base64 32)" >> /home/claude/mcp.env
```

Per Deviation 7 of the same tech-spec, `MNEMONIC_KEYPAIR_PATH` is **no
longer required for the OAuth flow** — signing happens client-side in the
webapp's WASM identity. The keypair is still required for legacy `full`
storage mode (Arweave + Solana writes) but irrelevant to OAuth bootstrap.

### Key Paths

| Path | Purpose |
|------|---------|
| `/home/claude/monorepo/` | Git repo |
| `/home/claude/monorepo/target/release/mnemonic-mcp` | MCP binary |
| `/home/claude/monorepo/webapp/dist/` | Static webapp build |
| `/home/claude/monorepo/keypair/id.json` | Ed25519 keypair (pubkey: DYVu4Bry3BzGVsR3Hj2iGVT5fNdWFoHw2zRxsdTmrG25) |
| `/home/claude/mcp.env` | Environment variables |
| `/home/claude/data/attestations.db` | SQLite database |
| `/home/claude/data/rag_chunks/protocol-knowledge.zip` | Downloadable knowledge artifact |
| `/etc/nginx/sites-available/mnemonic` | nginx config (webapp + `mnemonik.xyz/mcp`) |
| `/etc/nginx/sites-available/mnemonic-mcp` | nginx config (`mcp.mnemonik.xyz` subdomain, T14) |
| `/etc/systemd/system/mnemonic-mcp.service` | systemd service |

### Troubleshooting

```bash
# Check service status
sudo systemctl status mnemonic-mcp
docker ps  # Ollama container

# Logs
journalctl -u mnemonic-mcp -f          # MCP server logs
docker logs ollama --tail 50            # Ollama logs
sudo tail -f /var/log/nginx/error.log   # nginx logs

# Health checks
curl http://localhost:3000/health               # MCP direct
curl http://localhost:11434/api/tags            # Ollama models
curl https://mnemonik.xyz/health                # via nginx+SSL
```

### MCP-client Distribution

The hosted MCP at `mcp.mnemonik.xyz` is distributed three ways, all driven by the public `webapp/install` page:

1. **Cursor / VS Code deeplinks** — clicking the install button opens the editor with a pre-filled MCP HTTP config (Cursor: `cursor://anysphere.cursor-deeplink/mcp/install?...&config=<base64>`; VS Code: `vscode:mcp/install?<urlencoded JSON>`). Both clients then run the OAuth 2.1 + PKCE flow against `/.well-known/oauth-authorization-server`.
2. **Claude.ai custom connector** — paste `https://mcp.mnemonik.xyz/mcp` into Claude.ai's "Add custom connector" dialog. Claude.ai performs RFC 7591 dynamic client registration via `POST /oauth/register` and POSTs JSON-RPC to the apex `/` (not `/mcp`) — both routes mount the same handler.
3. **Smithery catalogue** — `smithery.yaml` at repo root advertises the hosted endpoint. Submission to Smithery is **manual** via the Smithery dashboard; CI does not auto-publish. Re-submit only when the protocol contract or transport changes.

### Known Issues

- **LLM response slow (30-60s):** 4GB RAM VPS with qwen2.5:1.5b on CPU. Upgrade VPS to 8GB+ RAM and use qwen2.5:3b or 7b for faster responses.
- **Dockerfile local-embed:** `rust:1-slim` base image needs `g++` (`libstdc++`) for fastembed/ONNX. Native build on VPS avoids this.

---

## Monitoring & Observability

**Logging:** `tracing` crate in `mcp/`, level via `RUST_LOG`. Browser console in webapp.

**Health check:** `GET /health` on `mcp/` HTTP transport — returns server version and storage mode.

**Metrics:** crates.io + npm download stats only. No app-level metrics for MVP.

**Error tracking:** Not configured. Errors surface in MCP client (Cursor/Claude Desktop) via JSON-RPC error responses.
