import { Link } from "react-router-dom";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { EXTERNAL_LINKS } from "../lib/links";

/**
 * Landing page (`/`).
 *
 * Tone per ux-guidelines.md: technical / precise. Short, factual sentences. No
 * marketing fluff, no exclamation marks, no emojis.
 *
 * Aesthetic direction: editorial-cryptographic ledger. Mono-numerals as the
 * personality marker, hairline rules, asymmetric step indices, atmospheric
 * background mesh — without leaving the existing dark + mint + Solana-purple
 * palette.
 */
export default function Landing() {
  return (
    <div className="relative min-h-screen overflow-hidden">
      <BackdropMesh />
      <SiteHeader />

      <main className="relative">
        <Hero />
        <Quickstart />
        <HowItWorks />
        <ResourceStrip />
      </main>

      <SiteFooter />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                 Backdrop                                   */
/* -------------------------------------------------------------------------- */

function BackdropMesh() {
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 -z-10 overflow-hidden"
    >
      {/* Soft mint glow, top-left */}
      <div className="absolute -top-40 -left-40 h-[34rem] w-[34rem] rounded-full bg-accent-primary/10 blur-3xl" />
      {/* Solana-purple counterweight, bottom-right */}
      <div className="absolute -bottom-32 -right-32 h-[28rem] w-[28rem] rounded-full bg-accent-secondary/10 blur-3xl" />
      {/* Hairline grid */}
      <svg
        className="absolute inset-0 h-full w-full opacity-[0.04]"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <pattern
            id="hairgrid"
            width="48"
            height="48"
            patternUnits="userSpaceOnUse"
          >
            <path
              d="M 48 0 L 0 0 0 48"
              fill="none"
              stroke="currentColor"
              strokeWidth="0.5"
            />
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#hairgrid)" />
      </svg>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                   Hero                                     */
/* -------------------------------------------------------------------------- */

function Hero() {
  return (
    <section className="mx-auto flex min-h-[78vh] max-w-6xl flex-col items-center justify-center gap-14 px-4 py-20 text-center sm:px-6 md:py-28">
      <ThesisBanner />

      <div className="flex w-full max-w-3xl flex-col items-center gap-7">
        <div className="flex items-center gap-3 font-mono text-[11px] uppercase tracking-[0.22em] text-accent-primary">
          <span
            aria-hidden="true"
            className="inline-block h-px w-10 bg-accent-primary"
          />
          <span>Mnemonic Protocol · v0.1</span>
        </div>

        <h1 className="text-balance text-5xl font-bold leading-[1.05] tracking-tight text-text-primary sm:text-6xl md:text-7xl">
          Verifiable memory{" "}
          <span className="block text-accent-primary">for AI agents.</span>
        </h1>

        <p className="max-w-2xl text-balance text-lg leading-relaxed text-text-muted">
          Persistent. Signed. Anchored on-chain. Memories your agents can carry
          across providers, sessions, and tools — and any third party can
          verify.
        </p>

        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-center">
          <Link
            to="/install"
            className="group inline-flex items-center justify-center gap-2 rounded-md bg-accent-primary px-6 py-3 text-sm font-semibold text-background shadow-[0_0_24px_-6px_var(--color-accent-primary)] transition-all hover:-translate-y-0.5 hover:shadow-[0_0_32px_-4px_var(--color-accent-primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-primary"
            data-testid="cta-get-started"
          >
            Get started
            <span
              aria-hidden="true"
              className="transition-transform group-hover:translate-x-0.5"
            >
              →
            </span>
          </Link>
          <Link
            to="/chat"
            className="inline-flex items-center justify-center gap-2 rounded-md border border-white/15 px-6 py-3 text-sm font-semibold text-text-primary transition-colors hover:border-accent-secondary hover:text-accent-secondary"
          >
            Try the protocol chat
          </Link>
        </div>
      </div>

      <dl className="grid w-full max-w-2xl grid-cols-3 gap-4">
        <Stat label="Bits / dim" value="2–4" caption="TurboQuant" />
        <Stat label="Anchor" value="Solana" caption="SPL Memo" />
        <Stat label="Storage" value="Arweave" caption="Permanent" />
      </dl>
    </section>
  );
}

function ThesisBanner() {
  return (
    <figure
      className="mx-auto flex w-full max-w-3xl flex-col items-center text-center"
      data-testid="protocol-thesis"
    >
      <div className="flex items-center gap-3 text-accent-primary">
        <span
          aria-hidden="true"
          className="inline-block h-px w-12 bg-accent-primary/60"
        />
        <span className="font-mono text-[11px] uppercase tracking-[0.22em]">
          Protocol thesis
        </span>
        <span
          aria-hidden="true"
          className="inline-block h-px w-12 bg-accent-primary/60"
        />
      </div>
      <blockquote className="mt-5 text-balance font-serif text-2xl italic leading-snug text-text-primary/90 sm:text-3xl">
        “Trustless agents cannot work without trustless agentic memory.”
      </blockquote>
    </figure>
  );
}

function Stat({
  label,
  value,
  caption,
}: {
  label: string;
  value: string;
  caption: string;
}) {
  return (
    <div className="flex flex-col items-center gap-1 rounded-md border border-white/5 bg-white/[0.02] px-2 py-3">
      <dt className="font-mono text-[10px] uppercase tracking-[0.18em] text-text-muted">
        {label}
      </dt>
      <dd className="text-lg font-semibold tracking-tight text-text-primary">
        {value}
      </dd>
      <span className="font-mono text-[10px] uppercase tracking-[0.16em] text-accent-primary/80">
        {caption}
      </span>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                                Quickstart                                  */
/* -------------------------------------------------------------------------- */

const QUICKSTART_STEPS = [
  {
    chip: "Open",
    title: "Open the install page",
    body: "Go to mnemonik.xyz/install in your everyday browser. One page handles identity, IDE wiring, and OAuth in a single flow.",
    meta: "~5s",
  },
  {
    chip: "Identity",
    title: "Create your identity",
    body: "Your browser generates an Ed25519 keypair locally. The private key never leaves the device. The public key is your identity across every IDE, every device, every model.",
    meta: "1 click",
  },
  {
    chip: "Install",
    title: "Install into the IDE of your choice",
    body: "Cursor, Claude Desktop, VS Code, Windsurf — pick your tool, click install, finish OAuth. The Mnemonic tools appear in your IDE's tool list automatically.",
    meta: "Any MCP client",
  },
  {
    chip: "Sign",
    title: "Ask your agent to store something",
    body: 'Just talk to it. "Remember that we shipped v0.2 on Tuesday with Alex as release owner." The agent calls mnemonic_sign_memory; the record is signed and anchored on-chain.',
    meta: "mnemonic_sign_memory",
  },
  {
    chip: "Recall",
    title: "Recall it from another device, another IDE",
    body: 'Re-import the same identity on a different device. Ask the agent: "What did we decide about the v0.2 release?" It searches by meaning, not keyword, and returns your signed memories — verifiable by anyone.',
    meta: "mnemonic_recall",
  },
];

function Quickstart() {
  return (
    <section
      aria-labelledby="quickstart"
      className="border-y border-white/5 bg-white/[0.015]"
      data-testid="quickstart"
    >
      <div className="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-24">
        <header className="mb-14 grid gap-6 md:grid-cols-[1fr_auto] md:items-end">
          <div className="space-y-3">
            <p className="font-mono text-[11px] uppercase tracking-[0.22em] text-accent-primary">
              Quickstart · 5 steps
            </p>
            <h2
              id="quickstart"
              className="text-balance text-4xl font-bold leading-tight tracking-tight text-text-primary sm:text-5xl"
            >
              From zero to{" "}
              <em className="font-serif text-accent-primary not-italic">
                cross-device recall
              </em>
              .
            </h2>
            <p className="max-w-xl text-text-muted">
              The whole loop — sign on Device A, recall on Device B — runs
              entirely through the webapp and your IDE of choice.
            </p>
          </div>
          <a
            href={EXTERNAL_LINKS.quickstart}
            target="_blank"
            rel="noopener noreferrer"
            className="group inline-flex items-center gap-2 self-start rounded-sm border border-white/10 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted transition-colors hover:border-accent-primary hover:text-accent-primary md:self-end"
          >
            Read the doc
            <span
              aria-hidden="true"
              className="transition-transform group-hover:translate-x-0.5"
            >
              ↗
            </span>
          </a>
        </header>

        <ol className="relative space-y-6">
          {/* Vertical ledger rail */}
          <div
            aria-hidden="true"
            className="absolute left-[2.65rem] top-2 bottom-2 hidden w-px bg-gradient-to-b from-accent-primary/40 via-white/5 to-transparent md:block"
          />

          {QUICKSTART_STEPS.map((step, idx) => (
            <li
              key={step.chip}
              className="group relative grid grid-cols-[auto_1fr] gap-4 sm:grid-cols-[auto_1fr_auto] sm:gap-6"
            >
              <Index n={idx + 1} />
              <div className="flex flex-col gap-2 rounded-md border border-white/5 bg-background/60 px-5 py-4 backdrop-blur-sm transition-colors group-hover:border-accent-primary/40">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="rounded-sm bg-accent-primary/10 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.18em] text-accent-primary">
                    {step.chip}
                  </span>
                  <h3 className="text-lg font-semibold tracking-tight text-text-primary">
                    {step.title}
                  </h3>
                </div>
                <p className="text-[15px] leading-relaxed text-text-muted">
                  {step.body}
                </p>
              </div>
              <span className="hidden self-center font-mono text-[10px] uppercase tracking-[0.18em] text-text-muted/70 sm:block">
                {step.meta}
              </span>
            </li>
          ))}
        </ol>

        <div className="mt-14 flex flex-col items-start gap-4 rounded-md border border-accent-primary/20 bg-accent-primary/[0.04] p-6 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <p className="font-mono text-[11px] uppercase tracking-[0.2em] text-accent-primary">
              Next
            </p>
            <p className="mt-1 max-w-xl text-text-primary">
              Start with the install page. Generation, signing, and recall all
              happen behind the same OAuth handshake.
            </p>
          </div>
          <Link
            to="/install"
            className="inline-flex items-center gap-2 rounded-md bg-accent-primary px-5 py-2.5 text-sm font-semibold text-background transition-opacity hover:opacity-90"
          >
            Open install
            <span aria-hidden="true">→</span>
          </Link>
        </div>
      </div>
    </section>
  );
}

function Index({ n }: { n: number }) {
  return (
    <div className="relative flex h-12 w-12 shrink-0 items-center justify-center rounded-full border border-white/10 bg-background font-mono text-sm tracking-tight text-accent-primary shadow-[inset_0_0_0_1px_rgba(0,212,180,0.08)]">
      {String(n).padStart(2, "0")}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/*                              How it works                                  */
/* -------------------------------------------------------------------------- */

function HowItWorks() {
  const steps = [
    {
      label: "01 · Identity",
      title: "Generate an Ed25519 keypair",
      body: "The keypair lives in your browser's local storage. The public key is your portable identity across providers; the secret never leaves the device.",
    },
    {
      label: "02 · Install",
      title: "Connect Mnemonic to your AI tool",
      body: "Cursor and VS Code accept a deeplink; Claude.ai takes a custom connector URL; Windsurf takes a JSON snippet. All four negotiate OAuth 2.1 + PKCE against mcp.mnemonik.xyz.",
    },
    {
      label: "03 · Sign",
      title: "Sign memories from any tool",
      body: "When the agent calls mnemonic_sign_memory, the server prepares the canonical-CBOR payload and redirects here; you approve in-browser, your keypair signs a COSE_Sign1 envelope, and the server persists the attestation.",
    },
    {
      label: "04 · Recall",
      title: "Recall and verify across providers",
      body: "Any tool authenticated with the same keypair can recall by semantic similarity and verify the Arweave + Solana anchors. Tampering is detectable; provenance is cryptographic.",
    },
  ];

  return (
    <section
      aria-labelledby="how-it-works"
      className="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28"
      data-testid="how-it-works"
    >
      <header className="mb-12 max-w-3xl space-y-3">
        <p className="font-mono text-[11px] uppercase tracking-[0.22em] text-accent-secondary">
          Under the hood
        </p>
        <h2
          id="how-it-works"
          className="text-balance text-4xl font-bold leading-tight tracking-tight text-text-primary sm:text-5xl"
        >
          How it works
        </h2>
        <p className="text-text-muted">
          Four moving parts. Each one inspectable, each one independently
          verifiable, no opaque vendor in the path.
        </p>
      </header>

      <ol className="grid gap-px overflow-hidden rounded-lg bg-white/5 md:grid-cols-2">
        {steps.map((step) => (
          <li
            key={step.label}
            className="relative flex flex-col gap-3 bg-background/95 p-7 transition-colors hover:bg-background"
          >
            <span className="font-mono text-[11px] uppercase tracking-[0.2em] text-accent-primary">
              {step.label}
            </span>
            <h3 className="text-lg font-semibold tracking-tight text-text-primary">
              {step.title}
            </h3>
            <p className="text-[15px] leading-relaxed text-text-muted">
              {step.body}
            </p>
          </li>
        ))}
      </ol>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/*                              Resource strip                                */
/* -------------------------------------------------------------------------- */

function ResourceStrip() {
  const cards = [
    {
      label: "Whitepaper",
      desc: "Architecture, threat model, design rationale.",
      href: EXTERNAL_LINKS.whitepaper,
    },
    {
      label: "Source",
      desc: "Open-source Rust workspace + webapp on GitHub.",
      href: EXTERNAL_LINKS.github,
    },
    {
      label: "Discord",
      desc: "Builders, researchers, protocol questions.",
      href: EXTERNAL_LINKS.discord,
    },
    {
      label: "Telegram",
      desc: "Community channel for ongoing announcements.",
      href: EXTERNAL_LINKS.telegram,
    },
  ];

  return (
    <section
      aria-labelledby="resources"
      className="mx-auto max-w-6xl px-4 pb-20 sm:px-6"
    >
      <h2
        id="resources"
        className="mb-6 font-mono text-[11px] uppercase tracking-[0.22em] text-text-muted"
      >
        Resources
      </h2>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {cards.map((c) => (
          <a
            key={c.label}
            href={c.href}
            target="_blank"
            rel="noopener noreferrer"
            className="group relative overflow-hidden rounded-md border border-white/10 bg-white/[0.02] p-5 transition-all hover:-translate-y-0.5 hover:border-accent-primary/40 hover:bg-white/[0.04]"
          >
            <div className="flex items-center justify-between font-mono text-[11px] uppercase tracking-[0.18em] text-accent-primary">
              <span>{c.label}</span>
              <span
                aria-hidden="true"
                className="transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5"
              >
                ↗
              </span>
            </div>
            <p className="mt-3 text-sm leading-relaxed text-text-muted">
              {c.desc}
            </p>
          </a>
        ))}
      </div>
    </section>
  );
}
