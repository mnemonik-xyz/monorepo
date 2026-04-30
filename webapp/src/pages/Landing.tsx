import { Link } from "react-router-dom";

/**
 * Landing page (`/`).
 *
 * Tone per ux-guidelines.md: technical / precise. Short, factual sentences. No
 * marketing fluff, no exclamation marks, no emojis.
 */
export default function Landing() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center px-4 py-12">
      <div className="w-full max-w-2xl space-y-10">
        <header className="space-y-3 text-center">
          <h1 className="text-4xl font-bold tracking-tight text-accent-primary sm:text-5xl">
            Verifiable memory for AI agents
          </h1>
          <p className="text-lg text-text-muted">
            Persistent, signed, on-chain-anchored attestations for AI workflows.
          </p>
        </header>

        <section className="space-y-6 text-left">
          <p className="leading-relaxed text-text-primary">
            Mnemonic Protocol stores agent memory as portable cryptographic
            attestations: each memory is canonicalized to deterministic CBOR,
            blake3-hashed, COSE_Sign1-signed with an Ed25519 identity, and
            optionally anchored on Arweave with a Solana SPL Memo timestamp.
          </p>
          <p className="leading-relaxed text-text-primary">
            Agents recall by semantic similarity over their own attestations,
            with cryptographic provenance. Memories survive across providers,
            sessions, and tools — and any tampering is detectable.
          </p>
          <p className="leading-relaxed text-text-muted italic">
            Trustless agents cannot work without trustless agentic memory.
          </p>
        </section>

        <nav className="flex flex-col items-center gap-4 sm:flex-row sm:justify-center">
          <Link
            to="/install"
            className="rounded-md bg-accent-primary px-6 py-3 text-sm font-semibold text-background transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
            data-testid="cta-get-started"
          >
            Get started
          </Link>
          <Link
            to="/chat"
            className="rounded-md border border-text-muted/30 px-6 py-3 text-sm font-semibold text-text-primary transition-colors hover:border-accent-secondary hover:text-accent-secondary"
          >
            Try the protocol chat
          </Link>
        </nav>

        <section
          aria-labelledby="how-it-works"
          className="space-y-6 text-left"
          data-testid="how-it-works"
        >
          <h2
            id="how-it-works"
            className="text-2xl font-semibold text-text-primary"
          >
            How it works
          </h2>
          <ol className="space-y-4">
            <li className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <p className="text-sm font-semibold text-accent-primary">
                1 · Generate an Ed25519 keypair
              </p>
              <p className="mt-1 text-sm text-text-muted">
                The keypair lives in your browser&rsquo;s local storage. The
                public key is your portable identity across providers; the
                secret key never leaves the device.
              </p>
            </li>
            <li className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <p className="text-sm font-semibold text-accent-primary">
                2 · Install the MCP connector in your AI tool
              </p>
              <p className="mt-1 text-sm text-text-muted">
                Cursor and VS Code accept a deeplink; Claude.ai takes a custom
                connector URL; WindSurf takes a JSON snippet for
                <code>mcp_config.json</code>. All four negotiate OAuth 2.1 +
                PKCE against <code>mcp.mnemonik.xyz</code>.
              </p>
            </li>
            <li className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <p className="text-sm font-semibold text-accent-primary">
                3 · Sign memories from any tool
              </p>
              <p className="mt-1 text-sm text-text-muted">
                When the agent calls <code>mnemonic_sign_memory</code>, the
                server prepares the canonical-CBOR payload and redirects to this
                webapp; you approve in-browser, your keypair signs a COSE_Sign1
                envelope, and the server persists the attestation.
              </p>
            </li>
            <li className="rounded-md border border-text-muted/20 bg-white/5 p-4">
              <p className="text-sm font-semibold text-accent-primary">
                4 · Recall and verify across providers
              </p>
              <p className="mt-1 text-sm text-text-muted">
                Any tool authenticated with the same keypair can recall by
                semantic similarity and verify the Arweave + Solana anchors.
                Tampering is detectable; provenance is cryptographic.
              </p>
            </li>
          </ol>
        </section>
      </div>
    </main>
  );
}
