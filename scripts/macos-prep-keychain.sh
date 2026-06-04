#!/usr/bin/env bash
# Pre-authorize the Mnemonic identity keychain entry on macOS so that any
# Apple-signed CLI tool (and unsigned local binaries like `node` invoked from
# Terminal) can read the secret without triggering the "Always Allow" GUI prompt.
#
# Why: Apple Keychain enforces a per-item ACL ("partition list") that only
# implicitly allows the creating process. When `xyz.mnemonik.identity` is
# created by the Node CLI but later read by the Rust mcp-server (or vice
# versa), macOS prompts for confirmation on each new caller — fine for
# desktop UX, but blocks unattended T15 smoke runs.
#
# This script widens the partition list to `apple-tool:,unsigned:`, which
# covers every reasonable invocation path:
#   - apple-tool: any Apple-signed CLI binary (e.g. /usr/bin/security itself)
#   - unsigned:   anything not code-signed (Homebrew binaries, dev builds,
#                 node from nvm, etc.)
#
# This is the same widening Apple's own keychain-migration tooling and many
# CI helpers use. It does NOT make the secret world-readable — keychain
# access still requires the user's session to be unlocked, and the partition
# list is per-item (only `xyz.mnemonik.identity/default` is affected).
#
# Cost: this script prompts ONCE for your login password (the keychain
# admin password). After it runs, subsequent reads of the entry from any
# tool are silent — including the T15 manual smoke matrix.
#
# Usage:
#   ./scripts/macos-prep-keychain.sh
#
# Idempotent — safe to re-run. Exits 0 if the entry doesn't exist yet
# (run `mnemonic whoami` once first to create it, then re-run this script).

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macos-prep-keychain.sh: this script is macOS-only. On Linux / Docker the keychain ACL concept does not apply (Secret Service has no equivalent per-item gating). Exiting 0." >&2
  exit 0
fi

SERVICE="xyz.mnemonik.identity"
ACCOUNT="default"
KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"

# Probe: does the entry exist?
if ! security find-generic-password -s "$SERVICE" -a "$ACCOUNT" "$KEYCHAIN" > /dev/null 2>&1; then
  echo "macos-prep-keychain.sh: no '$SERVICE/$ACCOUNT' entry in '$KEYCHAIN' yet — run 'mnemonic whoami' first to bootstrap it, then re-run this script." >&2
  exit 0
fi

echo "macos-prep-keychain.sh: widening partition list for '$SERVICE/$ACCOUNT' to allow apple-tool + unsigned callers (you'll be prompted for your login password)..." >&2

# Note: -k <password> is INSECURE (visible to ps) — we omit it and let the
# security CLI prompt interactively. -S sets the partition list. -s + -a
# match the specific item.
security set-generic-password-partition-list \
  -s "$SERVICE" \
  -a "$ACCOUNT" \
  -S "apple-tool:,unsigned:" \
  "$KEYCHAIN"

echo "macos-prep-keychain.sh: done. Subsequent reads of '$SERVICE/$ACCOUNT' will not prompt." >&2
