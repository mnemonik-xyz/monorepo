// Unit tests for `MnemonicClient` — covers the 5 tool methods, the
// pending-bundle / sign-callback flow, and JWT redaction in error paths.
//
// We inject:
//   - a mocked `fetch` (per-test, captured into a `calls` array so we can
//     assert request URL / method / headers / body),
//   - a mocked WASM module via `__setWasmForTesting` (deterministic COSE
//     output via `buildWasmMock`).
//
// Everything is pure ESM + Web APIs.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MnemonicClient } from "../src/client.js";
import {
  AuthError,
  IntegrityError,
  ServerError,
  UserError,
} from "../src/errors.js";
import { Keypair } from "../src/keypair.js";
import { LocalSigner } from "../src/signer.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";

// ── fetch mock plumbing ────────────────────────────────────────────────────

interface CapturedCall {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

interface CannedResponse {
  status?: number;
  headers?: Record<string, string>;
  body: unknown; // string => raw body, Uint8Array => arrayBuffer, object => JSON
  matchUrl?: (u: string) => boolean;
}

function makeMockFetch(responses: CannedResponse[]): {
  fetchImpl: typeof fetch;
  calls: CapturedCall[];
} {
  const calls: CapturedCall[] = [];
  let cursor = 0;
  const fetchImpl: typeof fetch = (async (
    url: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const u = typeof url === "string" ? url : url.toString();
    const headers: Record<string, string> = {};
    if (init?.headers) {
      const h = init.headers;
      if (h instanceof Headers) {
        h.forEach((v, k) => (headers[k.toLowerCase()] = v));
      } else if (Array.isArray(h)) {
        for (const [k, v] of h) headers[k.toLowerCase()] = v;
      } else {
        for (const [k, v] of Object.entries(h)) {
          headers[k.toLowerCase()] = String(v);
        }
      }
    }
    let parsedBody: unknown = init?.body;
    if (typeof init?.body === "string") {
      try {
        parsedBody = JSON.parse(init.body);
      } catch {
        parsedBody = init.body;
      }
    }
    calls.push({
      url: u,
      method: (init?.method ?? "GET").toUpperCase(),
      headers,
      body: parsedBody,
    });

    // Pick the next matching response (in order, skipping ones with a
    // matchUrl that doesn't fit). This lets tests stage responses in the
    // exact request order the SDK makes.
    while (cursor < responses.length) {
      const cand = responses[cursor]!;
      cursor++;
      if (cand.matchUrl && !cand.matchUrl(u)) continue;
      return buildResponse(cand);
    }
    throw new Error(`mockFetch: no canned response left for ${u}`);
  }) as typeof fetch;
  return { fetchImpl, calls };
}

function buildResponse(c: CannedResponse): Response {
  const status = c.status ?? 200;
  const headers = new Headers(c.headers ?? {});
  let body: BodyInit;
  if (c.body instanceof Uint8Array) {
    body = c.body;
    if (!headers.has("content-type"))
      headers.set("content-type", "application/octet-stream");
  } else if (typeof c.body === "string") {
    body = c.body;
  } else {
    body = JSON.stringify(c.body);
    if (!headers.has("content-type"))
      headers.set("content-type", "application/json");
  }
  return new Response(body, { status, headers });
}

/** Build an MCP `tools/call` JSON-RPC envelope around an arbitrary result. */
function mcpResult(result: unknown): Record<string, unknown> {
  return {
    jsonrpc: "2.0",
    id: 1,
    result: {
      content: [{ type: "text", text: JSON.stringify(result) }],
    },
  };
}

// ── shared setup ───────────────────────────────────────────────────────────

beforeEach(() => {
  __setWasmForTesting(buildWasmMock() as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

async function makeClient(opts?: {
  jwt?: string;
  responses?: CannedResponse[];
}): Promise<{
  client: MnemonicClient;
  calls: CapturedCall[];
  keypair: Keypair;
}> {
  const keypair = await Keypair.generate();
  const signer = new LocalSigner(keypair);
  const { fetchImpl, calls } = makeMockFetch(opts?.responses ?? []);
  const client = new MnemonicClient({
    baseUrl: "https://example.test",
    signer,
    fetch: fetchImpl,
    ...(opts?.jwt !== undefined ? { jwt: opts.jwt } : {}),
  });
  client.setKeypair(keypair);
  return { client, calls, keypair };
}

// ── construction ───────────────────────────────────────────────────────────

describe("MnemonicClient constructor", () => {
  it("rejects missing or non-http baseUrl", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    expect(
      () =>
        new MnemonicClient({
          baseUrl: "ftp://nope",
          signer,
        })
    ).toThrow(/baseUrl/);
  });

  it("rejects missing signer", () => {
    expect(
      () =>
        // @ts-expect-error — intentional bad input
        new MnemonicClient({ baseUrl: "https://example.test" })
    ).toThrow(/signer/);
  });

  it("strips trailing slashes from baseUrl", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    const { fetchImpl, calls } = makeMockFetch([
      { body: mcpResult({ server_pubkey: "abc" }) },
    ]);
    const client = new MnemonicClient({
      baseUrl: "https://example.test///",
      signer,
      fetch: fetchImpl,
    });
    await client.whoami();
    expect(calls[0]!.url).toBe("https://example.test/mcp");
  });
});

// ── whoami ─────────────────────────────────────────────────────────────────

describe("whoami", () => {
  it("returns the server identity from the canned shape", async () => {
    const { client, calls } = await makeClient({
      jwt: "eyJabc123def456ghi789jkl012mno345pqr.payload.sig",
      responses: [
        {
          body: mcpResult({
            server_pubkey: "ServerPub1",
            server_did: "did:key:server",
            extra: "field",
          }),
        },
      ],
    });
    const out = await client.whoami();
    expect(out.serverPubkey).toBe("ServerPub1");
    expect(out.serverDid).toBe("did:key:server");
    expect(out.raw.extra).toBe("field");
    // Authorization header carried the JWT.
    expect(calls[0]!.headers.authorization).toMatch(/^Bearer eyJ/);
  });

  it("does not crash when fields are missing", async () => {
    const { client } = await makeClient({
      responses: [{ body: mcpResult({}) }],
    });
    const out = await client.whoami();
    expect(out.serverPubkey).toBeUndefined();
    expect(out.raw).toEqual({});
  });
});

// ── signMemory: pending-bundle flow ───────────────────────────────────────

describe("signMemory pending-bundle flow", () => {
  it("decodes pending bytes, signs with COSE, posts callback without JWT", async () => {
    // Stage:
    //   1. POST /mcp tools/call mnemonic_sign_memory → correlation_id
    //   2. GET  /api/pending/<id>                    → CBOR bytes
    //   3. POST /api/sign-callback                   → attestation_id
    const correlationId = "corr-abc-123";
    const cborBytes = new Uint8Array([0xa1, 0x64, 0x74, 0x65, 0x73, 0x74]);
    const { client, calls, keypair } = await makeClient({
      jwt: "eyJsigners.payload.bearer123456789012345",
      responses: [
        { body: mcpResult({ correlation_id: correlationId }) },
        { body: cborBytes, headers: { "content-type": "application/cbor" } },
        {
          body: {
            attestation_id: "att-1",
            signed_at: "2026-04-29T00:00:00Z",
            status: "signed",
            content_hash: "cafebabe",
          },
        },
      ],
    });

    const result = await client.signMemory("hello world", { tags: ["a"] });

    // Result projection.
    expect(result.attestationId).toBe("att-1");
    expect(result.status).toBe("signed");
    expect(result.contentHash).toBe("cafebabe");

    // Three HTTP calls in the right order.
    expect(calls.length).toBe(3);

    const [openCall, pendingCall, callbackCall] = calls;

    // 1. tools/call carries Authorization.
    expect(openCall!.url).toBe("https://example.test/mcp");
    expect(openCall!.method).toBe("POST");
    expect(openCall!.headers.authorization).toMatch(/^Bearer eyJ/);
    const openBody = openCall!.body as { params: { arguments: unknown } };
    expect(openBody.params.arguments).toEqual({
      content: "hello world",
      tags: ["a"],
    });

    // 2. pending fetch is GET, encodes correlation_id, no Authorization needed.
    expect(pendingCall!.method).toBe("GET");
    expect(pendingCall!.url).toBe(
      `https://example.test/api/pending/${correlationId}`
    );

    // 3. sign-callback — POST with NO Authorization header.
    expect(callbackCall!.method).toBe("POST");
    expect(callbackCall!.url).toBe("https://example.test/api/sign-callback");
    expect(callbackCall!.headers.authorization).toBeUndefined();
    const cb = callbackCall!.body as Record<string, unknown>;
    expect(cb.correlation_id).toBe(correlationId);
    expect(cb.signer_pubkey).toBe(keypair.pubkey);
    expect(typeof cb.cose_signed_bytes).toBe("string");
    // base64 of a non-empty COSE envelope.
    expect((cb.cose_signed_bytes as string).length).toBeGreaterThan(0);
  });

  it("throws UserError on empty content", async () => {
    const { client } = await makeClient();
    await expect(client.signMemory("")).rejects.toThrow(/non-empty/);
  });

  it("throws ServerError if open response lacks correlation_id", async () => {
    const { client } = await makeClient({
      responses: [{ body: mcpResult({}) }],
    });
    await expect(client.signMemory("x")).rejects.toBeInstanceOf(ServerError);
  });

  it("throws ServerError on 410 from /api/pending", async () => {
    const { client } = await makeClient({
      responses: [
        { body: mcpResult({ correlation_id: "c1" }) },
        { status: 410, body: "gone" },
      ],
    });
    await expect(client.signMemory("x")).rejects.toBeInstanceOf(ServerError);
  });

  it("throws AuthError on 401 from /api/sign-callback", async () => {
    const { client } = await makeClient({
      responses: [
        { body: mcpResult({ correlation_id: "c1" }) },
        { body: new Uint8Array([0xa0]) },
        { status: 401, body: { error: "rejected" } },
      ],
    });
    await expect(client.signMemory("x")).rejects.toBeInstanceOf(AuthError);
  });
});

// ── recall ─────────────────────────────────────────────────────────────────

describe("recall", () => {
  it("normalizes hits + total from server shape", async () => {
    const { client } = await makeClient({
      responses: [
        {
          body: mcpResult({
            hits: [
              {
                attestation_id: "att-1",
                content: "first",
                similarity: 0.91,
                signed_at: "2026-04-29T00:00:00Z",
                tags: ["x"],
              },
              {
                attestation_id: "att-2",
                content: "second",
                similarity: 0.42,
              },
            ],
            total: 17,
          }),
        },
      ],
    });
    const out = await client.recall("query", { topK: 5, tags: ["x"] });
    expect(out.total).toBe(17);
    expect(out.hits.length).toBe(2);
    expect(out.hits[0]!.attestationId).toBe("att-1");
    expect(out.hits[0]!.tags).toEqual(["x"]);
  });
});

// ── verify ─────────────────────────────────────────────────────────────────

describe("verify", () => {
  it("returns verified discriminant", async () => {
    const { client } = await makeClient({
      responses: [
        {
          body: mcpResult({
            status: "verified",
            signer: "Pubkey9",
            arweave_tx: "ar-1",
            solana_tx: "sol-1",
          }),
        },
      ],
    });
    const out = await client.verify("att-1");
    expect(out.status).toBe("verified");
    if (out.status === "verified") {
      expect(out.signer).toBe("Pubkey9");
      expect(out.arweaveTx).toBe("ar-1");
    }
  });

  it("returns not_found discriminant", async () => {
    const { client } = await makeClient({
      responses: [{ body: mcpResult({ status: "not_found" }) }],
    });
    const out = await client.verify("att-missing");
    expect(out.status).toBe("not_found");
  });

  it("rejects empty attestationId", async () => {
    const { client } = await makeClient();
    await expect(client.verify("")).rejects.toThrow(/attestationId/);
  });
});

// ── proveIdentity ──────────────────────────────────────────────────────────

describe("proveIdentity", () => {
  it("returns server-provided pubkey/signature", async () => {
    const { client } = await makeClient({
      responses: [
        {
          body: mcpResult({
            pubkey: "ServerPub",
            challenge: "ch1",
            signature: "sig1",
            did: "did:key:s",
          }),
        },
      ],
    });
    const out = await client.proveIdentity("ch1");
    expect(out.pubkey).toBe("ServerPub");
    expect(out.signature).toBe("sig1");
    expect(out.did).toBe("did:key:s");
  });

  it("rejects empty challenge", async () => {
    const { client } = await makeClient();
    await expect(client.proveIdentity("")).rejects.toThrow(/challenge/);
  });
});

// ── JWT redaction in error messages ────────────────────────────────────────

describe("JWT redaction in error messages", () => {
  it("redacts JWTs from 401 server error bodies", async () => {
    // The server echoes back the user's JWT in the error body — common with
    // verbose token-validation errors. Our error must NOT leak it.
    const leakyJwt =
      "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    const { client } = await makeClient({
      jwt: leakyJwt,
      responses: [
        {
          status: 401,
          body: { error: `bad token: ${leakyJwt}` },
        },
      ],
    });
    let caught: unknown = null;
    try {
      await client.whoami();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    const msg = (caught as Error).message;
    expect(msg).not.toContain(leakyJwt);
    expect(msg).not.toMatch(/eyJ[A-Za-z0-9_-]{20,}/);
    expect(msg).toContain("[REDACTED-JWT]");
  });

  it("redacts JWT in malformed-signMemory open response", async () => {
    const leakyJwt =
      "eyJleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleakyleaky.payload.sig";
    const { client } = await makeClient({
      responses: [
        {
          body: mcpResult({
            // No correlation_id — triggers the `did not return correlation_id`
            // ServerError, which embeds the body. Body leaks a JWT here.
            note: `auth was ${leakyJwt}`,
          }),
        },
      ],
    });
    let caught: unknown = null;
    try {
      await client.signMemory("x");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ServerError);
    const msg = (caught as Error).message;
    expect(msg).not.toContain(leakyJwt);
    expect(msg).toContain("[REDACTED-JWT]");
  });
});

// ── 5xx, malformed JSON, network failure (callTool error branches) ─────────

describe("callTool error branches", () => {
  it("throws ServerError on 500 from /mcp with redacted body", async () => {
    const leakyJwt =
      "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    const { client } = await makeClient({
      jwt: leakyJwt,
      responses: [
        {
          status: 500,
          body: { error: `internal: ${leakyJwt}` },
        },
      ],
    });
    let caught: unknown = null;
    try {
      await client.whoami();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ServerError);
    expect((caught as ServerError).status).toBe(500);
    const msg = (caught as Error).message;
    expect(msg).toContain("HTTP 500");
    // Body content is surfaced and redacted.
    expect(msg).not.toContain(leakyJwt);
    expect(msg).toContain("[REDACTED-JWT]");
  });

  it("throws ServerError with malformed-JSON message on non-JSON 200", async () => {
    const { client } = await makeClient({
      responses: [{ status: 200, body: "<<<not json>>>" }],
    });
    let caught: unknown = null;
    try {
      await client.whoami();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ServerError);
    expect((caught as Error).message).toMatch(/malformed JSON/);
  });

  it("throws ServerError when JSON-RPC body is not an object", async () => {
    // res.json() returns an array → isRecord(parsed) is false.
    const { client } = await makeClient({
      responses: [{ status: 200, body: ["unexpected"] }],
    });
    await expect(client.whoami()).rejects.toBeInstanceOf(ServerError);
  });

  it("maps JSON-RPC error.code 401 to AuthError", async () => {
    const { client } = await makeClient({
      responses: [
        {
          status: 200,
          body: {
            jsonrpc: "2.0",
            id: 1,
            error: { code: 401, message: "token expired" },
          },
        },
      ],
    });
    await expect(client.whoami()).rejects.toBeInstanceOf(AuthError);
  });

  it("propagates JSON-RPC error.message via ServerError when no auth code", async () => {
    const { client } = await makeClient({
      responses: [
        {
          status: 200,
          body: {
            jsonrpc: "2.0",
            id: 1,
            error: { code: -32603, message: "boom" },
          },
        },
      ],
    });
    await expect(client.whoami()).rejects.toMatchObject({
      name: "ServerError",
      message: expect.stringContaining("boom"),
    });
  });

  it("surfaces network failure (fetch throws) as ServerError", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    const fetchImpl: typeof fetch = (async () => {
      throw new TypeError("fetch failed");
    }) as typeof fetch;
    const client = new MnemonicClient({
      baseUrl: "https://example.test",
      signer,
      fetch: fetchImpl,
    });
    let caught: unknown = null;
    try {
      await client.whoami();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ServerError);
    expect((caught as Error).message).toMatch(/network error/);
  });
});

// ── signMemory edge cases ──────────────────────────────────────────────────

describe("signMemory missing keypair", () => {
  it("throws UserError when setKeypair has not been called", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    const { fetchImpl } = makeMockFetch([]);
    const client = new MnemonicClient({
      baseUrl: "https://example.test",
      signer,
      fetch: fetchImpl,
    });
    // NOTE: no client.setKeypair(...) call.
    await expect(client.signMemory("hello")).rejects.toBeInstanceOf(UserError);
    await expect(client.signMemory("hello")).rejects.toThrow(/no keypair/);
  });

  it("throws IntegrityError when sign-callback omits attestation_id", async () => {
    const { client } = await makeClient({
      responses: [
        { body: mcpResult({ correlation_id: "c1" }) },
        { body: new Uint8Array([0xa0]) },
        { status: 200, body: { signed_at: "2026-04-29T00:00:00Z" } },
      ],
    });
    await expect(client.signMemory("x")).rejects.toBeInstanceOf(IntegrityError);
  });
});

// ── verify tampered + recall alternate shape + setJwt ──────────────────────

describe("verify tampered discriminant", () => {
  it("propagates tampered status with reason and signer", async () => {
    const { client } = await makeClient({
      responses: [
        {
          body: mcpResult({
            status: "tampered",
            signer: "PubX",
            reason: "hash_mismatch",
          }),
        },
      ],
    });
    const out = await client.verify("att-bad");
    expect(out.status).toBe("tampered");
    if (out.status === "tampered") {
      expect(out.signer).toBe("PubX");
      expect(out.reason).toBe("hash_mismatch");
    }
  });
});

describe("recall alternate response shapes", () => {
  it("accepts results-key alternate (results[] vs hits[])", async () => {
    const { client } = await makeClient({
      responses: [
        {
          body: mcpResult({
            results: [
              { attestation_id: "att-r", content: "alt", similarity: 0.5 },
            ],
          }),
        },
      ],
    });
    const out = await client.recall("query");
    expect(out.hits.length).toBe(1);
    expect(out.hits[0]!.attestationId).toBe("att-r");
    // total falls back to hits.length when missing on the wire.
    expect(out.total).toBe(1);
  });

  it("returns empty hits when neither hits nor results is present", async () => {
    const { client } = await makeClient({
      responses: [{ body: mcpResult({}) }],
    });
    const out = await client.recall("query");
    expect(out.hits).toEqual([]);
    expect(out.total).toBe(0);
  });
});

describe("setJwt setter", () => {
  it("attaches Bearer header to subsequent calls when setJwt() is called", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    const { fetchImpl, calls } = makeMockFetch([
      { body: mcpResult({ server_pubkey: "Pub1" }) },
    ]);
    const client = new MnemonicClient({
      baseUrl: "https://example.test",
      signer,
      fetch: fetchImpl,
    });
    // No JWT at construction.
    client.setJwt("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig");
    await client.whoami();
    expect(calls[0]!.headers.authorization).toMatch(/^Bearer eyJ/);
  });
});

// ── readBodySafely redact-then-slice (security-auditor finding #2) ─────────

describe("readBodySafely redact-then-slice", () => {
  it("redacts a JWT that would otherwise straddle the 500-char slice", async () => {
    // Build a body where a real JWT lands at offset 480 (so a slice-first
    // implementation would cut the JWT in half, leaving the head < 20 chars
    // after `eyJ` and skipping the regex). After this fix, redact runs first.
    const filler = "x".repeat(480);
    const leakyJwt =
      "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    const body = `${filler}${leakyJwt}`;
    const { client } = await makeClient({
      responses: [{ status: 500, body: { error: body } }],
    });
    let caught: unknown = null;
    try {
      await client.whoami();
    } catch (e) {
      caught = e;
    }
    const msg = (caught as Error).message;
    // The full JWT must NOT appear in the surfaced message.
    expect(msg).not.toContain(leakyJwt);
    // And no JWT-shape substring should leak past the slice/truncation.
    expect(msg).not.toMatch(/eyJ[A-Za-z0-9_-]{20,}/);
  });
});
