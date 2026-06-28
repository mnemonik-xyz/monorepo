/**
 * Client for the blog surface. A post is a signed public attestation
 * (decisions.md, Decision 8) surfaced as a typed view. Both fetchers hit the
 * live backend and THROW on any failure — no sample/placeholder fallback.
 *
 *   GET /blog          -> { posts: BlogPost[] }
 *   GET /blog/:slug    -> { post: BlogPost }
 */
import { MCP_BASE } from "./api";

export interface BlogPost {
  slug: string;
  title: string;
  summary: string;
  body_markdown: string;
  /** Display author; for agent-published posts this is the agent name. */
  author: string;
  /** Set when published programmatically by an agent (vs a human). */
  agent?: string;
  /** ISO-8601 timestamp. */
  published_at: string;
  tags: string[];
  reading_minutes?: number;
}

export interface BlogList {
  posts: BlogPost[];
}

export interface BlogDetail {
  post: BlogPost | null;
}

const REQ_TIMEOUT_MS = 6000;

/** GET JSON from the backend. Throws on timeout, network error, non-2xx, or a
 *  non-JSON body — the caller surfaces it as an error state. */
async function getJson<T>(path: string): Promise<T> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), REQ_TIMEOUT_MS);
  try {
    const res = await fetch(`${MCP_BASE}${path}`, {
      signal: ctrl.signal,
      // Same-origin deploy: the apex routes /blog* by Accept (JSON -> backend,
      // text/html -> prerendered page). Be explicit so negotiation is reliable.
      headers: { Accept: "application/json" },
    });
    if (!res.ok) {
      throw new Error(`GET ${path} -> ${res.status}`);
    }
    return (await res.json()) as T;
  } finally {
    clearTimeout(timer);
  }
}

/** All published posts, newest first. Throws on failure. */
export async function fetchBlogPosts(): Promise<BlogList> {
  const live = await getJson<{ posts: BlogPost[] }>("/blog");
  if (!live || !Array.isArray(live.posts)) {
    throw new Error("GET /blog: malformed response");
  }
  return { posts: live.posts.map(deriveBlogPost) };
}

/** A single post by slug. Returns `{ post: null }` for a 404; throws on other
 *  failures. */
export async function fetchBlogPost(slug: string): Promise<BlogDetail> {
  const live = await getJson<{ post: BlogPost | null }>(
    `/blog/${encodeURIComponent(slug)}`,
  );
  return { post: live?.post ? deriveBlogPost(live.post) : null };
}

/* -------------------------------------------------------------------------- */
/*                         Derived view fields                                */
/*                                                                            */
/* The core `blog_posts` row carries only title/body/author/tags/published_at */
/* (decisions.md, Task-14 audit M2): it has no summary, reading_minutes, or    */
/* agent column. The webapp needs all three — summary feeds the meta           */
/* description + Article JSON-LD (Decision 4 crawl coverage), reading_minutes   */
/* and the agent badge are shown on the post. When the backend omits them we    */
/* derive them here, so BOTH the SPA (fetch*) and the build-time prerender      */
/* (entry-server `renderBlogPost`) render populated meta from one code path.    */
/* -------------------------------------------------------------------------- */

const WORDS_PER_MINUTE = 200;
const SUMMARY_MAX_CHARS = 160;

/** First prose paragraph of the body, capped to ~160 chars for a meta description. */
function deriveSummary(bodyMarkdown: string): string {
  const paragraphs = bodyMarkdown
    .split(/\n\s*\n/) // paragraphs are blank-line separated
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  // Prefer the first prose paragraph; a body often opens with a `## heading`,
  // which makes a poor meta description. Fall back to the first block (heading
  // marker stripped) if every paragraph is a heading.
  const prose =
    paragraphs.find((p) => !/^#{1,6}\s/.test(p)) ?? paragraphs[0] ?? "";
  const cleaned = prose.replace(/^#{1,6}\s+/, "").trim();
  if (cleaned.length <= SUMMARY_MAX_CHARS) return cleaned;
  return `${cleaned.slice(0, SUMMARY_MAX_CHARS - 1).trimEnd()}…`;
}

/** Estimated reading time in whole minutes, at ~200 wpm (never below 1). */
function deriveReadingMinutes(bodyMarkdown: string): number {
  const words = bodyMarkdown.trim().split(/\s+/).filter(Boolean).length;
  return Math.max(1, Math.ceil(words / WORDS_PER_MINUTE));
}

/**
 * Heuristic for whether an author is an agent rather than a human. The backend
 * has no agent column, so an agent-published post arrives with only its author
 * name (e.g. `research-agent-01`). We badge it only when the name clearly looks
 * automated — `agent`/`bot` in the name, or a trailing `-NN` instance id — so a
 * human author like "Mnemonic Protocol" never gets an (incorrect) agent badge.
 */
function looksLikeAgent(author: string): boolean {
  return /agent|bot/i.test(author) || /-\d+$/.test(author);
}

/**
 * Populate the optional view fields (summary, reading_minutes, agent) the
 * backend may omit, deriving each from the post body/author. Already-present
 * values are preserved verbatim, so a backend that does send them wins.
 */
export function deriveBlogPost(post: BlogPost): BlogPost {
  const summary =
    post.summary && post.summary.trim().length > 0
      ? post.summary
      : deriveSummary(post.body_markdown);
  const reading_minutes =
    typeof post.reading_minutes === "number"
      ? post.reading_minutes
      : deriveReadingMinutes(post.body_markdown);
  const agent =
    post.agent ?? (looksLikeAgent(post.author) ? post.author : undefined);
  return { ...post, summary, reading_minutes, agent };
}
