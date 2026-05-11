// T18 — CloudClient unit tests. D13 anchors that MUST stay binding:
//
//   1. `deferred_signing_flow` — drives `signRemote` end-to-end with
//      mocked fetch for `POST /mcp` (mnemonic_sign_memory) +
//      `POST /api/sign-callback`. Asserts:
//        - correlation_id round-trips between the two calls,
//        - the callback body carries `cose_signed_bytes_b64` +
//          `signer_pubkey_base58`,
//        - the returned `{attestation_id, solana_tx, arweave_tx}`
//          mirrors the server payload.
//
//   2. `offline_enqueues_for_later` — `fetch` rejects with a network
//      error. The client maps it to `TransientSyncError`; the
//      service-worker drain then leaves the row in `pending_uploads`
//      for the next alarm tick. This test asserts the typed-error
//      branch is hit (the queue-side persistence is exercised in
//      `cloud-sync.test.ts`).
//
// Additional coverage:
//   - 401 surfaces `ReauthRequiredError`.
//   - 4xx (not 401) surfaces `PermanentSyncError`.
//   - 5xx surfaces `TransientSyncError`.
//   - `recallRemote` returns parsed hits.
//   - `verifyRemote` discriminated union.

import { describe, it, expect, vi } from "vitest";
import {
  CloudClient,
  PermanentSyncError,
  ReauthRequiredError,
  TransientSyncError,
} from "../../../../src/runtime/sync/cloud-client.js";

const BASE = "https://mc.example.test";
const JWT = "header.payload.sig";
const SIGNER_PUBKEY = "Bsigner1111111111111111111111111111111111";
const COSE_BYTES = new Uint8Array([0x84, 0x01, 0x02, 0x03, 0xff]);

/** Build a minimal `fetch` mock whose response queue is driven by the
 *  caller. Each call shifts one response off `queue`. Throws if the
 *  queue runs dry — the test should always set the response count to
 *  match the expected number of fetch calls. */
function queuedFetch(
  queue: Array<Response | Promise<Response> | Error>,
): typeof fetch {
  const impl = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const next = queue.shift();
    if (next === undefined) {
      throw new Error(
        `fetch called more times than queued (url=${String(input)})`,
      );
    }
    if (next instanceof Error) throw next;
    return next instanceof Promise ? await next : next;
  });
  return impl as unknown as typeof fetch;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

describe("CloudClient · construction", () => {
  it("throws when no jwt is supplied", () => {
    expect(() => new CloudClient({ jwt: "" })).toThrow(/jwt is required/);
  });
});

describe("CloudClient · deferred_signing_flow (D13 anchor)", () => {
  it("posts the correlation_id from /mcp into /api/sign-callback and returns the tx ids", async () => {
    const fetchCalls: Array<{ url: string; init?: RequestInit }> = [];
    const correlationId = "corr-abc-123";
    const responses: Response[] = [
      // 1) /mcp mnemonic_sign_memory
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: { correlation_id: correlationId, expires_in: 300 },
      }),
      // 2) /api/sign-callback
      jsonResponse(200, {
        status: "signed",
        attestation_id: "att-server-uuid-1",
        content_hash: "deadbeef".repeat(8),
        solana_tx: "SoLaNa1111111111111111",
        arweave_tx: "ArWeAvE1111111111111111",
        solana_explorer_url: "https://solscan.io/tx/SoLaNa1111111111111111",
        arweave_url: "https://arweave.net/ArWeAvE1111111111111111",
      }),
    ];
    const fetchImpl = vi.fn(
      async (url: RequestInfo | URL, init?: RequestInit) => {
        fetchCalls.push({ url: String(url), ...(init ? { init } : {}) });
        const r = responses.shift();
        if (!r) throw new Error("unexpected fetch");
        return r;
      },
    );

    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl as unknown as typeof fetch,
    });

    const result = await client.signRemote(
      {
        content: "hello cloud",
        tags: ["t1", "t2"],
        source_meta: { platform: "chatgpt" },
      },
      { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
    );

    expect(result).toEqual({
      attestation_id: "att-server-uuid-1",
      solana_tx: "SoLaNa1111111111111111",
      arweave_tx: "ArWeAvE1111111111111111",
    });

    // 1st fetch — /mcp tools/call with Bearer JWT.
    expect(fetchCalls[0]?.url).toBe(`${BASE}/mcp`);
    const mcpHeaders = (fetchCalls[0]?.init?.headers ?? {}) as Record<
      string,
      string
    >;
    expect(mcpHeaders.Authorization).toBe(`Bearer ${JWT}`);
    const mcpBody = JSON.parse(String(fetchCalls[0]?.init?.body));
    expect(mcpBody).toMatchObject({
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "mnemonic_sign_memory",
        arguments: {
          content: "hello cloud",
          tags: ["t1", "t2"],
          source_meta: { platform: "chatgpt" },
        },
      },
    });

    // 2nd fetch — /api/sign-callback with correlation_id + b64 COSE +
    // signer_pubkey, NO Bearer JWT (capability auth per D2/D9).
    expect(fetchCalls[1]?.url).toBe(`${BASE}/api/sign-callback`);
    const cbHeaders = (fetchCalls[1]?.init?.headers ?? {}) as Record<
      string,
      string
    >;
    expect(cbHeaders.Authorization).toBeUndefined();
    const cbBody = JSON.parse(String(fetchCalls[1]?.init?.body));
    expect(cbBody.correlation_id).toBe(correlationId);
    expect(cbBody.signer_pubkey).toBe(SIGNER_PUBKEY);
    // Base64 decode the cose bytes and verify they round-trip.
    const decoded = Uint8Array.from(atob(cbBody.cose_signed_bytes), (c) =>
      c.charCodeAt(0),
    );
    expect(Array.from(decoded)).toEqual(Array.from(COSE_BYTES));
  });
});

describe("CloudClient · offline_enqueues_for_later (D13 anchor)", () => {
  it("maps a network rejection on /mcp to TransientSyncError so the drain leaves the row queued", async () => {
    const fetchImpl = queuedFetch([
      Object.assign(new Error("fetch failed"), { name: "TypeError" }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });

    let caught: unknown = null;
    try {
      await client.signRemote(
        { content: "queued offline", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      );
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(TransientSyncError);
    expect((caught as TransientSyncError).message).toMatch(/network error/);
  });

  it("maps a network rejection on /api/sign-callback to TransientSyncError too", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: { correlation_id: "c-1", expires_in: 300 },
      }),
      Object.assign(new Error("network down"), { name: "TypeError" }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });

    await expect(
      client.signRemote(
        { content: "x", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      ),
    ).rejects.toBeInstanceOf(TransientSyncError);
  });
});

describe("CloudClient · typed errors", () => {
  it("throws ReauthRequiredError on 401 from /mcp", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(401, { error: { code: 401, message: "expired" } }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    await expect(
      client.signRemote(
        { content: "x", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      ),
    ).rejects.toBeInstanceOf(ReauthRequiredError);
  });

  it("throws ReauthRequiredError on 401 from /api/sign-callback", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: { correlation_id: "c-1", expires_in: 300 },
      }),
      new Response("unauth", { status: 401 }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    await expect(
      client.signRemote(
        { content: "x", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      ),
    ).rejects.toBeInstanceOf(ReauthRequiredError);
  });

  it("throws PermanentSyncError on 400 from /mcp", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(400, { error: { code: 400, message: "malformed" } }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    let caught: unknown = null;
    try {
      await client.signRemote(
        { content: "x", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      );
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(PermanentSyncError);
    expect((caught as PermanentSyncError).status).toBe(400);
  });

  it("throws PermanentSyncError on 410 (sign-callback expired) from /api/sign-callback", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: { correlation_id: "c-1", expires_in: 300 },
      }),
      new Response("gone", { status: 410 }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    let caught: unknown = null;
    try {
      await client.signRemote(
        { content: "x", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      );
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(PermanentSyncError);
    expect((caught as PermanentSyncError).status).toBe(410);
  });

  it("throws TransientSyncError on 503", async () => {
    const fetchImpl = queuedFetch([
      new Response("upstream down", { status: 503 }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    let caught: unknown = null;
    try {
      await client.signRemote(
        { content: "x", tags: [] },
        { cose_bytes: COSE_BYTES, signer_pubkey: SIGNER_PUBKEY },
      );
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(TransientSyncError);
    expect((caught as TransientSyncError).status).toBe(503);
  });
});

describe("CloudClient · recallRemote", () => {
  it("returns parsed hits from mnemonic_recall", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: {
          hits: [
            {
              attestation_id: "a1",
              content: "first",
              similarity: 0.91,
              tags: ["t"],
              signed_at: "2026-05-11T10:00:00Z",
              solana_tx: "SoL1",
              arweave_tx: "Ar1",
            },
            {
              attestation_id: "a2",
              content: "second",
              similarity: 0.42,
            },
          ],
        },
      }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    const hits = await client.recallRemote("test", 5);
    expect(hits).toHaveLength(2);
    expect(hits[0]).toMatchObject({
      attestation_id: "a1",
      similarity: 0.91,
      solana_tx: "SoL1",
      arweave_tx: "Ar1",
    });
    expect(hits[1]).toMatchObject({
      attestation_id: "a2",
      similarity: 0.42,
    });
  });

  it("returns [] for empty query without hitting the network", async () => {
    const fetchImpl = vi.fn();
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl as unknown as typeof fetch,
    });
    expect(await client.recallRemote("", 5)).toEqual([]);
    expect(await client.recallRemote("  ", 5)).toEqual([]);
    expect(fetchImpl).not.toHaveBeenCalled();
  });
});

describe("CloudClient · verifyRemote", () => {
  it("returns a verified-shape result", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: {
          status: "verified",
          signer: "Bsigner",
          solana_tx: "SoL",
          arweave_tx: "Ar",
        },
      }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    const r = await client.verifyRemote("att-1");
    expect(r).toEqual({
      status: "verified",
      signer: "Bsigner",
      solana_tx: "SoL",
      arweave_tx: "Ar",
    });
  });

  it("returns tampered with reason", async () => {
    const fetchImpl = queuedFetch([
      jsonResponse(200, {
        jsonrpc: "2.0",
        id: 1,
        result: {
          status: "tampered",
          signer: "Bsigner",
          reason: "content_hash_mismatch",
        },
      }),
    ]);
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl,
    });
    const r = await client.verifyRemote("att-1");
    expect(r).toEqual({
      status: "tampered",
      signer: "Bsigner",
      reason: "content_hash_mismatch",
    });
  });

  it("returns not_found for empty id without network", async () => {
    const fetchImpl = vi.fn();
    const client = new CloudClient({
      jwt: JWT,
      baseUrl: BASE,
      fetch: fetchImpl as unknown as typeof fetch,
    });
    expect(await client.verifyRemote("")).toEqual({ status: "not_found" });
    expect(fetchImpl).not.toHaveBeenCalled();
  });
});
