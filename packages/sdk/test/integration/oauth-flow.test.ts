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
