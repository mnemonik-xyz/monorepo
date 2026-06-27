import { StrictMode, type ReactElement } from "react";
import { renderToString } from "react-dom/server";
import { StaticRouter } from "react-router-dom/server";
import Analytics from "./pages/Analytics";
import Blog from "./pages/Blog";
import BlogPost from "./pages/BlogPost";
import Landing from "./pages/Landing";
import Ledger from "./pages/Ledger";
import { deriveBlogPost, type BlogPost as BlogPostData } from "./lib/blog";

/**
 * Server entry used only by the build-time prerender step (scripts/prerender.mjs).
 *
 * It renders the STATIC public routes to HTML strings so crawlers and social
 * scrapers receive real markup — per-route `<title>`/meta/canonical/OpenGraph +
 * JSON-LD (emitted by `<Seo>` via React 19 head hoisting) and the route's static
 * shell — without executing client JS. The SPA still hydrates for humans.
 *
 * Why a hand-written map instead of reusing `App.tsx`'s `<Routes>`: `App` is
 * hardwired to `<BrowserRouter>`, which depends on `window.history` and cannot
 * render under Node. Each page is wrapped in `<StaticRouter>` here so its
 * `<Link>`s and router hooks resolve at the correct location. The four static
 * routes are prerendered by `render`; the dynamic `/blog/:slug` pages are
 * prerendered per-post by `renderBlogPost` (T13) from the build-time post list.
 *
 * Data is fetched in `useEffect` (which never runs during SSR), so the snapshot
 * is the graceful crawl-shell (loading/empty state) — no backend is required at
 * build time. Client hydration swaps in live (or sample) data for humans.
 */

/** Canonical list of static routes to prerender. Single source of truth. */
export const PRERENDER_ROUTES = [
  "/",
  "/ledger",
  "/analytics",
  "/blog",
] as const;

const ROUTE_ELEMENTS: Record<string, () => ReactElement> = {
  "/": () => <Landing />,
  "/ledger": () => <Ledger />,
  "/analytics": () => <Analytics />,
  "/blog": () => <Blog />,
};

/** Render a single static route to an HTML string. Throws on unknown routes. */
export function render(url: string): string {
  const factory = ROUTE_ELEMENTS[url];
  if (!factory) {
    throw new Error(`entry-server: no prerender mapping for route "${url}"`);
  }
  return renderToString(
    <StrictMode>
      <StaticRouter location={url}>{factory()}</StaticRouter>
    </StrictMode>,
  );
}

/**
 * Render a single `/blog/:slug` post to an HTML string for build-time prerender.
 * `post` is the already-fetched post data; passing it as `initialPost` seeds
 * `<BlogPost>`'s loaded state so the snapshot carries the real title, body, and
 * Article JSON-LD (via `<Seo>`) — not the loading shell. The post title/body are
 * rendered through plain JSX + react-markdown (no `dangerouslySetInnerHTML`), and
 * all JSON-LD flows through `<Seo jsonLd>`'s `safeJsonLd` escaping, so an
 * attacker-influenced (agent-published) post cannot inject executable markup.
 */
export function renderBlogPost(slug: string, post: BlogPostData): string {
  return renderToString(
    <StrictMode>
      <StaticRouter location={`/blog/${slug}`}>
        <BlogPost initialPost={deriveBlogPost(post)} />
      </StaticRouter>
    </StrictMode>,
  );
}
