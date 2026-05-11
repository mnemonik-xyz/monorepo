// Asserts the popup runtime's keyEscrow facade runs the
// extension-bootstrap handshake (audit B1 / AUD-S-01) — i.e. callers
// pass their cached `aud=mcp` JWT but the wire-level fetch carries
// the redeemed `aud=extension` JWT.
//
// We patch globalThis.fetch and chrome.storage.local, then call the
// real runtime.keyEscrow.{fetch,upload,delete,rotate} surface and
// inspect the Authorization header on the /api/key-escrow* request.

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { createDefaultRuntime } from "../../../src/popup/runtime.js";
import { EXTENSION_SESSION_STORAGE_KEY } from "../../../src/auth/extension-bootstrap.js";

const MCP_JWT = "header.mcp.signature";
const EXT_JWT = "header.ext.signature";
const TICKET_ID = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

interface FetchCall {
  url: string;
  method: string;
  authorization: string | undefined;
}

let originalFetch: typeof globalThis.fetch | undefined;
let originalChrome: unknown;
let calls: FetchCall[];

const storageMap = new Map<string, unknown>();

beforeEach(() => {
  originalFetch = globalThis.fetch;
  originalChrome = (globalThis as { chrome?: unknown }).chrome;
  storageMap.clear();
  calls = [];
  // Minimal chrome.storage.local stub the bootstrap module reaches for.
  (globalThis as { chrome?: unknown }).chrome = {
    storage: {
      local: {
        async get(keys: string | string[]) {
          const out: Record<string, unknown> = {};
          const list = typeof keys === "string" ? [keys] : keys;
          for (const k of list)
            if (storageMap.has(k)) out[k] = storageMap.get(k);
          return out;
        },
        async set(items: Record<string, unknown>) {
          for (const [k, v] of Object.entries(items)) storageMap.set(k, v);
        },
        async remove(keys: string | string[]) {
          const list = typeof keys === "string" ? [keys] : keys;
          for (const k of list) storageMap.delete(k);
        },
      },
    },
  };

  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const headers = (init?.headers ?? {}) as Record<string, string>;
    calls.push({
      url,
      method: init?.method ?? "GET",
      authorization: headers.authorization,
    });
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
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (url.endsWith("/api/key-escrow")) {
      if (init?.method === "GET") {
        return new Response(
          JSON.stringify({
            ciphertext_b64: "AA==",
            nonce_b64: "AA==",
            kdf: "argon2id",
            kdf_params: '{"salt_b64":"AA==","m":8,"t":1,"p":1}',
            pubkey_base58: "PUB",
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }
      // PUT / DELETE — 200 OK.
      return new Response("", { status: 200 });
    }
    return new Response("not found", { status: 404 });
  }) as typeof globalThis.fetch;
});

afterEach(() => {
  if (originalFetch) globalThis.fetch = originalFetch;
  (globalThis as { chrome?: unknown }).chrome = originalChrome;
});

describe("popup runtime keyEscrow → bootstrap handshake (B1)", () => {
  it("fetch: runs the handshake then sends the ext JWT on /api/key-escrow", async () => {
    const rt = createDefaultRuntime();
    const blob = await rt.keyEscrow.fetch(MCP_JWT);
    expect(blob.pubkey_base58).toBe("PUB");

    const escrowGet = calls.find(
      (c) => c.url.endsWith("/api/key-escrow") && c.method === "GET",
    );
    expect(escrowGet).toBeDefined();
    expect(escrowGet!.authorization).toBe(`Bearer ${EXT_JWT}`);

    // Handshake fired before the escrow GET.
    const issueIdx = calls.findIndex((c) =>
      c.url.endsWith("/api/extension-bootstrap/issue"),
    );
    const redeemIdx = calls.findIndex((c) =>
      c.url.endsWith(`/api/extension-bootstrap/redeem/${TICKET_ID}`),
    );
    const getIdx = calls.findIndex(
      (c) => c.url.endsWith("/api/key-escrow") && c.method === "GET",
    );
    expect(issueIdx).toBeGreaterThanOrEqual(0);
    expect(redeemIdx).toBeGreaterThan(issueIdx);
    expect(getIdx).toBeGreaterThan(redeemIdx);
  });

  it("upload: PUT carries the ext JWT, not the mcp one", async () => {
    const rt = createDefaultRuntime();
    await rt.keyEscrow.upload(MCP_JWT, {
      ciphertext_b64: "AA==",
      nonce_b64: "AA==",
      kdf: "argon2id",
      kdf_params: '{"salt_b64":"AA==","m":8,"t":1,"p":1}',
      pubkey_base58: "PUB",
    });

    const escrowPut = calls.find(
      (c) => c.url.endsWith("/api/key-escrow") && c.method === "PUT",
    );
    expect(escrowPut).toBeDefined();
    expect(escrowPut!.authorization).toBe(`Bearer ${EXT_JWT}`);
    // The aud=mcp JWT must never appear on /api/key-escrow*.
    for (const c of calls) {
      if (c.url.endsWith("/api/key-escrow")) {
        expect(c.authorization).not.toBe(`Bearer ${MCP_JWT}`);
      }
    }
  });

  it("delete: DELETE carries the ext JWT", async () => {
    const rt = createDefaultRuntime();
    await rt.keyEscrow.delete(MCP_JWT);
    const del = calls.find(
      (c) => c.url.endsWith("/api/key-escrow") && c.method === "DELETE",
    );
    expect(del).toBeDefined();
    expect(del!.authorization).toBe(`Bearer ${EXT_JWT}`);
  });

  it("auth.ensureExtensionJwt: caches the redeemed JWT", async () => {
    const rt = createDefaultRuntime();
    const j1 = await rt.auth.ensureExtensionJwt(MCP_JWT);
    expect(j1).toBe(EXT_JWT);
    expect(storageMap.get(EXTENSION_SESSION_STORAGE_KEY)).toMatchObject({
      jwt: EXT_JWT,
    });

    const callsBefore = calls.length;
    const j2 = await rt.auth.ensureExtensionJwt(MCP_JWT);
    expect(j2).toBe(EXT_JWT);
    // Cache hit — no new network traffic.
    expect(calls.length).toBe(callsBefore);
  });
});
