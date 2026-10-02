# Standalone container deploy — Mnemonic MCP

This guide describes a **separately provisioned standalone host**. The current
hosted production service instead uses the coding-fabric-owned stack at
`/opt/mnemonik-server` (configurable), its existing data/key mounts and shared
Caddy. The manual [Deploy MCP workflow](../../.github/workflows/deploy-mcp.yml)
reuses that stack; it must not install this standalone nginx/Certbot stack there.
See the [pilot preparation runbook](../../work/protocol-product/pilot-runbook.md)
for candidate evidence, backup and financial-state-preserving rollback gates.

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
| `.github/workflows/deploy-mcp.yml` | Hosted fabric stack only; not a standalone-host installer. |

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
old `keypair/id.json` and a **consistent, offline or SQLite-backup-produced**
`data/attestations.db` into the volumes before step 8. These copy examples assume
the source database is already safely snapshotted and the destination is stopped:

```bash
docker run --rm -v monorepo_mcp-keypair:/k -v "$PWD/keypair":/src alpine \
  cp /src/id.json /k/id.json
docker run --rm -v monorepo_mcp-data:/d -v "$PWD/data":/src alpine \
  cp /src/attestations.db /d/attestations.db
```

## Routine updates
A successful image workflow publishes an image to GHCR; it does not deploy it.
Record the resolved image digest and confirm reader/schema compatibility before
an update. On a standalone host, select the verified image tag in the local
Compose configuration, then pull and recreate the service using that host's
existing project and mounts. Do not recreate data volumes.

For the hosted fabric service, use the manual **Deploy MCP** workflow after
approval. Its `main` ref maps to mutable `latest`; `v*` or `sha-*` refs select
image tags. Check the resolved digest before the change. This workflow changes
the fabric Compose image field; it does not use the standalone `MCP_IMAGE_TAG`
update procedure.

## Rollback
Before changing binaries, take a consistent SQLite backup and preserve keys,
configuration and all financial replay records. A live `.db` copy alone can
omit WAL transactions. Test the previous reader against current data in staging.
Do not replace the live financial database with an older snapshot or remove
volumes to make rollback work. Use the verified previous image with existing
mounts and verify paid-operation retries, artifact reads and identity afterward.
The hosted workflow's `rollback` action replaces an image; it does not prove
schema compatibility or automatically back up and reconcile financial state.

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
