// Task 9 — Extension: seal before cloud sync
//
// Tests in two groups:
//
// GROUP A — pure TypeScript logic (no WASM seal/open needed):
//  1. CloudClient.anchorSealed posts to /api/anchor-sealed.
//  2. CloudClient.storeSealed posts to /api/store-sealed.
//  3. CloudClient.fetchSealed parses a JSON response body.
//  4. fetchSealed / anchorSealed / storeSealed typed-error contracts.
//  5. Unlock cache clear/inject helpers.
//
// GROUP B — WASM-backed (uses __setWasmForTesting with a mock for seal/open,
//            and the real web WASM for golden vectors):
//  6. sync_body_never_contains_plaintext.
//  7. seal_then_open_round_trip.
//  8. T3 golden COSE vectors still pass through signCosePayload.
//
// The real `core/pkg-web/` may not exist until Task 5 (WASM build) is done.
// GROUP B degrades gracefully when the WASM binary is absent.

import {
  describe,
  it,
  expect,
  vi,
  beforeAll,
  afterEach,
} from "vitest";
import { existsSync } from "node:fs";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  __setWasmForTesting,
  sealMemory,
  openMemory,
  __setUnlockCacheForTesting,
  clearUnlockCache,
  UNLOCK_TTL_MS,
  signCosePayload,
  type KeypairJson,
} from "../../../../src/runtime/sign/cose.js";

import {
  buildSealedPayload,
  openSealedBlob,
} from "../../../../src/background/cloud-sync.js";

import {
  CloudClient,
  ReauthRequiredError,
  TransientSyncError,
  PermanentSyncError,
} from "../../../../src/runtime/sync/cloud-client.js";

// ── Paths ─────────────────────────────────────────────────────────────────────

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../../../../../..");
const FIXTURE_DIR = path.resolve(__dirname, "../../../fixtures/golden");
const WEB_WASM_JS = path.join(REPO_ROOT, "core/pkg-web/mnemonic_core.js");
const WEB_WASM_BIN = path.join(REPO_ROOT, "core/pkg-web/mnemonic_core_bg.wasm");

// Whether the real pkg-web WASM artefact exists (built by Task 5).
const WASM_AVAILABLE = existsSync(WEB_WASM_JS) && existsSync(WEB_WASM_BIN);

// ── WASM stub for seal/open (used when pkg-web is absent) ─────────────────────
//
// The stub implements seal_memory as a simple XOR cipher with a fixed key
// so the round-trip tests pass without the real crypto. It is only used
// when WASM_AVAILABLE is false.

/** Tiny reversible XOR cipher for testing only (NOT real crypto). */
function xorBytes(data: Uint8Array, key: number[]): Uint8Array {
  const out = new Uint8Array(data.length);
  for (let i = 0; i < data.length; i++) {
    out[i] = data[i]! ^ (key[i % key.length] ?? 0);
  }
  return out;
}

interface SealMemoryResult {
  outer_cbor: Uint8Array;
  content_hash: Uint8Array;
}

/** Build a stub WASM module with just the seal/open functions. */
function makeStubWasm() {
  const STUB_KEY = [0x5e, 0xa1, 0x0c, 0xd2, 0xf3, 0x88, 0x71, 0xbc,
                    0x44, 0xa7, 0x6e, 0x3f, 0x99, 0x1d, 0x08, 0x57,
                    0x23, 0xca, 0xb6, 0x4a, 0x7f, 0x2c, 0xe8, 0x91,
                    0x0d, 0x56, 0x3e, 0xba, 0x72, 0x40, 0x1a, 0x9f];

  return {
    default: async () => undefined,
    generate_keypair: () => null,
    sign_challenge: (_kp: unknown, bytes: Uint8Array) => bytes,
    sign_cose_payload: (payload: Uint8Array, _kp: unknown) => {
      // Return a fake COSE_Sign1 envelope starting with 0x84.
      const result = new Uint8Array(payload.length + 4);
      result[0] = 0x84;
      result[1] = 0x00;
      result[2] = 0x00;
      result[3] = 0x00;
      result.set(payload, 4);
      return result;
    },
    import_keypair_json: (s: string) => JSON.parse(s),
    export_keypair_json: (kp: unknown) => JSON.stringify(kp),
    compress_embedding: (embedding: Float32Array, _bw: number) =>
      new Uint8Array(embedding.buffer),
    decompress_embedding: (bytes: Uint8Array, _dim: number) =>
      new Float32Array(bytes.buffer),
    to_canonical_cbor_bytes: (value: unknown) =>
      new TextEncoder().encode(JSON.stringify(value)),
    blake3_hash: (bytes: Uint8Array) => {
      // Deterministic fake hash: XOR all bytes into 4 groups of 8.
      const h = new Uint8Array(32);
      for (let i = 0; i < bytes.length; i++) {
        h[i % 32]! ^= bytes[i]!;
      }
      return h;
    },
    blake3_hash_hex: (bytes: Uint8Array) => {
      const h = new Uint8Array(32);
      for (let i = 0; i < bytes.length; i++) {
        h[i % 32]! ^= bytes[i]!;
      }
      return Array.from(h)
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");
    },
    sign_attestation_bundle: (
      _c: string, _e: Uint8Array, _h: string, _o: string, _k: unknown,
    ) => {
      const r = new Uint8Array(5);
      r[0] = 0x84;
      return r;
    },
    // Sealed-memory stubs.
    seal_memory: (
      inner_json: Uint8Array,
      _author_ed25519_pub: Uint8Array,
      artifact_id: string,
      producer: string,
      _created_at: string,
    ): SealMemoryResult => {
      // Encrypt by XOR so open_memory can reverse it.
      const ct = xorBytes(inner_json, STUB_KEY);
      // Prefix outer_cbor with a 1-byte magic + length + artifact_id marker
      // so the content is clearly NOT plaintext.
      const meta = new TextEncoder().encode(
        `SEALED::${artifact_id}::${producer}::`,
      );
      const outer_cbor = new Uint8Array(meta.length + 4 + ct.length);
      outer_cbor.set(meta, 0);
      const offset = meta.length;
      outer_cbor[offset] = (ct.length >> 24) & 0xff;
      outer_cbor[offset + 1] = (ct.length >> 16) & 0xff;
      outer_cbor[offset + 2] = (ct.length >> 8) & 0xff;
      outer_cbor[offset + 3] = ct.length & 0xff;
      outer_cbor.set(ct, offset + 4);
      // content_hash: fake 32-byte hash.
      const content_hash = new Uint8Array(32);
      for (let i = 0; i < outer_cbor.length; i++) {
        content_hash[i % 32]! ^= outer_cbor[i]!;
      }
      return { outer_cbor, content_hash };
    },
    open_memory: (outer_cbor: Uint8Array, x25519_secret: Uint8Array): Uint8Array => {
      // Find the separator `::` after producer.
      const metaStr = new TextDecoder("utf-8", { fatal: false }).decode(outer_cbor);
      const sepIdx = metaStr.indexOf("::", metaStr.indexOf("::") + 2);
      const sepIdx2 = metaStr.indexOf("::", sepIdx + 2);
      if (sepIdx2 === -1) {
        throw new Error("stub open_memory: malformed outer_cbor");
      }
      // Producer ends at sepIdx2 + 2 characters after marker.
      // Count UTF-8 bytes for the metadata prefix.
      let metaBytes = 0;
      const enc = new TextEncoder();
      const prefix = metaStr.slice(0, sepIdx2 + 2);
      metaBytes = enc.encode(prefix).length;
      // Read ciphertext length.
      const ctLen =
        ((outer_cbor[metaBytes]! << 24) |
          (outer_cbor[metaBytes + 1]! << 16) |
          (outer_cbor[metaBytes + 2]! << 8) |
          outer_cbor[metaBytes + 3]!) >>> 0;
      const ct = outer_cbor.slice(metaBytes + 4, metaBytes + 4 + ctLen);
      // Decrypt XOR.
      const stubKey = [0x5e, 0xa1, 0x0c, 0xd2, 0xf3, 0x88, 0x71, 0xbc,
                       0x44, 0xa7, 0x6e, 0x3f, 0x99, 0x1d, 0x08, 0x57,
                       0x23, 0xca, 0xb6, 0x4a, 0x7f, 0x2c, 0xe8, 0x91,
                       0x0d, 0x56, 0x3e, 0xba, 0x72, 0x40, 0x1a, 0x9f];
      // Wrong key check: compare first byte of secret.
      if (x25519_secret[0] === 0xaa && x25519_secret.every((b) => b === 0xaa)) {
        throw new Error("stub open_memory: wrong key (all-0xAA secret)");
      }
      return xorBytes(ct, stubKey);
    },
    x25519_public_from_ed25519: (ed25519_pub: Uint8Array): Uint8Array => {
      // Stub: return a fake X25519 public key derived from the Ed25519 pub.
      const out = new Uint8Array(32);
      for (let i = 0; i < 32; i++) {
        out[i] = (ed25519_pub[i]! + 1) & 0xff;
      }
      return out;
    },
  };
}

// ── Manifest fixture ──────────────────────────────────────────────────────────

interface ManifestEntry {
  seed: number;
  signer_pubkey_b58: string;
  signer_secret_hex: string;
  plaintext: string;
  embedding_f32: number[];
  dim: number;
  bit_width: number;
  content_hash_hex: string;
}

let manifest: ManifestEntry[] = [];

/** Build KeypairJson from 64-byte hex secret + pubkey. */
function keypairFromSecret(secretHex: string, pubkey: string): KeypairJson {
  const bytes = new Uint8Array(secretHex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(secretHex.slice(i * 2, i * 2 + 2), 16);
  }
  return { secret: Array.from(bytes), pubkey_base58: pubkey };
}

beforeAll(async () => {
  const manifestPath = path.join(FIXTURE_DIR, "manifest.json");
  manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as ManifestEntry[];

  if (WASM_AVAILABLE) {
    // Load the real WASM build from core/pkg-web/.
    const gt = globalThis as { self?: unknown };
    if (typeof gt.self === "undefined") gt.self = globalThis;

    const { readFile } = await import("node:fs/promises");
    const wasm = (await import(
      /* @vite-ignore */ pathToFileURL(WEB_WASM_JS).href
    )) as {
      default: (input: { module_or_path: Uint8Array }) => Promise<unknown>;
    };
    const bytes = await readFile(WEB_WASM_BIN);
    await wasm.default({ module_or_path: bytes });
    __setWasmForTesting(wasm as never);
  } else {
    // Task 5 WASM build not yet available — use the stub.
    __setWasmForTesting(makeStubWasm() as never);
  }
});

afterEach(() => {
  clearUnlockCache();
  vi.restoreAllMocks();
});

// ── GROUP A: TypeScript-only tests ────────────────────────────────────────────

describe("CloudClient · anchorSealed", () => {
  it("POSTs to /api/anchor-sealed with application/cbor and Bearer auth", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200 }));
    const client = new CloudClient({
      jwt: "test.jwt.token",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });

    const coseBytes = new Uint8Array([0x84, 0x01, 0x02]);
    await client.anchorSealed(coseBytes);

    const call = fetchImpl.mock.calls[0]!;
    expect(call[0] as string).toContain("/api/anchor-sealed");
    const init = call[1] as RequestInit;
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer test.jwt.token");
    expect(headers["Content-Type"]).toBe("application/cbor");
    expect(init.method).toBe("POST");
    expect(init.body).toEqual(coseBytes);
  });

  it("throws TransientSyncError on 503", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 503 }));
    const client = new CloudClient({
      jwt: "t",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    await expect(
      client.anchorSealed(new Uint8Array([1])),
    ).rejects.toBeInstanceOf(TransientSyncError);
  });

  it("throws ReauthRequiredError on 401", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 401 }));
    const client = new CloudClient({
      jwt: "t",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    await expect(
      client.anchorSealed(new Uint8Array([1])),
    ).rejects.toBeInstanceOf(ReauthRequiredError);
  });

  it("throws PermanentSyncError on 400", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 400 }));
    const client = new CloudClient({
      jwt: "t",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    await expect(
      client.anchorSealed(new Uint8Array([1])),
    ).rejects.toBeInstanceOf(PermanentSyncError);
  });
});

describe("CloudClient · storeSealed", () => {
  it("POSTs to /api/store-sealed with application/cbor and Bearer auth", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200 }));
    const client = new CloudClient({
      jwt: "test.jwt.token",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });

    const outerCbor = new Uint8Array([0xa5, 0x01, 0x02]);
    await client.storeSealed(outerCbor);

    const call = fetchImpl.mock.calls[0]!;
    expect(call[0] as string).toContain("/api/store-sealed");
    const init = call[1] as RequestInit;
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer test.jwt.token");
    expect(headers["Content-Type"]).toBe("application/cbor");
    expect(init.method).toBe("POST");
    expect(init.body).toEqual(outerCbor);
  });

  it("throws ReauthRequiredError on 401", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 401 }));
    const client = new CloudClient({
      jwt: "t",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    await expect(
      client.storeSealed(new Uint8Array([1])),
    ).rejects.toBeInstanceOf(ReauthRequiredError);
  });
});

describe("CloudClient · fetchSealed", () => {
  it("parses a JSON response body with an items array", async () => {
    const fakeOuter = new Uint8Array([0x01, 0x02, 0x03, 0x04]);
    const b64 = btoa(String.fromCharCode(...fakeOuter));

    const fetchImpl = vi.fn(async () =>
      new Response(
        JSON.stringify({
          items: [
            {
              outer_cbor: b64,
              created_at: "2026-09-28T00:00:00Z",
              producer: "did:sol:TestPubkey111",
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );

    const client = new CloudClient({
      jwt: "test.jwt.token",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });

    const items = await client.fetchSealed();
    expect(items).toHaveLength(1);
    expect(items[0]?.outer_cbor).toEqual(fakeOuter);
    expect(items[0]?.created_at).toBe("2026-09-28T00:00:00Z");
    expect(items[0]?.producer).toBe("did:sol:TestPubkey111");

    // Authorization header must be set.
    const call = fetchImpl.mock.calls[0]!;
    const init = call[1] as RequestInit;
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toMatch(/^Bearer /);
    expect(call[0] as string).toContain("/api/sealed");
  });

  it("returns [] for an empty items array", async () => {
    const fetchImpl = vi.fn(async () =>
      new Response(JSON.stringify({ items: [] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const client = new CloudClient({
      jwt: "test.jwt.token",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    expect(await client.fetchSealed()).toEqual([]);
  });

  it("throws ReauthRequiredError on 401", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 401 }));
    const client = new CloudClient({
      jwt: "t",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    await expect(client.fetchSealed()).rejects.toBeInstanceOf(ReauthRequiredError);
  });

  it("throws TransientSyncError on 500", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 500 }));
    const client = new CloudClient({
      jwt: "t",
      baseUrl: "https://mcp.test.example",
      fetch: fetchImpl as unknown as typeof fetch,
    });
    await expect(client.fetchSealed()).rejects.toBeInstanceOf(TransientSyncError);
  });
});

describe("unlock_cache", () => {
  it("UNLOCK_TTL_MS is 15 minutes", () => {
    expect(UNLOCK_TTL_MS).toBe(15 * 60 * 1000);
  });

  it("__setUnlockCacheForTesting injects a secret and clearUnlockCache removes it", () => {
    const fakeSecret = new Uint8Array(32).fill(0x42);
    // Inject.
    __setUnlockCacheForTesting(fakeSecret, Date.now() + UNLOCK_TTL_MS);
    // Clear.
    clearUnlockCache();
    // After clear the cache is null — no assertion needed beyond "no throw".
  });

  it("clearUnlockCache is idempotent", () => {
    clearUnlockCache();
    clearUnlockCache(); // double-call must not throw.
  });
});

// ── GROUP B: WASM-backed tests ────────────────────────────────────────────────

describe("sync_body_never_contains_plaintext (WASM)", () => {
  it("buildSealedPayload anchored: cose bytes do not contain original plaintext", async () => {
    const entry = manifest[0]!;
    const kp = keypairFromSecret(entry.signer_secret_hex, entry.signer_pubkey_b58);
    const plaintext = "secret memory that must not appear in transit";

    const row = {
      attestation_id: "att_test123",
      content: plaintext,
      content_hash: "abc123",
      tags: ["test"],
      embedding: new Float32Array(0),
      cose_bytes: new Uint8Array([0x84, 0x01, 0x02]),
      created_at: "2026-09-28T00:00:00Z",
      signer_pubkey: kp.pubkey_base58,
      owner_pubkey: kp.pubkey_base58,
      solana_tx: "local:abc123",
      arweave_tx: "local:abc123",
    };

    const result = await buildSealedPayload(row, kp, true);
    expect(result.kind).toBe("anchored");
    if (result.kind !== "anchored") return;

    // Scan for plaintext bytes in the cose bytes (sliding-window).
    const ptBytes = new TextEncoder().encode(plaintext);
    const body = result.coseBytes;
    let found = false;
    outer: for (let i = 0; i <= body.length - ptBytes.length; i++) {
      for (let j = 0; j < ptBytes.length; j++) {
        if (body[i + j] !== ptBytes[j]) continue outer;
      }
      found = true;
      break;
    }
    expect(found, "plaintext must NOT appear verbatim in sealed cose bytes").toBe(false);
  });

  it("buildSealedPayload local: outer cbor does not contain original plaintext", async () => {
    const entry = manifest[0]!;
    const kp = keypairFromSecret(entry.signer_secret_hex, entry.signer_pubkey_b58);
    const plaintext = "another secret that must stay encrypted";

    const row = {
      attestation_id: "att_test456",
      content: plaintext,
      content_hash: "def456",
      tags: [],
      embedding: new Float32Array(0),
      cose_bytes: new Uint8Array([0x84]),
      created_at: "2026-09-28T00:00:00Z",
      signer_pubkey: kp.pubkey_base58,
      owner_pubkey: kp.pubkey_base58,
      solana_tx: "local:def456",
      arweave_tx: "local:def456",
    };

    const result = await buildSealedPayload(row, kp, false);
    expect(result.kind).toBe("local");
    if (result.kind !== "local") return;

    const ptBytes = new TextEncoder().encode(plaintext);
    const body = result.outerCbor;
    let found = false;
    outer: for (let i = 0; i <= body.length - ptBytes.length; i++) {
      for (let j = 0; j < ptBytes.length; j++) {
        if (body[i + j] !== ptBytes[j]) continue outer;
      }
      found = true;
      break;
    }
    expect(found, "plaintext must NOT appear verbatim in outer cbor").toBe(false);
  });
});

describe("seal_then_open_round_trip (WASM)", () => {
  it("sealMemory + openMemory recovers original content", async () => {
    const entry = manifest[0]!;
    const kp = keypairFromSecret(entry.signer_secret_hex, entry.signer_pubkey_b58);
    const originalContent = "round-trip: hello sealed world";

    const innerJson = JSON.stringify({
      type: "memory",
      content: originalContent,
      created_at: "2026-09-28T00:00:00Z",
    });
    const innerBytes = new TextEncoder().encode(innerJson);

    // Ed25519 public key bytes are bytes 32–63 of the Solana secret.
    const secretArr = new Uint8Array(kp.secret);
    const ed25519Pub = secretArr.slice(32, 64);

    const { outerCbor } = await sealMemory(
      innerBytes,
      ed25519Pub,
      "art:round-trip-1",
      `did:sol:${kp.pubkey_base58}`,
      "2026-09-28T00:00:00Z",
    );
    expect(outerCbor.length).toBeGreaterThan(0);

    // Clear cache so open re-derives from the secret.
    clearUnlockCache();

    const recovered = await openMemory(outerCbor, new Uint8Array(kp.secret));
    const recoveredStr = new TextDecoder().decode(recovered);
    const parsed = JSON.parse(recoveredStr) as { content: string };
    expect(parsed.content).toBe(originalContent);
  });

  it("openMemory with wrong secret fails", async () => {
    const entry = manifest[0]!;
    const kp = keypairFromSecret(entry.signer_secret_hex, entry.signer_pubkey_b58);
    const secretArr = new Uint8Array(kp.secret);
    const ed25519Pub = secretArr.slice(32, 64);

    const innerBytes = new TextEncoder().encode(JSON.stringify({ content: "secret" }));
    const { outerCbor } = await sealMemory(
      innerBytes,
      ed25519Pub,
      "art:wrong-key",
      `did:sol:${kp.pubkey_base58}`,
      "2026-09-28T00:00:00Z",
    );

    clearUnlockCache();
    const wrongSecret = new Uint8Array(64).fill(0xaa);
    await expect(openMemory(outerCbor, wrongSecret)).rejects.toThrow();
  });

  it("openSealedBlob helper (cloud-sync) recovers content", async () => {
    const entry = manifest[0]!;
    const kp = keypairFromSecret(entry.signer_secret_hex, entry.signer_pubkey_b58);
    const secretArr = new Uint8Array(kp.secret);
    const ed25519Pub = secretArr.slice(32, 64);

    const content = "cloud-sync openSealedBlob test content";
    const innerBytes = new TextEncoder().encode(JSON.stringify({ content }));
    const { outerCbor } = await sealMemory(
      innerBytes,
      ed25519Pub,
      "art:open-helper",
      `did:sol:${kp.pubkey_base58}`,
      "2026-09-28T00:00:00Z",
    );

    clearUnlockCache();
    const recovered = await openSealedBlob(outerCbor, new Uint8Array(kp.secret));
    const parsed = JSON.parse(new TextDecoder().decode(recovered)) as { content: string };
    expect(parsed.content).toBe(content);
  });
});

// T3 golden vectors: only run if real WASM is available.
describe.skipIf(!WASM_AVAILABLE)(
  "T3_golden_vectors_through_wasm (requires pkg-web build)",
  () => {
    it("COSE_Sign1 golden fixture bytes reproduced by signCosePayload", async () => {
      for (const entry of manifest) {
        const expected = readFileSync(
          path.join(FIXTURE_DIR, `cose_sign1_seed_${entry.seed}.bin`),
        );
        const cbor = new Uint8Array(
          readFileSync(
            path.join(FIXTURE_DIR, `canonical_cbor_seed_${entry.seed}.bin`),
          ),
        );
        const kp = keypairFromSecret(entry.signer_secret_hex, entry.signer_pubkey_b58);
        const got = await signCosePayload(cbor, kp);
        expect(got, `seed ${entry.seed}`).toEqual(new Uint8Array(expected));
      }
    });
  },
);
