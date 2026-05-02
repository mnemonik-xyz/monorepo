#!/usr/bin/env bash
# Tier 2 smoke — Claude Desktop MCP install (config-file path, no deeplink).
#
# Claude Desktop does not support a `claude://` MCP install deeplink. The
# canonical install method is editing
# ~/Library/Application Support/Claude/claude_desktop_config.json
# directly and restarting the app.
#
# Per the user's report (2026-05-02): "Claude desktop seems working — not
# having such problem". This script is a positive control — it asserts the
# config we ship works on the platform that's known-good, so a regression
# anywhere else can be triaged against this baseline.
#
# What it does:
#   1. Read existing config (preserve other servers).
#   2. Add / overwrite the "Mnemonic" entry pointing at our hosted MCP.
#   3. Restart Claude Desktop (kill + relaunch).
#   4. Assert the config file was written.
#
# What it does NOT do:
#   - Drive the OAuth flow (Claude Desktop handles this on first tool call).
#   - Verify a tool call returns a JWT-protected response.

set -euo pipefail

APP_NAME="Mnemonic"
MCP_URL="https://mcp.mnemonik.xyz/mcp"
CONFIG_PATH="${HOME}/Library/Application Support/Claude/claude_desktop_config.json"
APP_PATH="/Applications/Claude.app"

red() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
info() { printf '\033[34m[smoke/claude-desktop]\033[0m %s\n' "$*"; }

# ── Pre-flight ──────────────────────────────────────────────────────────────
if [[ ! -d "${APP_PATH}" ]]; then
  red "Claude.app not found in /Applications — install Claude Desktop first."
  exit 2
fi

mkdir -p "$(dirname "${CONFIG_PATH}")"

info "Claude Desktop version: $(/usr/bin/defaults read "${APP_PATH}/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo unknown)"
info "Config path: ${CONFIG_PATH}"

# ── Merge config ────────────────────────────────────────────────────────────
python3 - "${CONFIG_PATH}" "${APP_NAME}" "${MCP_URL}" <<'PY'
import json, os, sys
path, name, url = sys.argv[1], sys.argv[2], sys.argv[3]

cfg = {}
if os.path.exists(path):
    try:
        with open(path) as f:
            cfg = json.load(f)
    except json.JSONDecodeError:
        print(f"WARN: {path} is malformed; overwriting", file=sys.stderr)
        cfg = {}

mcp_servers = cfg.setdefault("mcpServers", {})
mcp_servers[name] = {"url": url, "type": "http"}

with open(path, "w") as f:
    json.dump(cfg, f, indent=2)
print(f"wrote {path}: {name} -> {url}")
PY

# ── Restart Claude Desktop ─────────────────────────────────────────────────
info "Restarting Claude Desktop..."
osascript -e 'tell application "Claude" to quit' >/dev/null 2>&1 || true
sleep 2
open -a "Claude"

# ── Assert ────────────────────────────────────────────────────────────────
python3 - "${CONFIG_PATH}" "${APP_NAME}" "${MCP_URL}" <<'PY'
import json, sys
path, name, url = sys.argv[1], sys.argv[2], sys.argv[3]
cfg = json.load(open(path))
entry = cfg.get("mcpServers", {}).get(name)
if not entry:
    print("FAIL: entry not found after write", file=sys.stderr)
    sys.exit(1)
if entry.get("url") != url:
    print(f"FAIL: url mismatch", file=sys.stderr)
    sys.exit(1)
print("OK: claude_desktop_config.json contains Mnemonic entry.")
PY

green "PASS: Claude Desktop config updated. Wait ~10s, then trigger any"
green "Mnemonic tool from the chat. First call should pop the OAuth browser."
