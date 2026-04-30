// Integration: full OAuth 2.1 + PKCE round-trip against the mock server.
//
// Drives:
//   - tech-spec § Testing § Integration tests / Decision 5 (PKCE binding)
//   - test-reviewer fault-coverage finding (callback-timeout)

import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
} from "vitest";

import {
  buildAuthorizeUrl,
  exchangeCodeForToken,
  parseJwtPayload,
} from "../../src/oauth.js";
import { AuthError } from "../../src/errors.js";
import { startMockServer, type MockServer } from "../mock-server.js";

let server: MockServer;
const ORIGINAL_FETCH = globalThis.fetch;

beforeAll(async () => {
  server = await startMockServer();
});

afterAll(async () => {
  // Close any hung sockets first (the callback-timeout fault leaves
  // half-open requests on the server side).
  server.closeAllConnections();
  await server.close();
});

beforeEach(() => {
  server.reset();
});

afterEach(() => {
  globalThis.fetch = ORIGINAL_FETCH;
});

describe("oauth happy path", () => {
  it("authorize → token round-trip yields a valid JWT", async () => {
    const redirectUri = "http://127.0.0.1:33418/callback";
    const built = await buildAuthorizeUrl({
      baseUrl: server.url,
      clientId: "mnemonic-cli",
      redirectUri,
    });

    // Drive the authorize endpoint to harvest a code (in real life the
    // browser handles the 302 redirect; here we follow it manually).
    const authzRes = await fetch(built.url, { redirect: "manual" });
    expect(authzRes.status).toBe(302);
    const location = authzRes.headers.get("location") ?? "";
    const cbUrl = new URL(location);
    const code = cbUrl.searchParams.get("code") ?? "";
    const state = cbUrl.searchParams.get("state") ?? "";
    expect(code.length).toBeGreaterThan(0);
    expect(state).toBe(built.state);

    const tok = await exchangeCodeForToken({
      baseUrl: server.url,
      code,
      state,
      redirectUri,
      sessionId: built.sessionId,
    });
    expect(tok.jwt.split(".")).toHaveLength(3);
    const payload = parseJwtPayload(tok.jwt);
    expect(payload.sub).toBe("test-user-pubkey");
    expect(payload.exp).toBeGreaterThan(Math.floor(Date.now() / 1000));
  });
});

describe("oauth fault: callback-timeout", () => {
  it("AuthError is raised when /oauth/token never responds", async () => {
    server.withFault("callback-timeout");
    const redirectUri = "http://127.0.0.1:33418/callback";
    const built = await buildAuthorizeUrl({
      baseUrl: server.url,
      clientId: "mnemonic-cli",
      redirectUri,
    });

    // Authorize step works (only /oauth/token is gated by the fault).
    const authzRes = await fetch(built.url, { redirect: "manual" });
    const cbUrl = new URL(authzRes.headers.get("location") ?? "");
    const code = cbUrl.searchParams.get("code") ?? "";
    const state = cbUrl.searchParams.get("state") ?? "";

    // Wrap globalThis.fetch with an AbortController so the SDK's bare
    // `fetch(tokenUrl, ...)` call (in oauth.ts) inherits a 250ms deadline.
    // On abort, oauth.ts catches the rejection and throws AuthError.
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const ac = new AbortController();
      const timer = setTimeout(() => ac.abort(), 250);
      return ORIGINAL_FETCH(input, { ...init, signal: ac.signal }).finally(() =>
        clearTimeout(timer)
      );
    }) as typeof fetch;

    await expect(
      exchangeCodeForToken({
        baseUrl: server.url,
        code,
        state,
        redirectUri,
        sessionId: built.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
  }, 5000);
});

describe("oauth: production RFC 6749 wire shape (regression)", () => {
  // T11 audit (A04): the SDK MUST accept the canonical RFC 6749 §5.1 token
  // response shape produced by mcp/src/oauth.rs::token_handler:
  //   { access_token: <jwt>, token_type: "Bearer", expires_in: <secs>, scope: "mcp" }
  // Pre-T11, exchangeCodeForToken required a `jwt` field and threw
  // AuthError("malformed body") against the live server. The mock-server
  // hid this by emitting the legacy {jwt, expires_at} shape. This test
  // bypasses the mock and stubs `fetch` with the exact production payload
  // so that any future regression that drops `access_token` support fails
  // here, regardless of mock-server changes.
  it("accepts {access_token, token_type, expires_in, scope} from production server", async () => {
    const redirectUri = "http://127.0.0.1:33418/callback";
    const built = await buildAuthorizeUrl({
      baseUrl: server.url,
      clientId: "mnemonic-cli",
      redirectUri,
    });

    // Drive the authorize step against the real mock so the SDK has a
    // valid (state, sessionId) pair to consume. Then stub fetch on the
    // POST /oauth/token call to return the production wire shape.
    const authzRes = await fetch(built.url, { redirect: "manual" });
    const cbUrl = new URL(authzRes.headers.get("location") ?? "");
    const code = cbUrl.searchParams.get("code") ?? "";
    const state = cbUrl.searchParams.get("state") ?? "";

    // Build a JWT-shaped string mimicking the real server's HS256 output.
    // (Decision 6 — the SDK's parseJwtPayload validates alg=HS256, so a
    // bare "test-token" won't survive a downstream parseJwtPayload call.
    // For exchangeCodeForToken itself we only need a non-empty string.)
    const fakeAccessToken = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.sig";
    const expiresIn = 3600;

    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString();
      if (url.endsWith("/oauth/token") && init?.method === "POST") {
        return Promise.resolve(
          new Response(
            JSON.stringify({
              access_token: fakeAccessToken,
              token_type: "Bearer",
              expires_in: expiresIn,
              scope: "mcp",
            }),
            {
              status: 200,
              headers: { "content-type": "application/json" },
            }
          )
        );
      }
      return ORIGINAL_FETCH(input, init);
    }) as typeof fetch;

    const before = Date.now();
    const tok = await exchangeCodeForToken({
      baseUrl: server.url,
      code,
      state,
      redirectUri,
      sessionId: built.sessionId,
    });
    const after = Date.now();

    // jwt comes from access_token verbatim.
    expect(tok.jwt).toBe(fakeAccessToken);

    // expiresAt was computed from expires_in (seconds → ISO).
    const expiresAtMs = new Date(tok.expiresAt).getTime();
    expect(expiresAtMs).toBeGreaterThanOrEqual(before + expiresIn * 1000);
    expect(expiresAtMs).toBeLessThanOrEqual(after + expiresIn * 1000 + 100);
  });

  it("legacy {jwt, expires_at} shape still accepted (back-compat)", async () => {
    const redirectUri = "http://127.0.0.1:33418/callback";
    const built = await buildAuthorizeUrl({
      baseUrl: server.url,
      clientId: "mnemonic-cli",
      redirectUri,
    });
    const authzRes = await fetch(built.url, { redirect: "manual" });
    const cbUrl = new URL(authzRes.headers.get("location") ?? "");
    const code = cbUrl.searchParams.get("code") ?? "";
    const state = cbUrl.searchParams.get("state") ?? "";

    const legacyJwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.legacy-sig";
    const legacyExpiresAt = "2030-01-01T00:00:00.000Z";

    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input.toString();
      if (url.endsWith("/oauth/token") && init?.method === "POST") {
        return Promise.resolve(
          new Response(
            JSON.stringify({ jwt: legacyJwt, expires_at: legacyExpiresAt }),
            {
              status: 200,
              headers: { "content-type": "application/json" },
            }
          )
        );
      }
      return ORIGINAL_FETCH(input, init);
    }) as typeof fetch;

    const tok = await exchangeCodeForToken({
      baseUrl: server.url,
      code,
      state,
      redirectUri,
      sessionId: built.sessionId,
    });
    expect(tok.jwt).toBe(legacyJwt);
    expect(tok.expiresAt).toBe(legacyExpiresAt);
  });
});

describe("oauth fault: 5xx-on-token-exchange", () => {
  it("AuthError on token endpoint 500", async () => {
    server.withFault("5xx-on-token-exchange");
    const redirectUri = "http://127.0.0.1:33418/callback";
    const built = await buildAuthorizeUrl({
      baseUrl: server.url,
      clientId: "mnemonic-cli",
      redirectUri,
    });
    const authzRes = await fetch(built.url, { redirect: "manual" });
    const cbUrl = new URL(authzRes.headers.get("location") ?? "");
    const code = cbUrl.searchParams.get("code") ?? "";
    const state = cbUrl.searchParams.get("state") ?? "";

    await expect(
      exchangeCodeForToken({
        baseUrl: server.url,
        code,
        state,
        redirectUri,
        sessionId: built.sessionId,
      })
    ).rejects.toBeInstanceOf(AuthError);
  });
});
