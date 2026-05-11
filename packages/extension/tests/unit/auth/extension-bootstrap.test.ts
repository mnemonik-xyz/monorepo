// Tests for the extension-bootstrap handshake (audit B1 / AUD-S-01).
//
// Covers:
//   - happy path: POST /issue returns ticket → GET /redeem returns the
//     aud=extension JWT.
//   - single-use ticket: a second GET against the same ticket returns
//     404 → ExtensionBootstrapTicketNotFoundError.
//   - expired ticket: server returns 404 → same typed error.
//   - rate-limited: POST /issue 429 → ExtensionBootstrapRateLimitError.
//   - 401 on /issue (wrong audience / expired mcp JWT) → AuthError.
//   - ensureExtensionJwt caches the JWT in storage and reuses it.

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  redeemExtensionJwt,
  ensureExtensionJwt,
  getExtensionJwt,
  clearExtensionSession,
  EXTENSION_SESSION_STORAGE_KEY,
  ExtensionBootstrapRateLimitError,
  ExtensionBootstrapTicketNotFoundError,
  type StorageArea,
} from "../../../src/auth/extension-bootstrap.js";
import { AuthError } from "../../../src/auth/types.js";

const MCP_JWT = "header.mcp-payload.signature";
const EXT_JWT = "header.ext-payload.signature";
const TICKET_ID = "11111111-2222-4333-8444-555555555555";

function makeStorage(): { area: StorageArea; map: Map<string, unknown> } {
  const map = new Map<string, unknown>();
  return {
    map,
    area: {
      async get(keys) {
        const out: Record<string, unknown> = {};
        const list = typeof keys === "string" ? [keys] : keys;
        for (const k of list) if (map.has(k)) out[k] = map.get(k);
        return out;
      },
      async set(items) {
        for (const [k, v] of Object.entries(items)) map.set(k, v);
      },
      async remove(keys) {
        const list = typeof keys === "string" ? [keys] : keys;
        for (const k of list) map.delete(k);
      },
    },
  };
}

let originalFetch: typeof globalThis.fetch | undefined;

beforeEach(() => {
  originalFetch = globalThis.fetch;
});
afterEach(() => {
  if (originalFetch) globalThis.fetch = originalFetch;
});

function installFetch(
  handler: (url: string, init?: RequestInit) => Response | Promise<Response>,
): { calls: { url: string; init?: RequestInit }[] } {
  const calls: { url: string; init?: RequestInit }[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const entry: { url: string; init?: RequestInit } = { url };
    if (init !== undefined) entry.init = init;
    calls.push(entry);
    return await handler(url, init);
  }) as typeof globalThis.fetch;
  return { calls };
}

describe("redeemExtensionJwt — happy path", () => {
  it("issues a ticket then redeems it for an aud=extension JWT", async () => {
    const { calls } = installFetch((url) => {
      if (url.endsWith("/api/extension-bootstrap/issue")) {
        return new Response(JSON.stringify({ ticket_id: TICKET_ID }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      if (url.endsWith(`/api/extension-bootstrap/redeem/${TICKET_ID}`)) {
        return new Response(
          JSON.stringify({
            access_token: EXT_JWT,
            aud: "extension",
            expires_in: 3600,
          }),
          {
            status: 200,
            headers: { "content-type": "application/json" },
          },
        );
      }
      return new Response("not found", { status: 404 });
    });

    const out = await redeemExtensionJwt(MCP_JWT, {
      serverOrigin: "https://srv.example",
    });

    expect(out.jwt).toBe(EXT_JWT);
    expect(out.expiresAt).toBeGreaterThan(Date.now());
    expect(calls).toHaveLength(2);
    // First call: POST issue with Bearer.
    expect(calls[0]!.url).toBe(
      "https://srv.example/api/extension-bootstrap/issue",
    );
    expect(calls[0]!.init?.method).toBe("POST");
    const headers = calls[0]!.init?.headers as Record<string, string>;
    expect(headers.authorization).toBe(`Bearer ${MCP_JWT}`);
    // Second call: GET redeem with no Authorization header (capability).
    expect(calls[1]!.url).toBe(
      `https://srv.example/api/extension-bootstrap/redeem/${TICKET_ID}`,
    );
    expect(calls[1]!.init?.method).toBe("GET");
    const redeemHeaders = (calls[1]!.init?.headers ?? {}) as Record<
      string,
      string
    >;
    expect(redeemHeaders.authorization).toBeUndefined();
  });
});

describe("redeemExtensionJwt — single-use ticket / expired", () => {
  it("redeem 404 surfaces ExtensionBootstrapTicketNotFoundError", async () => {
    installFetch((url) => {
      if (url.endsWith("/api/extension-bootstrap/issue")) {
        return new Response(JSON.stringify({ ticket_id: TICKET_ID }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      // Second GET — server says ticket consumed / expired / unknown.
      return new Response(JSON.stringify({ error: "ticket_not_found" }), {
        status: 404,
        headers: { "content-type": "application/json" },
      });
    });

    await expect(
      redeemExtensionJwt(MCP_JWT, { serverOrigin: "https://srv.example" }),
    ).rejects.toBeInstanceOf(ExtensionBootstrapTicketNotFoundError);
  });
});

describe("redeemExtensionJwt — rate limited", () => {
  it("issue 429 surfaces ExtensionBootstrapRateLimitError", async () => {
    installFetch(() => {
      return new Response(
        JSON.stringify({ error: "too_many_pending_tickets" }),
        {
          status: 429,
          headers: { "content-type": "application/json" },
        },
      );
    });

    await expect(
      redeemExtensionJwt(MCP_JWT, { serverOrigin: "https://srv.example" }),
    ).rejects.toBeInstanceOf(ExtensionBootstrapRateLimitError);
  });
});

describe("redeemExtensionJwt — invalid mcp JWT", () => {
  it("issue 401 surfaces AuthError", async () => {
    installFetch(
      () =>
        new Response(JSON.stringify({ error: "jwt_invalid" }), { status: 401 }),
    );
    await expect(
      redeemExtensionJwt(MCP_JWT, { serverOrigin: "https://srv.example" }),
    ).rejects.toBeInstanceOf(AuthError);
  });

  it("rejects an empty mcpJwt before any fetch", async () => {
    const { calls } = installFetch(
      () => new Response(JSON.stringify({}), { status: 200 }),
    );
    await expect(
      redeemExtensionJwt("", { serverOrigin: "https://srv.example" }),
    ).rejects.toBeInstanceOf(AuthError);
    expect(calls).toHaveLength(0);
  });
});

describe("ensureExtensionJwt — caching", () => {
  it("first call hits the network; second call serves from cache", async () => {
    const { calls } = installFetch((url) => {
      if (url.endsWith("/api/extension-bootstrap/issue")) {
        return new Response(JSON.stringify({ ticket_id: TICKET_ID }), {
          status: 200,
        });
      }
      return new Response(
        JSON.stringify({
          access_token: EXT_JWT,
          aud: "extension",
          expires_in: 3600,
        }),
        { status: 200 },
      );
    });

    const { area, map } = makeStorage();

    const first = await ensureExtensionJwt(MCP_JWT, {
      serverOrigin: "https://srv.example",
      storage: area,
    });
    expect(first).toBe(EXT_JWT);
    expect(map.get(EXTENSION_SESSION_STORAGE_KEY)).toMatchObject({
      jwt: EXT_JWT,
    });
    expect(calls.length).toBe(2);

    const second = await ensureExtensionJwt(MCP_JWT, {
      serverOrigin: "https://srv.example",
      storage: area,
    });
    expect(second).toBe(EXT_JWT);
    // No new fetches — cache served.
    expect(calls.length).toBe(2);
  });

  it("re-handshakes when cached JWT is within the refresh buffer", async () => {
    const { calls } = installFetch(() => {
      // Distinct call sites; the URL fork below decides which to return.
      return new Response(JSON.stringify({ ticket_id: TICKET_ID }), {
        status: 200,
      });
    });
    // Override with a per-URL handler.
    globalThis.fetch = (async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const url = typeof input === "string" ? input : input.toString();
      const entry: { url: string; init?: RequestInit } = { url };
      if (init !== undefined) entry.init = init;
      calls.push(entry);
      if (url.endsWith("/api/extension-bootstrap/issue")) {
        return new Response(JSON.stringify({ ticket_id: TICKET_ID }), {
          status: 200,
        });
      }
      return new Response(
        JSON.stringify({
          access_token: `${EXT_JWT}-${String(calls.length)}`,
          aud: "extension",
          expires_in: 3600,
        }),
        { status: 200 },
      );
    }) as typeof globalThis.fetch;

    const { area, map } = makeStorage();
    // Seed an almost-expired blob.
    map.set(EXTENSION_SESSION_STORAGE_KEY, {
      jwt: "stale-jwt",
      expiresAt: Date.now() + 1_000, // < 60s buffer
    });

    const out = await ensureExtensionJwt(MCP_JWT, {
      serverOrigin: "https://srv.example",
      storage: area,
    });
    expect(out).not.toBe("stale-jwt");
    expect(out.startsWith(EXT_JWT)).toBe(true);
    // Stale blob was overwritten.
    const blob = map.get(EXTENSION_SESSION_STORAGE_KEY) as {
      jwt: string;
      expiresAt: number;
    };
    expect(blob.jwt).toBe(out);
    expect(blob.expiresAt).toBeGreaterThan(Date.now() + 60_000);
  });
});

describe("getExtensionJwt / clearExtensionSession", () => {
  it("returns null when no session is cached", async () => {
    const { area } = makeStorage();
    expect(await getExtensionJwt(area)).toBeNull();
  });

  it("clears the cached session", async () => {
    const { area, map } = makeStorage();
    map.set(EXTENSION_SESSION_STORAGE_KEY, {
      jwt: EXT_JWT,
      expiresAt: Date.now() + 3_600_000,
    });
    expect(await getExtensionJwt(area)).toBe(EXT_JWT);
    await clearExtensionSession(area);
    expect(await getExtensionJwt(area)).toBeNull();
  });
});
