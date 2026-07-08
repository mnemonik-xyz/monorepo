#!/usr/bin/env bash
#
# One-time TLS bootstrap for the compose stack. Solves the nginx<->certbot
# chicken-and-egg: nginx won't start without a cert, certbot needs nginx to
# serve the ACME challenge. Creates a throwaway self-signed cert at the stable
# path `/etc/letsencrypt/live/mnemonic/`, starts nginx, then replaces it with a
# real LetsEncrypt cert. Idempotent — re-running with an existing real cert is
# a no-op (certbot sees it as up to date).
#
#   cd /home/claude/monorepo
#   ./mcp/deploy/init-letsencrypt.sh
#
# Reads DOMAIN + CERTBOT_EMAIL from mcp.env. Override the cert SANs with DOMAINS
# (space-separated) for the apex + MCP subdomain, e.g.:
#   DOMAINS="mnemonik.xyz mcp.mnemonik.xyz" ./mcp/deploy/init-letsencrypt.sh
set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root (where docker-compose.yml lives)

# shellcheck disable=SC1091
set -a; [ -f mcp.env ] && . ./mcp.env; set +a

DOMAIN=${DOMAIN:?set DOMAIN in mcp.env}
CERTBOT_EMAIL=${CERTBOT_EMAIL:?set CERTBOT_EMAIL in mcp.env}
DOMAINS=${DOMAINS:-"$DOMAIN mcp.$DOMAIN"}
CERT_NAME=mnemonic
STAGING=${STAGING:-0}   # set 1 to hit LetsEncrypt staging while testing

compose() { docker compose --env-file mcp.env "$@"; }
live="/etc/letsencrypt/live/${CERT_NAME}"

echo "[init-tls] domains: ${DOMAINS}  email: ${CERTBOT_EMAIL}  staging: ${STAGING}"

# 1. Seed a self-signed cert so nginx can bind :443 on first boot.
echo "[init-tls] creating throwaway self-signed cert at ${live}"
compose run --rm --entrypoint sh certbot -c "\
  mkdir -p '${live}' && \
  openssl req -x509 -nodes -newkey rsa:2048 -days 1 \
    -keyout '${live}/privkey.pem' -out '${live}/fullchain.pem' \
    -subj '/CN=localhost'"

# 2. Bring up nginx (+ its deps) so the ACME webroot is reachable.
echo "[init-tls] starting nginx"
compose up -d nginx

# 3. Wipe the dummy and request the real cert via the webroot challenge.
echo "[init-tls] requesting real certificate"
domain_args=""; for d in $DOMAINS; do domain_args="$domain_args -d $d"; done
staging_arg=""; [ "$STAGING" = "1" ] && staging_arg="--staging"
compose run --rm --entrypoint sh certbot -c "\
  rm -rf '/etc/letsencrypt/live/${CERT_NAME}' \
         '/etc/letsencrypt/archive/${CERT_NAME}' \
         '/etc/letsencrypt/renewal/${CERT_NAME}.conf'; \
  certbot certonly --webroot -w /var/www/certbot \
    --cert-name '${CERT_NAME}' ${domain_args} \
    --email '${CERTBOT_EMAIL}' --agree-tos --no-eff-email \
    --non-interactive ${staging_arg}"

# 4. Reload nginx with the real cert.
echo "[init-tls] reloading nginx"
compose exec nginx nginx -s reload
echo "[init-tls] done. TLS is live for: ${DOMAINS}"
