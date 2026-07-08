# Containerized deploy — Mnemonic MCP

Full docker-compose stack: **mcp** (Rust HTTP server, fastembed) + **ollama**
(chat LLM) + **nginx** (TLS + reverse proxy + static webapp) + **certbot**
(auto-renew). The MCP image is built in CI and pushed to GHCR, so a VPS only
needs Docker — no Rust/Node toolchain, no on-box compile.

## Files
| File | Purpose |
|------|---------|
| `mcp/Dockerfile` | Production image: g++/ONNX for `local-embed`, `docs/` baked in for the RAG seed, non-root, healthcheck. |
| `docker-compose.yml` | The stack. Co-located by default; Ollama is profile-gated for split deploys. |
| `nginx.conf` | Same-origin API routes + SPA + TLS (domain-agnostic cert path `.../live/mnemonic/`). |
| `mcp/deploy/compose.env.example` | Template → copy to `mcp.env`, fill secrets. |
| `mcp/deploy/init-letsencrypt.sh` | One-time TLS bootstrap (nginx↔certbot chicken-and-egg). |
| `.github/workflows/build-mcp-image.yml` | Build + push image to GHCR. |
| `.github/workflows/deploy-mcp.yml` | Manual pull + `compose up -d` on the VPS. |

## Fresh VPS — first deploy (Case A: same domain, new box)

Prereq: DNS `A` records for `mnemonik.xyz` and `mcp.mnemonik.xyz` → the new IP.

```bash
# 1. Install Docker only (Ubuntu). Nothing else is needed on the host.
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker "$USER"   # re-login for the group to take effect

# 2. (4 GB box) add swap — fastembed + ollama are memory-hungry.
sudo fallocate -l 4G /swapfile && sudo chmod 600 /swapfile
sudo mkswap /swapfile && sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab

# 3. Get the compose + nginx files (the app itself ships as a GHCR image).
git clone https://github.com/mnemonik-xyz/monorepo.git /home/claude/monorepo
cd /home/claude/monorepo

# 4. Configure env + secrets.
cp mcp/deploy/compose.env.example mcp.env
#   Fill MCP_JWT_SECRET + MCP_REFRESH_SALT (generate, keep stable):
echo "MCP_JWT_SECRET=$(openssl rand -base64 32)"   >> mcp.env
echo "MCP_REFRESH_SALT=$(openssl rand -base64 32)" >> mcp.env
#   Edit DOMAIN, CERTBOT_EMAIL, MCP_PUBLIC_BASE_URL, Google OAuth as needed.
#   Then dedupe any keys you appended twice.

# 5. (optional) build the static webapp for nginx to serve. Skip if the webapp
#    is hosted elsewhere (Cloudflare Pages) — nginx still proxies the API.
#    Requires Node; or copy a prebuilt dist/ up. See webapp/README.

# 6. Pull images (mcp from GHCR, build ollama locally on first run).
docker compose --env-file mcp.env pull mcp
docker compose --env-file mcp.env build ollama

# 7. TLS bootstrap (issues the LetsEncrypt cert, starts nginx).
./mcp/deploy/init-letsencrypt.sh

# 8. Bring the whole stack up.
docker compose --env-file mcp.env up -d

# 9. Verify.
curl -fsS https://mnemonik.xyz/health
docker compose --env-file mcp.env ps
```

The `mcp` container generates its Ed25519 keypair into the `mcp-keypair` volume
on first boot. **To preserve the existing server identity** (pubkey
`DYVu4Bry3BzGVsR3Hj2iGVT5fNdWFoHw2zRxsdTmrG25`) and prior attestations, copy the
old `keypair/id.json` and `data/attestations.db` into the volumes before step 8:

```bash
docker run --rm -v monorepo_mcp-keypair:/k -v "$PWD/keypair":/src alpine \
  cp /src/id.json /k/id.json
docker run --rm -v monorepo_mcp-data:/d -v "$PWD/data":/src alpine \
  cp /src/attestations.db /d/attestations.db
```

## Routine updates
Push to `main` → `build-mcp-image.yml` publishes `:latest`. Then ship it:
Actions UI → **Deploy MCP** → `apply` (pins `MCP_IMAGE_TAG`, `compose pull`,
`up -d`, health-gates, prunes). Or on the box: `docker compose --env-file
mcp.env pull mcp && docker compose --env-file mcp.env up -d`.

## Rollback
Deploy MCP → `rollback` with `ref: vX.Y.Z` (a tag GHCR already has an image
for). Sets `MCP_IMAGE_TAG=vX.Y.Z` and recreates the container. Seconds, no build.

## Split host (Ollama or nginx elsewhere)
Everything is co-located by default. To move a piece to another host, edit
`mcp.env`:
- **Ollama on another host:** `COMPOSE_PROFILES=` (empty) and
  `OLLAMA_URL=http://<ollama-host>:11434`. The local `ollama` service is then
  skipped and `mcp` talks to the remote one.
- **nginx on another host** (this box is API-only): `MCP_BIND=0.0.0.0` (firewall
  `:3000` to the proxy host), drop the `nginx`/`certbot` services from the
  stack, and point the remote nginx `proxy_pass` at `http://<mcp-host>:3000`.

## GHCR access
CI pushes to `ghcr.io/mnemonik-xyz/mnemonic-mcp`. If the package is **private**,
log the VPS into GHCR once so `compose pull` works:
```bash
echo "$GHCR_PAT" | docker login ghcr.io -u <user> --password-stdin
```
Or set the package visibility to **public** in GitHub → Packages (no login).

## Troubleshooting
```bash
docker compose --env-file mcp.env ps
docker compose --env-file mcp.env logs -f mcp      # server + RAG seed
docker compose --env-file mcp.env logs -f ollama
docker compose --env-file mcp.env logs -f nginx
docker compose --env-file mcp.env exec certbot certbot certificates
curl -fsS http://127.0.0.1:3000/health             # mcp direct (bypass nginx)
```
- **mcp unhealthy on first boot:** it downloads the ~22 MB fastembed model +
  runs the RAG seed; `start_period` is 180s. Check `logs mcp`.
- **nginx won't start / cert errors:** run `init-letsencrypt.sh` (the cert path
  `/etc/letsencrypt/live/mnemonic/` must exist). Use `STAGING=1` while testing
  to avoid LetsEncrypt rate limits.
- **chat slow (30–60s):** CPU inference on a 4 GB box. Upgrade RAM / use a
  bigger `OLLAMA_MODEL`, or point `OLLAMA_URL` at a GPU host.
