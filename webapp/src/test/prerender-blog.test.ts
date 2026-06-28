import { describe, expect, it } from "vitest";
import { renderBlogPost } from "../entry-server";
import type { BlogPost } from "../lib/blog";
import {
  buildSitemap,
  isValidSlug,
  renderDocument,
  splitHead,
} from "../../scripts/prerender-lib.mjs";

/**
 * Security + correctness gate for the Task-13 per-post prerender.
 *
 * Blog posts are agent-published: their title, summary, and body are
 * attacker-influenceable. This suite feeds a post whose fields carry the classic
 * stored-XSS payloads — `</script>`, `<img onerror=...>`, and a bare `>` — and
 * proves the build-time render + head-extraction pipeline neither executes them
 * nor truncates the document head.
 *
 * Two boundaries are asserted:
 *  - Executable injection is blocked: JSON-LD is `<`-escaped (safeJsonLd),
 *    the <title> text is HTML-escaped, and the markdown body escapes raw HTML —
 *    so no payload survives as a live tag in <head> or #root.
 *  - Head extraction stays correct (the T12 carried-forward finding). React 19
 *    escapes `<`/`>` inside attribute values, so a payload never reaches the head
 *    as a raw `>` in practice; the quote-aware <meta>/<link> matchers in splitHead
 *    are a defence layer proven directly against a synthetic raw-`>` attribute so
 *    a future regression cannot silently truncate and drop following head tags.
 */

const XSS_POST: BlogPost = {
  slug: "xss-probe",
  title: "Pwn </script><img src=x onerror=alert(1)> a > b",
  summary: "Break <script> out </script> with a raw > and <img onerror=x>",
  body_markdown:
    "Body with <img src=x onerror=alert(2)> and </script> and a > b.\n",
  author: "evil-agent",
  published_at: "2026-06-27T00:00:00.000Z",
  tags: ["xss"],
};

describe("prerender blog post (security)", () => {
  it("escapes all post-derived JSON-LD, title, and body — no executable injection", () => {
    const { head, body } = splitHead(renderBlogPost(XSS_POST.slug, XSS_POST));

    // JSON-LD: safeJsonLd replaces < and > with their \u00xx escapes, so the
    // ld+json block contains NO raw angle brackets — `</script>` cannot break
    // out of the inline script.
    const ld =
      head.match(
        /<script type="application\/ld\+json">([\s\S]*?)<\/script>/,
      )?.[1] ?? "";
    expect(ld.length).toBeGreaterThan(0);
    expect(ld).not.toContain("<");
    expect(ld).not.toContain(">");
    expect(ld).toContain("\\u003c"); // payload present, but escaped

    // <title> text is HTML-escaped (React escapes element text children).
    expect(head).toContain("&lt;/script&gt;");

    // #root body: the markdown renderer escapes raw HTML, so the payload appears
    // only as inert text — never as a live <img> or a breakout </script>.
    expect(body).not.toContain("<img");
    expect(body).not.toContain("</script>");
    expect(body).toContain("&lt;img");
  });

  it("keeps the full head when post fields carry angle-bracket payloads", () => {
    const { head } = splitHead(renderBlogPost(XSS_POST.slug, XSS_POST));

    // Even with `</script>` / `<img …>` / `>` in the title + summary, every head
    // tag React hoists AFTER the description/og meta survives extraction — the
    // canonical link, og:url, and <title> are all present and untruncated.
    expect(head).toContain('rel="canonical"');
    expect(head).toContain('property="og:url"');
    expect(head).toContain("<title>");
  });

  it("splitHead tokenises tags with > inside quoted attribute values", () => {
    // Isolated from React: a quoted attribute may legally contain `>`; the
    // following <link>/<meta> must still be extracted in full.
    const snippet =
      "<title>T</title>" +
      '<meta name="description" content="a > b"/>' +
      '<link rel="canonical" href="https://x/"/>' +
      '<meta property="og:title" content="c < d > e"/>';
    const { head, body } = splitHead(snippet);
    expect(head).toContain('content="a > b"');
    expect(head).toContain('rel="canonical"');
    expect(head).toContain('content="c < d > e"');
    expect(body).toBe(""); // every tag lifted into head, nothing left behind
  });

  it("assembles a full document with body injected and JSON-LD escaped", () => {
    const template =
      "<!doctype html><html><head>" +
      "<!-- seo:start -->\n<!-- seo:end -->" +
      '</head><body><div id="root"></div></body></html>';
    const doc = renderDocument(
      template,
      renderBlogPost(XSS_POST.slug, XSS_POST),
    );

    expect(doc).toContain('<div id="root">');
    expect(doc).not.toContain('<div id="root"></div>'); // no longer empty
    expect(doc).toContain("\\u003c/script\\u003e"); // escaped JSON-LD survived
    // No live <img> tag survives anywhere: the payload is only ever inert,
    // escaped text (`&lt;img …&gt;`), so its `onerror=` never attaches to a tag.
    expect(doc).not.toContain("<img");
  });
});

describe("prerender sitemap + slug validation", () => {
  it("accepts server kebab slugs and rejects traversal / unsafe forms", () => {
    expect(isValidSlug("hello-world")).toBe(true);
    expect(isValidSlug("anchoring-on-solana-and-arweave")).toBe(true);
    expect(isValidSlug("../etc/passwd")).toBe(false);
    expect(isValidSlug("Hello")).toBe(false); // uppercase
    expect(isValidSlug("-leading")).toBe(false);
    expect(isValidSlug("trailing-")).toBe(false);
    expect(isValidSlug("a--b")).toBe(false); // empty segment
    expect(isValidSlug("")).toBe(false);
  });

  it("builds a sitemap with static public routes plus every blog slug, omitting private routes", () => {
    const xml = buildSitemap(["hello-world", "anchoring-on-solana"]);
    expect(xml.startsWith("<?xml")).toBe(true);
    expect(xml).toContain("<loc>https://mnemonik.xyz/blog/hello-world</loc>");
    expect(xml).toContain(
      "<loc>https://mnemonik.xyz/blog/anchoring-on-solana</loc>",
    );
    // static public routes
    expect(xml).toContain("<loc>https://mnemonik.xyz/</loc>");
    expect(xml).toContain("<loc>https://mnemonik.xyz/blog</loc>");
    expect(xml).toContain("<loc>https://mnemonik.xyz/install</loc>");
    // private/interactive routes never appear
    expect(xml).not.toContain("/chat");
    expect(xml).not.toContain("/sign");
    expect(xml).not.toContain("/oauth");
    expect(xml.trim().endsWith("</urlset>")).toBe(true);
  });

  it("emits a static-only sitemap when there are no posts (fallback path)", () => {
    const xml = buildSitemap([]);
    expect(xml).toContain("<loc>https://mnemonik.xyz/blog</loc>");
    expect(xml).not.toContain("/blog/");
  });
});
