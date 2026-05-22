#!/usr/bin/env bash
# Cross-language keychain interop test.
# Drives both Rust mnemonic-mcp and Node @mnemonik-xyz/cli against the same
# gnome-keyring (or OS keychain) and asserts byte-equality of secrets.
#
# Test structure:
#   A. Rust writes to keychain → Node reads, compare pubkeys
#   B. Node writes to keychain → Rust reads, compare pubkeys
#   C. Legacy file migration: Rust migrates legacy-identity.json to keychain
#      stub → Node reads from migrated stub
#
# Environment: MNEMONIC_QUIET=1 suppresses bootstrap stderr noise.
# Exit on first failure, cleanup keychain on exit.

set -euo pipefail

# Cleanup trap: remove keychain entry and temp homes on exit
cleanup() {
  # Clear the keychain entry (suppress errors if not present)
  secret-tool clear service xyz.mnemonik.identity account default 2>/dev/null || true
  # Remove temp directories
  [ -n "${HOME_A:-}" ] && rm -rf "$HOME_A" || true
  [ -n "${HOME_B:-}" ] && rm -rf "$HOME_B" || true
  [ -n "${HOME_C:-}" ] && rm -rf "$HOME_C" || true
}
trap cleanup EXIT

# Configure environment for both sides
export MNEMONIC_QUIET=1
export STORAGE_MODE=local
export PAYMENT_MODE=none
export EMBED_PROVIDER=openai
export OPENAI_API_KEY=test

# Resolve paths relative to repo root
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MCP_BINARY="${REPO_ROOT}/target/debug/mnemonic-mcp"
NODE_CLI="${REPO_ROOT}/packages/cli/dist/cli.js"
LEGACY_FIXTURE="${REPO_ROOT}/tests/fixtures/legacy-identity.json"

# === Sub-test A: Rust write → Node read ===
echo "=== A: Rust write → Node read ==="

HOME_A=$(mktemp -d)
export HOME="$HOME_A"

# Rust mcp-server: run mnemonic_whoami via stdio
RUST_RESPONSE=$(
  "$MCP_BINARY" --transport stdio 2>/dev/null <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemonic_whoami","arguments":{}}}
EOF
)

# Parse Rust response: result.public_key (the `whoami` tool returns "public_key")
RUST_PUBKEY=$(echo "$RUST_RESPONSE" | jq -r '.result.public_key')
echo "Rust pubkey: $RUST_PUBKEY"

# Node CLI: run whoami --json
NODE_RESPONSE=$(node "$NODE_CLI" whoami --json 2>/dev/null || echo '{}')
NODE_PUBKEY=$(echo "$NODE_RESPONSE" | jq -r '.pubkey')
echo "Node pubkey: $NODE_PUBKEY"

if [ "$RUST_PUBKEY" != "$NODE_PUBKEY" ]; then
  echo "FAIL A: pubkey mismatch: Rust=$RUST_PUBKEY vs Node=$NODE_PUBKEY"
  exit 1
fi
echo "PASS A: pubkeys match"

# === Sub-test B: Node write → Rust read ===
echo ""
echo "=== B: Node write → Rust read ==="

HOME_B=$(mktemp -d)
export HOME="$HOME_B"

# Clear keychain
secret-tool clear service xyz.mnemonik.identity account default 2>/dev/null || true

# Node CLI: run whoami --json (creates identity on first run)
NODE_RESPONSE_B=$(node "$NODE_CLI" whoami --json 2>/dev/null || echo '{}')
NODE_PUBKEY_B=$(echo "$NODE_RESPONSE_B" | jq -r '.pubkey')
echo "Node pubkey: $NODE_PUBKEY_B"

# Rust mcp-server: run mnemonic_whoami via stdio
RUST_RESPONSE_B=$(
  "$MCP_BINARY" --transport stdio 2>/dev/null <<'EOF'
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mnemonic_whoami","arguments":{}}}
EOF
)

RUST_PUBKEY_B=$(echo "$RUST_RESPONSE_B" | jq -r '.result.public_key')
echo "Rust pubkey: $RUST_PUBKEY_B"

if [ "$RUST_PUBKEY_B" != "$NODE_PUBKEY_B" ]; then
  echo "FAIL B: pubkey mismatch: Node=$NODE_PUBKEY_B vs Rust=$RUST_PUBKEY_B"
  exit 1
fi
echo "PASS B: pubkeys match"

# === Sub-test C: Legacy migration ===
echo ""
echo "=== C: Legacy migration → Rust migrates → Node reads ==="

HOME_C=$(mktemp -d)
export HOME="$HOME_C"

# Clear keychain
secret-tool clear service xyz.mnemonik.identity account default 2>/dev/null || true

# Set up legacy file
mkdir -p "$HOME_C/.mnemonic"
cp "$LEGACY_FIXTURE" "$HOME_C/.mnemonic/identity.json"

# Extract expected pubkey from legacy file
LEGACY_PUBKEY=$(jq -r '.pubkey_base58' "$HOME_C/.mnemonic/identity.json")
echo "Legacy pubkey from fixture: $LEGACY_PUBKEY"

# Rust mcp-server: run mnemonic_whoami (should migrate legacy → stub + keychain)
RUST_RESPONSE_C=$(
  "$MCP_BINARY" --transport stdio 2>/dev/null <<'EOF'
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mnemonic_whoami","arguments":{}}}
EOF
)

RUST_PUBKEY_C=$(echo "$RUST_RESPONSE_C" | jq -r '.result.public_key')
echo "Rust pubkey after migration: $RUST_PUBKEY_C"

if [ "$RUST_PUBKEY_C" != "$LEGACY_PUBKEY" ]; then
  echo "FAIL C.1: migration changed pubkey: expected=$LEGACY_PUBKEY vs got=$RUST_PUBKEY_C"
  exit 1
fi
echo "PASS C.1: pubkey preserved after migration"

# Verify identity.json was migrated to stub (contains keychain_ref)
if ! jq -e '.keychain_ref' "$HOME_C/.mnemonic/identity.json" > /dev/null 2>&1; then
  echo "FAIL C.2: identity.json not migrated to stub format (missing keychain_ref)"
  exit 1
fi
echo "PASS C.2: identity.json migrated to stub format"

# Node CLI: run whoami --json (should read from stub + keychain)
NODE_RESPONSE_C=$(node "$NODE_CLI" whoami --json 2>/dev/null || echo '{}')
NODE_PUBKEY_C=$(echo "$NODE_RESPONSE_C" | jq -r '.pubkey')
echo "Node pubkey after migration: $NODE_PUBKEY_C"

if [ "$NODE_PUBKEY_C" != "$LEGACY_PUBKEY" ]; then
  echo "FAIL C.3: Node pubkey mismatch after migration: expected=$LEGACY_PUBKEY vs got=$NODE_PUBKEY_C"
  exit 1
fi
echo "PASS C.3: Node reads migrated stub correctly"

echo ""
echo "ALL CROSS-LANG TESTS PASSED"
