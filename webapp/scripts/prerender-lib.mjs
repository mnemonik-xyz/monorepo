// Pure, side-effect-free helpers shared by scripts/prerender.mjs and the
// prerender test suite (src/test/prerender-blog.test.ts). Kept in its own module
// so the test can import the real extraction / sitemap / slug logic WITHOUT
// triggering prerender.mjs's top-level `main()` (which performs a full SSR build).
//
// Nothing here touches the filesystem or the network; callers own all I/O.

/** Canonical production origin. Mirrors ORIGIN in src/lib/seo.tsx. */
export const ORIGIN = "https://mnemonik.xyz";

/** Marker block in dist/index.html that the route's <head> is spliced into. */
export const SEO_BLOCK = /<!-- seo:start -->[\s\S]*?<!-- seo:end -->/;
/** The empty SPA mount point the prerendered body is injected into. */
export const ROOT_DIV = '<div id="root"></div>';

/**
 * Split a server-rendered route string into head tags and body markup.
 *
 * Every <title>, <meta>, <link> and application/ld+json <script> in the rendered
 * output originates from <Seo> (page components render none themselves), so
 * extracting by tag type is exact, not heuristic. React 19 hoists title/meta/link
 * to the front of the stream and renders the ld+json inline.
 *
 * ROBUSTNESS (T13): the <meta> and <link> matchers are QUOTE-AWARE, not "up to
 * the first `>`". React 19's server renderer DOES escape `<`, `>`, `&` and `"`
 * inside attribute values (an agent-published title like `a > b` or a
 * `</script><img onerror=x>` payload is emitted as `content="a &gt; b"` /
 * `&lt;/script&gt;&lt;img…`), so in practice a raw `>` never reaches the head
 * stream and even a naive `<meta[^>]*>` would not truncate today. We do NOT rely
 * on that: matching `(?:"[^"]*"|'[^']*'|[^>"'])*` consumes whole quoted runs so a
 * tag terminates only at its real closing `>` regardless of what a future React
 * version (or a hand-rolled head tag) puts inside a quoted attribute. Extraction
 * therefore can never silently corrupt following head tags. The executable-
 * injection boundary is held independently by safeJsonLd (ld+json) and React text
 * escaping (<title>, #root body); this matcher is the correctness/defence layer.
 */
export function splitHead(html) {
  const title = [];
  const meta = [];
  const link = [];
  const ld = [];
  const body = html
    .replace(/<title>[\s\S]*?<\/title>/g, (m) => (title.push(m), ""))
    .replace(
      /<script type="application\/ld\+json">[\s\S]*?<\/script>/g,
      (m) => (ld.push(m), ""),
    )
    .replace(/<meta\b(?:"[^"]*"|'[^']*'|[^>"'])*>/g, (m) => (meta.push(m), ""))
    .replace(/<link\b(?:"[^"]*"|'[^']*'|[^>"'])*>/g, (m) => (link.push(m), ""));
  const head = [...title, ...meta, ...link, ...ld]
    .map((tag) => `    ${tag}`)
    .join("\n");
  return { head, body };
}

/**
 * Splice a rendered route's head + body into the built SPA template. Uses
 * function replacers so `$&`, `$\``, `$'`, `$<n>` in live blog content are
 * copied verbatim rather than interpreted as replacement patterns.
 */
export function renderDocument(template, html) {
  const { head, body } = splitHead(html);
  return template
    .replace(SEO_BLOCK, () => `<!-- seo:start -->\n${head}\n    <!-- seo:end -->`)
    .replace(ROOT_DIV, () => `<div id="root">${body}</div>`);
}

/**
 * A slug is safe to use as a path segment and URL component only if it is the
 * kebab-case form the server emits (`slugify` in mcp/src/publish.rs): lowercase
 * ASCII alphanumerics with single internal dashes, no leading/trailing dash.
 * Rejecting anything else blocks path traversal (`../`), absolute paths, and
 * XML/URL metacharacters from an attacker-influenced build-time post list.
 */
export function isValidSlug(slug) {
  return typeof slug === "string" && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(slug);
}

/** Escape the five XML predefined entities for safe text in <loc>. */
export function xmlEscape(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

/**
 * Public, crawlable routes that always belong in the sitemap, independent of
 * any blog content. Private/interactive routes (/chat, /sign/:id, /oauth/consent)
 * are deliberately omitted. Mirrors the hand-maintained baseline that Vite copies
 * from public/sitemap.xml; the prerender step overwrites that copy with this list
 * plus the live blog slugs.
 */
export const STATIC_SITEMAP_ROUTES = [
  { path: "/", changefreq: "weekly", priority: "1.0" },
  { path: "/ledger", changefreq: "daily", priority: "0.8" },
  { path: "/analytics", changefreq: "daily", priority: "0.7" },
  { path: "/blog", changefreq: "weekly", priority: "0.7" },
  { path: "/install", changefreq: "monthly", priority: "0.6" },
  { path: "/roadmap", changefreq: "monthly", priority: "0.5" },
  { path: "/privacy", changefreq: "yearly", priority: "0.3" },
];

/**
 * Build sitemap.xml from the static public routes plus one entry per published
 * blog slug. `slugs` is assumed pre-validated by the caller (isValidSlug); each
 * <loc> is XML-escaped regardless as defence in depth.
 */
export function buildSitemap(slugs = []) {
  const entry = (loc, changefreq, priority) =>
    `  <url>\n    <loc>${xmlEscape(loc)}</loc>\n    <changefreq>${changefreq}</changefreq>\n    <priority>${priority}</priority>\n  </url>`;
  const urls = [
    ...STATIC_SITEMAP_ROUTES.map((r) =>
      entry(`${ORIGIN}${r.path}`, r.changefreq, r.priority),
    ),
    ...slugs.map((slug) =>
      entry(`${ORIGIN}/blog/${slug}`, "weekly", "0.6"),
    ),
  ];
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.join("\n")}\n</urlset>\n`;
}
