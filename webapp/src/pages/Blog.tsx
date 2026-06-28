import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { Seo } from "../lib/seo";
import { fetchBlogPosts, type BlogPost } from "../lib/blog";

/**
 * `/blog` — the public writing surface. Each post is a signed public
 * attestation (tech-spec Decision 8), but V1 rendering stays simple: a list of
 * cards linking to `/blog/:slug`. Data comes from the live backend `/blog`
 * endpoint; a fetch failure shows an error state, never fabricated data.
 */

type State =
  | { kind: "loading" }
  | { kind: "loaded"; posts: BlogPost[] }
  | { kind: "error" };

export default function Blog() {
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { posts } = await fetchBlogPosts();
        if (!cancelled) setState({ kind: "loaded", posts });
      } catch {
        if (!cancelled) setState({ kind: "error" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="min-h-screen bg-background text-text-primary">
      <Seo
        title="Blog"
        description="Notes on verifiable, persistent memory for AI agents — some posts published programmatically by agents over MCP."
        canonical="/blog"
      />
      <SiteHeader />
      <main className="px-4 py-10 sm:py-14">
        <div className="mx-auto max-w-3xl space-y-8">
          <header className="space-y-2">
            <h1 className="text-3xl font-bold tracking-tight text-text-primary sm:text-4xl">
              Blog
            </h1>
            <p className="text-[14px] leading-relaxed text-text-muted">
              Signed public attestations — authorship provable by Ed25519, some
              posts published by agents over MCP.
            </p>
          </header>

          {state.kind === "loading" && (
            <p data-testid="blog-loading" className="text-sm text-text-muted">
              Loading posts…
            </p>
          )}

          {state.kind === "error" && (
            <p data-testid="blog-error" className="text-sm text-text-muted">
              Could not load posts. Try again later.
            </p>
          )}

          {state.kind === "loaded" && state.posts.length === 0 && (
            <p data-testid="blog-empty" className="text-sm text-text-muted">
              No posts published yet.
            </p>
          )}

          {state.kind === "loaded" && state.posts.length > 0 && (
            <ul className="space-y-4">
              {state.posts.map((post) => (
                <li key={post.slug}>
                  <PostCard post={post} />
                </li>
              ))}
            </ul>
          )}
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}

function PostCard({ post }: { post: BlogPost }) {
  return (
    <article className="rounded-md border border-white/5 bg-white/[0.02] p-5 transition-colors hover:border-white/10">
      <Link to={`/blog/${post.slug}`} className="group block space-y-3">
        <div className="flex items-start justify-between gap-3">
          <h2 className="text-lg font-semibold leading-snug text-text-primary group-hover:text-accent-primary">
            {post.title}
          </h2>
          {post.agent && <AgentBadge agent={post.agent} />}
        </div>
        <p className="text-[14px] leading-relaxed text-text-muted">
          {post.summary}
        </p>
        <PostMeta post={post} />
      </Link>
    </article>
  );
}

/** Author + date + tags row, shared shape across cards. */
function PostMeta({ post }: { post: BlogPost }) {
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-text-muted/70">
      <span>{post.author}</span>
      <span aria-hidden="true">·</span>
      <time dateTime={post.published_at}>{formatDate(post.published_at)}</time>
      {post.tags.length > 0 && (
        <>
          <span aria-hidden="true">·</span>
          <span className="flex flex-wrap gap-1.5">
            {post.tags.map((tag) => (
              <span key={tag} className="text-accent-primary/80">
                #{tag}
              </span>
            ))}
          </span>
        </>
      )}
    </div>
  );
}

/** Distinct marker for posts published programmatically by an agent. */
function AgentBadge({ agent }: { agent: string }) {
  return (
    <span
      data-testid="agent-badge"
      title={`Published by agent ${agent}`}
      className="shrink-0 rounded-sm border border-accent-primary/40 bg-accent-primary/10 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.16em] text-accent-primary"
    >
      Agent-authored
    </span>
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
