// T19 — Google OAuth interceptor for Playwright extension tests.
//
// The real flow is `chrome.identity.launchWebAuthFlow` → opens a popup
// to `accounts.google.com/o/oauth2/v2/auth?...&redirect_uri=...&state=...`.
// The user signs in → Google redirects to the launchWebAuthFlow callback
// URL with `?code=...&state=...`, which the extension parses.
//
// In tests we cannot stage a real Google sign-in. Instead we route
// `accounts.google.com/o/oauth2/**` to a synthetic 302 response that
// preserves the inbound `state` parameter and substitutes a known
// `code` + simulated `google_sub`. The server-side mock then exchanges
// that code for a deterministic JWT.

import type { BrowserContext } from "@playwright/test";

export interface OAuthFixture {
  /** Stable Google subject id surfaced through the mock token endpoint. */
  google_sub: string;
  /** Mailbox-shaped string the popup renders as the signed-in label. */
  email: string;
  /** Synthetic auth-code echoed in the 302 redirect. */
  auth_code: string;
}

export const DEFAULT_OAUTH: OAuthFixture = {
  google_sub: "test-google-sub-001",
  email: "tester@example.com",
  auth_code: "test-auth-code-001",
};

/** Install the OAuth interceptor. Returns a `dispose()` that the test
 *  may call to remove the route (otherwise it stays bound to the
 *  context for its lifetime, which is fine — context.close() drops
 *  everything). */
export async function mockGoogleOAuth(
  context: BrowserContext,
  fixture: OAuthFixture = DEFAULT_OAUTH,
): Promise<() => Promise<void>> {
  // Match every `accounts.google.com/o/oauth2/*` regardless of subpath.
  await context.route(
    /https:\/\/accounts\.google\.com\/o\/oauth2\//,
    (route) => {
      const url = new URL(route.request().url());
      const redirect = url.searchParams.get("redirect_uri");
      const state = url.searchParams.get("state") ?? "";
      if (!redirect) {
        return route.fulfill({ status: 400, body: "missing redirect_uri" });
      }
      const target = new URL(redirect);
      target.searchParams.set("code", fixture.auth_code);
      target.searchParams.set("state", state);
      return route.fulfill({
        status: 302,
        headers: { location: target.toString() },
      });
    },
  );

  return async () => {
    await context.unroute(/https:\/\/accounts\.google\.com\/o\/oauth2\//);
  };
}
