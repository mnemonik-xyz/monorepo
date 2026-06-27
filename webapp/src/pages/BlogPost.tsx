import type { ComponentPropsWithoutRef } from "react";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { Seo, articleJsonLd } from "../lib/seo";
import { fetchBlogPost, type BlogPost } from "../lib/blog";

/**
 * `/blog/:slug` — a single post. The body is `body_markdown`, rendered with
 * react-markdown + remark-gfm. Raw-HTML rendering is intentionally NOT enabled
 * (no `rehype-raw`): react-markdown escapes embedded HTML and strips dangerous
 * URL protocols by default, so an attacker-controlled (agent-published) body
 * cannot inject markup or `javascript:` links (tech-spec: render markdown
 * safely, no raw HTML injection). An unknown slug renders a graceful not-found.
 *
 * `initialPost` is supplied ONLY by the build-time prerender (entry-server's
 * `renderBlogPost`): it seeds the loaded state so the server snapshot carries the
 * real title + body + Article JSON-LD for crawlers, instead of the loading shell.
 * The browser bundle renders `<BlogPost />` with no prop, so humans still fetch
 * the live post in `useEffect` (which never runs during SSR).
 */

type State =
  | { kind: "loading" }
  | { kind: "loaded"; post: BlogPost; sample: boolean }
  | { kind: "not-found" }
  | { kind: "error" };

export default function BlogPost({
  initialPost,
}: {
  initialPost?: BlogPost;
} = {}) {
  const { slug } = useParams<{ slug: string }>();
  const [state, setState] = useState<State>(
    initialPost
      ? { kind: "loaded", post: initialPost, sample: false }
      : { kind: "loading" },
  );

  useEffect(() => {
    if (!slug) {
      setState({ kind: "not-found" });
      return;
    }
    let cancelled = false;
    setState({ kind: "loading" });
    (async () => {
      try {
        const { post, sample } = await fetchBlogPost(slug);
        if (cancelled) return;
        setState(
          post ? { kind: "loaded", post, sample } : { kind: "not-found" },
        );
      } catch {
        if (!cancelled) setState({ kind: "error" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [slug]);

  return (
    <div className="min-h-screen bg-background text-text-primary">
      {state.kind === "loaded" && <PostSeo post={state.post} />}
      <SiteHeader />
      <main className="px-4 py-10 sm:py-14">
        <div className="mx-auto max-w-3xl space-y-8">
          <Link
            to="/blog"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            ← All posts
          </Link>

          {state.kind === "loading" && (
            <p data-testid="post-loading" className="text-sm text-text-muted">
              Loading post…
            </p>
          )}

          {state.kind === "error" && (
            <p data-testid="post-error" className="text-sm text-text-muted">
              Could not load this post. Try again later.
            </p>
          )}

          {state.kind === "not-found" && <NotFound />}

          {state.kind === "loaded" && (
            <Article post={state.post} sample={state.sample} />
          )}
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}

function PostSeo({ post }: { post: BlogPost }) {
  const canonical = `/blog/${post.slug}`;
  return (
    <Seo
      title={post.title}
      description={post.summary}
      canonical={canonical}
      type="article"
      jsonLd={articleJsonLd({
        title: post.title,
        description: post.summary,
        url: canonical,
        author: post.author,
        publishedAt: post.published_at,
      })}
    />
  );
}

function Article({ post, sample }: { post: BlogPost; sample: boolean }) {
  return (
    <article className="space-y-6">
      {sample && <SampleBanner />}
      <header className="space-y-3">
        <h1 className="text-3xl font-bold tracking-tight text-text-primary sm:text-4xl">
          {post.title}
        </h1>
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-text-muted/70">
          <span>{post.author}</span>
          {post.agent && (
            <span
              data-testid="agent-badge"
              title={`Published by agent ${post.agent}`}
              className="rounded-sm border border-accent-primary/40 bg-accent-primary/10 px-2 py-0.5 text-[10px] text-accent-primary"
            >
              Agent-authored
            </span>
          )}
          <span aria-hidden="true">·</span>
          <time dateTime={post.published_at}>
            {formatDate(post.published_at)}
          </time>
          {typeof post.reading_minutes === "number" && (
            <>
              <span aria-hidden="true">·</span>
              <span>{post.reading_minutes} min read</span>
            </>
          )}
        </div>
        {post.tags.length > 0 && (
          <div className="flex flex-wrap gap-1.5 font-mono text-[11px] tracking-[0.14em] text-accent-primary/80">
            {post.tags.map((tag) => (
              <span key={tag}>#{tag}</span>
            ))}
          </div>
        )}
      </header>

      <div
        data-testid="post-body"
        className="space-y-4 border-t border-white/5 pt-6 text-text-primary"
      >
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={MARKDOWN_COMPONENTS}
        >
          {post.body_markdown}
        </ReactMarkdown>
      </div>
    </article>
  );
}

function NotFound() {
  return (
    <div data-testid="post-not-found" className="space-y-3">
      <h1 className="text-2xl font-bold text-text-primary">Post not found</h1>
      <p className="text-[14px] leading-relaxed text-text-muted">
        This post does not exist or is no longer published.
      </p>
    </div>
  );
}

function SampleBanner() {
  return (
    <p
      data-testid="sample-banner"
      className="rounded-md border border-amber-400/20 bg-amber-400/[0.06] px-4 py-2 font-mono text-[11px] uppercase tracking-[0.16em] text-amber-200/80"
    >
      Sample data (not live)
    </p>
  );
}

/** ISO-8601 → readable date; falls back to the raw string if unparseable. */
function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// Tailwind-styled overrides for the markdown renderer, mirroring the pattern in
// Roadmap.tsx (kept local per task scope) and extended with the block elements a
// long-form post body uses: lists, fenced code, strong/em, hr.
const MARKDOWN_COMPONENTS = {
  h1: (props: ComponentPropsWithoutRef<"h1">) => (
    <h1
      className="text-2xl font-bold tracking-tight text-text-primary"
      {...props}
    />
  ),
  h2: (props: ComponentPropsWithoutRef<"h2">) => (
    <h2
      className="mt-8 border-b border-white/5 pb-2 font-mono text-xs uppercase tracking-[0.18em] text-accent-primary"
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
    // Spread incoming props FIRST so the computed safe target/rel always win:
    // external links get noopener/noreferrer regardless of what the AST carries.
    <a
      className="text-accent-primary underline-offset-2 hover:underline"
      {...props}
      target={props.href?.startsWith("http") ? "_blank" : undefined}
      rel={props.href?.startsWith("http") ? "noopener noreferrer" : undefined}
    />
  ),
  ul: (props: ComponentPropsWithoutRef<"ul">) => (
    <ul
      className="list-disc space-y-1 pl-5 text-[14px] leading-relaxed text-text-muted"
      {...props}
    />
  ),
  ol: (props: ComponentPropsWithoutRef<"ol">) => (
    <ol
      className="list-decimal space-y-1 pl-5 text-[14px] leading-relaxed text-text-muted"
      {...props}
    />
  ),
  li: (props: ComponentPropsWithoutRef<"li">) => (
    <li className="marker:text-text-muted/50" {...props} />
  ),
  blockquote: (props: ComponentPropsWithoutRef<"blockquote">) => (
    <blockquote
      className="border-l-2 border-accent-primary/40 pl-3 font-mono text-[11px] uppercase tracking-[0.14em] text-text-muted/70"
      {...props}
    />
  ),
  strong: (props: ComponentPropsWithoutRef<"strong">) => (
    <strong className="font-semibold text-text-primary" {...props} />
  ),
  hr: (props: ComponentPropsWithoutRef<"hr">) => (
    <hr className="border-white/5" {...props} />
  ),
  pre: (props: ComponentPropsWithoutRef<"pre">) => (
    <pre
      className="overflow-x-auto rounded-md border border-white/5 bg-white/[0.04] p-4 font-mono text-[12px] text-text-primary"
      {...props}
    />
  ),
  code: (props: ComponentPropsWithoutRef<"code">) => (
    <code
      className="rounded bg-white/[0.06] px-1 py-0.5 font-mono text-[12px] text-text-primary"
      {...props}
    />
  ),
  table: (props: ComponentPropsWithoutRef<"table">) => (
    <table
      className="w-full border-collapse text-left text-[13px] text-text-muted"
      {...props}
    />
  ),
  th: (props: ComponentPropsWithoutRef<"th">) => (
    <th
      className="border-b border-white/10 py-1 pr-4 font-semibold"
      {...props}
    />
  ),
  td: (props: ComponentPropsWithoutRef<"td">) => (
    <td className="border-b border-white/5 py-1 pr-4" {...props} />
  ),
};
