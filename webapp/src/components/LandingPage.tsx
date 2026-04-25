interface LandingPageProps {
  onStartChat: () => void;
}

function LandingPage({ onStartChat }: LandingPageProps) {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center px-4 py-12">
      <div className="w-full max-w-2xl space-y-10 text-center">
        <header>
          <h1 className="text-4xl font-bold tracking-tight text-accent-primary sm:text-5xl">
            Mnemonic Protocol
          </h1>
          <p className="mt-3 text-lg text-text-muted">
            Verifiable memory infrastructure for AI agents.
          </p>
        </header>

        <section
          className="space-y-6 text-left"
          aria-label="Protocol description"
        >
          <p className="leading-relaxed text-text-primary">
            AI agents increasingly operate across long-running tasks, tools,
            sessions, and providers, but their memory remains fragile. Context
            windows are temporary, vendor-native memory is hard to audit, and
            conventional vector stores provide persistence without cryptographic
            provenance.
          </p>

          <p className="leading-relaxed text-text-primary">
            Mnemonic Protocol introduces a verifiable memory layer for AI
            agents. It treats memory as a portable, signed artifact rather than
            an opaque database row: something an agent can{" "}
            <span className="font-mono text-accent-primary">recall</span>, carry
            across systems, and prove has not been silently changed. Memories
            can run fully locally for speed and development, or be persisted to
            decentralized infrastructure so third parties can independently
            verify integrity, authorship, and timestamped existence.
          </p>

          <p className="leading-relaxed text-text-primary">
            Exposed through the{" "}
            <span className="font-mono text-accent-primary">
              Model Context Protocol (MCP)
            </span>
            , Mnemonic is usable by current agent clients over{" "}
            <span className="font-mono text-text-muted">HTTP</span> or{" "}
            <span className="font-mono text-text-muted">stdio</span>. The core
            implementation provides tools for identity,{" "}
            <span className="font-mono text-accent-primary">
              memory signing
            </span>
            , verification, challenge signing, and{" "}
            <span className="font-mono text-accent-primary">recall</span>.
          </p>

          <p className="leading-relaxed text-text-muted italic">
            Trustless agents cannot work without trustless agentic memory.
          </p>
        </section>

        <nav className="flex flex-col items-center gap-4 sm:flex-row sm:justify-center">
          <button
            type="button"
            onClick={onStartChat}
            className="rounded-md bg-accent-primary px-6 py-3 text-sm font-semibold text-background transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
          >
            Start chat
          </button>
          <a
            href="/download-knowledge"
            download
            className="rounded-md border border-text-muted/30 px-6 py-3 text-sm font-semibold text-text-primary transition-colors hover:border-accent-secondary hover:text-accent-secondary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-secondary"
          >
            Download protocol knowledge
          </a>
        </nav>
      </div>
    </main>
  );
}

export default LandingPage;
