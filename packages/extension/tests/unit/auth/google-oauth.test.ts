// T16 D13-binding TDD anchors for the Google sign-in client.
//
// Both anchors must pass under `bun test` (D13 + tech-spec). The chrome.*
// surface is hand-stubbed via `globalThis.chrome` so the tests are runtime-
// agnostic (no jsdom dependency, no fetch polyfill assumptions).
//
// Anchor 1: `pkce_state_mismatch_rejects` — a tampered callback URL whose
//   `state` does NOT match the value the client sent must throw BEFORE the
//   POST `/oauth/token` exchange happens (RFC 7636 §4.4 / RFC 6749 §10.12).
//
// Anchor 2: `redirect_uri_uses_chrome_identity_helper` — the URL handed to
//   `chrome.identity.launchWebAuthFlow` must carry exactly
//   `chrome.identity.getRedirectURL('google')` as `redirect_uri` (D5: the
//   server is the only party that needs to know how to talk to Google;
//   the extension only ever uses the chromiumapp.org deeplink).

// Uses vitest imports — works under vitest natively and under `bun test`
// via bun's vitest-compat shim (Decision 11 of the repo CI policy makes
// `bun test` the uniform JS-runner; vitest is the local-dev test command).
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { signInWithGoogle } from "../../../src/auth/google-oauth.js";
import { AuthError } from "../../../src/auth/types.js";

// ── Test doubles ─────────────────────────────────────────────────────────────

interface ChromeIdentityStub {
  getRedirectURL: (path: string) => string;
  launchWebAuthFlow: (
    details: { url: string; interactive: boolean },
    cb: (responseUrl?: string) => void,
  ) => void;
}

interface ChromeStub {
  identity: ChromeIdentityStub;
  runtime?: { lastError?: { message: string } | undefined };
}

const REDIRECT_SENTINEL = "https://test-extension-id.chromiumapp.org/google";

let originalChrome: unknown;
let originalFetch: typeof globalThis.fetch | undefined;
let fetchCalls: { url: string; init?: RequestInit }[] = [];

function installChromeMock(
  launchHandler: (url: string) => string | undefined,
): { capturedUrl: { value: string | null } } {
  const captured: { value: string | null } = { value: null };
  const stub: ChromeStub = {
    identity: {
      getRedirectURL: (_path: string) => REDIRECT_SENTINEL,
      launchWebAuthFlow: (details, cb) => {
        captured.value = details.url;
        const responseUrl = launchHandler(details.url);
        // Mirror chrome.identity behaviour: callback may receive undefined
        // (user cancelled) which sets chrome.runtime.lastError. Tests that
        // want the success path return a string redirect URL.
        if (responseUrl === undefined) {
          stub.runtime = {
            lastError: { message: "user_cancelled" },
          };
        }
        cb(responseUrl);
      },
    },
    runtime: { lastError: undefined },
  };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).chrome = stub;
  return { capturedUrl: captured };
}

function installFetchMock(
  handler: (url: string, init?: RequestInit) => Response,
): void {
  fetchCalls = [];
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    // `exactOptionalPropertyTypes` rejects `{ init: undefined }`; only
    // include the key when the caller actually supplied an init.
    const entry: { url: string; init?: RequestInit } = { url };
    if (init !== undefined) entry.init = init;
    fetchCalls.push(entry);
    return Promise.resolve(handler(url, init));
  }) as typeof globalThis.fetch;
}

// Build a minimally valid base64url-encoded JWT segment so the token
// exchange happy path can decode it without needing a real signer.
function b64url(obj: unknown): string {
  const json = JSON.stringify(obj);
  const b64 = btoa(json);
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function fakeIdToken(claims: Record<string, unknown>): string {
  const header = b64url({ alg: "RS256", typ: "JWT" });
  const payload = b64url(claims);
  // Signature is irrelevant — the server already validated it, the
  // extension only parses claims.
  return `${header}.${payload}.sig`;
}

function fakeAccessToken(): string {
  // Server-issued JWT (HS256). We never verify it client-side; the
  // popup just stores it.
  return [
    b64url({ alg: "HS256", typ: "JWT" }),
    b64url({
      sub: "pub_base58_user",
      google_sub: "google-sub-12345",
      iat: Math.floor(Date.now() / 1000),
      exp: Math.floor(Date.now() / 1000) + 3600,
    }),
    "sig",
  ].join(".");
}

beforeEach(() => {
  originalChrome = (globalThis as { chrome?: unknown }).chrome;
  originalFetch = globalThis.fetch;
});

afterEach(() => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).chrome = originalChrome;
  if (originalFetch) globalThis.fetch = originalFetch;
});

// ── Anchor 1: state mismatch rejects ─────────────────────────────────────────

describe("signInWithGoogle — pkce_state_mismatch_rejects", () => {
  it("throws an OAuthError with code 'state_mismatch' before any token POST", async () => {
    let tokenExchangeCalled = false;

    installChromeMock((authorizeUrl) => {
      // Server-side flow would echo back our `state`; instead we forge a
      // bogus value to simulate a CSRF / leaked-code attack.
      const original = new URL(authorizeUrl);
      const _ = original.searchParams.get("state"); // intentionally unused
      const cb = new URL(REDIRECT_SENTINEL);
      cb.searchParams.set("code", "server-issued-code");
      cb.searchParams.set("state", "ATTACKER-CONTROLLED-STATE");
      return cb.toString();
    });

    installFetchMock(() => {
      tokenExchangeCalled = true;
      return new Response(
        JSON.stringify({
          access_token: fakeAccessToken(),
          id_token: fakeIdToken({ sub: "google-sub-12345" }),
          token_type: "Bearer",
          expires_in: 3600,
          scope: "mcp",
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    });

    let caught: unknown = undefined;
    try {
      await signInWithGoogle({ serverBaseUrl: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("state mismatch");
    // The critical RFC 7636 §4.4 guarantee: no token exchange happened.
    expect(tokenExchangeCalled).toBe(false);
    expect(fetchCalls.length).toBe(0);
  });
});

// ── Anchor 2: redirect_uri uses chrome.identity helper ──────────────────────

describe("signInWithGoogle — redirect_uri_uses_chrome_identity_helper", () => {
  it("passes exactly chrome.identity.getRedirectURL('google') to launchWebAuthFlow", async () => {
    const { capturedUrl } = installChromeMock((authorizeUrl) => {
      // Echo state through; happy path so we observe the real URL.
      const original = new URL(authorizeUrl);
      const state = original.searchParams.get("state") ?? "";
      const cb = new URL(REDIRECT_SENTINEL);
      cb.searchParams.set("code", "server-issued-code");
      cb.searchParams.set("state", state);
      return cb.toString();
    });

    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            access_token: fakeAccessToken(),
            id_token: fakeIdToken({
              sub: "google-sub-12345",
              email: "alice@example.com",
              name: "Alice",
              picture: "https://example.com/p.png",
            }),
            token_type: "Bearer",
            expires_in: 3600,
            scope: "mcp",
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    );

    const result = await signInWithGoogle({ serverBaseUrl: "https://mc.test" });

    expect(capturedUrl.value).not.toBeNull();
    const url = new URL(capturedUrl.value as string);
    expect(url.searchParams.get("redirect_uri")).toBe(REDIRECT_SENTINEL);

    // The token exchange POST must also carry the IDENTICAL redirect_uri
    // (RFC 6749 §4.1.3 — defeat swap-redirect on a leaked code).
    expect(fetchCalls.length).toBe(1);
    const tokenCall = fetchCalls[0];
    expect(tokenCall).toBeDefined();
    const body = JSON.parse(String(tokenCall?.init?.body ?? "{}"));
    expect(body.redirect_uri).toBe(REDIRECT_SENTINEL);
    expect(typeof body.code).toBe("string");
    expect(typeof body.code_verifier).toBe("string");

    // Sanity: returned shape matches the public contract.
    expect(typeof result.jwt).toBe("string");
    expect(result.googleSub).toBe("google-sub-12345");
    expect(result.profile.email).toBe("alice@example.com");
    expect(result.profile.name).toBe("Alice");
  });
});

// ── Supporting coverage (PKCE shape, user-cancellation) ─────────────────────

describe("signInWithGoogle — PKCE wire shape", () => {
  it("sends S256 challenge derived from the verifier", async () => {
    let challengeFromUrl: string | null = null;
    let verifierFromBody: string | null = null;

    installChromeMock((authorizeUrl) => {
      const original = new URL(authorizeUrl);
      challengeFromUrl = original.searchParams.get("code_challenge");
      const state = original.searchParams.get("state") ?? "";
      expect(original.searchParams.get("code_challenge_method")).toBe("S256");
      const cb = new URL(REDIRECT_SENTINEL);
      cb.searchParams.set("code", "ok-code");
      cb.searchParams.set("state", state);
      return cb.toString();
    });

    installFetchMock((_url, init) => {
      const body = JSON.parse(String(init?.body ?? "{}"));
      verifierFromBody = body.code_verifier;
      return new Response(
        JSON.stringify({
          access_token: fakeAccessToken(),
          id_token: fakeIdToken({ sub: "google-sub-12345" }),
          token_type: "Bearer",
          expires_in: 3600,
          scope: "mcp",
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    });

    await signInWithGoogle({ serverBaseUrl: "https://mc.test" });

    expect(challengeFromUrl).not.toBeNull();
    expect(verifierFromBody).not.toBeNull();
    // The two `expect(...).not.toBeNull()` lines above guarantee the
    // assignments fired; assert via `unknown` cast to satisfy
    // `noImplicitAny`-style strictness without `as string` on a `null`.
    const verifier = verifierFromBody as unknown as string;
    const challenge = challengeFromUrl as unknown as string;
    // Verifier must be base64url, no padding (RFC 7636 §4.1).
    expect(verifier).toMatch(/^[A-Za-z0-9_-]+$/);
    // Challenge must be 43 chars (base64url-no-pad of a 32-byte SHA-256).
    expect(challenge.length).toBe(43);

    // Verify the challenge actually equals SHA-256(verifier) — proves the
    // client did the PKCE derivation, not just stuffed a random string.
    const enc = new TextEncoder().encode(verifier);
    const digest = await crypto.subtle.digest("SHA-256", enc);
    const expected = btoa(String.fromCharCode(...new Uint8Array(digest)))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
    expect(challengeFromUrl).toBe(expected);
  });
});

describe("signInWithGoogle — user cancellation", () => {
  it("throws OAuthError with code 'user_cancelled' when launchWebAuthFlow returns undefined", async () => {
    installChromeMock(() => undefined);
    installFetchMock(() => new Response("never reached", { status: 500 }));

    let caught: unknown = undefined;
    try {
      await signInWithGoogle({ serverBaseUrl: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message.toLowerCase()).toContain("cancel");
    expect(fetchCalls.length).toBe(0);
  });
});

describe("signInWithGoogle — token endpoint failure", () => {
  it("throws OAuthError with code 'token_exchange_failed' on non-2xx", async () => {
    installChromeMock((authorizeUrl) => {
      const original = new URL(authorizeUrl);
      const state = original.searchParams.get("state") ?? "";
      const cb = new URL(REDIRECT_SENTINEL);
      cb.searchParams.set("code", "ok-code");
      cb.searchParams.set("state", state);
      return cb.toString();
    });
    installFetchMock(
      () =>
        new Response(JSON.stringify({ error: "invalid_grant" }), {
          status: 401,
          headers: { "content-type": "application/json" },
        }),
    );

    let caught: unknown = undefined;
    try {
      await signInWithGoogle({ serverBaseUrl: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("401");
  });

  it("surfaces RFC 6749 §5.2 error + error_description in the AuthError message", async () => {
    installChromeMock((authorizeUrl) => {
      const original = new URL(authorizeUrl);
      const state = original.searchParams.get("state") ?? "";
      const cb = new URL(REDIRECT_SENTINEL);
      cb.searchParams.set("code", "ok-code");
      cb.searchParams.set("state", state);
      return cb.toString();
    });
    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            error: "invalid_grant",
            error_description: "PKCE verifier mismatch",
          }),
          {
            status: 400,
            headers: { "content-type": "application/json" },
          },
        ),
    );

    let caught: unknown = undefined;
    try {
      await signInWithGoogle({ serverBaseUrl: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    const msg = (caught as AuthError).message;
    expect(msg).toContain("400");
    expect(msg).toContain("invalid_grant");
    expect(msg).toContain("PKCE verifier mismatch");
  });
});

describe("extractProfile — security hardening (SEC-MIN-1, SEC-MIN-2)", () => {
  // Hand-rolled b64url for hermetic tests.
  function b64u(obj: unknown): string {
    return btoa(JSON.stringify(obj))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
  }
  function mkIdToken(claims: Record<string, unknown>): string {
    return [b64u({ alg: "RS256", typ: "JWT" }), b64u(claims), "sig"].join(".");
  }

  it("clamps overlong name / email / sub fields", async () => {
    const { extractProfile } =
      await import("../../../src/auth/google-oauth.js");
    const oversized = "a".repeat(10_000);
    const token = mkIdToken({
      sub: "g-sub",
      email: oversized,
      name: oversized,
      picture: "https://lh3.googleusercontent.com/p.png",
    });
    const profile = extractProfile(token, "header.payload.sig");
    expect(profile.email.length).toBeLessThanOrEqual(256);
    expect(profile.name.length).toBeLessThanOrEqual(256);
    expect(profile.sub).toBe("g-sub");
  });

  it("rejects non-https picture URLs (defence-in-depth XSS hardening)", async () => {
    const { extractProfile } =
      await import("../../../src/auth/google-oauth.js");
    const cases = [
      "javascript:alert(1)", // classic XSS payload
      "data:text/html,<script>alert(1)</script>", // data: URL
      "http://example.com/p.png", // plain http
      "not-a-url-at-all",
      "", // empty
    ];
    for (const picture of cases) {
      const token = mkIdToken({ sub: "g-sub", picture });
      const profile = extractProfile(token, "header.payload.sig");
      expect(profile.picture).toBe("");
    }
  });

  it("accepts and normalises a valid https picture URL", async () => {
    const { extractProfile } =
      await import("../../../src/auth/google-oauth.js");
    const token = mkIdToken({
      sub: "g-sub",
      picture: "https://lh3.googleusercontent.com/a/p.png",
    });
    const profile = extractProfile(token, "header.payload.sig");
    expect(profile.picture).toBe("https://lh3.googleusercontent.com/a/p.png");
  });
});

describe("signInWithGoogle — jwtExpiresAt", () => {
  it("returns wall-clock expiry derived from server's expires_in", async () => {
    installChromeMock((authorizeUrl) => {
      const original = new URL(authorizeUrl);
      const state = original.searchParams.get("state") ?? "";
      const cb = new URL(REDIRECT_SENTINEL);
      cb.searchParams.set("code", "ok-code");
      cb.searchParams.set("state", state);
      return cb.toString();
    });
    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            access_token: fakeAccessToken(),
            id_token: fakeIdToken({ sub: "google-sub-12345" }),
            token_type: "Bearer",
            // Pick a distinctive value so we can assert it travelled
            // through `signInWithGoogle` rather than the 1h default.
            expires_in: 1234,
            scope: "mcp",
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    );

    const before = Date.now();
    const result = await signInWithGoogle({
      serverBaseUrl: "https://mc.test",
    });
    const after = Date.now();
    // Expiry must be roughly `now + 1234s`. Allow a wide window for
    // CI clock jitter — the point is to prove the value is NOT the 1h
    // (3600s) default.
    expect(result.jwtExpiresAt).toBeGreaterThanOrEqual(before + 1234 * 1000);
    expect(result.jwtExpiresAt).toBeLessThanOrEqual(after + 1234 * 1000 + 100);
  });
});
