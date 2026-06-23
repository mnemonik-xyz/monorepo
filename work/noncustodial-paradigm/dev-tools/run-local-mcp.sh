#!/usr/bin/env bash
# Launch a local mnemonic-mcp in `local` mode for live round-trips.
# Generates ephemeral JWT secret + refresh salt if not provided, and reuses the
# fastembed model cache (see fetch-fastembed-model.sh). Prints the JWT secret so
# you can mint a Bearer token with mint-jwt.cjs.
#
# Prereqs: libdbus-1-dev installed; binary built with --features local-embed:
#   cargo build -p mnemonic-mcp --release --features local-embed
#
# Usage: PORT=4000 CACHE_DIR=$HOME/.fe ./run-local-mcp.sh
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
PORT="${PORT:-4000}"
CACHE_DIR="${FASTEMBED_CACHE_DIR:-${CACHE_DIR:-$HOME/.fe}}"
SECRET="${MCP_JWT_SECRET:-$(head -c 32 /dev/urandom | base64)}"
SALT="${MCP_REFRESH_SALT:-$(head -c 32 /dev/urandom | base64)}"

echo "MCP_JWT_SECRET=$SECRET"   # needed by mint-jwt.cjs to mint a Bearer token
echo "listening on http://127.0.0.1:$PORT/mcp"

cd "$ROOT"
exec env \
  FASTEMBED_CACHE_DIR="$CACHE_DIR" HF_HUB_OFFLINE=1 \
  MCP_JWT_SECRET="$SECRET" MCP_REFRESH_SALT="$SALT" \
  STORAGE_MODE=local PAYMENT_MODE=none EMBED_PROVIDER=fastembed \
  ./target/release/mnemonic-mcp --transport http --port "$PORT"
