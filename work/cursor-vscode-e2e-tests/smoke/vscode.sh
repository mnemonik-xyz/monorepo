#!/usr/bin/env bash
# Tier 2 smoke — VS Code MCP install via deeplink.
#
# Mirrors smoke/cursor.sh but for VS Code's `vscode://mcp/install?<json>`
# deeplink format (see webapp/src/components/InstallButtons.tsx for why
# the double-slash form is required for Safari compat).
#
# Tested with: Visual Studio Code 1.93+ + GitHub Copilot Chat extension
# (which is what handles MCP servers).
#
# VS Code's MCP config lives at one of:
#   - User scope:        ~/Library/Application Support/Code/User/mcp.json
#                       (or settings.json with `"github.copilot.chat.mcp.servers": {...}`)
#   - Workspace scope:   .vscode/mcp.json
# We assert against the user-scope path since that's where the install
# deeplink lands.

set -euo pipefail

APP_NAME="Mnemonic"
MCP_URL="https://mcp.mnemonik.xyz/mcp"
CONFIG_CANDIDATES=(
  "${HOME}/Library/Application Support/Code/User/mcp.json"
  "${HOME}/Library/Application Support/Code - Insiders/User/mcp.json"
)
TIMEOUT_S=15

red() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
info() { printf '\033[34m[smoke/vscode]\033[0m %s\n' "$*"; }

# ── Pre-flight ──────────────────────────────────────────────────────────────
VSCODE_PATH=""
for cand in "/Applications/Visual Studio Code.app" "/Applications/Visual Studio Code - Insiders.app"; do
  if [[ -d "${cand}" ]]; then
    VSCODE_PATH="${cand}"
    break
  fi
done
if [[ -z "${VSCODE_PATH}" ]]; then
  red "VS Code not found in /Applications — install VS Code (or Insiders) first."
  exit 2
fi
if ! command -v cliclick >/dev/null 2>&1; then
  red "cliclick missing — \`brew install cliclick\` first."
  exit 2
fi

info "VS Code path: ${VSCODE_PATH}"
info "MCP URL:      ${MCP_URL}"

# ── Snapshot pre-state ──────────────────────────────────────────────────────
declare -A PRE_HASH
for cfg in "${CONFIG_CANDIDATES[@]}"; do
  if [[ -f "${cfg}" ]]; then
    PRE_HASH["${cfg}"]="$(shasum -a 256 "${cfg}" | awk '{print $1}')"
  fi
done

# ── Build the install deeplink (vscode:// double-slash for Safari compat) ───
JSON=$(printf '{"name":"%s","type":"http","url":"%s"}' "${APP_NAME}" "${MCP_URL}")
ENC_JSON=$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "${JSON}")
DEEPLINK="vscode://mcp/install?${ENC_JSON}"
info "Deeplink: ${DEEPLINK:0:80}..."

# ── Trigger install ─────────────────────────────────────────────────────────
info "Triggering install via macOS URL scheme..."
open "${DEEPLINK}"

# ── Click through ──────────────────────────────────────────────────────────
sleep 3
info "Pressing Return to confirm install dialog..."
cliclick kp:return || true

# ── Wait for any candidate config to update ────────────────────────────────
info "Waiting up to ${TIMEOUT_S}s for VS Code MCP config to update..."
UPDATED_PATH=""
for ((i = 0; i < TIMEOUT_S; i++)); do
  for cfg in "${CONFIG_CANDIDATES[@]}"; do
    if [[ -f "${cfg}" ]]; then
      POST="$(shasum -a 256 "${cfg}" | awk '{print $1}')"
      if [[ "${POST}" != "${PRE_HASH[$cfg]:-}" ]]; then
        UPDATED_PATH="${cfg}"
        break 2
      fi
    fi
  done
  sleep 1
done

if [[ -z "${UPDATED_PATH}" ]]; then
  red "FAIL: no VS Code MCP config file changed. VS Code may have rejected the deeplink, or your VS Code uses a different config schema (settings.json embedded MCP)."
  echo "Checked paths:"
  for cfg in "${CONFIG_CANDIDATES[@]}"; do echo "  ${cfg}"; done
  exit 1
fi

info "Updated config: ${UPDATED_PATH}"

# ── Assert ─────────────────────────────────────────────────────────────────
python3 - "${UPDATED_PATH}" "${APP_NAME}" "${MCP_URL}" <<'PY'
import json, sys
path, name, url = sys.argv[1], sys.argv[2], sys.argv[3]
cfg = json.load(open(path))
servers = cfg.get("servers") or cfg.get("mcpServers") or {}
entry = servers.get(name)
if not entry:
    print(f"FAIL: {name!r} entry missing", file=sys.stderr)
    print("Found servers:", list(servers.keys()), file=sys.stderr)
    sys.exit(1)
got_url = entry.get("url")
if got_url != url:
    print(f"FAIL: url mismatch — expected {url!r}, got {got_url!r}", file=sys.stderr)
    sys.exit(1)
print(f"OK: VS Code MCP config has Mnemonic entry pointing at {got_url}")
PY

green "PASS: VS Code accepted the install deeplink and persisted the entry."
