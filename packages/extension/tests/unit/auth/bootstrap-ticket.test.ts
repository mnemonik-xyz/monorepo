// Extension-bootstrap ticket flow tests (FW-S-01 fix).
//
// Drives the aud=mcp → aud=extension JWT handshake end-to-end against a
// mock fetch and asserts:
//   - POST /api/extension-bootstrap/issue carries Bearer <jwt_mcp>
//   - GET  /api/extension-bootstrap/redeem/:ticket has NO Authorization
//   - the resulting aud=extension access_token is the value persisted
//     onto Session.extensionJwt and used by subsequent key-escrow ops.

import { describe, it, expect, beforeEach } from "vitest";
import {
  issueTicket,
  redeemTicket,
  getOrRefreshExtensionJwt,
} from "../../../src/auth/bootstrap-ticket.js";
import { setSession, type StorageArea } from "../../../src/auth/session.js";
import type { Session } from "../../../src/auth/types.js";

interface FetchCall {
  url: string;
  init?: RequestInit;
}

function makeFakeFetch(responses: Array<(call: FetchCall) => Response>): {
  fetch: typeof fetch;
  calls: FetchCall[];
} {
  const calls: FetchCall[] = [];
  let idx = 0;
  const fakeFetch: typeof fetch = (async (
    url: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const u = typeof url === "string" ? url : url.toString();
    const call: FetchCall = { url: u };
    if (init !== undefined) call.init = init;
    calls.push(call);
    const handler = responses[idx++];
    if (!handler) throw new Error(`no handler for fetch #${idx} → ${u}`);
    return handler(call);
  }) as typeof fetch;
  return { fetch: fakeFetch, calls };
}

function authHeader(init: RequestInit | undefined): string | null {
  const h = init?.headers as Record<string, string> | undefined;
  if (!h) return null;
  return h.authorization ?? h.Authorization ?? null;
}

const MCP_JWT = "header.payloadMCP.signature";
const EXT_JWT = "header.payloadEXT.signature";
const TICKET_ID = "00000000-0000-0000-0000-000000000001";

describe("bootstrap-ticket — handshake sequence", () => {
  it("issueTicket POSTs Bearer aud=mcp JWT and returns the ticket UUID", async () => {
    const { fetch, calls } = makeFakeFetch([
      () =>
        new Response(JSON.stringify({ ticket_id: TICKET_ID }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
    ]);
    const ticket = await issueTicket(MCP_JWT, {
      serverOrigin: "https://mc.test",
      fetchImpl: fetch,
    });
    expect(ticket).toBe(TICKET_ID);
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe("https://mc.test/api/extension-bootstrap/issue");
    expect(calls[0]?.init?.method).toBe("POST");
    expect(authHeader(calls[0]?.init)).toBe(`Bearer ${MCP_JWT}`);
  });

  it("redeemTicket GETs with NO Authorization header and returns the aud=extension JWT", async () => {
    const { fetch, calls } = makeFakeFetch([
      () =>
        new Response(
          JSON.stringify({
            access_token: EXT_JWT,
            aud: "extension",
            expires_in: 3600,
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    ]);
    const result = await redeemTicket(TICKET_ID, {
      serverOrigin: "https://mc.test",
      fetchImpl: fetch,
    });
    expect(result.access_token).toBe(EXT_JWT);
    expect(result.expires_in).toBe(3600);
    expect(calls).toHaveLength(1);
    expect(calls[0]?.init?.method).toBe("GET");
    expect(authHeader(calls[0]?.init)).toBeNull();
    expect(calls[0]?.url).toBe(
      `https://mc.test/api/extension-bootstrap/redeem/${TICKET_ID}`,
    );
  });

  it("getOrRefreshExtensionJwt drives issue → redeem and caches the result on the Session", async () => {
    const { fetch, calls } = makeFakeFetch([
      () =>
        new Response(JSON.stringify({ ticket_id: TICKET_ID }), { status: 200 }),
      () =>
        new Response(
          JSON.stringify({
            access_token: EXT_JWT,
            aud: "extension",
            expires_in: 3600,
          }),
          { status: 200 },
        ),
    ]);
    // Plug a fake storage to capture the persisted session.
    const store: Record<string, unknown> = {};
    const fakeStorage: StorageArea = {
      async get(keys: string | string[]) {
        const out: Record<string, unknown> = {};
        const ks = Array.isArray(keys) ? keys : [keys];
        for (const k of ks) if (k in store) out[k] = store[k];
        return out;
      },
      async set(items: Record<string, unknown>) {
        Object.assign(store, items);
      },
      async remove(keys: string | string[]) {
        const ks = Array.isArray(keys) ? keys : [keys];
        for (const k of ks) delete store[k];
      },
    };

    const session: Session = {
      jwt: MCP_JWT,
      googleSub: "google-sub-1",
      profile: {
        sub: "google-sub-1",
        email: "u@example.com",
        email_verified: true,
        name: "U",
        picture: "",
      },
      jwtExpiresAt: Date.now() + 600_000,
      signedInAt: Date.now(),
    };
    await setSession(session, fakeStorage);

    const fixedNow = 1_700_000_000_000;
    const jwt = await getOrRefreshExtensionJwt(session, {
      serverOrigin: "https://mc.test",
      fetchImpl: fetch,
      now: () => fixedNow,
    });
    expect(jwt).toBe(EXT_JWT);
    expect(calls).toHaveLength(2);
    // Step 1: POST /issue with aud=mcp Bearer
    expect(calls[0]?.url).toContain("/api/extension-bootstrap/issue");
    expect(authHeader(calls[0]?.init)).toBe(`Bearer ${MCP_JWT}`);
    // Step 2: GET /redeem/:ticket without Bearer
    expect(calls[1]?.url).toContain(`/redeem/${TICKET_ID}`);
    expect(authHeader(calls[1]?.init)).toBeNull();
  });

  it("getOrRefreshExtensionJwt reuses the cached extensionJwt while fresh", async () => {
    const { fetch, calls } = makeFakeFetch([]);
    const sessionWithCache: Session = {
      jwt: MCP_JWT,
      googleSub: "g",
      profile: {
        sub: "g",
        email: "",
        email_verified: false,
        name: "",
        picture: "",
      },
      jwtExpiresAt: Date.now() + 600_000,
      signedInAt: Date.now(),
      extensionJwt: "cached-ext-jwt",
      extensionJwtExpiresAt: Date.now() + 600_000,
    };
    const jwt = await getOrRefreshExtensionJwt(sessionWithCache, {
      serverOrigin: "https://mc.test",
      fetchImpl: fetch,
    });
    expect(jwt).toBe("cached-ext-jwt");
    expect(calls).toHaveLength(0);
  });

  it("redeemTicket maps 404 to AuthError (don't leak ticket-namespace probes)", async () => {
    const { fetch } = makeFakeFetch([
      () =>
        new Response(
          JSON.stringify({
            error: "ticket not found, already consumed, or expired",
          }),
          { status: 404 },
        ),
    ]);
    await expect(
      redeemTicket(TICKET_ID, {
        serverOrigin: "https://mc.test",
        fetchImpl: fetch,
      }),
    ).rejects.toMatchObject({
      message: expect.stringContaining("ticket not found"),
    });
  });
});

beforeEach(() => {
  // No global state to reset.
});
