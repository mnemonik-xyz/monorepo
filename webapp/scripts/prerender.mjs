// Build-time prerender of the STATIC public routes (Task 12).
//
// After `vite build` emits the SPA into dist/, this script renders each static
// route (/, /ledger, /analytics, /blog) to real HTML so crawlers and social
// scrapers get a per-route <head> (title / description / canonical / OpenGraph +
// Twitter / JSON-LD, produced by <Seo> via React 19 head hoisting) plus the
// route's static shell — without executing client JS. The SPA still hydrates for
// humans (main.tsx does a fresh client render into #root, replacing the snapshot).
//
// Approach (Vite SSG, no headless browser): we build an SSR bundle of
// src/entry-server.tsx with Vite's JS API into a temp dir, render each route with
// react-dom/server, lift the head tags into the template's marked SEO block, and
// inject the body into #root. Chosen over a puppeteer/Playwright crawl of
// `vite preview` because it needs no browser binary — keeping `npm run build`
// deterministic and cross-platform (VPS deploy safe) — and adds zero new deps
// (react-dom/server + react-router-dom/server ship with packages already present).
//
// No backend is required: pages fetch in useEffect (which never runs during SSR),
// so the snapshot is the graceful crawl-shell (Decision 2). CSP is untouched: the
// only inline script is the non-executable application/ld+json block (already
// emitted by <Seo>, escaped by safeJsonLd), and no new origins are introduced.

import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { build } from "vite";

const __dirname = dirname(fileURLToPath(import.meta.url));
const webappRoot = resolve(__dirname, "..");
const distDir = resolve(webappRoot, "dist");
// Temp output for the SSR bundle. Lives under node_modules/ so it is already
// gitignored and never shipped in the static dist/.
const ssrOutDir = resolve(webappRoot, "node_modules/.prerender");

const SEO_BLOCK = /<!-- seo:start -->[\s\S]*?<!-- seo:end -->/;
const ROOT_DIV = '<div id="root"></div>';

/**
 * Split a server-rendered route string into head tags and body markup.
 *
 * Every <title>, <meta>, <link> and application/ld+json <script> in the rendered
 * output originates from <Seo> (the page components render none of these
 * themselves), so extracting by tag type is exact, not heuristic. React 19 hoists
 * title/meta/link to the front of the stream and renders the ld+json inline; we
 * collect all of them for the <head> and return the remaining markup for #root.
 */
function splitHead(html) {
  const title = [];
  const meta = [];
  const link = [];
  const ld = [];
  const body = html
    .replace(/<title>[\s\S]*?<\/title>/g, (m) => (title.push(m), ""))
    .replace(/<script type="application\/ld\+json">[\s\S]*?<\/script>/g, (m) => (ld.push(m), ""))
    .replace(/<meta\b[^>]*?>/g, (m) => (meta.push(m), ""))
    .replace(/<link\b[^>]*?>/g, (m) => (link.push(m), ""));
  const head = [...title, ...meta, ...link, ...ld]
    .map((tag) => `    ${tag}`)
    .join("\n");
  return { head, body };
}

/** dist path for a route: "/" -> dist/index.html, "/x" -> dist/x/index.html. */
function outPathFor(route) {
  if (route === "/") return resolve(distDir, "index.html");
  return resolve(distDir, `.${route}/index.html`);
}

async function main() {
  // 1. Build the SSR bundle of the route tree.
  await build({
    root: webappRoot,
    logLevel: "warn",
    build: {
      ssr: resolve(webappRoot, "src/entry-server.tsx"),
      outDir: ssrOutDir,
      emptyOutDir: true,
      copyPublicDir: false,
    },
  });

  const { render, PRERENDER_ROUTES } = await import(
    resolve(ssrOutDir, "entry-server.js")
  );

  // 2. Read the built SPA shell as the template for every route.
  const template = await readFile(resolve(distDir, "index.html"), "utf8");
  if (!SEO_BLOCK.test(template)) {
    throw new Error(
      "prerender: SEO marker block (<!-- seo:start -->...<!-- seo:end -->) not found in dist/index.html",
    );
  }
  if (!template.includes(ROOT_DIV)) {
    throw new Error(`prerender: '${ROOT_DIV}' not found in dist/index.html`);
  }

  // 3. Render each static route into its own HTML file.
  for (const route of PRERENDER_ROUTES) {
    const { head, body } = splitHead(render(route));
    const doc = template
      .replace(SEO_BLOCK, `<!-- seo:start -->\n${head}\n    <!-- seo:end -->`)
      .replace(ROOT_DIV, `<div id="root">${body}</div>`);
    const outPath = outPathFor(route);
    await mkdir(dirname(outPath), { recursive: true });
    await writeFile(outPath, doc, "utf8");
    console.log(`prerendered ${route} -> ${outPath.replace(`${webappRoot}/`, "")}`);
  }

  // 4. Drop the SSR bundle; only the static dist/ ships.
  await rm(ssrOutDir, { recursive: true, force: true });
}

main().catch((err) => {
  console.error("prerender failed:", err);
  process.exitCode = 1;
});
