// Unit tests for the refresh-token surface (issue #33):
//   - refreshAccessToken (grant_type=refresh_token)
//   - loginWithIdentity surfaces the server's refresh_token
//
// Pure ESM; fetch is injected per call.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { AuthError, UserError } from "../src/errors.js";
import { Keypair } from "../src/keypair.js";
import {
  BROWSERLESS_REDIRECT_URI,
  loginWithIdentity,
  readJwtExp,
  refreshAccessToken,
} from "../src/oauth.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";

function b64url(s: string): string {
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function makeJwt(sub: string, expSecondsFromNow = 3600): string {
  const now = Math.floor(Date.now() / 1000);
  return `${b64url(JSON.stringify({ alg: "HS256", typ: "JWT" }))}.${b64url(
    JSON.stringify({ sub, iat: now, exp: now + expSecondsFromNow })
  )}.sig`;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("refreshAccessToken", () => {
  it("posts grant_type=refresh_token and returns the rotated pair", async () => {
    const calls: Array<{ url: string; body: Record<string, unknown> }> = [];
    const jwt = makeJwt("PUBKEY_1");
    const fetchImpl = (async (url: RequestInfo | URL, init?: RequestInit) => {
      calls.push({
        url: String(url),
        body: JSON.parse(String(init?.body)) as Record<string, unknown>,
      });
      return json({
        access_token: jwt,
        token_type: "Bearer",
        expires_in: 3600,
        scope: "mcp",
        refresh_token: "rt-2",
      });
    }) as typeof fetch;

    const r = await refreshAccessToken({
      baseUrl: "https://srv",
      refreshToken: "rt-1",
      fetch: fetchImpl,
    });

    expect(r.jwt).toBe(jwt);
    expect(r.sub).toBe("PUBKEY_1");
    expect(r.refreshToken).toBe("rt-2");
    expect(Date.parse(r.expiresAt)).toBeGreaterThan(Date.now());
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe("https://srv/oauth/token");
    expect(calls[0]!.body).toEqual({
      grant_type: "refresh_token",
      refresh_token: "rt-1",
      client_id: "mnemonic-cli",
    });
  });

  it("keeps the old refresh token when the server does not rotate", async () => {
    const fetchImpl = (async () =>
      json({ access_token: makeJwt("P"), expires_in: 60 })) as typeof fetch;
    const r = await refreshAccessToken({
      baseUrl: "https://srv",
      refreshToken: "rt-keep",
      fetch: fetchImpl,
    });
    expect(r.refreshToken).toBe("rt-keep");
  });

  it("maps invalid_grant to AuthError without echoing the token", async () => {
    const fetchImpl = (async () =>
      json({ error: "invalid_grant" }, 400)) as typeof fetch;
    const err = await refreshAccessToken({
      baseUrl: "https://srv",
      refreshToken: "secret-refresh-token-value",
      fetch: fetchImpl,
    }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(AuthError);
    expect((err as Error).message).toMatch(/invalid_grant/);
    expect((err as Error).message).not.toContain("secret-refresh-token-value");
  });

  it("maps a network failure to AuthError", async () => {
    const fetchImpl = (async () => {
      throw new TypeError("fetch failed");
    }) as typeof fetch;
    await expect(
      refreshAccessToken({
        baseUrl: "https://srv",
        refreshToken: "rt",
        fetch: fetchImpl,
      })
    ).rejects.toBeInstanceOf(AuthError);
  });

  it("rejects a body without access_token", async () => {
    const fetchImpl = (async () => json({ refresh_token: "x" })) as typeof fetch;
    await expect(
      refreshAccessToken({
        baseUrl: "https://srv",
        refreshToken: "rt",
        fetch: fetchImpl,
      })
    ).rejects.toThrow(/malformed body/);
  });

  it("rejects an empty refresh token before any request", async () => {
    let called = false;
    const fetchImpl = (async () => {
      called = true;
      return json({});
    }) as typeof fetch;
    await expect(
      refreshAccessToken({ baseUrl: "https://srv", refreshToken: "", fetch: fetchImpl })
    ).rejects.toBeInstanceOf(UserError);
    expect(called).toBe(false);
  });
});

describe("readJwtExp", () => {
  it("reads exp from an expired token without throwing", () => {
    const jwt = makeJwt("P", -100);
    const exp = readJwtExp(jwt);
    expect(typeof exp).toBe("number");
    expect(exp! * 1000).toBeLessThan(Date.now());
  });

  it("returns undefined for a malformed token", () => {
    expect(readJwtExp("not-a-jwt")).toBeUndefined();
    expect(readJwtExp("a.%%%.c")).toBeUndefined();
  });
});

describe("loginWithIdentity refresh token", () => {
  beforeEach(() => {
    __setWasmForTesting(buildWasmMock() as never);
  });
  afterEach(() => {
    __setWasmForTesting(null);
  });

  function server(pubkey: string, tokenExtra: Record<string, unknown>) {
    return (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      if (method === "GET") {
        return json({
          challenge_cbor: btoa("\u0001\u0002\u0003"),
          state: new URL(url).searchParams.get("state"),
        });
      }
      if (url === "https://srv/oauth/authorize") {
        const body = JSON.parse(String(init?.body)) as { state: string };
        return json({
          code: "c1",
          state: body.state,
          redirect_uri: BROWSERLESS_REDIRECT_URI,
        });
      }
      return json({
        access_token: makeJwt(pubkey),
        expires_in: 3600,
        ...tokenExtra,
      });
    }) as typeof fetch;
  }

  it("returns refreshToken when the token endpoint issues one", async () => {
    const kp = await Keypair.generate();
    const r = await loginWithIdentity({
      baseUrl: "https://srv",
      clientId: "mnemonic-cli",
      keypair: kp,
      fetch: server(kp.pubkey, { refresh_token: "rt-login" }),
    });
    expect(r.refreshToken).toBe("rt-login");
  });

  it("omits refreshToken when the server does not issue one", async () => {
    const kp = await Keypair.generate();
    const r = await loginWithIdentity({
      baseUrl: "https://srv",
      clientId: "mnemonic-cli",
      keypair: kp,
      fetch: server(kp.pubkey, {}),
    });
    expect("refreshToken" in r).toBe(false);
  });
});
