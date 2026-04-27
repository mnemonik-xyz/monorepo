import { useState } from "react";

/**
 * Three deeplink buttons for the supported AI tools.
 *
 * Cursor + VS Code use their respective custom URI schemes — both encode the
 * MCP server config in the URL so the user lands directly in the
 * "install MCP server" prompt with the URL pre-filled.
 *
 * Claude.ai has no deeplink scheme yet, so the third action opens a modal
 * with copy-to-clipboard instructions for `Settings → Connectors → Add custom
 * connector`.
 *
 * The MCP HTTP endpoint is `https://mcp.mnemonik.xyz/mcp` (per smithery.yaml +
 * Decision 5). All three integrations point to the same endpoint and reuse the
 * same OAuth flow established in Task 4.
 */

const MCP_URL = "https://mcp.mnemonik.xyz/mcp";
const MCP_HOST = "mcp.mnemonik.xyz";

function cursorDeeplink(): string {
  // Cursor's `cursor://anysphere.cursor-deeplink/mcp/install` accepts a
  // base64-encoded JSON config. For HTTP MCP servers (streamable HTTP per
  // MCP spec 2025) Cursor expects `{url, type: "http"}` — a `{url}`-only
  // payload is recognized as a URL handle but doesn't open the install
  // dialog reliably across Cursor versions. Including `type: "http"`
  // matches the explicit-transport pattern Cursor docs recommend for
  // remote MCP servers.
  // Reference: https://cursor.com/docs/context/mcp/install-links
  const config = JSON.stringify({ url: MCP_URL, type: "http" });
  const b64 = btoa(config);
  const params = new URLSearchParams({ name: "Mnemonic", config: b64 });
  return `cursor://anysphere.cursor-deeplink/mcp/install?${params.toString()}`;
}

function vscodeDeeplink(): string {
  // VS Code 1.93+ MCP install deeplink format:
  //
  //     vscode:mcp/install?<URL-encoded-JSON-config>
  //
  // The whole query string is a single URL-encoded JSON object, NOT
  // multiple `key=value` query params. Using URLSearchParams here would
  // produce `?name=Mnemonic&url=...` which VS Code does not parse — the
  // browser opens VS Code but the install dialog never appears (this is
  // exactly the bug a user hit during T15 post-deploy QA).
  //
  // Per VS Code MCP docs (code.visualstudio.com/docs/copilot/customization/mcp-servers
  // → "Use MCP install links"):
  //   - HTTP transport: { name, type: "http", url }
  //   - stdio transport: { name, command, args }
  // We use HTTP (streamable per Decision 1).
  const config = { name: "Mnemonic", type: "http", url: MCP_URL };
  return `vscode:mcp/install?${encodeURIComponent(JSON.stringify(config))}`;
}

interface InstallButtonsProps {
  onClaudeAiClick?: () => void;
}

export default function InstallButtons({
  onClaudeAiClick,
}: InstallButtonsProps) {
  const [showClaudeModal, setShowClaudeModal] = useState(false);
  const [copied, setCopied] = useState(false);

  const handleClaudeAi = () => {
    setShowClaudeModal(true);
    onClaudeAiClick?.();
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
    </section>
  );
}
