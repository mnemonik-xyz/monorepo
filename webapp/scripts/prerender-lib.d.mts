// Hand-written declarations for the JS prerender helpers (scripts/prerender-lib.mjs)
// so src/test/prerender-blog.test.ts can import the real implementation under
// `tsc -b` (the project does not enable allowJs). Keep in sync with the .mjs.

export declare const ORIGIN: string;
export declare const SEO_BLOCK: RegExp;
export declare const ROOT_DIV: string;

export declare function splitHead(html: string): { head: string; body: string };
export declare function renderDocument(template: string, html: string): string;
export declare function isValidSlug(slug: unknown): boolean;
export declare function xmlEscape(s: string): string;

export interface SitemapRoute {
  path: string;
  changefreq: string;
  priority: string;
}
export declare const STATIC_SITEMAP_ROUTES: SitemapRoute[];
export declare function buildSitemap(slugs?: string[]): string;
