/**
 * `<Seo>` — per-route document metadata via React 19's native head hoisting.
 *
 * React 19 hoists `<title>`, `<meta>`, and `<link>` rendered anywhere in the
 * tree into `<head>`, so each page renders its own `<Seo>` and gets correct
 * title / description / canonical / OpenGraph + Twitter / JSON-LD. No external
 * origin is used, so the strict CSP (`script-src 'self'`) is preserved — the
 * JSON-LD is an inline script, which is same-origin, not a remote `src`.
 */

const SITE_NAME = "Mnemonic Protocol";
const ORIGIN = "https://mnemonik.xyz";
const DEFAULT_IMAGE = `${ORIGIN}/og-image.png`;

export interface SeoProps {
  /** Page title; site name is appended unless `exactTitle` is set. */
  title: string;
  description: string;
  /** Path (e.g. "/ledger") or absolute URL; becomes the canonical + og:url. */
  canonical: string;
  /** Absolute image URL for OG/Twitter cards. */
  image?: string;
  /** "website" (default) or "article". */
  type?: "website" | "article";
  /** Use `title` verbatim instead of appending the site name. */
  exactTitle?: boolean;
  /** One or more JSON-LD objects injected as application/ld+json. */
  jsonLd?: Record<string, unknown> | Array<Record<string, unknown>>;
}

function absolute(path: string): string {
  if (/^https?:\/\//.test(path)) return path;
  return `${ORIGIN}${path.startsWith("/") ? path : `/${path}`}`;
}

export function Seo({
  title,
  description,
  canonical,
  image = DEFAULT_IMAGE,
  type = "website",
  exactTitle = false,
  jsonLd,
}: SeoProps) {
  const fullTitle = exactTitle ? title : `${title} — ${SITE_NAME}`;
  const url = absolute(canonical);
  const blocks = jsonLd ? (Array.isArray(jsonLd) ? jsonLd : [jsonLd]) : [];

  return (
    <>
      <title>{fullTitle}</title>
      <meta name="description" content={description} />
      <link rel="canonical" href={url} />

      <meta property="og:type" content={type} />
      <meta property="og:site_name" content={SITE_NAME} />
      <meta property="og:title" content={fullTitle} />
      <meta property="og:description" content={description} />
      <meta property="og:url" content={url} />
      <meta property="og:image" content={image} />

      <meta name="twitter:card" content="summary_large_image" />
      <meta name="twitter:title" content={fullTitle} />
      <meta name="twitter:description" content={description} />
      <meta name="twitter:image" content={image} />

      {blocks.map((block, i) => (
        <script
          // eslint-disable-next-line react/no-danger
          key={i}
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: JSON.stringify(block) }}
        />
      ))}
    </>
  );
}

/** JSON-LD for the protocol as an Organization (used on the landing page). */
export function organizationJsonLd(): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@type": "Organization",
    name: SITE_NAME,
    url: ORIGIN,
    description:
      "Verifiable, persistent memory for AI agents — signed, anchored on Solana, accessed via MCP.",
    sameAs: ["https://github.com/mnemonik-xyz/monorepo"],
  };
}

/** JSON-LD for a blog post (BlogPosting), used on /blog/:slug. */
export function articleJsonLd(args: {
  title: string;
  description: string;
  url: string;
  author: string;
  publishedAt: string;
}): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@type": "BlogPosting",
    headline: args.title,
    description: args.description,
    url: absolute(args.url),
    datePublished: args.publishedAt,
    author: { "@type": "Person", name: args.author },
    publisher: { "@type": "Organization", name: SITE_NAME },
  };
}

export default Seo;
