import { useState } from "react";

/**
 * Four install entry points for the supported AI tools.
 *
 * Cursor + VS Code use their respective custom URI schemes — both encode the
 * MCP server config in the URL so the user lands directly in the
 * "install MCP server" prompt with the URL pre-filled.
 *
 * Claude.ai has no deeplink scheme yet, so the third action opens a modal
 * with copy-to-clipboard instructions for `Settings → Connectors → Add custom
 * connector`.
 *
 * WindSurf's `windsurf://windsurf-mcp-registry?serverName=` deeplink only
 * accepts entries from WindSurf's first-party MCP registry — it cannot encode
 * an arbitrary remote HTTP MCP URL. Until Mnemonic is published to that
 * registry, we mirror the Claude.ai modal: a copy-the-URL prompt that
 * instructs the user to paste the JSON snippet into
 * `~/.codeium/windsurf/mcp_config.json`.
 * Reference: https://docs.windsurf.com/windsurf/cascade/mcp
 *
 * The MCP HTTP endpoint is `https://mcp.mnemonik.xyz/mcp` (per smithery.yaml +
 * Decision 5). All four integrations point to the same endpoint and reuse the
 * same OAuth flow established in Task 4.
 */

const MCP_URL = "https://mcp.mnemonik.xyz/mcp";
// Claude.ai's "Add custom connector" form expects a full URL incl.
// scheme (https://). Without it, Claude shows "Invalid connector URL"
// before the OAuth flow even starts.
const MCP_HOST = "https://mcp.mnemonik.xyz/mcp";

/**
 * VS Code MCP install-link scheme — opaque URI form per VS Code docs:
 *   https://code.visualstudio.com/docs/copilot/customization/mcp-servers
 *   §"Use MCP install links"
 *
 * MUST be `vscode:mcp/install?<JSON>` (single colon, NO `//`). With the
 * hierarchical `vscode://mcp/install?...` form, VS Code's URI parser
 * yields authority="mcp" + path="/install", which does not match the
 * MCP install handler (registered for path `mcp/install`). Result:
 * VS Code window opens, install dialog never appears.
 *
 * Do not change without verifying in actual VS Code on macOS+Windows.
 * Both InstallButtons.test.tsx (unit) and webapp/e2e/install.spec.ts
 * (Playwright) assert this exact prefix as a regression guard.
 */
export const VSCODE_INSTALL_PREFIX = "vscode:mcp/install?";

/**
 * Read the webapp's current JWT from localStorage IF it exists AND has not
 * expired. Returns null otherwise.
 *
 * Used by the install deeplinks so a logged-in webapp user can hand-off
 * their JWT to the IDE in one click — bypasses Cursor's broken MCP-OAuth
 * UI gap (where the Connect button never appears for non-directory servers
 * and the SDK silently fails to launch the browser on 401).
 *
 * The JWT is short-lived (1h TTL) but rotating once is far cheaper than
 * the manual `~/.cursor/mcp.json` paste workaround.
 */
function readWebappJwt(): string | null {
  if (typeof localStorage === "undefined") return null;
  const jwt = localStorage.getItem("mnemonic.jwt");
  if (!jwt) return null;
  try {
    const parts = jwt.split(".");
    if (parts.length !== 3) return null;
    const payloadB64 = parts[1] ?? "";
    const padded = payloadB64 + "=".repeat((4 - (payloadB64.length % 4)) % 4);
    const payload = JSON.parse(
      atob(padded.replace(/-/g, "+").replace(/_/g, "/"))
    );
    const exp = typeof payload?.exp === "number" ? payload.exp : 0;
    if (exp * 1000 <= Date.now()) return null;
  } catch {
    return null;
  }
  return jwt;
}

/**
 * Build the IDE config object passed via the install deeplink.
 *
 * `bakeJwt` controls whether a non-expired webapp JWT is included as
 * `headers: { Authorization: "Bearer <jwt>" }`. This is intentionally
 * opt-in per IDE:
 *
 *   - Cursor: `bakeJwt = true`. Cursor 3.2.16's MCP UI does not surface
 *     a Connect / Authorize button for non-directory MCP servers, so we
 *     hand the JWT off via the install deeplink to skip OAuth entirely.
 *     Cursor's deeplink JSON validator is lenient and forwards arbitrary
 *     fields through to mcp.json.
 *
 *   - VS Code: `bakeJwt = false`. VS Code's install-link JSON validator
 *     is STRICT — it accepts only `{name, type, url}` for HTTP MCP entries
 *     (per the docs). Including `headers` causes the install dialog to
 *     silently abort: VS Code opens but no UI appears. VS Code 1.93+
 *     handles MCP OAuth natively, so the JWT bake-in isn't needed there
 *     anyway. (Past regression: 2026-05-02 — `headers` field broke VS
 *     Code install for every logged-in user.)
 */
function buildInstallConfig(
  extras: Record<string, unknown> = {},
  bakeJwt: boolean = false
): Record<string, unknown> {
  const config: Record<string, unknown> = {
    url: MCP_URL,
    type: "http",
    ...extras,
  };
  if (bakeJwt) {
    const jwt = readWebappJwt();
    if (jwt) {
      config.headers = { Authorization: `Bearer ${jwt}` };
    }
  }
  return config;
}

function cursorDeeplink(): string {
  // Cursor's `cursor://anysphere.cursor-deeplink/mcp/install` accepts a
  // base64-encoded JSON config. For HTTP MCP servers (streamable HTTP per
  // MCP spec 2025) Cursor expects `{url, type: "http"}` — a `{url}`-only
  // payload is recognized as a URL handle but doesn't open the install
  // dialog reliably across Cursor versions. Including `type: "http"`
  // matches the explicit-transport pattern Cursor docs recommend for
  // remote MCP servers.
  //
  // When the webapp user is logged in, we ALSO include `headers` carrying
  // the current JWT — Cursor's deeplink handler stores it in mcp.json,
  // and every subsequent /mcp call sends `Authorization: Bearer <jwt>`.
  // This bypasses Cursor's MCP-OAuth UI (which doesn't render a Connect
  // button for non-directory servers) and skips the OAuth dance entirely.
  // Reference: https://cursor.com/docs/context/mcp/install-links
  const config = JSON.stringify(buildInstallConfig({}, /* bakeJwt */ true));
  const b64 = btoa(config);
  const params = new URLSearchParams({ name: "Mnemonic", config: b64 });
  return `cursor://anysphere.cursor-deeplink/mcp/install?${params.toString()}`;
}

function vscodeDeeplink(): string {
  // VS Code 1.93+ MCP install deeplink format (per VS Code docs
  // code.visualstudio.com/docs/copilot/customization/mcp-servers
  // → "Use MCP install links"):
  //
  //     vscode:mcp/install?<URL-encoded-JSON-config>
  //
  // Two things are easy to get wrong here — both have shipped as
  // regressions in this repo before, both produce the SAME symptom
  // (VS Code window opens, no install dialog):
  //
  //  1. URL FORM: must be the opaque `vscode:` (single colon, no `//`).
  //     With `vscode://mcp/install?...` VS Code's URI parser yields
  //     authority="mcp" + path="/install"; the install handler matches
  //     on path === "mcp/install" and never fires. macOS launches VS
  //     Code for both forms (scheme-only dispatch), but only the opaque
  //     form actually triggers the install handler. See
  //     VSCODE_INSTALL_PREFIX above.
  //
  //  2. CONFIG SHAPE: VS Code's install-link JSON validator is strict —
  //     accepts ONLY {name, type:"http", url} for HTTP. Extra fields
  //     (e.g. `headers`) make the validator silently reject. The on-disk
  //     mcp.json schema DOES allow `headers` for HTTP entries, but the
  //     install-deeplink schema is a separate, stricter code path.
  //     Therefore: never pass JWT/headers via this deeplink. VS Code
  //     1.93+ handles MCP OAuth natively, so it's not needed.
  //
  // Both regressions actually shipped on 2026-05-02 (a5ee738 + 2fad606),
  // breaking the install for every user until reverted.
  //
  // The whole query string is a single URL-encoded JSON object, NOT
  // multiple `key=value` query params (that's a third easy mis-step:
  // URLSearchParams output is also unrecognized).
  const config = buildInstallConfig(
    { name: "Mnemonic" },
    /* bakeJwt — see fn docs, MUST stay false for VS Code */ false
  );
  return `${VSCODE_INSTALL_PREFIX}${encodeURIComponent(
    JSON.stringify(config)
  )}`;
}

// JSON snippet that goes into `~/.codeium/windsurf/mcp_config.json`. WindSurf
// accepts either `serverUrl` or `url` for remote HTTP MCP entries; we use
// `serverUrl` to match the WindSurf docs' canonical example. Pretty-printed
// (2-space indent) so a paste into the user's config file produces a
// readable diff.
const WINDSURF_CONFIG_SNIPPET = JSON.stringify(
  {
    mcpServers: {
      mnemonic: {
        serverUrl: MCP_URL,
      },
    },
  },
  null,
  2
);

interface InstallButtonsProps {
  onClaudeAiClick?: () => void;
  onWindsurfClick?: () => void;
}

export default function InstallButtons({
  onClaudeAiClick,
  onWindsurfClick,
}: InstallButtonsProps) {
  const [showClaudeModal, setShowClaudeModal] = useState(false);
  const [showWindsurfModal, setShowWindsurfModal] = useState(false);
  const [copied, setCopied] = useState(false);
  const [windsurfCopied, setWindsurfCopied] = useState(false);

  const handleClaudeAi = () => {
    setShowClaudeModal(true);
    onClaudeAiClick?.();
  };

  const handleWindsurf = () => {
    setShowWindsurfModal(true);
    onWindsurfClick?.();
  };

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(MCP_HOST);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // clipboard may be unavailable (HTTP, no user gesture) — ignore.
    }
  };

  const handleCopyWindsurf = async () => {
    try {
      await navigator.clipboard.writeText(WINDSURF_CONFIG_SNIPPET);
      setWindsurfCopied(true);
      setTimeout(() => setWindsurfCopied(false), 2000);
    } catch {
      // clipboard may be unavailable (HTTP, no user gesture) — ignore.
    }
  };

  return (
    <section
      className="space-y-4"
      aria-label="Install Mnemonic in your AI tool"
    >
      <h2 className="text-lg font-semibold text-text-primary">Install</h2>
      <div className="flex flex-col gap-3">
        <a
          href={cursorDeeplink()}
          className="rounded-md border border-text-muted/30 bg-white/5 px-4 py-3 text-sm font-medium text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
          data-testid="install-cursor"
        >
          Install in Cursor
        </a>
        <a
          href={vscodeDeeplink()}
          className="rounded-md border border-text-muted/30 bg-white/5 px-4 py-3 text-sm font-medium text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
          data-testid="install-vscode"
        >
          Install in VS Code
        </a>
        <button
          type="button"
          onClick={handleClaudeAi}
          className="rounded-md border border-text-muted/30 bg-white/5 px-4 py-3 text-left text-sm font-medium text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
          data-testid="install-claude-ai"
        >
          Add to Claude.ai
        </button>
        <button
          type="button"
          onClick={handleWindsurf}
          className="rounded-md border border-text-muted/30 bg-white/5 px-4 py-3 text-left text-sm font-medium text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
          data-testid="install-windsurf"
        >
          Install in WindSurf
        </button>
      </div>

      <p className="text-xs text-text-muted">
        After install, the OAuth flow signs the request using your keypair. Make
        sure your keypair backup is downloaded — losing it means losing access
        to your memories.
      </p>

      {showClaudeModal && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="claude-modal-title"
          className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 px-4"
        >
          <div className="w-full max-w-md rounded-lg border border-text-muted/30 bg-background p-6 shadow-lg">
            <h3
              id="claude-modal-title"
              className="text-base font-semibold text-text-primary"
            >
              Add to Claude.ai
            </h3>
            <p className="mt-3 text-sm text-text-muted">
              Claude.ai does not support deeplinks. Open Claude, go to{" "}
              <span className="font-mono text-text-primary">
                Settings → Connectors → Add custom connector
              </span>{" "}
              and paste:
            </p>
            <div className="mt-4 flex items-center gap-2 rounded-md border border-text-muted/20 bg-white/5 px-3 py-2">
              <code
                className="flex-1 font-mono text-sm text-accent-primary"
                data-testid="claude-paste-url"
              >
                {MCP_HOST}
              </code>
              <button
                type="button"
                onClick={handleCopy}
                className="shrink-0 rounded bg-accent-primary px-3 py-1 text-xs font-medium text-background transition-opacity hover:opacity-90"
                aria-label="Copy connector URL"
              >
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
            <div className="mt-5 flex justify-end">
              <button
                type="button"
                onClick={() => setShowClaudeModal(false)}
                className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {showWindsurfModal && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="windsurf-modal-title"
          className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 px-4"
        >
          <div className="w-full max-w-md rounded-lg border border-text-muted/30 bg-background p-6 shadow-lg">
            <h3
              id="windsurf-modal-title"
              className="text-base font-semibold text-text-primary"
            >
              Install in WindSurf
            </h3>
            <p className="mt-3 text-sm text-text-muted">
              WindSurf&rsquo;s deeplink registry only accepts first-party
              entries, so add Mnemonic by editing{" "}
              <span className="font-mono text-text-primary">
                ~/.codeium/windsurf/mcp_config.json
              </span>{" "}
              and pasting:
            </p>
            <div className="mt-4 rounded-md border border-text-muted/20 bg-white/5 px-3 py-2">
              <pre
                className="overflow-x-auto font-mono text-xs text-accent-primary"
                data-testid="windsurf-config-snippet"
              >
                {WINDSURF_CONFIG_SNIPPET}
              </pre>
              <div className="mt-2 flex justify-end">
                <button
                  type="button"
                  onClick={handleCopyWindsurf}
                  className="shrink-0 rounded bg-accent-primary px-3 py-1 text-xs font-medium text-background transition-opacity hover:opacity-90"
                  aria-label="Copy WindSurf config snippet"
                >
                  {windsurfCopied ? "Copied" : "Copy"}
                </button>
              </div>
            </div>
            <p className="mt-3 text-xs text-text-muted">
              Reload WindSurf, then run any tool — Cascade prompts the OAuth
              flow on the first call.
            </p>
            <div className="mt-5 flex justify-end">
              <button
                type="button"
                onClick={() => setShowWindsurfModal(false)}
                className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
