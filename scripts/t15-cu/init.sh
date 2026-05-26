#!/usr/bin/env bash
# T15 in-container init — run from the agent's bash tool ONCE per container.
#
# Idempotent. Installs platform-specific deps, starts the keyring daemon for
# the keyring row, and builds the Rust mcp + Node CLI artifacts. Result paths
# (the binary, the CLI dist/, the fixtures) end up at the locations the T15
# scenario references via $T15_REPO_PATH.
#
# The agent should run this first thing, then run the T15 scenario steps.
#
# Usage (inside the CU container):
#   bash /work/monorepo/scripts/t15-cu/init.sh
#
# Env (set by compose.yml):
#   T15_PLATFORM   ubuntu22_keyring | ubuntu22_headless | docker_alpine
#   T15_REPO_PATH  /work/monorepo

set -euo pipefail

: "${T15_PLATFORM:=ubuntu22_headless}"
: "${T15_REPO_PATH:=/work/monorepo}"

log() { printf '[t15-init] %s\n' "$*" >&2; }

# -----------------------------------------------------------------------------
# Sanity
# -----------------------------------------------------------------------------
log "platform: $T15_PLATFORM | repo: $T15_REPO_PATH"
test -d "$T15_REPO_PATH" || { log "ERROR: $T15_REPO_PATH not mounted"; exit 1; }
cd "$T15_REPO_PATH"

# -----------------------------------------------------------------------------
# 1. Platform-specific packages
# -----------------------------------------------------------------------------
case "$T15_PLATFORM" in
  ubuntu22_keyring)
    log "installing gnome-keyring + dbus-x11 + libsecret-tools + jq"
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      gnome-keyring dbus-x11 libsecret-tools jq curl ca-certificates
    ;;
  ubuntu22_headless)
    log "installing jq + curl only (no keyring)"
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      jq curl ca-certificates
    ;;
  docker_alpine)
    log "installing docker.io for docker-in-docker + jq"
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      docker.io jq curl ca-certificates
    # The container would need `--privileged` or socket mount to run nested
    # docker. Defer the actual alpine run to the agent; we just stage the
    # tooling here.
    log "NOTE: nested 'docker run alpine' requires --privileged or mounting the host docker socket; see scripts/t15-cu/README.md"
    ;;
  *)
    log "ERROR: unsupported T15_PLATFORM=$T15_PLATFORM (expected ubuntu22_keyring | ubuntu22_headless | docker_alpine)"
    exit 2
    ;;
esac

# -----------------------------------------------------------------------------
# 2. Toolchain — Node 20 + Rust stable
# -----------------------------------------------------------------------------
if ! command -v node >/dev/null 2>&1 || [ "$(node -v | cut -dv -f2 | cut -d. -f1)" -lt 20 ]; then
  log "installing Node 20 LTS"
  curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
  sudo apt-get install -y -qq nodejs
fi
log "node: $(node -v) | npm: $(npm -v)"

if ! command -v cargo >/dev/null 2>&1; then
  log "installing rustup + stable toolchain"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi
log "cargo: $(cargo --version)"

# -----------------------------------------------------------------------------
# 3. Build mnemonic-mcp (Rust)
# -----------------------------------------------------------------------------
if [ ! -x "$T15_REPO_PATH/target/release/mnemonic-mcp" ]; then
  log "building mnemonic-mcp --release (first time ~5-10 min in a clean container)"
  cargo build -p mnemonic-mcp --release
else
  log "mnemonic-mcp already built (cached in t15-cargo-target volume)"
fi

# -----------------------------------------------------------------------------
# 4. Install + build Node CLI
# -----------------------------------------------------------------------------
if [ ! -f "$T15_REPO_PATH/packages/cli/dist/bin/mnemonic.js" ]; then
  log "running npm install + build for @mnemonik-xyz/cli"
  npm install --workspaces --include-workspace-root --no-audit --no-fund
  npm run build --workspace=@mnemonik-xyz/cli
else
  log "@mnemonik-xyz/cli already built (cached in t15-cli-node-modules volume)"
fi

# -----------------------------------------------------------------------------
# 5. Start the keyring daemon (ubuntu22_keyring only)
# -----------------------------------------------------------------------------
if [ "$T15_PLATFORM" = "ubuntu22_keyring" ]; then
  if ! pgrep -x gnome-keyring-d >/dev/null 2>&1; then
    log "launching D-Bus + gnome-keyring-daemon"
    # The Anthropic CU container runs a desktop session under xvfb that may
    # not have D-Bus already. Idempotent setup:
    if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
      eval "$(dbus-launch --sh-syntax)"
      export DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID
    fi
    # Unlock with an empty password (test environment; no real secrets).
    echo -n "" | gnome-keyring-daemon --unlock --components=secrets >/dev/null 2>&1 || true
    gnome-keyring-daemon --start --components=secrets >/dev/null 2>&1 &
    sleep 1
    log "gnome-keyring-daemon started (pid $!)"
  else
    log "gnome-keyring-daemon already running"
  fi
fi

# -----------------------------------------------------------------------------
# 6. Drop the copy-paste prompt onto the desktop
# -----------------------------------------------------------------------------
PROMPT_DST="$HOME/Desktop/T15-PROMPT.md"
mkdir -p "$(dirname "$PROMPT_DST")"

cat > "$PROMPT_DST" <<EOF
# T15 — paste this into the Claude chat

You are a Computer Use agent. Run the T15 cross-platform smoke matrix scenario at:

  $T15_REPO_PATH/work/invisible-identity/scenarios/T15-smoke-matrix.md

Use these inputs (already exported in the shell environment, but listed here for clarity):

  T15_PLATFORM           $T15_PLATFORM
  T15_REPO_PATH          $T15_REPO_PATH
  T15_WEBAPP_URL         ${T15_WEBAPP_URL:-(unset — Step 6 will be deferred)}
  T15_WEBAPP_AUTH_COOKIE ${T15_WEBAPP_AUTH_COOKIE:+set} ${T15_WEBAPP_AUTH_COOKIE:-(unset)}
  T15_OUTPUT_DIR         /work/monorepo/work/invisible-identity/logs/working

Pre-flight is DONE — the init script already:
  - installed platform deps for $T15_PLATFORM
  - installed Node 20 + Rust stable
  - built mnemonic-mcp --release and @mnemonik-xyz/cli
  - (for ubuntu22_keyring) started D-Bus + gnome-keyring-daemon

So you can skip §1 of the scenario file (pre-flight verification) and start directly at §2 Step 1 (clean state).

Follow §2–§7 verbatim. Run all 6 steps even if early ones fail (capture diagnostics, continue). After cleanup (§3), write the per-platform JSON result to:

  /work/monorepo/work/invisible-identity/logs/working/T15-smoke-result-$T15_PLATFORM.json

Then return a one-paragraph human summary at the end with the verdict (pass / fail / blocked), counts of failed steps, and any unexpected interruptions.

Failure handling rules (§5) are BINDING:
  - never modify source code
  - never commit, push, merge, or open PRs
  - never inject keystrokes into unexpected UI surfaces — only the GUI events specified in steps
  - 60s per-step timeout
  - cleanup (§3) runs even on early termination
  - NEVER log the contents of identity.json's "secret" field or any keychain entry bytes

Begin.
EOF

log "prompt written to $PROMPT_DST"
log "open it via the in-container file manager or 'cat' it in the terminal, then paste into the Claude chat."

log "init complete. T15 ready for $T15_PLATFORM."
