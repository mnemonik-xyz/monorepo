import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { EXTERNAL_LINKS } from "../lib/links";

/**
 * Sticky minimal site header. Three zones:
 *   left   — wordmark anchored to /
 *   centre — Docs dropdown (Whitepaper, ResearchGate), the public-surface links
 *            (Ledger, Analytics, Blog), Roadmap, and the GitHub link
 *   right  — Install pill (desktop only)
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
          <DocsMenu />
          <NavLink to="/ledger" label="Ledger" srLabel="Evidence ledger" />
          <NavLink
            to="/analytics"
            label="Analytics"
            srLabel="Attestation analytics"
          />
          <NavLink to="/blog" label="Blog" srLabel="Protocol blog" />
          <NavLink to="/roadmap" label="Roadmap" srLabel="Project roadmap" />
          <ResourceLink
            href={EXTERNAL_LINKS.github}
            label="GitHub"
            srLabel="GitHub repository"
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

interface NavLinkProps {
  to: string;
  label: string;
  srLabel: string;
}

/** Internal route link in the masthead, with the hairline underline hover. */
function NavLink({ to, label, srLabel }: NavLinkProps) {
  return (
    <Link
      to={to}
      className="group relative px-1.5 py-1 font-mono text-[11px] uppercase tracking-[0.16em] text-text-muted transition-colors hover:text-text-primary sm:px-2"
      aria-label={srLabel}
    >
      <span
        aria-hidden="true"
        className="absolute inset-x-1 -bottom-0.5 h-px origin-left scale-x-0 bg-accent-primary/70 transition-transform duration-300 group-hover:scale-x-100"
      />
      {label}
    </Link>
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

/* -------------------------------------------------------------------------- */
/*                                Docs dropdown                                */
/* -------------------------------------------------------------------------- */

function DocsMenu() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: PointerEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="group inline-flex items-center gap-1 px-1.5 py-1 font-mono text-[11px] uppercase tracking-[0.16em] text-text-muted transition-colors hover:text-text-primary sm:px-2"
      >
        <span className="relative">
          <span
            aria-hidden="true"
            className={`absolute inset-x-0 -bottom-0.5 h-px origin-left bg-accent-primary/70 transition-transform duration-300 ${
              open ? "scale-x-100" : "scale-x-0 group-hover:scale-x-100"
            }`}
          />
          Docs
        </span>
        <span
          aria-hidden="true"
          className={`text-[9px] transition-transform duration-200 ${
            open ? "rotate-180" : ""
          }`}
        >
          ▾
        </span>
      </button>

      {open && (
        <div
          role="menu"
          aria-label="Documentation"
          className="absolute right-0 top-full z-40 mt-2 min-w-[14rem] overflow-hidden rounded-md border border-white/10 bg-background/95 shadow-[0_14px_32px_-12px_rgba(0,0,0,0.6)] backdrop-blur-md"
        >
          <DocsMenuItem
            href={EXTERNAL_LINKS.whitepaper}
            label="Whitepaper"
            sub="Architecture · threat model · rationale"
            onSelect={() => setOpen(false)}
          />
          <div className="h-px bg-white/5" aria-hidden="true" />
          <DocsMenuItem
            href={EXTERNAL_LINKS.researchgate}
            label="ResearchGate"
            sub="Sublinear Verifiable Recall — publication"
            onSelect={() => setOpen(false)}
          />
        </div>
      )}
    </div>
  );
}

function DocsMenuItem({
  href,
  label,
  sub,
  onSelect,
}: {
  href: string;
  label: string;
  sub: string;
  onSelect: () => void;
}) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      role="menuitem"
      onClick={onSelect}
      className="group block px-4 py-3 transition-colors hover:bg-white/[0.04] focus-visible:bg-white/[0.06] focus-visible:outline-none"
    >
      <div className="flex items-center justify-between gap-3 font-mono text-[11px] uppercase tracking-[0.18em] text-accent-primary">
        <span>{label}</span>
        <span
          aria-hidden="true"
          className="transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5"
        >
          ↗
        </span>
      </div>
      <p className="mt-1 font-sans text-[12px] leading-snug text-text-muted">
        {sub}
      </p>
    </a>
  );
}
