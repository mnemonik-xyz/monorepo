# T15 — Claude Computer Use launcher

Wraps [Anthropic's `computer-use-demo` container](https://github.com/anthropics/anthropic-quickstarts/tree/main/computer-use-demo) with the T15 smoke-matrix scenario pre-baked. Spin up, paste a prompt, watch Claude drive the Ubuntu desktop.

## What this covers

| T15 row | Supported here? |
|---|---|
| `ubuntu22_headless` | ✅ container IS ubuntu22; no keyring daemon started |
| `ubuntu22_keyring` | ✅ `init.sh` installs gnome-keyring + dbus-x11 and launches the daemon |
| `docker_alpine` | ⚠️ partial — needs `--privileged` or socket mount for nested docker; see [docker_alpine row](#docker_alpine-row) below |
| `windows11` | ❌ no Anthropic reference container for Windows; use a Win11 VM with VNC or run the human checklist |
| `macos14` | ❌ no Anthropic reference container for macOS; run the human checklist on your Mac or use [Tart](https://tart.run) / GHA `macos-15` |

## Prerequisites

- Docker Desktop or Docker Engine 24+ with `docker compose` v2
- `ANTHROPIC_API_KEY` from console.anthropic.com (Claude API access; CU feature must be enabled on your account)
- ~6 GB free disk for the image + named volumes
- First run downloads the image (~3 GB) and builds the Rust + Node artifacts inside the container (~5–10 min). Subsequent runs reuse the named volumes — ~30s startup.

## Quick start

```bash
cd /Users/syi/src/sessions/monorepo

export ANTHROPIC_API_KEY=sk-ant-...

# Pick one row at a time:
T15_PLATFORM=ubuntu22_headless  docker compose -f scripts/t15-cu/compose.yml up -d
# or
T15_PLATFORM=ubuntu22_keyring   docker compose -f scripts/t15-cu/compose.yml up -d
```

Open <http://localhost:8080> in your browser. The combined view shows the Claude chat on the left and the live container desktop on the right.

**First action inside the chat:** ask Claude to open a terminal and run

```bash
bash /work/monorepo/scripts/t15-cu/init.sh
```

After ~5–10 min on first run, the init script:

1. Installs platform-specific deps (gnome-keyring etc.)
2. Installs Node 20 + Rust stable
3. Builds `mnemonic-mcp --release` and `@mnemonik-xyz/cli`
4. Starts the keyring daemon if the row needs one
5. Writes the **copy-paste prompt** to `~/Desktop/T15-PROMPT.md`

Then `cat ~/Desktop/T15-PROMPT.md` (or open it in the in-container text editor) and paste the contents into the Claude chat. Claude reads the T15 scenario file, runs all 6 steps, and writes the JSON result to a path on the host (because the repo is bind-mounted):

```
work/invisible-identity/logs/working/T15-smoke-result-<platform>.json
```

When the run is done:

```bash
docker compose -f scripts/t15-cu/compose.yml down
```

The named volumes persist (so the next platform run reuses cached builds). To wipe them:

```bash
docker compose -f scripts/t15-cu/compose.yml down -v
```

## Optional env vars

| Variable | Purpose | Default |
|---|---|---|
| `T15_PLATFORM` | Which row to run | `ubuntu22_headless` |
| `T15_WEBAPP_URL` | Base URL for Step 6 redemption (e.g. `http://host.docker.internal:5173` to talk to a local webapp dev server) | unset → Step 6 deferred |
| `T15_WEBAPP_AUTH_COOKIE` | Pre-baked auth cookie for Step 6 unattended browser redemption | unset → Step 6 marked `deferred: requires_webapp_login` |
| `OPENAI_API_KEY` | The mcp binary needs *some* embedder; `openai` + a fake key works because the T15 scenario never invokes `embed()` | `test-not-real` |

## docker_alpine row

The alpine row exists to verify the file-fallback path on a fully sandboxed musl-libc environment. Two ways to do it:

**Sibling container (recommended)** — easier, no privileges needed. From the host (NOT inside the CU container):

```bash
docker run --rm -it \
  -v "$PWD":/work/monorepo \
  -w /work/monorepo \
  -e MNEMONIC_QUIET=1 -e STORAGE_MODE=local -e PAYMENT_MODE=none \
  -e EMBED_PROVIDER=openai -e OPENAI_API_KEY=test \
  node:20-alpine sh -c '
    cd packages/cli && npm install --omit=dev
    node ./dist/bin/mnemonic.js whoami
    cat ~/.mnemonic/identity.json
    node ./dist/bin/mnemonic.js identity status --json
  '
```

Run this from a terminal on the host and treat the output as the row's evidence. Claude doesn't need to drive it.

**Nested docker (alternative)** — agent runs it inside the CU container. Requires adding to `compose.yml`:

```yaml
    privileged: true        # or:
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
```

This widens the container's blast radius, so we don't enable it by default. Add the line yourself if you really want Claude to drive nested docker.

## Cost estimate

| Row | Approx Claude turns | API cost (Opus 4.7) | Wall-clock |
|---|---|---|---|
| `ubuntu22_headless` | 10–15 | $0.50–$1.00 | 3–5 min after init |
| `ubuntu22_keyring` | 12–20 | $0.70–$1.50 | 4–6 min after init |
| `docker_alpine` (sibling) | 0 (host-driven) | $0 | 2 min |
| **Total** | — | **~$1.50–$2.50** | ~15 min for both Linux rows + alpine |

Add ~10 min for the first-time init (image pull + cargo + npm build). Subsequent runs of the same row are instant.

## Layout

```
scripts/t15-cu/
├── compose.yml      # docker compose: bind-mount + named volumes + ports + env
├── init.sh          # in-container setup, idempotent
├── README.md        # this file
```

## Troubleshooting

- **Port 8080 already in use** — change the port mapping in `compose.yml` (e.g. `"8081:8080"`).
- **Image pull fails** — Anthropic's image is on GitHub Container Registry (`ghcr.io`); make sure docker can reach it (no corporate proxy blocking ghcr). Check with `docker pull ghcr.io/anthropics/anthropic-quickstarts:computer-use-demo-latest`.
- **`cargo build` runs out of memory** — Rust uses a lot of RAM during link. In Docker Desktop, raise the VM memory to 8 GB+ (Settings → Resources).
- **Keyring daemon won't start in `ubuntu22_keyring`** — the CU container's desktop may not have D-Bus running. The init script tries `dbus-launch` first; if that fails, run inside the container: `eval "$(dbus-launch --sh-syntax)"` then re-run `init.sh`.
- **Claude refuses to run because "computer use" not enabled on your account** — go to console.anthropic.com → check Settings → enable the beta. CU is currently in public beta but may require explicit opt-in on some account tiers.

## See also

- The full scenario: [`work/invisible-identity/scenarios/T15-smoke-matrix.md`](../../work/invisible-identity/scenarios/T15-smoke-matrix.md)
- Human-driven alternative: `work/invisible-identity/logs/working/T15-smoke-matrix-checklist.md` (local-only)
- macOS pre-auth helper: [`scripts/macos-prep-keychain.sh`](../macos-prep-keychain.sh)
