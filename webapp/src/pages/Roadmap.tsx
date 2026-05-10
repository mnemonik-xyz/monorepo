import type { ComponentPropsWithoutRef } from "react";
import { Link } from "react-router-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
// Vite ?raw import: docs/ROADMAP.md is the single source of truth; this
// inlines its contents as a string at build time. Updating the .md is
// the only way to change the roadmap.
import roadmapMd from "../../../docs/ROADMAP.md?raw";

/**
 * `/roadmap` — renders docs/ROADMAP.md as long-form prose. This is the
 * first usage of the markdown-rendering pattern that we'll extend to the
 * rest of docs/ (whitepaper, quickstart, etc.).
 */
export default function Roadmap() {
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
            data-testid="roadmap-article"
            className="space-y-6 text-text-primary"
          >
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={MARKDOWN_COMPONENTS}
            >
              {roadmapMd}
            </ReactMarkdown>
          </article>
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}

// Tailwind-styled overrides for the markdown renderer. Kept local to this
// file for now; if we add more markdown pages we'll lift this into a shared
// `<DocsMarkdown>` component.
const MARKDOWN_COMPONENTS = {
  h1: (props: ComponentPropsWithoutRef<"h1">) => (
    <h1
      className="text-3xl font-bold tracking-tight text-text-primary sm:text-4xl"
      {...props}
    />
  ),
  h2: (props: ComponentPropsWithoutRef<"h2">) => (
    <h2
      className="mt-10 border-b border-white/5 pb-2 font-mono text-xs uppercase tracking-[0.18em] text-accent-primary"
      {...props}
    />
  ),
  h3: (props: ComponentPropsWithoutRef<"h3">) => (
    <h3
      className="mt-6 font-mono text-sm font-semibold text-text-primary"
      {...props}
    />
  ),
  p: (props: ComponentPropsWithoutRef<"p">) => (
    <p className="text-[14px] leading-relaxed text-text-muted" {...props} />
  ),
  a: (props: ComponentPropsWithoutRef<"a">) => (
    <a
      className="text-accent-primary underline-offset-2 hover:underline"
      target={props.href?.startsWith("http") ? "_blank" : undefined}
      rel={props.href?.startsWith("http") ? "noopener noreferrer" : undefined}
      {...props}
    />
  ),
  blockquote: (props: ComponentPropsWithoutRef<"blockquote">) => (
    <blockquote
      className="border-l-2 border-accent-primary/40 pl-3 font-mono text-[11px] uppercase tracking-[0.14em] text-text-muted/70"
      {...props}
    />
  ),
  code: (props: ComponentPropsWithoutRef<"code">) => (
    <code
      className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary"
      {...props}
    />
  ),
};
