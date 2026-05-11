import { Link } from "react-router-dom";
import { EXTERNAL_LINKS } from "../lib/links";

/**
 * Site-wide footer. Designed to read like the colophon of a technical paper:
 * groups of links with mono labels, hairline separators, no marketing flourish.
 */
export default function SiteFooter() {
  return (
    <footer className="relative mt-24 border-t border-white/5 bg-background/40">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-accent-primary/30 to-transparent"
      />

      <div className="mx-auto grid max-w-6xl gap-10 px-4 py-12 sm:px-6 md:grid-cols-[1.4fr_1fr_1fr_1fr]">
        <div className="space-y-4">
          <Link
            to="/"
            className="flex items-center gap-2 font-mono text-xs uppercase tracking-[0.18em] text-text-primary"
          >
            <span
              aria-hidden="true"
              className="inline-block h-1.5 w-1.5 rounded-full bg-accent-primary shadow-[0_0_12px_var(--color-accent-primary)]"
            />
            <span className="font-semibold">Mnemonic</span>
            <span className="text-text-muted/60">/ protocol</span>
          </Link>
          <p className="max-w-sm text-sm leading-relaxed text-text-muted">
            Verifiable, persistent memory for AI agents. Open-source under
            Apache 2.0. Built with care, designed to be inspected.
          </p>
        </div>

        <FooterColumn title="Read">
          <FooterLink external href={EXTERNAL_LINKS.whitepaper}>
            Whitepaper
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.howItWorks}>
            How it works
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.quickstart}>
            Quickstart
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.paper}>
            Foundational paper
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.researchgate}>
            ResearchGate publication
          </FooterLink>
        </FooterColumn>

        <FooterColumn title="Build">
          <FooterLink external href={EXTERNAL_LINKS.github}>
            GitHub
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.issues}>
            Issues
          </FooterLink>
          <FooterLink internal href="/install">
            Install
          </FooterLink>
          <FooterLink internal href="/chat">
            Protocol chat
          </FooterLink>
        </FooterColumn>

        <FooterColumn title="Talk">
          <FooterLink external href={EXTERNAL_LINKS.discord}>
            Discord
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.telegram}>
            Telegram · community
          </FooterLink>
          <FooterLink external href={EXTERNAL_LINKS.telegramAnnouncements}>
            Telegram · announcements
          </FooterLink>
        </FooterColumn>
      </div>

      <div className="border-t border-white/5">
        <div className="mx-auto flex max-w-6xl flex-col gap-2 px-4 py-5 font-mono text-[11px] uppercase tracking-[0.16em] text-text-muted/70 sm:flex-row sm:items-center sm:justify-between sm:px-6">
          <span>© {new Date().getFullYear()} Mnemonic Protocol</span>
          <span className="flex items-center gap-3">
            <span aria-hidden="true" className="h-px w-6 bg-text-muted/30" />
            <span>Apache 2.0 · v0.1</span>
          </span>
        </div>
      </div>
    </footer>
  );
}

function FooterColumn({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-3">
      <h3 className="font-mono text-[11px] uppercase tracking-[0.18em] text-text-muted">
        {title}
      </h3>
      <ul className="space-y-2 text-sm">{children}</ul>
    </div>
  );
}

function FooterLink({
  href,
  children,
  external,
  internal,
}: {
  href: string;
  children: React.ReactNode;
  external?: boolean;
  internal?: boolean;
}) {
  if (internal) {
    return (
      <li>
        <Link
          to={href}
          className="text-text-primary/80 transition-colors hover:text-accent-primary"
        >
          {children}
        </Link>
      </li>
    );
  }
  if (external) {
    return (
      <li>
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="text-text-primary/80 transition-colors hover:text-accent-primary"
        >
          {children}
        </a>
      </li>
    );
  }
  return null;
}
