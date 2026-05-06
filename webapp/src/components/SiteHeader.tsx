import { Link } from "react-router-dom";
import { EXTERNAL_LINKS } from "../lib/links";

/**
 * Sticky minimal site header. Three zones:
 *   left  — wordmark anchored to /
 *   right — terse mono links to the public resources
 *
 * Designed to read like a research-paper masthead, not a marketing nav.
 */
export default function SiteHeader() {
  return (
    <header className="sticky top-0 z-30 border-b border-white/5 bg-background/80 backdrop-blur-md">
      <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-3 sm:px-6">
        <Link
          to="/"
          className="group flex items-center gap-2 font-mono text-xs uppercase tracking-[0.18em] text-text-primary"
          aria-label="Mnemonic Protocol — home"
        >
          <span
            aria-hidden="true"
            className="inline-block h-1.5 w-1.5 rounded-full bg-accent-primary shadow-[0_0_12px_var(--color-accent-primary)] transition-transform group-hover:scale-125"
          />
          <span className="font-semibold">Mnemonic</span>
          <span className="text-text-muted/60">/ protocol</span>
        </Link>

        <nav
          aria-label="External resources"
          className="flex items-center gap-1 sm:gap-3"
        >
          <ResourceLink
            href={EXTERNAL_LINKS.github}
            label="GitHub"
            srLabel="GitHub repository"
          />
          <ResourceLink
            href={EXTERNAL_LINKS.whitepaper}
            label="Whitepaper"
            srLabel="Read the whitepaper"
          />
          <ResourceLink
            href={EXTERNAL_LINKS.researchgate}
            label="ResearchGate"
            srLabel="ResearchGate publication"
          />
          <Link
            to="/install"
            className="ml-2 hidden rounded-sm border border-accent-primary/40 bg-accent-primary/10 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.16em] text-accent-primary transition-colors hover:border-accent-primary hover:bg-accent-primary/20 sm:inline-flex"
          >
            Install →
          </Link>
        </nav>
      </div>
    </header>
  );
}

interface ResourceLinkProps {
  href: string;
  label: string;
  srLabel: string;
}

function ResourceLink({ href, label, srLabel }: ResourceLinkProps) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="group relative px-1.5 py-1 font-mono text-[11px] uppercase tracking-[0.16em] text-text-muted transition-colors hover:text-text-primary sm:px-2"
      aria-label={srLabel}
    >
      <span
        aria-hidden="true"
        className="absolute inset-x-1 -bottom-0.5 h-px origin-left scale-x-0 bg-accent-primary/70 transition-transform duration-300 group-hover:scale-x-100"
      />
      {label}
    </a>
  );
}
