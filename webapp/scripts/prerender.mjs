// Build-time prerender of the public routes (Task 12 static routes + Task 13
// per-post /blog/:slug pages and dynamic sitemap).
//
// After `vite build` emits the SPA into dist/, this script renders each route to
// real HTML so crawlers and social scrapers get a per-route <head> (title /
// description / canonical / OpenGraph + Twitter / JSON-LD, produced by <Seo> via
// React 19 head hoisting) plus the route's static shell — without executing
// client JS. The SPA still hydrates for humans (main.tsx does a fresh client
// render into #root, replacing the snapshot).
//
// Approach (Vite SSG, no headless browser): we build an SSR bundle of
// src/entry-server.tsx with Vite's JS API into a temp dir, render each route with
// react-dom/server, lift the head tags into the template's marked SEO block, and
// inject the body into #root. Chosen over a puppeteer/Playwright crawl of
// `vite preview` because it needs no browser binary — keeping `npm run build`
// deterministic and cross-platform (VPS deploy safe) — and adds zero new deps.
//
// T13 adds the DYNAMIC half: the published blog posts are fetched once at build
// time (GET ${VITE_MCP_BASE}/blog, or a local fixture for offline preview/tests).
// Each post is rendered to dist/blog/<slug>/index.html with its own head + body,
// and dist/sitemap.xml is regenerated from the static routes plus every slug.
// GRACEFUL FALLBACK: if no backend/fixture is available (or the fetch fails), the
// blog-post prerender is skipped and the build still succeeds with a static-only
// sitemap. Pure extraction/sitemap/slug logic lives in ./prerender-lib.mjs.

import { fileURLToPath } from "node:url";
import { dirname, resolve, sep } from "node:path";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { build } from "vite";
import {
  SEO_BLOCK,
  ROOT_DIV,
  renderDocument,
  buildSitemap,
  isValidSlug,
} from "./prerender-lib.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const webappRoot = resolve(__dirname, "..");
const distDir = resolve(webappRoot, "dist");
// Temp output for the SSR bundle. Lives under node_modules/ so it is already
// gitignored and never shipped in the static dist/.
const ssrOutDir = resolve(webappRoot, "node_modules/.prerender");

/** dist path for a static route: "/" -> dist/index.html, "/x" -> dist/x/index.html. */
function outPathFor(route) {
  if (route === "/") return resolve(distDir, "index.html");
  return resolve(distDir, `.${route}/index.html`);
}

/** How long to wait for the build-time GET /blog before falling back. */
const BLOG_FETCH_TIMEOUT_MS = 4000;

/**
 * Resolve the published post list at build time. Order of preference:
 *   1. PRERENDER_BLOG_FIXTURE — path to a local JSON file ({posts:[...]} or
 *      [...]). Lets `npm run build` produce real /blog/:slug HTML offline (used
 *      by the prerender test and for previewing post SEO without a backend).
 *   2. VITE_MCP_BASE — fetch GET ${base}/blog (the same base the SPA uses).
 *   3. neither set / fetch fails / times out -> [] (graceful fallback).
 * Never throws: any failure degrades to [] so the build cannot be broken by an
 * unreachable or misbehaving backend.
 */
async function fetchBlogPosts() {
  const fixture = process.env.PRERENDER_BLOG_FIXTURE?.trim();
  if (fixture) {
    try {
      const raw = await readFile(resolve(webappRoot, fixture), "utf8");
      const data = JSON.parse(raw);
      const posts = Array.isArray(data) ? data : data?.posts;
      if (Array.isArray(posts)) {
        console.log(`prerender: using ${posts.length} post(s) from fixture ${fixture}`);
        return posts;
      }
      console.warn(`prerender: fixture ${fixture} has no post array — skipping blog prerender`);
      return [];
    } catch (err) {
      console.warn(`prerender: could not read fixture ${fixture} (${err.message}) — skipping blog prerender`);
      return [];
    }
  }

  const base = process.env.VITE_MCP_BASE?.trim();
  if (!base) {
    console.log("prerender: no VITE_MCP_BASE and no fixture — skipping blog prerender (static sitemap only)");
    return [];
  }

  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), BLOG_FETCH_TIMEOUT_MS);
  try {
    const res = await fetch(`${base}/blog`, { signal: ctrl.signal });
    if (!res.ok) {
      console.warn(`prerender: GET ${base}/blog returned ${res.status} — skipping blog prerender`);
      return [];
    }
    const data = await res.json();
    const posts = Array.isArray(data?.posts) ? data.posts : [];
    console.log(`prerender: fetched ${posts.length} post(s) from ${base}/blog`);
    return posts;
  } catch (err) {
    console.warn(`prerender: GET ${base}/blog failed (${err.message}) — skipping blog prerender`);
    return [];
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Render each fetched post to dist/blog/<slug>/index.html. Returns the slugs
 * actually written (used for the sitemap). Each post is isolated in try/catch so
 * one malformed post cannot fail the whole build, and every slug is validated
 * against the server's kebab-case shape before it becomes a filesystem path —
 * blocking traversal from an attacker-influenced post list.
 */
async function prerenderBlogPosts(posts, template, renderBlogPost) {
  const written = [];
  for (const post of posts) {
    const slug = post?.slug;
    if (!isValidSlug(slug)) {
      console.warn(`prerender: skipping post with invalid slug ${JSON.stringify(slug)}`);
      continue;
    }
    const outPath = resolve(distDir, "blog", slug, "index.html");
    // Defence in depth: the validated slug already cannot escape, but assert the
    // resolved path stays under dist/ before any write.
    if (!outPath.startsWith(distDir + sep)) {
      console.warn(`prerender: skipping post whose path escapes dist/: ${slug}`);
      continue;
    }
    try {
      const doc = renderDocument(template, renderBlogPost(slug, post));
      await mkdir(dirname(outPath), { recursive: true });
      await writeFile(outPath, doc, "utf8");
      written.push(slug);
      console.log(`prerendered /blog/${slug} -> ${outPath.replace(`${webappRoot}/`, "")}`);
    } catch (err) {
      console.warn(`prerender: failed to render /blog/${slug} (${err.message}) — skipping`);
    }
  }
  return written;
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

  const { render, renderBlogPost, PRERENDER_ROUTES } = await import(
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
    const doc = renderDocument(template, render(route));
    const outPath = outPathFor(route);
    await mkdir(dirname(outPath), { recursive: true });
    await writeFile(outPath, doc, "utf8");
    console.log(`prerendered ${route} -> ${outPath.replace(`${webappRoot}/`, "")}`);
  }

  // 4. Fetch the published posts and prerender one HTML file per slug. Degrades
  //    to an empty list (static-only sitemap) when no backend/fixture is present.
  const posts = await fetchBlogPosts();
  const slugs = await prerenderBlogPosts(posts, template, renderBlogPost);

  // 5. Regenerate dist/sitemap.xml from the static routes + every written slug,
  //    overwriting the static baseline Vite copied from public/sitemap.xml.
  await writeFile(resolve(distDir, "sitemap.xml"), buildSitemap(slugs), "utf8");
  console.log(`prerender: wrote sitemap.xml with ${slugs.length} blog slug(s)`);

  // 6. Drop the SSR bundle; only the static dist/ ships.
  await rm(ssrOutDir, { recursive: true, force: true });
}

main().catch((err) => {
  console.error("prerender failed:", err);
  process.exitCode = 1;
});
