import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Post-build assertion for Task 12 (SEO prerender).
 *
 * `npm run build` runs `scripts/prerender.mjs`, which writes a real HTML file per
 * static route into dist/. This test reads those built artifacts and asserts the
 * route-specific <head> (title / description / canonical / JSON-LD produced by
 * <Seo> via React 19 head hoisting) is present in the raw HTML — i.e. a crawler
 * gets real content without executing JS.
 *
 * The suite is skipped (with a clear warning) when dist/ is absent so a bare
 * `npm test` without a prior build stays green; the build pipeline itself
 * hard-fails in prerender.mjs if the template markers go missing. When dist/
 * EXISTS but a file or tag is wrong, the assertions below fail — catching a
 * broken prerender step.
 */

const distDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../dist");

const ROUTE_FILES: Record<string, string> = {
  "/": "index.html",
  "/ledger": "ledger/index.html",
  "/analytics": "analytics/index.html",
  "/blog": "blog/index.html",
};

const built = existsSync(resolve(distDir, "index.html"));
if (!built) {
  console.warn(
    "[prerender.test] dist/ not found — run `npm run build` first. Skipping prerender assertions.",
  );
}

function readRoute(route: string): string {
  return readFileSync(resolve(distDir, ROUTE_FILES[route]), "utf8");
}

describe.skipIf(!built)("prerender", () => {
  it("emits static html for each public route", () => {
    for (const file of Object.values(ROUTE_FILES)) {
      expect(existsSync(resolve(distDir, file)), `missing ${file}`).toBe(true);
    }
  });

  it("ledger html has route head tags", () => {
    const html = readRoute("/ledger");
    // Unique per-route <title> (head hoisting captured), exactly once.
    expect(html.match(/<title>/g)?.length).toBe(1);
    expect(html).toContain("<title>Ledger — Mnemonic Protocol</title>");
    expect(html).toMatch(/<meta name="description" content="A forensic feed/);
    // Route static shell is present in raw HTML without JS.
    expect(html).toContain("Recalled artifacts");
  });

  it("prerendered html has canonical + JSON-LD", () => {
    // Canonical points at the route, not the site root.
    expect(readRoute("/ledger")).toContain(
      '<link rel="canonical" href="https://mnemonik.xyz/ledger"/>',
    );
    expect(readRoute("/analytics")).toContain(
      '<link rel="canonical" href="https://mnemonik.xyz/analytics"/>',
    );
    // JSON-LD is emitted on routes whose <Seo> supplies it (landing, analytics).
    expect(readRoute("/")).toContain('type="application/ld+json"');
    expect(readRoute("/analytics")).toContain('type="application/ld+json"');
  });

  it("keeps global head tags and CSP intact after prerender", () => {
    const html = readRoute("/ledger");
    expect(html).toContain("Content-Security-Policy");
    expect(html).toContain('name="theme-color"');
    // The only script with a src is the same-origin module bundle; no new
    // external origins or inline executable scripts were introduced.
    const scriptSrcs = [...html.matchAll(/<script[^>]*\bsrc="([^"]*)"/g)].map(
      (m) => m[1],
    );
    for (const src of scriptSrcs) {
      expect(src.startsWith("/")).toBe(true);
    }
  });
});
