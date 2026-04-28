import { Link } from "react-router-dom";
import IdentityPanel from "../components/IdentityPanel";
import InstallButtons from "../components/InstallButtons";

/**
 * Install hub (`/install`). Two columns on desktop, stacked on mobile:
 *   - Install buttons (Cursor / VS Code / Claude.ai / WindSurf).
 *   - Identity panel (Generate / Import / Export keypair).
 */
export default function Install() {
  return (
    <main className="min-h-screen px-4 py-10">
      <div className="mx-auto max-w-4xl space-y-8">
        <header className="space-y-2">
          <Link
            to="/"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            ← Back
          </Link>
          <h1 className="text-3xl font-bold tracking-tight text-text-primary">
            Install
          </h1>
          <p className="text-sm text-text-muted">
            Connect Mnemonic to your AI tool, then create or import the Ed25519
            keypair that will sign your memories.
          </p>
        </header>

        <div className="grid gap-8 md:grid-cols-2">
          <InstallButtons />
          <IdentityPanel />
        </div>

        <section
          aria-labelledby="install-instructions"
          className="space-y-4"
          data-testid="install-instructions"
        >
          <h2
            id="install-instructions"
            className="text-lg font-semibold text-text-primary"
          >
            Step-by-step
          </h2>
          <p className="text-sm text-text-muted">
            Generate or import your keypair first — both flows below redirect to
            OAuth, which signs the request with the local Ed25519 secret.
          </p>

          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                Cursor
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Install in Cursor</span>{" "}
                  above.
                </li>
                <li>2. Approve the deeplink prompt in your browser.</li>
                <li>
                  3. Confirm{" "}
                  <span className="font-mono text-text-primary">Mnemonic</span>{" "}
                  in the Cursor MCP install dialog.
                </li>
                <li>4. Complete OAuth in the popup window.</li>
              </ol>
            </article>

            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                VS Code
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Install in VS Code</span>{" "}
                  above.
                </li>
                <li>
                  2. Approve the deeplink prompt; VS Code 1.93+ is required.
                </li>
                <li>
                  3. Accept the{" "}
                  <span className="font-mono text-text-primary">Mnemonic</span>{" "}
                  MCP server in the install dialog.
                </li>
                <li>
                  4. Sign in via OAuth when GitHub Copilot first calls a tool.
                </li>
              </ol>
            </article>

            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                Claude.ai
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Add to Claude.ai</span>{" "}
                  and copy the URL.
                </li>
                <li>
                  2. In Claude, open{" "}
                  <span className="font-mono text-text-primary">
                    Settings → Connectors → Add custom connector
                  </span>
                  .
                </li>
                <li>3. Paste the URL and submit.</li>
                <li>4. Complete OAuth and approve the connector.</li>
              </ol>
            </article>

            <article className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <h3 className="text-sm font-semibold text-accent-primary">
                WindSurf
              </h3>
              <ol className="mt-2 space-y-1 text-sm text-text-muted">
                <li>
                  1. Click{" "}
                  <span className="text-text-primary">Install in WindSurf</span>{" "}
                  above and copy the JSON snippet.
                </li>
                <li>
                  2. Open{" "}
                  <span className="font-mono text-text-primary">
                    ~/.codeium/windsurf/mcp_config.json
                  </span>{" "}
                  and merge it into{" "}
                  <span className="font-mono text-text-primary">
                    mcpServers
                  </span>
                  .
                </li>
                <li>3. Reload WindSurf so Cascade picks up the new server.</li>
                <li>4. Complete OAuth on the first tool call.</li>
              </ol>
            </article>
          </div>
        </section>
      </div>
    </main>
  );
}
