import { Link } from "react-router-dom";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { EXTERNAL_LINKS } from "../lib/links";

/**
 * `/privacy` — privacy policy for the public Mnemonic Protocol surface
 * (mnemonik.xyz landing + chat demo, mcp.mnemonik.xyz MCP server).
 *
 * Tone per ux-guidelines.md: technical / precise. No marketing language,
 * no apologies, no exclamation marks. The policy describes what the
 * protocol actually does — keypairs, SQLite, Arweave/Solana anchors —
 * rather than generic SaaS boilerplate.
 */
export default function Privacy() {
  return (
    <div className="min-h-screen bg-background text-text-primary">
      <SiteHeader />
      <main className="px-4 py-10 sm:py-14">
        <div className="mx-auto max-w-3xl space-y-8">
          <Link
            to="/"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            ← Back
          </Link>

          <article
            data-testid="privacy-article"
            className="space-y-10 text-text-primary"
          >
            <header className="space-y-3">
              <p className="font-mono text-[11px] uppercase tracking-[0.18em] text-accent-primary">
                Legal · v1.0
              </p>
              <h1 className="text-3xl font-bold tracking-tight sm:text-4xl">
                Privacy Policy
              </h1>
              <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-text-muted">
                Effective date: 2026-06-11
              </p>
              <p className="text-[14px] leading-relaxed text-text-muted">
                This policy describes what data the Mnemonic Protocol handles
                when you use the public website at{" "}
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  mnemonik.xyz
                </code>{" "}
                and the hosted MCP server at{" "}
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  mcp.mnemonik.xyz
                </code>
                . Mnemonic is open-source under Apache 2.0; the canonical
                reference for its data handling is the source code, not this
                document.
              </p>
            </header>

            <Section title="1. What Mnemonic is">
              <P>
                Mnemonic Protocol is verifiable, persistent memory for AI
                agents. A memory item is semantically embedded,
                scalar-quantised, canonicalised to deterministic CBOR, hashed
                with BLAKE3, and signed as a COSE_Sign1 artifact under an
                Ed25519 keypair. In full mode the signed bytes are also
                persisted on Arweave and timestamped on Solana via an SPL Memo
                transaction.
              </P>
              <P>
                Because the cryptographic surface — keypair, hash, signature,
                anchor transaction — is the substrate of the protocol itself,
                the privacy properties of what you put into Mnemonic are
                determined by the design choices described below, not by an
                operator policy.
              </P>
            </Section>

            <Section title="2. Operating modes and what they imply">
              <Subsection title="Local mode">
                <P>
                  In <strong className="text-text-primary">local mode</strong>{" "}
                  the protocol writes only to a local SQLite database on the
                  machine running the MCP server. No content, hash, or anchor
                  leaves the host. This is the default mode for self-hosted
                  installations and for the{" "}
                  <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                    mnemonic_sign_memory
                  </code>{" "}
                  tool unless the caller explicitly opts in.
                </P>
              </Subsection>
              <Subsection title="Full mode (on-chain participation)">
                <P>
                  In <strong className="text-text-primary">full mode</strong>{" "}
                  the signed canonical CBOR bytes are uploaded to Arweave (via
                  Irys as an ANS-104 bundle item) and a content identifier is
                  published on Solana as an SPL Memo transaction. Both are
                  public, permanent, decentralised ledgers. Anything an
                  attestation contains — including the embedded vector and any
                  plaintext your client chose to include in the canonical body —
                  becomes world-readable and, in practical terms,{" "}
                  <em className="text-text-primary">irrevocable</em>.
                </P>
                <P>
                  Do not place secrets, credentials, personally identifiable
                  information about third parties, or any content you may later
                  need to remove from public record into a full-mode
                  attestation. Cryptographic shredding (per-cell key erasure) is
                  described in the whitepaper as a research direction; the
                  public MVP does not implement it.
                </P>
              </Subsection>
            </Section>

            <Section title="3. Data we handle when you use mnemonik.xyz">
              <P>The public landing page and chat demo at mnemonik.xyz:</P>
              <Bullets>
                <li>
                  serve static assets and a WebAssembly client that runs locally
                  in your browser;
                </li>
                <li>do not set advertising or third-party tracking cookies;</li>
                <li>
                  may produce standard HTTP server logs (IP address, user-agent,
                  requested path, timestamp) used solely for operational
                  diagnostics and abuse prevention. These logs are retained for
                  no longer than thirty (30) days.
                </li>
              </Bullets>
              <P>
                The in-page chat is intended as a demonstration of Mnemonic with
                a local model. Text you enter into the chat is sent to the demo
                backend so it can produce a reply and is not retained beyond the
                lifetime of the response unless you explicitly export it as an
                attestation.
              </P>
            </Section>

            <Section title="4. The hosted MCP server (mcp.mnemonik.xyz)">
              <P>
                When you connect to the hosted MCP server from a client such as
                Cursor or Claude Desktop, the server processes requests issued
                by the five MCP tools (
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  whoami
                </code>
                ,{" "}
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  sign_memory
                </code>
                ,{" "}
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  verify
                </code>
                ,{" "}
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  prove_identity
                </code>
                ,{" "}
                <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                  recall
                </code>
                ). For each tool call:
              </P>
              <Bullets>
                <li>
                  the public Ed25519 key derived from your agent identity is
                  recorded so that recall can scope results to you;
                </li>
                <li>
                  the content you submit to{" "}
                  <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                    sign_memory
                  </code>{" "}
                  is persisted in the server's attestation database in the mode
                  selected by the request (
                  <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                    mode: "local"
                  </code>{" "}
                  by default,{" "}
                  <code className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary">
                    mode: "participate"
                  </code>{" "}
                  for on-chain anchoring);
                </li>
                <li>
                  if you have opted into payment, an API token or x402 payment
                  identifier is associated with the call for the purpose of
                  metering and accounting.
                </li>
              </Bullets>
              <P>
                We do not sell, share, or otherwise disclose attestation content
                to third parties. We do not use it to train models. The only
                entity reading your private (local-mode) attestations is the MCP
                server itself, when answering your own recall queries.
              </P>
            </Section>

            <Section title="5. Cryptographic identity">
              <P>
                Your agent identity is an Ed25519 keypair generated on your
                machine. The private key never leaves the client process and is
                not transmitted to mnemonik.xyz or any third party. The
                corresponding public key is treated as a pseudonymous identifier
                — it appears in attestation headers, in the on-chain anchor when
                full mode is used, and in operational logs.
              </P>
              <P>
                Possession of the private key is the only way to issue
                attestations under that identity. If the key is lost,
                attestations remain verifiable but no new ones can be signed in
                its name. If the key is leaked, prior attestations remain valid;
                future writes under that identity must be assumed to be
                untrusted.
              </P>
            </Section>

            <Section title="6. Public ledgers: Arweave and Solana">
              <P>
                Full-mode attestations are written to two independent public
                ledgers:
              </P>
              <Bullets>
                <li>
                  <strong className="text-text-primary">Arweave</strong> stores
                  the signed CBOR bytes as an ANS-104 bundle item. Arweave is
                  designed to be permanent; once a transaction confirms, its
                  contents are not deletable by Mnemonic, by any operator, or by
                  Arweave miners.
                </li>
                <li>
                  <strong className="text-text-primary">Solana</strong> records
                  an SPL Memo transaction containing a content identifier and
                  the public key of the signer. Solana transactions are public
                  and not deletable.
                </li>
              </Bullets>
              <P>
                Operators of Mnemonic — including the maintainers of
                mnemonik.xyz — cannot retract data committed to Arweave or
                Solana. Use full mode only with content for which permanent
                public disclosure is acceptable.
              </P>
            </Section>

            <Section title="7. Retention and deletion">
              <P>
                For local-mode attestations held in the hosted MCP server's
                database, you may request deletion of all rows associated with
                your public key by contacting us at the address below. We will
                process such a request within thirty (30) days.
              </P>
              <P>
                For full-mode attestations, deletion from Arweave and Solana is{" "}
                <strong className="text-text-primary">
                  not technically possible
                </strong>
                . On request we can remove the corresponding row from the hosted
                server's local index, which prevents it from appearing in recall
                results served by mnemonik.xyz, but the underlying anchor on the
                public ledger persists indefinitely.
              </P>
              <P>
                Operational logs are retained for at most thirty (30) days.
                Database backups follow a rolling retention window appropriate
                for disaster recovery and are encrypted at rest.
              </P>
            </Section>

            <Section title="8. Children">
              <P>
                Mnemonic is a developer tool. It is not directed at children
                under the age of 13, and we do not knowingly process data
                submitted by them.
              </P>
            </Section>

            <Section title="9. Security">
              <P>
                The protocol's integrity properties — content-addressed hashes,
                COSE signatures, public anchors — are the primary security
                mechanism for attestations. The hosted MCP server additionally
                serves all traffic over TLS, runs with a least-privilege system
                user, and stores its signing keypair on the host filesystem with
                restrictive permissions. Source code, including the security
                model, is open to inspection at{" "}
                <a
                  href={EXTERNAL_LINKS.github}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent-primary underline-offset-2 hover:underline"
                >
                  the project repository
                </a>
                .
              </P>
            </Section>

            <Section title="10. Changes to this policy">
              <P>
                Material changes to this policy are versioned in the repository
                and reflected on this page with an updated effective date. We do
                not currently send proactive notifications when the policy
                changes; verify the effective date at the top of this page
                before relying on a specific clause.
              </P>
            </Section>

            <Section title="11. Contact">
              <P>
                Questions, deletion requests, or security disclosures relating
                to this policy may be sent to{" "}
                <a
                  href="mailto:hello@mnemonik.xyz"
                  className="text-accent-primary underline-offset-2 hover:underline"
                >
                  hello@mnemonik.xyz
                </a>
                . For protocol-level issues, please open an issue at{" "}
                <a
                  href={EXTERNAL_LINKS.issues}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent-primary underline-offset-2 hover:underline"
                >
                  github.com/mnemonik-xyz/monorepo/issues
                </a>
                .
              </P>
            </Section>
          </article>
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-4">
      <h2 className="border-b border-white/5 pb-2 font-mono text-xs uppercase tracking-[0.18em] text-accent-primary">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Subsection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <h3 className="font-mono text-sm font-semibold text-text-primary">
        {title}
      </h3>
      {children}
    </div>
  );
}

function P({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[14px] leading-relaxed text-text-muted">{children}</p>
  );
}

function Bullets({ children }: { children: React.ReactNode }) {
  return (
    <ul className="list-disc space-y-1.5 pl-5 text-[14px] leading-relaxed text-text-muted marker:text-accent-primary/60">
      {children}
    </ul>
  );
}
