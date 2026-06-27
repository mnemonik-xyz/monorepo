import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Post-build gate for Task 12 (SEO prerender).
 *
 * `npm run build` runs `scripts/prerender.mjs`, which writes a real HTML file per
 * static route into dist/. This suite reads those built artifacts and asserts the
 * route-specific <head> (title / description / canonical / OpenGraph + Twitter /
 * JSON-LD produced by <Seo> via React 19 head hoisting) plus the no-JS body shell
 * — i.e. a crawler gets real, route-specific content without executing JS.
 *
 * This is a GATE, not a soft check: it must FAIL when dist/ is missing or STALE,
 * because a passing run against a stale dist/ would hide a broken prerender step
 * (the exact failure mode the test-reviewer caught). So instead of skipping when
 * dist/ is absent we hard-fail (see "dist was built fresh"), and we assert each
 * artifact's mtime is newer than every prerender source input.
 */

const webappRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const distDir = resolve(webappRoot, "dist");
const ORIGIN = "https://mnemonik.xyz";
const DEFAULT_IMAGE = `${ORIGIN}/og-image.png`;

interface RouteSpec {
  /** dist path relative to dist/. */
  file: string;
  /** Full <title> (page title + site name), unique per route. */
  title: string;
  /** Expected canonical + og:url (route-specific, never the bare origin leak). */
  url: string;
  /** A string only present in this route's no-JS static shell (#root content). */
  shell: string;
  /** Whether <Seo> supplies JSON-LD for this route. */
  jsonLd: boolean;
}

// Single source of truth for the four prerendered routes. Titles/urls/shell come
// from each page's <Seo> props and static markup (see pages/*.tsx).
const ROUTES = {
  "/": {
    file: "index.html",
    title: "Verifiable memory for AI agents — Mnemonic Protocol",
    url: `${ORIGIN}/`,
    shell: "Mnemonic Protocol · v0.1",
    jsonLd: true,
  },
  "/ledger": {
    file: "ledger/index.html",
    title: "Ledger — Mnemonic Protocol",
    url: `${ORIGIN}/ledger`,
    shell: "Recalled artifacts",
    jsonLd: false,
  },
  "/analytics": {
    file: "analytics/index.html",
    title: "Analytics — attestations over time — Mnemonic Protocol",
    url: `${ORIGIN}/analytics`,
    shell: "Network analytics",
    jsonLd: true,
  },
  "/blog": {
    file: "blog/index.html",
    title: "Blog — Mnemonic Protocol",
    url: `${ORIGIN}/blog`,
    shell: "Signed public attestations",
    jsonLd: false,
  },
} satisfies Record<string, RouteSpec>;

function readRoute(file: string): string {
  return readFileSync(resolve(distDir, file), "utf8");
}

/** content="" of a <meta> by its name/property key (attribute order tolerant). */
function metaContent(html: string, key: string): string {
  const re = new RegExp(
    `<meta[^>]*\\b(?:property|name)="${key}"[^>]*\\bcontent="([^"]*)"`,
  );
  return html.match(re)?.[1] ?? "";
}

/** Newest mtime (ms) across every input the prerender output derives from. */
function newestSourceMtimeMs(): number {
  const inputs = [
    resolve(webappRoot, "scripts/prerender.mjs"),
    resolve(webappRoot, "index.html"),
  ];
  const srcDir = resolve(webappRoot, "src");
  for (const rel of readdirSync(srcDir, { recursive: true }) as string[]) {
    if (!/\.(ts|tsx)$/.test(rel)) continue;
    if (/\.test\.[tj]sx?$/.test(rel)) continue; // tests aren't prerender inputs
    if (rel.split(sep).includes("test")) continue;
    inputs.push(resolve(srcDir, rel));
  }
  let max = 0;
  for (const f of inputs) {
    const s = statSync(f);
    if (s.isFile() && s.mtimeMs > max) max = s.mtimeMs;
  }
  return max;
}

describe("prerender", () => {
  it("dist was built fresh (regenerate-then-assert, never stale)", () => {
    const newestSrc = newestSourceMtimeMs();
    for (const { file } of Object.values(ROUTES)) {
      const path = resolve(distDir, file);
      // Hard-fail (not skip) when an artifact is missing: a green run must mean
      // the prerender actually ran.
      expect(statSync(path).isFile(), `missing dist artifact ${file}`).toBe(
        true,
      );
      // Freshness: artifact must be newer than every source — a stale dist/ left
      // over from an earlier build (the bug we are guarding against) fails here.
      expect(
        statSync(path).mtimeMs,
        `stale dist artifact ${file} — rebuild with \`npm run build\``,
      ).toBeGreaterThanOrEqual(newestSrc);
    }
  });

  it("each route has a unique, route-specific <title> (exactly once)", () => {
    const titles: string[] = [];
    for (const [route, spec] of Object.entries(ROUTES)) {
      const html = readRoute(spec.file);
      expect(html.match(/<title>/g)?.length, `title count on ${route}`).toBe(1);
      expect(html, `title on ${route}`).toContain(
        `<title>${spec.title}</title>`,
      );
      titles.push(spec.title);
    }
    // Cross-route uniqueness: a default-block leak would repeat one title.
    expect(new Set(titles).size).toBe(4);
  });

  it("each route has a route-specific canonical (not the bare origin leak)", () => {
    const canonicals: string[] = [];
    for (const [route, spec] of Object.entries(ROUTES)) {
      expect(readRoute(spec.file), `canonical on ${route}`).toContain(
        `<link rel="canonical" href="${spec.url}"/>`,
      );
      canonicals.push(spec.url);
    }
    expect(new Set(canonicals).size).toBe(4);
  });

  it("each route has route-specific OpenGraph + Twitter tags", () => {
    const ogUrls: string[] = [];
    for (const [route, spec] of Object.entries(ROUTES)) {
      const html = readRoute(spec.file);
      // og:url/og:title must be the ROUTE's, not a shared default block — a leak
      // would put og:url=https://mnemonik.xyz/ on every route.
      expect(metaContent(html, "og:url"), `og:url on ${route}`).toBe(spec.url);
      expect(metaContent(html, "og:title"), `og:title on ${route}`).toBe(
        spec.title,
      );
      // Social card presence (shared values are fine for these two).
      expect(metaContent(html, "og:image"), `og:image on ${route}`).toBe(
        DEFAULT_IMAGE,
      );
      expect(
        metaContent(html, "twitter:card"),
        `twitter:card on ${route}`,
      ).toBe("summary_large_image");
      ogUrls.push(metaContent(html, "og:url"));
    }
    // Distinct og:url per route catches the default-block leak directly.
    expect(new Set(ogUrls).size).toBe(4);
  });

  it("each route's no-JS body shell carries its own content", () => {
    const marker = '<div id="root">';
    for (const [route, spec] of Object.entries(ROUTES)) {
      const html = readRoute(spec.file);
      const start = html.indexOf(marker);
      expect(start, `no #root on ${route}`).toBeGreaterThanOrEqual(0);
      // #root must hold the prerendered shell, not the empty `<div id="root">
      // </div>` placeholder (a crawler with JS disabled would see nothing).
      const afterRoot = html.slice(start + marker.length);
      expect(afterRoot.startsWith("</div>"), `empty #root on ${route}`).toBe(
        false,
      );
      // Route-specific static markup (not <head>) — proves per-route content.
      expect(html, `route shell on ${route}`).toContain(spec.shell);
    }
  });

  it("emits JSON-LD only on routes whose <Seo> supplies it", () => {
    for (const [route, spec] of Object.entries(ROUTES)) {
      const html = readRoute(spec.file);
      expect(
        html.includes('type="application/ld+json"'),
        `json-ld on ${route}`,
      ).toBe(spec.jsonLd);
    }
  });

  it("keeps global head tags and CSP intact, with same-origin scripts only", () => {
    const html = readRoute(ROUTES["/ledger"].file);
    expect(html).toContain("Content-Security-Policy");
    expect(html).toContain('name="theme-color"');
    // The only script with a src is the same-origin module bundle; no new
    // external origins or inline executable scripts were introduced.
    const scriptSrcs = [...html.matchAll(/<script[^>]*\bsrc="([^"]*)"/g)].map(
      (m) => m[1] ?? "",
    );
    for (const src of scriptSrcs) {
      expect(src.startsWith("/")).toBe(true);
    }
  });
});
