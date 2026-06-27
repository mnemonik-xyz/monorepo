import { StrictMode, type ReactElement } from "react";
import { renderToString } from "react-dom/server";
import { StaticRouter } from "react-router-dom/server";
import Analytics from "./pages/Analytics";
import Blog from "./pages/Blog";
import Landing from "./pages/Landing";
import Ledger from "./pages/Ledger";

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
 * `<Link>`s and router hooks resolve at the correct location. Only the four
 * static routes are prerendered; the dynamic `/blog/:slug` is handled by T13.
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
