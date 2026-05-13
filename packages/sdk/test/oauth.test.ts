// Unit tests for @mnemonik-xyz/sdk's OAuth 2.1 + PKCE primitives.
//
// Pure ESM, runtime-agnostic — no `node:*` imports. The mocked `fetch` is
// installed on `globalThis` for each test that exercises an HTTP path.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BROWSERLESS_REDIRECT_URI,
  buildAuthorizeUrl,
  exchangeCodeForToken,
  generatePkceVerifier,
  getSession,
  loginWithIdentity,
  parseJwtPayload,
  PENDING_SESSION_MAX,
  PENDING_SESSION_TTL_MS,
  pendingAuthSessions,
  pkceChallenge,
  randomState,
  setSession,
} from "../src/oauth.js";
import { AuthError } from "../src/errors.js";
import { Keypair } from "../src/keypair.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";

// ── helpers ────────────────────────────────────────────────────────────────

function base64UrlEncode(input: string | Uint8Array): string {
  const bytes =
    typeof input === "string" ? new TextEncoder().encode(input) : input;
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]!);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Minimal JWT builder for tests — header/payload only, signature is dummy. */
function makeJwt(
  payload: Record<string, unknown>,
  header: Record<string, unknown> = { alg: "HS256", typ: "JWT" }
): string {
  const h = base64UrlEncode(JSON.stringify(header));
  const body = base64UrlEncode(JSON.stringify(payload));
  return `${h}.${body}.signature`;
}

const ORIGINAL_FETCH = globalThis.fetch;

afterEach(() => {
  pendingAuthSessions.clear();
  vi.restoreAllMocks();
  globalThis.fetch = ORIGINAL_FETCH;
});

// ── PKCE math ──────────────────────────────────────────────────────────────

describe("pkceChallenge", () => {
  // RFC 7636 Appendix B (the canonical fixture):
  //   code_verifier  = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
  //   code_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
  it("matches RFC 7636 Appendix B fixture", async () => {
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const got = await pkceChallenge(verifier);
    expect(got).toBe(expected);
  });

  it("produces base64url with no padding and url-safe charset", async () => {
    const v = generatePkceVerifier();
    const c = await pkceChallenge(v);
    expect(c).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(c).not.toContain("=");
    // SHA-256 → 32 bytes → 43 base64url chars (no padding).
    expect(c.length).toBe(43);
  });
});

// ── PKCE verifier + state randomness ───────────────────────────────────────

describe("generatePkceVerifier / randomState", () => {
  it("verifier is 43 base64url chars from a 32-byte source", () => {
    const v = generatePkceVerifier();
    expect(v).toMatch(/^[A-Za-z0-9_-]{43}$/);
  });

  it("verifier is unique across calls (basic randomness sanity)", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 32; i++) seen.add(generatePkceVerifier());
    expect(seen.size).toBe(32);
  });

  it("state is 43 base64url chars and unique across calls", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 32; i++) {
      const s = randomState();
      expect(s).toMatch(/^[A-Za-z0-9_-]{43}$/);
      seen.add(s);
    }
    expect(seen.size).toBe(32);
  });
});

// ── buildAuthorizeUrl ──────────────────────────────────────────────────────

describe("buildAuthorizeUrl", () => {
  it("emits all required PKCE+OAuth2.1 query params", async () => {
    const result = await buildAuthorizeUrl({
      baseUrl: "https://mcp.mnemonik.xyz",
      clientId: "mnemonic-cli",
      redirectUri: "http://127.0.0.1:33418/callback",
    });

    const url = new URL(result.url);
    expect(url.origin).toBe("https://mcp.mnemonik.xyz");
    expect(url.pathname).toBe("/oauth/authorize");
    expect(url.searchParams.get("response_type")).toBe("code");
    expect(url.searchParams.get("client_id")).toBe("mnemonic-cli");
    expect(url.searchParams.get("redirect_uri")).toBe(
      "http://127.0.0.1:33418/callback"
    );
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(url.searchParams.get("code_challenge")).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(url.searchParams.get("state")).toBe(result.state);
    expect(url.searchParams.get("scope")).toBe("mcp");
  });

  it("respects a caller-supplied scope", async () => {
    const r = await buildAuthorizeUrl({
      baseUrl: "https://mcp.mnemonik.xyz",
      clientId: "mnemonic-cli",
      redirectUri: "http://127.0.0.1:33418/callback",
      scope: "mcp openid",
    });
    const u = new URL(r.url);
    expect(u.searchParams.get("scope")).toBe("mcp openid");
  });

  it("stores {verifier, state, redirectUri, sessionId} keyed by sessionId", async () => {
    const r = await buildAuthorizeUrl({
      baseUrl: "https://mcp.mnemonik.xyz",
      clientId: "mnemonic-cli",
      redirectUri: "http://127.0.0.1:7777/callback",
    });
    const stored = pendingAuthSessions.get(r.sessionId);
    expect(stored).toBeDefined();
    expect(stored!.verifier).toBe(r.verifier);
    expect(stored!.state).toBe(r.state);
    expect(stored!.redirectUri).toBe("http://127.0.0.1:7777/callback");
    expect(stored!.sessionId).toBe(r.sessionId);
  });

  it("challenge in URL matches SHA-256(verifier) of the same session", async () => {
    const r = await buildAuthorizeUrl({
      baseUrl: "https://mcp.mnemonik.xyz",
      clientId: "mnemonic-cli",
      redirectUri: "http://127.0.0.1:1234/callback",
    });
    const u = new URL(r.url);
    const challenge = u.searchParams.get("code_challenge")!;
    expect(challenge).toBe(await pkceChallenge(r.verifier));
  });

  it("two builds produce two distinct sessionIds", async () => {
    const a = await buildAuthorizeUrl({
      baseUrl: "https://x.example",
      clientId: "mnemonic-cli",
      redirectUri: "http://127.0.0.1:1/callback",
    });
    const b = await buildAuthorizeUrl({
      baseUrl: "https://x.example",
      clientId: "mnemonic-cli",
      redirectUri: "http://127.0.0.1:1/callback",
    });
    expect(a.sessionId).not.toBe(b.sessionId);
    expect(pendingAuthSessions.size).toBe(2);
  });
});

// ── exchangeCodeForToken ───────────────────────────────────────────────────

describe("exchangeCodeForToken", () => {
  const REDIRECT = "http://127.0.0.1:33418/callback";
  const BASE = "https://mcp.mnemonik.xyz";

  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn();
    // Direct global assignment is portable across vitest + bun's test runner;
    // vi.stubGlobal isn't available under bun's compatibility shim.
    (globalThis as { fetch: unknown }).fetch = fetchMock;
  });

  it("rejects on state mismatch and does NOT issue HTTP", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });

    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "abc",
        state: "totally-different-state",
        redirectUri: REDIRECT,
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);

    expect(fetchMock).not.toHaveBeenCalled();
    // Session is consumed on validation failure.
    expect(pendingAuthSessions.has(seeded.sessionId)).toBe(false);
  });

  it("rejects on redirect_uri mismatch and does NOT issue HTTP", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });

    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "abc",
        state: seeded.state,
        redirectUri: "http://127.0.0.1:9999/callback",
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);

    expect(fetchMock).not.toHaveBeenCalled();
    expect(pendingAuthSessions.has(seeded.sessionId)).toBe(false);
  });

  it("rejects when sessionId is unknown (no HTTP issued)", async () => {
    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "abc",
        state: "doesnt-matter",
        redirectUri: REDIRECT,
        sessionId: "00000000-0000-0000-0000-000000000000",
      })
    ).rejects.toBeInstanceOf(AuthError);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("happy path POSTs JSON with all required fields and returns {jwt, expiresAt}", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    const expectedJwt = makeJwt({
      sub: "abc",
      exp: Math.floor(Date.now() / 1000) + 3600,
      iat: Math.floor(Date.now() / 1000),
    });

    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          jwt: expectedJwt,
          expires_at: "2030-01-01T00:00:00.000Z",
        }),
        { status: 200, headers: { "content-type": "application/json" } }
      )
    );

    const result = await exchangeCodeForToken({
      baseUrl: BASE,
      code: "the-code",
      state: seeded.state,
      redirectUri: REDIRECT,
      sessionId: seeded.sessionId,
    });

    expect(result.jwt).toBe(expectedJwt);
    expect(result.expiresAt).toBe("2030-01-01T00:00:00.000Z");

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe(`${BASE}/oauth/token`);
    expect(init.method).toBe("POST");
    expect(init.headers["content-type"]).toBe("application/json");

    const body = JSON.parse(init.body as string);
    expect(body).toEqual({
      grant_type: "authorization_code",
      code: "the-code",
      code_verifier: seeded.verifier,
      redirect_uri: REDIRECT,
      client_id: "mnemonic-cli",
    });
  });

  it("clears the session on success (second exchange throws session-not-found)", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    const jwt = makeJwt({
      sub: "abc",
      exp: Math.floor(Date.now() / 1000) + 3600,
      iat: Math.floor(Date.now() / 1000),
    });
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({ jwt, expires_at: "2030-01-01T00:00:00Z" }),
        {
          status: 200,
        }
      )
    );

    await exchangeCodeForToken({
      baseUrl: BASE,
      code: "c",
      state: seeded.state,
      redirectUri: REDIRECT,
      sessionId: seeded.sessionId,
    });
    expect(pendingAuthSessions.has(seeded.sessionId)).toBe(false);

    // Second call with same sessionId throws session-not-found.
    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "c",
        state: seeded.state,
        redirectUri: REDIRECT,
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
  });

  it("non-2xx response throws AuthError and clears the session", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    fetchMock.mockResolvedValueOnce(
      new Response("nope", { status: 401, statusText: "Unauthorized" })
    );

    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "c",
        state: seeded.state,
        redirectUri: REDIRECT,
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
    expect(pendingAuthSessions.has(seeded.sessionId)).toBe(false);
  });

  it("malformed JSON body throws AuthError and clears the session", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    fetchMock.mockResolvedValueOnce(
      new Response("not-json{{{", {
        status: 200,
        headers: { "content-type": "application/json" },
      })
    );
    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "c",
        state: seeded.state,
        redirectUri: REDIRECT,
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
    expect(pendingAuthSessions.has(seeded.sessionId)).toBe(false);
  });

  it("body with neither access_token nor jwt throws AuthError", async () => {
    // T11 audit (A04): SDK accepts both `access_token` (RFC 6749 canonical)
    // and `jwt` (legacy). A response that has NEITHER must surface as
    // malformed-body AuthError.
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ token_type: "Bearer" }), { status: 200 })
    );
    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "c",
        state: seeded.state,
        redirectUri: REDIRECT,
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
  });

  it("network failure preserves session for retry", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    fetchMock.mockRejectedValueOnce(new TypeError("fetch failed"));
    await expect(
      exchangeCodeForToken({
        baseUrl: BASE,
        code: "c",
        state: seeded.state,
        redirectUri: REDIRECT,
        sessionId: seeded.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
    // Session preserved for retry on network errors.
    expect(pendingAuthSessions.has(seeded.sessionId)).toBe(true);
  });

  it("falls back to a synthetic expiresAt when server omits expires_at", async () => {
    const seeded = await buildAuthorizeUrl({
      baseUrl: BASE,
      clientId: "mnemonic-cli",
      redirectUri: REDIRECT,
    });
    const jwt = makeJwt({
      sub: "abc",
      exp: Math.floor(Date.now() / 1000) + 3600,
      iat: Math.floor(Date.now() / 1000),
    });
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ jwt }), { status: 200 })
    );
    const result = await exchangeCodeForToken({
      baseUrl: BASE,
      code: "c",
      state: seeded.state,
      redirectUri: REDIRECT,
      sessionId: seeded.sessionId,
    });
    expect(typeof result.expiresAt).toBe("string");
    // ISO 8601 sanity.
    expect(() => new Date(result.expiresAt).toISOString()).not.toThrow();
  });
});

// ── parseJwtPayload ────────────────────────────────────────────────────────

describe("parseJwtPayload", () => {
  it("extracts sub/exp/iat from a valid token", () => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = makeJwt({
      sub: "PUBKEY-base58",
      exp: now + 3600,
      iat: now,
      extra: "ignored",
    });
    const payload = parseJwtPayload(jwt);
    expect(payload.sub).toBe("PUBKEY-base58");
    expect(payload.exp).toBe(now + 3600);
    expect(payload.iat).toBe(now);
  });

  it("throws on empty token", () => {
    expect(() => parseJwtPayload("")).toThrow(AuthError);
  });

  it("throws on malformed (not 3 segments)", () => {
    expect(() => parseJwtPayload("only.two")).toThrow(AuthError);
    expect(() => parseJwtPayload("a.b.c.d")).toThrow(AuthError);
  });

  it("throws when payload is not valid JSON", () => {
    const header = base64UrlEncode(
      JSON.stringify({ alg: "HS256", typ: "JWT" })
    );
    const badPayload = base64UrlEncode("not-json{{{");
    const jwt = `${header}.${badPayload}.sig`;
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
  });

  it("throws on missing sub", () => {
    const jwt = makeJwt({
      exp: Math.floor(Date.now() / 1000) + 3600,
      iat: Math.floor(Date.now() / 1000),
    });
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
  });

  it("throws on missing exp", () => {
    const jwt = makeJwt({
      sub: "abc",
      iat: Math.floor(Date.now() / 1000),
    });
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
  });

  it("throws on missing iat", () => {
    const jwt = makeJwt({
      sub: "abc",
      exp: Math.floor(Date.now() / 1000) + 3600,
    });
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
  });

  it("throws on expired token", () => {
    const past = Math.floor(Date.now() / 1000) - 60;
    const jwt = makeJwt({ sub: "abc", exp: past, iat: past - 3600 });
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
  });

  it("throws when payload is a JSON array (not an object)", () => {
    const header = base64UrlEncode(
      JSON.stringify({ alg: "HS256", typ: "JWT" })
    );
    const arrPayload = base64UrlEncode(JSON.stringify([1, 2, 3]));
    const jwt = `${header}.${arrPayload}.sig`;
    // Array.isArray guard rejects this before the missing-sub check fires.
    expect(() => parseJwtPayload(jwt)).toThrow(/payload is not an object/);
  });

  // ── header alg whitelist (Decision 6, security defence-in-depth) ─────────

  it("rejects alg=none (security)", () => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = makeJwt(
      { sub: "abc", exp: now + 3600, iat: now },
      { alg: "none", typ: "JWT" }
    );
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
    expect(() => parseJwtPayload(jwt)).toThrow(/alg must be HS256, got: none/);
  });

  it("rejects alg=RS256 (non-whitelisted algorithm)", () => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = makeJwt(
      { sub: "abc", exp: now + 3600, iat: now },
      { alg: "RS256", typ: "JWT" }
    );
    expect(() => parseJwtPayload(jwt)).toThrow(/alg must be HS256, got: RS256/);
  });

  it("rejects header missing alg", () => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = makeJwt(
      { sub: "abc", exp: now + 3600, iat: now },
      { typ: "JWT" }
    );
    expect(() => parseJwtPayload(jwt)).toThrow(/header missing `alg`/);
  });

  it("rejects header that is JSON but not an object (array)", () => {
    const headerSeg = base64UrlEncode(JSON.stringify(["HS256"]));
    const bodySeg = base64UrlEncode(
      JSON.stringify({
        sub: "abc",
        exp: Math.floor(Date.now() / 1000) + 3600,
        iat: Math.floor(Date.now() / 1000),
      })
    );
    const jwt = `${headerSeg}.${bodySeg}.sig`;
    expect(() => parseJwtPayload(jwt)).toThrow(/header is not an object/);
  });

  // ── malformed base64 → AuthError, not DOMException (F8) ──────────────────

  it("throws AuthError (not DOMException) on payload with invalid base64url chars", () => {
    const headerSeg = base64UrlEncode(
      JSON.stringify({ alg: "HS256", typ: "JWT" })
    );
    // '!@#$' are outside the base64url alphabet → atob throws DOMException.
    const jwt = `${headerSeg}.!@#$.sig`;
    let thrown: unknown;
    try {
      parseJwtPayload(jwt);
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AuthError);
    // Pin the class name explicitly — guards against a future change that
    // accidentally lets the raw DOMException leak.
    expect((thrown as Error).name).toBe("AuthError");
  });

  it("throws AuthError on header with invalid base64url chars", () => {
    const bodySeg = base64UrlEncode(
      JSON.stringify({
        sub: "abc",
        exp: Math.floor(Date.now() / 1000) + 3600,
        iat: Math.floor(Date.now() / 1000),
      })
    );
    const jwt = `%%%.${bodySeg}.sig`;
    let thrown: unknown;
    try {
      parseJwtPayload(jwt);
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(AuthError);
  });

  // ── F9: close coverage of !json / typeof !== "object" branches ───────────

  it.each([
    ["null", "null"],
    ["number", "42"],
    ["string", '"hello"'],
  ])("throws when payload JSON is %s (not an object)", (_label, jsonText) => {
    const headerSeg = base64UrlEncode(
      JSON.stringify({ alg: "HS256", typ: "JWT" })
    );
    const bodySeg = base64UrlEncode(jsonText);
    const jwt = `${headerSeg}.${bodySeg}.sig`;
    expect(() => parseJwtPayload(jwt)).toThrow(AuthError);
  });
});

// ── pendingAuthSessions TTL + size cap (C1) ─────────────────────────────────

describe("pendingAuthSessions TTL + size cap", () => {
  // We avoid `vi.useFakeTimers()` / `vi.setSystemTime` because the bun-vitest
  // compatibility shim doesn't implement `setSystemTime`. Spying on Date.now
  // directly is portable across both runners and is sufficient for TTL math.
  // `withMockedNow` is async-aware: the spy stays installed until `fn`'s
  // returned promise settles (otherwise the `finally` would restore Date.now
  // before any awaited continuation runs).
  async function withMockedNow<T>(
    fn: (advance: (ms: number) => void) => T | Promise<T>
  ): Promise<T> {
    let current = 1_700_000_000_000; // arbitrary fixed epoch
    const spy = vi.spyOn(Date, "now").mockImplementation(() => current);
    try {
      return await fn((ms) => {
        current += ms;
      });
    } finally {
      spy.mockRestore();
    }
  }

  it("getSession returns undefined for expired entry and evicts it", async () => {
    await withMockedNow((advance) => {
      setSession("s-1", {
        verifier: "v",
        state: "st",
        redirectUri: "http://127.0.0.1:1/cb",
        sessionId: "s-1",
        createdAt: Date.now(),
      });

      // Within TTL — still present.
      advance(PENDING_SESSION_TTL_MS - 1000);
      expect(getSession("s-1")).toBeDefined();

      // Past TTL — gone, and the underlying map no longer holds it.
      advance(2000);
      expect(getSession("s-1")).toBeUndefined();
      expect(pendingAuthSessions.has("s-1")).toBe(false);
    });
  });

  it("setSession evicts the oldest entry when the cap is exceeded (FIFO)", () => {
    // Fill to the cap.
    for (let i = 0; i < PENDING_SESSION_MAX; i++) {
      setSession(`id-${i}`, {
        verifier: "v",
        state: "st",
        redirectUri: "http://127.0.0.1:1/cb",
        sessionId: `id-${i}`,
        createdAt: Date.now(),
      });
    }
    expect(pendingAuthSessions.size).toBe(PENDING_SESSION_MAX);
    expect(pendingAuthSessions.has("id-0")).toBe(true);

    // Inserting one more evicts the oldest (FIFO = first insertion).
    setSession("id-extra", {
      verifier: "v",
      state: "st",
      redirectUri: "http://127.0.0.1:1/cb",
      sessionId: "id-extra",
      createdAt: Date.now(),
    });
    expect(pendingAuthSessions.size).toBe(PENDING_SESSION_MAX);
    expect(pendingAuthSessions.has("id-0")).toBe(false);
    expect(pendingAuthSessions.has("id-extra")).toBe(true);
  });

  it("buildAuthorizeUrl-stored session expires after TTL (integration)", async () => {
    await withMockedNow(async (advance) => {
      const seeded = await buildAuthorizeUrl({
        baseUrl: "https://mcp.mnemonik.xyz",
        clientId: "mnemonic-cli",
        redirectUri: "http://127.0.0.1:1/cb",
      });
      expect(getSession(seeded.sessionId)).toBeDefined();
      advance(PENDING_SESSION_TTL_MS + 1);
      expect(getSession(seeded.sessionId)).toBeUndefined();
    });
  });
});

// ────────────────────────────────────────────────────────────────────────────
// loginWithIdentity — browserless OAuth (issue #27 fix)
// ────────────────────────────────────────────────────────────────────────────

describe("loginWithIdentity", () => {
  const mockChallenge = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
  const mockChallengeB64 = btoa(
    String.fromCharCode(...mockChallenge)
  );

  beforeEach(() => {
    __setWasmForTesting(buildWasmMock() as never);
  });
  afterEach(() => {
    __setWasmForTesting(null);
  });

  function makeJwt(sub: string): string {
    const header = { alg: "HS256", typ: "JWT" };
    const payload = {
      sub,
      iat: Math.floor(Date.now() / 1000),
      exp: Math.floor(Date.now() / 1000) + 3600,
    };
    return `${base64UrlEncode(JSON.stringify(header))}.${base64UrlEncode(
      JSON.stringify(payload)
    )}.sig`;
  }

  /**
   * Build a three-endpoint mock server that mirrors the real server's
   * contract for the programmatic OAuth flow. Captures every call so tests
   * can assert URL shape, query params, and request bodies.
   */
  function buildMockOAuthServer(pubkey: string): {
    fetchImpl: typeof fetch;
    calls: Array<{ url: string; method: string; body?: unknown }>;
  } {
    const calls: Array<{ url: string; method: string; body?: unknown }> = [];
    const fetchImpl = async (
      input: RequestInfo | URL,
      init?: RequestInit
    ): Promise<Response> => {
      const url = typeof input === "string" ? input : input.toString();
      const method = init?.method ?? "GET";
      let body: unknown;
      if (typeof init?.body === "string") {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }
      calls.push({ url, method, body });

      if (method === "GET" && url.startsWith("https://srv/oauth/authorize")) {
        return new Response(
          JSON.stringify({
            challenge_cbor: mockChallengeB64,
            state: new URL(url).searchParams.get("state"),
            exp: Math.floor(Date.now() / 1000) + 60,
          }),
          { status: 200, headers: { "content-type": "application/json" } }
        );
      }
      if (
        method === "POST" &&
        url === "https://srv/oauth/authorize"
      ) {
        const reqBody = body as {
          state: string;
          signature: string;
          signer_pubkey: string;
        };
        if (reqBody.signer_pubkey !== pubkey) {
          return new Response(
            JSON.stringify({ error: "signer pubkey mismatch" }),
            { status: 401, headers: { "content-type": "application/json" } }
          );
        }
        return new Response(
          JSON.stringify({
            code: "code-fixture-1",
            state: reqBody.state,
            redirect_uri: BROWSERLESS_REDIRECT_URI,
          }),
          { status: 200, headers: { "content-type": "application/json" } }
        );
      }
      if (method === "POST" && url === "https://srv/oauth/token") {
        return new Response(
          JSON.stringify({
            access_token: makeJwt(pubkey),
            token_type: "Bearer",
            expires_in: 3600,
            scope: "mcp",
          }),
          { status: 200, headers: { "content-type": "application/json" } }
        );
      }
      return new Response("not found", { status: 404 });
    };
    return { fetchImpl, calls };
  }

  it("end-to-end: signs challenge, exchanges code, returns JWT.sub === keypair.pubkey", async () => {
    const kp = await Keypair.generate();
    const { fetchImpl, calls } = buildMockOAuthServer(kp.pubkey);

    const result = await loginWithIdentity({
      baseUrl: "https://srv",
      clientId: "mnemonic-cli",
      keypair: kp,
      fetch: fetchImpl,
    });

    expect(result.jwt).toMatch(/^[\w-]+\.[\w-]+\.[\w-]+$/);
    expect(result.sub).toBe(kp.pubkey);
    expect(typeof result.expiresAt).toBe("string");

    // Three calls in canonical order.
    expect(calls).toHaveLength(3);
    expect(calls[0]!.method).toBe("GET");
    expect(calls[0]!.url).toContain("/oauth/authorize");
    expect(calls[0]!.url).toContain(`pubkey=${encodeURIComponent(kp.pubkey)}`);
    // Sentinel redirect_uri propagates and is loopback-shaped.
    expect(calls[0]!.url).toContain(
      `redirect_uri=${encodeURIComponent(BROWSERLESS_REDIRECT_URI)}`
    );

    expect(calls[1]!.method).toBe("POST");
    expect(calls[1]!.url).toBe("https://srv/oauth/authorize");
    const authBody = calls[1]!.body as {
      state: string;
      signature: string;
      signer_pubkey: string;
    };
    expect(authBody.signer_pubkey).toBe(kp.pubkey);
    // Raw 64-byte Ed25519 → base64 (no padding rstrip) = 88 chars.
    expect(authBody.signature).toHaveLength(88);

    expect(calls[2]!.method).toBe("POST");
    expect(calls[2]!.url).toBe("https://srv/oauth/token");
    const tokenBody = calls[2]!.body as {
      code: string;
      code_verifier: string;
      redirect_uri: string;
    };
    expect(tokenBody.code).toBe("code-fixture-1");
    expect(tokenBody.redirect_uri).toBe(BROWSERLESS_REDIRECT_URI);
    expect(tokenBody.code_verifier).toMatch(/^[A-Za-z0-9_-]{43,}$/);
  });

  it("rejects when authorize-init returns non-2xx", async () => {
    const kp = await Keypair.generate();
    const fetchImpl: typeof fetch = async () =>
      new Response("denied", { status: 400 });

    await expect(
      loginWithIdentity({
        baseUrl: "https://srv",
        clientId: "mnemonic-cli",
        keypair: kp,
        fetch: fetchImpl,
      })
    ).rejects.toBeInstanceOf(AuthError);
  });

  it("rejects when authorize-init returns mismatched state", async () => {
    const kp = await Keypair.generate();
    const fetchImpl: typeof fetch = async () =>
      new Response(
        JSON.stringify({
          challenge_cbor: mockChallengeB64,
          state: "EVIL-INJECTED-STATE",
          exp: 0,
        }),
        { status: 200, headers: { "content-type": "application/json" } }
      );

    await expect(
      loginWithIdentity({
        baseUrl: "https://srv",
        clientId: "mnemonic-cli",
        keypair: kp,
        fetch: fetchImpl,
      })
    ).rejects.toMatchObject({
      message: expect.stringMatching(/state mismatch/),
    });
  });

  it("rejects when POST /oauth/authorize returns 401 (server pubkey mismatch)", async () => {
    const kp = await Keypair.generate();
    // Server expects a DIFFERENT pubkey, so the POST will 401.
    const { fetchImpl } = buildMockOAuthServer("DIFFERENT_PUBKEY_xyz");

    await expect(
      loginWithIdentity({
        baseUrl: "https://srv",
        clientId: "mnemonic-cli",
        keypair: kp,
        fetch: fetchImpl,
      })
    ).rejects.toMatchObject({
      message: expect.stringMatching(/authorize returned 401/),
    });
  });
});
