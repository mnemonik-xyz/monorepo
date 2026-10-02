// Unit tests for sealed-memory methods (Task 7).
//
// All crypto goes through the WASM mock (`buildWasmMock` + `buildSealedWasmMock`).
// Network calls go through the fetch mock established in client.test.ts.
//
// Coverage:
//   1. signMemory throws IntegrityError when SEALED_V1 bundle content mismatches input
//   2. signMemory passes when SEALED_V1 bundle content matches input
//   3. sealMemory + openMemory round-trip (mock WASM)
//   4. share("link") returns URL with #k= fragment
//   5. importLink parses #k= fragment and opens (mock WASM)
//   6. listGrants returns normalized grant entries
//   7. recallSealed ranks locally by cosine similarity
//   8. recallSealed uses pluggable Embedder

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MnemonicClient } from "../src/client.js";
import { IntegrityError, ServerError, UserError } from "../src/errors.js";
import { Keypair } from "../src/keypair.js";
import { LocalSigner } from "../src/signer.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";

// ── shared types ────────────────────────────────────────────────────────────

interface CannedResponse {
  status?: number;
  headers?: Record<string, string>;
  body: unknown;
  matchUrl?: (u: string) => boolean;
}

interface CapturedCall {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
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
    calls.push({ url: u, method: (init?.method ?? "GET").toUpperCase(), headers, body: parsedBody });

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
    if (!headers.has("content-type")) headers.set("content-type", "application/octet-stream");
  } else if (typeof c.body === "string") {
    body = c.body;
  } else {
    body = JSON.stringify(c.body);
    if (!headers.has("content-type")) headers.set("content-type", "application/json");
  }
  return new Response(body, { status, headers });
}

// ── WASM mock with sealed extensions ────────────────────────────────────────
//
// The sealed WASM functions (seal_memory, open_memory, open_with_key,
// parse_link_fragment, link_fragment) work on fake data for SDK-level tests.
// We don't need to reproduce the actual XChaCha20Poly1305 encryption here —
// the WASM golden tests in core/ cover that. We just need the mock to behave
// consistently so the SDK wiring is exercised.

function buildSealedWasmMock() {
  const base = buildWasmMock();

  // Store: memory_hash → { outer_cbor, k, content }
  const store = new Map<string, { outer_cbor: Uint8Array; k: Uint8Array; content: string }>();

  // seal_memory: encrypt content into a fake outer_cbor that encodes the content.
  // We embed the content in the outer_cbor so open_memory can recover it.
  // Fake format: first 8 bytes = "SEALED_V" (sentinel), next 4 bytes = content length, rest = content.
  const seal_memory = (
    inner_json: Uint8Array,
    _author_ed25519_pub: Uint8Array,
    artifact_id: string,
    _producer: string,
    _created_at: string
  ): { outer_cbor: Uint8Array; content_hash: Uint8Array } => {
    // Generate a fake K (just a counter-based key for determinism in tests).
    const k = new Uint8Array(32);
    k.fill(0xaa);

    // Build outer_cbor: sentinel + inner_json.
    const sentinel = new TextEncoder().encode("SEALED_V");
    const lenBuf = new Uint8Array(4);
    const view = new DataView(lenBuf.buffer);
    view.setUint32(0, inner_json.length, false);

    const outer_cbor = new Uint8Array(sentinel.length + lenBuf.length + inner_json.length + k.length);
    outer_cbor.set(sentinel, 0);
    outer_cbor.set(lenBuf, sentinel.length);
    outer_cbor.set(inner_json, sentinel.length + lenBuf.length);
    outer_cbor.set(k, sentinel.length + lenBuf.length + inner_json.length);

    // Compute fake content_hash as XOR of all bytes mod 256, repeated 32 times.
    const content_hash = new Uint8Array(32);
    for (let i = 0; i < outer_cbor.length; i++) {
      content_hash[i % 32] ^= outer_cbor[i]!;
    }

    // Parse inner_json to extract content for the store.
    let content = "";
    try {
      const obj = JSON.parse(new TextDecoder().decode(inner_json)) as Record<string, unknown>;
      if (typeof obj.content === "string") content = obj.content;
    } catch {
      content = new TextDecoder().decode(inner_json);
    }

    const hashHex = Array.from(content_hash).map(b => b.toString(16).padStart(2, "0")).join("");
    store.set(hashHex, { outer_cbor, k, content });
    store.set(artifact_id, { outer_cbor, k, content });

    return { outer_cbor, content_hash };
  };

  // open_memory: recover inner_json from fake outer_cbor.
  // Searches for "SEALED_V" sentinel (it may not be at offset 0 if the bundle
  // has a detection prefix like "sealed" prepended for tests).
  const open_memory = (outer_cbor: Uint8Array, _x25519_secret: Uint8Array): Uint8Array => {
    const sentinel = new TextEncoder().encode("SEALED_V");
    // Find sentinel position.
    let sentinelPos = -1;
    for (let i = 0; i <= outer_cbor.length - sentinel.length; i++) {
      let match = true;
      for (let j = 0; j < sentinel.length; j++) {
        if (outer_cbor[i + j] !== sentinel[j]) { match = false; break; }
      }
      if (match) { sentinelPos = i; break; }
    }
    if (sentinelPos === -1) {
      throw new Error("open_memory: not a SEALED_V1 bundle (no SEALED_V sentinel)");
    }
    const dataOffset = sentinelPos + sentinel.length;
    const view = new DataView(outer_cbor.buffer, outer_cbor.byteOffset + dataOffset, 4);
    const len = view.getUint32(0, false);
    return outer_cbor.slice(dataOffset + 4, dataOffset + 4 + len);
  };

  // open_with_key: same as open_memory for this mock (we don't check K).
  const open_with_key = (outer_cbor: Uint8Array, _k: Uint8Array): Uint8Array => {
    return open_memory(outer_cbor, new Uint8Array(32));
  };

  // link_fragment: encode K as k=<base64url>.
  const link_fragment_fn = (k: Uint8Array): string => {
    const b64 = btoa(String.fromCharCode(...k))
      .replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    return `k=${b64}`;
  };

  // parse_link_fragment: decode fragment back to K.
  const parse_link_fragment = (fragment: string): Uint8Array => {
    if (!fragment.startsWith("k=")) throw new Error("invalid fragment: no k= prefix");
    const b64 = fragment.slice(2).replace(/-/g, "+").replace(/_/g, "/");
    const bin = atob(b64);
    return new Uint8Array(bin.split("").map(c => c.charCodeAt(0)));
  };

  return {
    ...base,
    seal_memory,
    open_memory,
    open_with_key,
    link_fragment: link_fragment_fn,
    parse_link_fragment,
    _store: store,
  };
}

// ── shared setup ────────────────────────────────────────────────────────────

let sealedMock: ReturnType<typeof buildSealedWasmMock>;

beforeEach(() => {
  sealedMock = buildSealedWasmMock();
  __setWasmForTesting(sealedMock as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

async function makeClient(opts?: {
  jwt?: string;
  responses?: CannedResponse[];
}): Promise<{ client: MnemonicClient; calls: CapturedCall[]; keypair: Keypair }> {
  const keypair = await Keypair.generate();
  const signer = new LocalSigner(keypair);
  const { fetchImpl, calls } = makeMockFetch(opts?.responses ?? []);
  const client = new MnemonicClient({
    baseUrl: "https://example.test",
    signer,
    fetch: fetchImpl,
    jwt: opts?.jwt ?? "eyJhbGciOiJIUzI1NiJ9.e30.sig",
  });
  client.setKeypair(keypair);
  return { client, calls, keypair };
}

// ── signMemory SEALED_V1 integrity guard ─────────────────────────────────────

describe("signMemory SEALED_V1 integrity guard", () => {
  function makeSealedBundle(content: string): Uint8Array {
    // Build the fake sealed bundle using our mock seal_memory logic.
    // We prepend the ASCII sequence for the string "sealed" (as it would appear
    // as a CBOR text value in a real SEALED_V1 artifact) so that
    // verifySealedBundleIfNeeded detects it as a sealed bundle.
    // Then add the SEALED_V mock format for open_memory to parse.
    const innerJson = JSON.stringify({ type: "memory", content });
    const innerBytes = new TextEncoder().encode(innerJson);
    // "sealed" prefix so verifySealedBundleIfNeeded detects this as a sealed bundle.
    const sealedMarker = new TextEncoder().encode("sealed");
    const sentinel = new TextEncoder().encode("SEALED_V");
    const lenBuf = new Uint8Array(4);
    new DataView(lenBuf.buffer).setUint32(0, innerBytes.length, false);
    // Layout: [sealedMarker][sentinel][lenBuf][innerBytes][K(32)]
    const outerCbor = new Uint8Array(
      sealedMarker.length + sentinel.length + lenBuf.length + innerBytes.length + 32
    );
    outerCbor.set(sealedMarker, 0);
    outerCbor.set(sentinel, sealedMarker.length);
    outerCbor.set(lenBuf, sealedMarker.length + sentinel.length);
    outerCbor.set(innerBytes, sealedMarker.length + sentinel.length + lenBuf.length);
    // Fill the K slot with 0xaa.
    outerCbor.fill(0xaa, sealedMarker.length + sentinel.length + lenBuf.length + innerBytes.length);
    return outerCbor;
  }

  it("throws IntegrityError when SEALED_V1 decrypted content mismatches input", async () => {
    const signedContent = "original content";
    const bundleContent = "different content"; // mismatch!
    const sealedBundle = makeSealedBundle(bundleContent);

    const { client } = await makeClient({
      responses: [
        // 1. sign_memory → correlation_id
        {
          body: {
            jsonrpc: "2.0",
            id: 1,
            result: { content: [{ type: "text", text: JSON.stringify({ correlation_id: "corr-sealed-1" }) }] },
          },
        },
        // 2. GET /api/pending → SEALED_V1 bundle with DIFFERENT content
        {
          body: sealedBundle,
          headers: { "content-type": "application/cbor" },
        },
      ],
    });

    await expect(
      client.signMemory(signedContent)
    ).rejects.toBeInstanceOf(IntegrityError);
  });

  it("proceeds normally when SEALED_V1 decrypted content matches input", async () => {
    const content = "matching content";
    const sealedBundle = makeSealedBundle(content);

    const { client } = await makeClient({
      responses: [
        {
          body: {
            jsonrpc: "2.0",
            id: 1,
            result: { content: [{ type: "text", text: JSON.stringify({ correlation_id: "corr-sealed-2" }) }] },
          },
        },
        { body: sealedBundle, headers: { "content-type": "application/cbor" } },
        {
          body: {
            attestation_id: "att-sealed-1",
            signed_at: "2026-09-28T00:00:00Z",
            status: "signed",
          },
        },
      ],
    });

    const result = await client.signMemory(content);
    expect(result.attestationId).toBe("att-sealed-1");
    expect(result.status).toBe("signed");
  });

  it("does not fire on a normal (non-sealed) CBOR bundle", async () => {
    // A normal memory bundle does not contain the "SEALED_V" sentinel.
    const normalCbor = new Uint8Array([0xa1, 0x64, 0x74, 0x65, 0x73, 0x74]);

    const { client } = await makeClient({
      responses: [
        {
          body: {
            jsonrpc: "2.0",
            id: 1,
            result: { content: [{ type: "text", text: JSON.stringify({ correlation_id: "corr-normal" }) }] },
          },
        },
        { body: normalCbor, headers: { "content-type": "application/cbor" } },
        { body: { attestation_id: "att-normal", signed_at: "2026-09-28T00:00:00Z", status: "signed" } },
      ],
    });

    // Should not throw IntegrityError — normal CBOR passes through.
    const result = await client.signMemory("any content");
    expect(result.attestationId).toBe("att-normal");
  });
});

// ── sealMemory ──────────────────────────────────────────────────────────────

describe("sealMemory", () => {
  it("stores signed bytes locally without a hosted request", async () => {
    const {client,calls} = await makeClient();
    const result=await client.sealMemory("hello sealed",{mode:"store",tags:["private"]});
    expect(result.memoryHash).toMatch(/^[a-f0-9]{64}$/);
    expect(result.signedBytes?.[0]).toBe(0x84);
    expect(calls).toHaveLength(0);
    expect((await client.openMemory(result.memoryHash)).content).toBe("hello sealed");
  });

  it("rejects an unverified receipt and retains original bytes for retry", async () => {
    const {client,calls}=await makeClient({responses:[{body:{memory_hash:"deadbeef"}}]});
    await expect(client.sealMemory("anchor me",{mode:"anchor"})).rejects.toBeInstanceOf(IntegrityError);
    expect(calls[0]!.url).toContain("/api/ingest-artifact");
    expect(client.preparedSealedMemories()).toHaveLength(1);
  });

  it("falls back to local content_hash when server omits memory_hash", async () => {
    const { client } = await makeClient({
      responses: [{ body: {} }],
    });
    const result = await client.sealMemory("no hash from server", { mode: "store" });
    // Should still return a non-empty hex hash (derived locally).
    expect(result.memoryHash).toMatch(/^[0-9a-f]+$/);
    expect(result.memoryHash.length).toBeGreaterThan(0);
  });

  it("preserves structured payment challenges and operation identity",async()=>{
    const {client,calls}=await makeClient({responses:[{status:428,body:{operation_id:"op-1",status:"awaiting_wallet_link",retry_action:"resubmit_same_operation_and_original_bytes"}}]});
    try {await client.ingestPreparedMemory(new Uint8Array([0x84]),{operationId:"op-1"});throw new Error("expected challenge");}
    catch(e) {expect(e).toBeInstanceOf(ServerError);expect((e as ServerError).status).toBe(428);expect((e as ServerError).cause).toMatchObject({response:{operation_id:"op-1"},operationId:"op-1"});}
    expect(calls).toHaveLength(1);
  });

  it("throws UserError on empty content", async () => {
    const { client } = await makeClient();
    await expect(client.sealMemory("", { mode: "store" })).rejects.toBeInstanceOf(UserError);
  });

  it("throws AuthError on 401", async () => {
    const { client } = await makeClient({
      responses: [{ status: 401, body: { error: "unauthorized" } }],
    });
    await expect(client.sealMemory("secret", { mode: "anchor" })).rejects.toBeInstanceOf(
      (await import("../src/errors.js")).AuthError
    );
  });
});

// ── openMemory ──────────────────────────────────────────────────────────────

describe("openMemory", () => {
  it("opens restored bytes with identity key", async () => {
    // Build the fake sealed blob encoding "secret note".
    const innerJson = JSON.stringify({ type: "memory", content: "secret note" });
    const innerBytes = new TextEncoder().encode(innerJson);
    const sentinel = new TextEncoder().encode("SEALED_V");
    const lenBuf = new Uint8Array(4);
    new DataView(lenBuf.buffer).setUint32(0, innerBytes.length, false);
    const outerCbor = new Uint8Array(sentinel.length + lenBuf.length + innerBytes.length + 32);
    outerCbor.set(sentinel);
    outerCbor.set(lenBuf, sentinel.length);
    outerCbor.set(innerBytes, sentinel.length + lenBuf.length);

    const { client } = await makeClient({
      responses: [
        { body: outerCbor, headers: { "content-type": "application/cbor" } },
      ],
    });

    const result = await client.openMemory(outerCbor);
    expect(result.content).toBe("secret note");
    expect(result.innerJson instanceof Uint8Array).toBe(true);
  });

  it("accepts raw Uint8Array bytes directly (no fetch)", async () => {
    const innerJson = JSON.stringify({ type: "memory", content: "direct bytes" });
    const innerBytes = new TextEncoder().encode(innerJson);
    const sentinel = new TextEncoder().encode("SEALED_V");
    const lenBuf = new Uint8Array(4);
    new DataView(lenBuf.buffer).setUint32(0, innerBytes.length, false);
    const outerCbor = new Uint8Array(sentinel.length + lenBuf.length + innerBytes.length + 32);
    outerCbor.set(sentinel);
    outerCbor.set(lenBuf, sentinel.length);
    outerCbor.set(innerBytes, sentinel.length + lenBuf.length);

    const { client } = await makeClient({ responses: [] });
    const result = await client.openMemory(outerCbor);
    expect(result.content).toBe("direct bytes");
  });

  it("throws IntegrityError when decryption fails", async () => {
    // Pass garbage bytes that fail our mock's sentinel check.
    const garbage = new Uint8Array([0xff, 0x00, 0x01, 0x02]);
    const { client } = await makeClient({
      responses: [
        { body: garbage, headers: { "content-type": "application/cbor" } },
      ],
    });
    await expect(client.openMemory(garbage)).rejects.toBeInstanceOf(IntegrityError);
  });
});

// ── sealMemory + openMemory round-trip ───────────────────────────────────────

describe("sealMemory + openMemory round-trip (mock WASM)", () => {
  it("seals then opens recovering original content", async () => {
    const content = "round-trip content";
    let storedCbor: Uint8Array | null = null;

    const { client } = await makeClient({
      responses: [
        {
          // sealMemory → store-sealed
          body: { memory_hash: "cafebabe" },
        },
        {
          // openMemory → fetch by hash: return the stored outer_cbor.
          // We need to capture it from the sealMemory call — but we can
          // also just build a consistent mock.
          body: new Uint8Array(0), // overridden below
        },
      ],
    });

    // Build a consistent mock fetch that captures the sealMemory call's outer_cbor
    // and serves it back for openMemory.
    const innerJson = JSON.stringify({ type: "memory", content });
    const innerBytes = new TextEncoder().encode(innerJson);
    const sentinel = new TextEncoder().encode("SEALED_V");
    const lenBuf = new Uint8Array(4);
    new DataView(lenBuf.buffer).setUint32(0, innerBytes.length, false);
    storedCbor = new Uint8Array(sentinel.length + lenBuf.length + innerBytes.length + 32);
    storedCbor.set(sentinel);
    storedCbor.set(lenBuf, sentinel.length);
    storedCbor.set(innerBytes, sentinel.length + lenBuf.length);

    const { fetchImpl: roundTripFetch } = makeMockFetch([
      { body: { memory_hash: "cafebabe" } },
      { body: storedCbor, headers: { "content-type": "application/cbor" } },
    ]);

    const keypair = await Keypair.generate();
    const roundTripClient = new MnemonicClient({
      baseUrl: "https://example.test",
      signer: new LocalSigner(keypair),
      fetch: roundTripFetch,
      jwt: "eyJ.e30.sig",
    });
    roundTripClient.setKeypair(keypair);

    const { memoryHash } = await roundTripClient.sealMemory(content, { mode: "store" });
    expect(memoryHash).toMatch(/^[a-f0-9]{64}$/);

    const opened = await roundTripClient.openMemory(memoryHash);
    expect(opened.content).toBe(content);
  });
});

// ── share("link") ────────────────────────────────────────────────────────────

describe("share migration", () => {
  it("rejects retired hosted grant creation before any request", async () => {
    const {client,calls}=await makeClient();
    await expect(client.share("deadbeef","link")).rejects.toThrow(/retired/);
    expect(calls).toHaveLength(0);
  });
});

describe("importLink", () => {
  it("parses #k= fragment and opens the memory", async () => {
    const content = "imported secret";
    const innerJson = JSON.stringify({ type: "memory", content });
    const innerBytes = new TextEncoder().encode(innerJson);
    const sentinel = new TextEncoder().encode("SEALED_V");
    const lenBuf = new Uint8Array(4);
    new DataView(lenBuf.buffer).setUint32(0, innerBytes.length, false);
    const outerCbor = new Uint8Array(sentinel.length + lenBuf.length + innerBytes.length + 32);
    outerCbor.set(sentinel);
    outerCbor.set(lenBuf, sentinel.length);
    outerCbor.set(innerBytes, sentinel.length + lenBuf.length);

    // Build K fragment (32 bytes, all 0x55).
    const k = new Uint8Array(32).fill(0x55);
    const b64 = btoa(String.fromCharCode(...k)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    const shareUrl = `https://example.test/open/aabbccdd#k=${b64}`;

    const { client } = await makeClient({
      responses: [
        { body: outerCbor, headers: { "content-type": "application/cbor" } },
      ],
    });

    const result = await client.importLink(shareUrl);
    expect(result.content).toBe(content);
  });

  it("throws UserError when URL has no fragment", async () => {
    const { client } = await makeClient();
    await expect(client.importLink("https://example.test/open/abc")).rejects.toBeInstanceOf(UserError);
    await expect(client.importLink("https://example.test/open/abc")).rejects.toThrow(/fragment/);
  });

  it("throws UserError when fragment does not start with k=", async () => {
    const { client } = await makeClient();
    await expect(client.importLink("https://example.test/open/abc#notk=xyz")).rejects.toBeInstanceOf(UserError);
  });

  it("throws IntegrityError when decryption fails", async () => {
    // Serve garbage bytes so open_with_key fails.
    const garbage = new Uint8Array([0xff, 0x00]);
    const k = new Uint8Array(32).fill(0x55);
    const b64 = btoa(String.fromCharCode(...k)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    const shareUrl = `https://example.test/open/badmem#k=${b64}`;

    const { client } = await makeClient({
      responses: [
        { body: garbage, headers: { "content-type": "application/cbor" } },
      ],
    });
    await expect(client.importLink(shareUrl)).rejects.toBeInstanceOf(IntegrityError);
  });
});

// ── listGrants ───────────────────────────────────────────────────────────────

describe("listGrants", () => {
  it("returns normalized grant entries", async () => {
    const { client, calls } = await makeClient({
      responses: [
        {
          body: {
            grants: [
              { grant_id: "g1", memory_hash: "hash1", reader: "did:key:abc", created_at: "2026-09-28T00:00:00Z" },
              { grant_id: "g2", memory_hash: "hash2", created_at: "2026-09-27T00:00:00Z" },
            ],
          },
        },
      ],
    });

    const grants = await client.listGrants();
    expect(grants).toHaveLength(2);
    expect(grants[0]!.grantId).toBe("g1");
    expect(grants[0]!.memoryHash).toBe("hash1");
    expect(grants[0]!.reader).toBe("did:key:abc");
    expect(grants[1]!.reader).toBeUndefined();

    // URL includes ?reader= param.
    expect(calls[0]!.url).toMatch(/\/api\/grants\?reader=/);
    expect(calls[0]!.method).toBe("GET");
  });

  it("returns empty array when grants field is absent", async () => {
    const { client } = await makeClient({
      responses: [{ body: {} }],
    });
    const grants = await client.listGrants();
    expect(grants).toEqual([]);
  });

  it("throws AuthError on 403", async () => {
    const { client } = await makeClient({
      responses: [{ status: 403, body: "forbidden" }],
    });
    const { AuthError } = await import("../src/errors.js");
    await expect(client.listGrants()).rejects.toBeInstanceOf(AuthError);
  });
});

// ── recallSealed ─────────────────────────────────────────────────────────────

describe("recallSealed", () => {
  const embedder={embed:async(text:string)=>new Float32Array(text==="low"?[0,1]:text==="mid"?[1,1]:[1,0])};
  it("ranks locally opened memories without any hosted request",async()=>{
    const {client,calls}=await makeClient();
    const low=await client.sealMemory("low",{mode:"store"});
    const high=await client.sealMemory("high",{mode:"store"});
    await client.sealMemory("mid",{mode:"store"});
    const hits=await client.recallSealed("query",{embedder,topK:3});
    expect(hits.map(h=>h.memoryHash)).toEqual([high.memoryHash,expect.any(String),low.memoryHash]);
    expect(hits[0]!.similarity).toBeCloseTo(1);expect(hits[2]!.similarity).toBe(0);
    expect(calls).toHaveLength(0);
  });
  it("honors topK",async()=>{
    const {client}=await makeClient();await client.sealMemory("low",{mode:"store"});await client.sealMemory("high",{mode:"store"});
    expect(await client.recallSealed("q",{embedder,topK:1})).toHaveLength(1);
  });
  it("requires an explicit embedder before touching the network",async()=>{
    const {client,calls}=await makeClient();await expect(client.recallSealed("private query")).rejects.toThrow(/local embedder/);expect(calls).toHaveLength(0);
  });
  it("empty local cache returns no hits",async()=>{
    const {client}=await makeClient();expect(await client.recallSealed("q",{embedder})).toEqual([]);
  });
  it("rejects invalid result bounds",async()=>{
    const {client}=await makeClient();await expect(client.recallSealed("q",{embedder,topK:-1})).rejects.toBeInstanceOf(UserError);
  });
});

// ── T3 golden-vector structural invariants (JS side) ─────────────────────────
//
// The T3 golden vectors are not exported as JSON files — they live in
// core/tests/golden_fixtures.rs and core/tests/integration_sealed_wasm.rs.
// We verify the same structural invariants here using the WASM mock:
//   A. seal produces a non-empty outer_cbor and 32-byte content_hash.
//   B. open_memory recovers the original inner JSON.
//   C. open_with_key produces the same result as open_memory.

describe("T3 golden vector structural invariants (mock WASM)", () => {
  it("A: seal_memory produces non-empty outer_cbor and 32-byte content_hash", () => {
    const innerJson = new TextEncoder().encode(
      JSON.stringify({ type: "memory", content: "golden vector content" })
    );
    const pub = new Uint8Array(32).fill(0x42);
    const result = sealedMock.seal_memory!(
      innerJson, pub, "art:golden-vector", "did:test:golden", "2026-09-28T00:00:00Z"
    ) as { outer_cbor: Uint8Array; content_hash: Uint8Array };

    expect(result.outer_cbor.length).toBeGreaterThan(0);
    expect(result.content_hash.length).toBe(32);
  });

  it("B: open_memory recovers original content from outer_cbor", () => {
    const innerJson = new TextEncoder().encode(
      JSON.stringify({ type: "memory", content: "golden vector content" })
    );
    const pub = new Uint8Array(32).fill(0x42);
    const { outer_cbor } = sealedMock.seal_memory!(
      innerJson, pub, "art:golden-b", "did:test:golden", "2026-09-28T00:00:00Z"
    ) as { outer_cbor: Uint8Array; content_hash: Uint8Array };

    const x25519Secret = new Uint8Array(32);
    const recovered = sealedMock.open_memory!(outer_cbor, x25519Secret);
    expect(recovered).toEqual(innerJson);
  });

  it("C: open_with_key produces same result as open_memory", () => {
    const innerJson = new TextEncoder().encode(
      JSON.stringify({ type: "memory", content: "same result" })
    );
    const pub = new Uint8Array(32).fill(0x42);
    const { outer_cbor } = sealedMock.seal_memory!(
      innerJson, pub, "art:golden-c", "did:test:golden", "2026-09-28T00:00:00Z"
    ) as { outer_cbor: Uint8Array; content_hash: Uint8Array };

    const k = new Uint8Array(32);
    const fromOpenMemory = sealedMock.open_memory!(outer_cbor, new Uint8Array(32));
    const fromOpenWithKey = sealedMock.open_with_key!(outer_cbor, k);
    expect(fromOpenMemory).toEqual(fromOpenWithKey);
  });

  it("link_fragment + parse_link_fragment round-trip", () => {
    const k = new Uint8Array(32).fill(0x55);
    const fragment = sealedMock.link_fragment!(k);
    expect(fragment).toMatch(/^k=/);
    const k2 = sealedMock.parse_link_fragment!(fragment);
    expect(k2).toEqual(k);
  });
});
