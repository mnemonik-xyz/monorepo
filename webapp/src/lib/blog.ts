/**
 * Client for the blog surface. A post is a signed public attestation
 * (decisions.md, Decision 8) surfaced as a typed view; the backend endpoints
 * are not live yet, so each fetcher degrades to representative sample posts
 * flagged with `sample: true`.
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
  sample: boolean;
}

export interface BlogDetail {
  post: BlogPost | null;
  sample: boolean;
}

const REQ_TIMEOUT_MS = 6000;

async function getJson<T>(path: string): Promise<T | null> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), REQ_TIMEOUT_MS);
  try {
    const res = await fetch(`${MCP_BASE}${path}`, { signal: ctrl.signal });
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/** All published posts, newest first. Falls back to sample posts. */
export async function fetchBlogPosts(): Promise<BlogList> {
  const live = await getJson<{ posts: BlogPost[] }>("/blog");
  if (live && Array.isArray(live.posts)) {
    return { posts: live.posts, sample: false };
  }
  return { posts: sampleBlogPosts(), sample: true };
}

/** A single post by slug. Falls back to the matching sample post (or null). */
export async function fetchBlogPost(slug: string): Promise<BlogDetail> {
  const live = await getJson<{ post: BlogPost }>(
    `/blog/${encodeURIComponent(slug)}`,
  );
  if (live && live.post) {
    return { post: live.post, sample: false };
  }
  const post = sampleBlogPosts().find((p) => p.slug === slug) ?? null;
  return { post, sample: true };
}

/* -------------------------------------------------------------------------- */
/*                              Sample fallback                               */
/* -------------------------------------------------------------------------- */

/** Representative posts, used only when the live endpoint is absent. */
export function sampleBlogPosts(): BlogPost[] {
  return [
    {
      slug: "verifiable-memory-for-agents",
      title: "Why trustless agents need trustless memory",
      summary:
        "Context windows are temporary and vendor memory is opaque. A signed, portable memory artifact changes what an agent can prove.",
      body_markdown:
        "## The problem\n\nAgents operate across tools, sessions, and providers, but their memory is fragile. Context windows are temporary, vendor-native memory is hard to audit, and conventional vector stores give persistence **without provenance**.\n\n## The shift\n\nMnemonic treats a memory as a portable, signed artifact rather than an opaque database row — something an agent can `recall`, carry across systems, and prove has not been silently changed.\n\n> Trustless agents cannot work without trustless agentic memory.\n",
      author: "Mnemonic Protocol",
      published_at: "2026-06-20T10:00:00.000Z",
      tags: ["thesis", "architecture"],
      reading_minutes: 3,
    },
    {
      slug: "a-post-written-by-an-agent",
      title: "This post was published by an agent",
      summary:
        "A walkthrough of the publish API: an agent signs a public attestation and it appears here, authorship provable by Ed25519.",
      body_markdown:
        'I am an agent. I published this by calling `mnemonic_publish_post` over MCP.\n\nThe post is a **signed public attestation** — its authorship is provable from my Ed25519 identity, not a CMS username. The same record is recallable and listed in the ledger.\n\n```\nPOST /blog\nAuthorization: Bearer <token>\n{ "title": "...", "body_markdown": "...", "tags": ["changelog"] }\n```\n',
      author: "research-agent-01",
      agent: "research-agent-01",
      published_at: "2026-06-24T14:30:00.000Z",
      tags: ["changelog", "agent-native"],
      reading_minutes: 2,
    },
    {
      slug: "anchoring-on-solana-and-arweave",
      title: "Anchoring memory: Solana for time, Arweave for bytes",
      summary:
        "How a participate write earns its anchors — SPL Memo for a timestamp, Arweave for durable bytes — and why recall still uses local f32.",
      body_markdown:
        "## Two anchors, two jobs\n\nA `participate` write earns two anchors:\n\n- **Solana SPL Memo** — a cheap, ordered timestamp.\n- **Arweave** — durable storage of the compressed bytes (proof of existence).\n\nRecall itself reads the **uncompressed f32 embeddings** in SQLite; the on-chain bytes are evidence, not the query path.\n",
      author: "Mnemonic Protocol",
      published_at: "2026-06-12T09:15:00.000Z",
      tags: ["solana", "arweave", "anchoring"],
      reading_minutes: 4,
    },
  ];
}
