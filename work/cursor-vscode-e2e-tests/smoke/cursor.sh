#!/usr/bin/env bash
# Tier 2 smoke — Cursor MCP install via deeplink.
#
# What it asserts (file-system level, no AX-opaque UI scraping):
#   1. Cursor is installed and the deeplink scheme is registered.
#   2. Triggering the install URL adds an entry to `~/.cursor/mcp.json`.
#   3. The entry has the expected URL + transport type.
#
# What it does NOT assert:
#   - Whether Cursor's MCP UI shows a Connect button (Cursor-side UX gap).
#   - Whether OAuth flow opens a browser automatically (per Cursor 3.2.16
#     observed behavior — see decisions.md, only first-install pops browser).
#
# Per m13v's note on issue #59: terminal output and AX widgets are too
# fragile to script reliably. The MCP config file IS persisted — that's our
# single source of truth.
#
# Prerequisites:
#   - macOS with Cursor installed and run at least once
#   - cliclick installed: brew install cliclick
#   - First-run permission grant in System Settings → Privacy &
#     Security → Accessibility (allow Terminal / iTerm / your shell host).
#
# Tested with: Cursor 3.2.16 (darwin arm64). Bump the inline version note
# below when you re-run on a different release.

set -euo pipefail

APP_NAME="Mnemonic"
MCP_URL="https://mcp.mnemonik.xyz/mcp"
CONFIG_PATH="${HOME}/.cursor/mcp.json"
TIMEOUT_S=15

red() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
info() { printf '\033[34m[smoke/cursor]\033[0m %s\n' "$*"; }

# ── Pre-flight ──────────────────────────────────────────────────────────────
if [[ ! -d "/Applications/Cursor.app" ]]; then
  red "Cursor.app not found in /Applications — install Cursor first."
  exit 2
fi
if ! command -v cliclick >/dev/null 2>&1; then
  red "cliclick missing — \`brew install cliclick\` first."
  exit 2
fi

info "Cursor version: $(/usr/bin/defaults read /Applications/Cursor.app/Contents/Info CFBundleShortVersionString 2>/dev/null || echo unknown)"
info "MCP URL:        ${MCP_URL}"
info "Config path:    ${CONFIG_PATH}"

# ── Snapshot pre-state ──────────────────────────────────────────────────────
PRE_HASH=""
if [[ -f "${CONFIG_PATH}" ]]; then
  PRE_HASH="$(shasum -a 256 "${CONFIG_PATH}" | awk '{print $1}')"
  info "Pre-state mcp.json sha256: ${PRE_HASH:0:16}..."
fi

# ── Build the install deeplink ──────────────────────────────────────────────
CONFIG_JSON=$(printf '{"url":"%s","type":"http"}' "${MCP_URL}")
CONFIG_B64=$(printf '%s' "${CONFIG_JSON}" | base64 | tr -d '\n')
DEEPLINK="cursor://anysphere.cursor-deeplink/mcp/install?name=${APP_NAME}&config=${CONFIG_B64}"
info "Deeplink: ${DEEPLINK:0:80}..."

# ── Trigger install ─────────────────────────────────────────────────────────
info "Triggering install via macOS URL scheme..."
open "${DEEPLINK}"

# ── Click through the install dialog ────────────────────────────────────────
# Cursor's MCP install dialog shows roughly 1-2 seconds after the deeplink.
# It has an "Install" button. cliclick simulates a Return press to confirm.
sleep 2
info "Pressing Return to confirm install dialog..."
cliclick kp:return || true

# ── Wait for mcp.json to update ─────────────────────────────────────────────
info "Waiting up to ${TIMEOUT_S}s for ${CONFIG_PATH} to update..."
for ((i = 0; i < TIMEOUT_S; i++)); do
  if [[ -f "${CONFIG_PATH}" ]]; then
    POST_HASH="$(shasum -a 256 "${CONFIG_PATH}" | awk '{print $1}')"
    if [[ "${POST_HASH}" != "${PRE_HASH}" ]]; then
      info "Config changed at ${i}s."
      break
    fi
  fi
  sleep 1
done

# ── Assert ───────────────────────────────────────────────────────────────────
if [[ ! -f "${CONFIG_PATH}" ]]; then
  red "FAIL: ${CONFIG_PATH} was not created. Cursor may have rejected the deeplink."
  exit 1
fi

# Parse + validate. Use python instead of jq to avoid an extra brew dep.
python3 - "${CONFIG_PATH}" "${APP_NAME}" "${MCP_URL}" <<'PY'
import json, sys
path, name, url = sys.argv[1], sys.argv[2], sys.argv[3]
cfg = json.load(open(path))
servers = cfg.get("mcpServers") or cfg.get("servers") or {}
entry = servers.get(name)
if not entry:
    print(f"FAIL: {name!r} entry missing from {path}", file=sys.stderr)
    print("Found servers:", list(servers.keys()), file=sys.stderr)
    sys.exit(1)
got_url = entry.get("url") or entry.get("serverUrl")
if got_url != url:
    print(f"FAIL: url mismatch — expected {url!r}, got {got_url!r}", file=sys.stderr)
    sys.exit(1)
got_type = entry.get("type") or entry.get("transport") or "http"
if got_type != "http":
    print(f"FAIL: transport must be http, got {got_type!r}", file=sys.stderr)
    sys.exit(1)
print(f"OK: Mnemonic entry present, url={got_url}, type={got_type}")
PY

green "PASS: Cursor accepted the install deeplink and persisted the entry."
echo
echo "Manual next steps (per m13v's note: AX-opaque, not scripted):"
echo "  1. Open Cursor → click on Mnemonic in MCP servers list"
echo "  2. Look for Connect / Authorize button (Cursor 3.2.16: may not appear)"
echo "  3. If no button — paste a JWT into the entry's headers field manually,"
echo "     restart Cursor, retry tool call. See manual-verify.md for the"
echo "     full ladder."
