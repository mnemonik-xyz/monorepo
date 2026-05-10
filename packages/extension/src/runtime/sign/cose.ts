// WASM crypto-pipeline wrapper for the extension. Loads
// `@mnemonic/core`'s WASM module once per realm and exposes typed
// helpers for the four pipeline stages the browser-side signing flow
// runs:
//
//   1. compress(f32[]) → bytes        (TurboQuant, bit_width=4)
//   2. canonicalCbor(artifact) → bytes (RFC 8949 §4.2 deterministic)
//   3. blake3(bytes) → 32-byte digest  (content_hash, hex via _hex variant)
//   4. signCose(payload, kp) → bytes   (COSE_Sign1, alg=EdDSA)
//
// All four are byte-identical to `core`'s native path. Parity is
// guarded by `tests/unit/sign/cose.test.ts` against the golden fixture
// set committed at `tests/fixtures/golden/`.
//
// Loader strategy: dynamic import + `default()` initialization, single
// shared promise per realm. The WASM artefact ships under
// `wasm/mnemonic_core{.js,_bg.wasm}` next to the bundled extension JS.
// During development `vite-plugin-crxjs` resolves these from
// `node_modules/@mnemonik-xyz/sdk/dist/wasm/` (T02 mirrors them there).
//
// Ergonomic note: the SDK already loads the same WASM under its own
// realm — duplicating the loader here keeps the extension free of any
// SDK-internal init order assumptions and lets us swap to a worker-
// embedded WASM later (T04) without changing call sites.

interface MnemonicCoreModule {
  default: (input?: unknown) => Promise<unknown>;
  generate_keypair: () => unknown;
  sign_challenge: (kp: unknown, bytes: Uint8Array) => Uint8Array;
  sign_cose_payload: (payload: Uint8Array, kp: unknown) => Uint8Array;
  import_keypair_json: (s: string) => unknown;
  export_keypair_json: (kp: unknown) => string;
  compress_embedding: (embedding: Float32Array, bit_width: number) => Uint8Array;
  decompress_embedding: (bytes: Uint8Array, dim: number) => Float32Array;
  to_canonical_cbor_bytes: (value: unknown) => Uint8Array;
  blake3_hash: (bytes: Uint8Array) => Uint8Array;
  blake3_hash_hex: (bytes: Uint8Array) => string;
}

let modulePromise: Promise<MnemonicCoreModule> | null = null;
let testOverride: MnemonicCoreModule | null = null;

/** @internal — tests inject the WASM module directly to avoid the
 *  bundler-resolved dynamic import (vitest's jsdom env can't reach the
 *  packaged `wasm/mnemonic_core.js` at the production path). */
export function __setWasmForTesting(mod: MnemonicCoreModule | null): void {
  testOverride = mod;
  modulePromise = null;
}

async function loadWasm(): Promise<MnemonicCoreModule> {
  if (testOverride) return testOverride;
  if (modulePromise) return modulePromise;
  modulePromise = (async () => {
    const url = new URL("./wasm/mnemonic_core.js", import.meta.url);
    const mod = (await import(/* @vite-ignore */ url.href)) as MnemonicCoreModule;
    if (typeof mod.default === "function") {
      await mod.default();
    }
    return mod;
  })();
  return modulePromise;
}

export interface KeypairJson {
  /** 64-byte Solana keypair format (32-byte seed || 32-byte pubkey). */
  secret: number[];
  /** Base58 Ed25519 public key. */
  pubkey_base58: string;
}

/** Compress an f32 embedding via TurboQuant (default 4-bit). Same bytes
 *  the server stores for an identical input vector — drift breaks
 *  recall byte-for-byte. */
export async function compressEmbedding(
  embedding: Float32Array,
  bitWidth = 4,
): Promise<Uint8Array> {
  if (embedding.length === 0) {
    throw new Error("compressEmbedding: embedding must not be empty");
  }
  const wasm = await loadWasm();
  return wasm.compress_embedding(embedding, bitWidth);
}

/** Decompress TurboQuant bytes back to an approximate f32 vector.
 *  `dim` MUST match the original embedding's dimension (encoded inside
 *  the bytes — mismatch throws). */
export async function decompressEmbedding(
  bytes: Uint8Array,
  dim: number,
): Promise<Float32Array> {
  const wasm = await loadWasm();
  return wasm.decompress_embedding(bytes, dim);
}

/** Canonicalize a `MEMORY_V1` artifact to CBOR bytes. Field order is
 *  schema-defined (not alphabetical) — see `core/src/codec/schema.rs`. */
export async function toCanonicalCbor(artifact: unknown): Promise<Uint8Array> {
  if (!artifact || typeof artifact !== "object" || Array.isArray(artifact)) {
    throw new Error("toCanonicalCbor: artifact must be a plain object");
  }
  const wasm = await loadWasm();
  return wasm.to_canonical_cbor_bytes(artifact);
}

/** Raw 32-byte blake3 digest of `bytes`. */
export async function blake3Hash(bytes: Uint8Array): Promise<Uint8Array> {
  const wasm = await loadWasm();
  return wasm.blake3_hash(bytes);
}

/** Hex string form of {@link blake3Hash} — convenience for content_hash
 *  fields the server echoes as lowercase hex. */
export async function blake3HashHex(bytes: Uint8Array): Promise<string> {
  const wasm = await loadWasm();
  return wasm.blake3_hash_hex(bytes);
}

/** Sign canonical-CBOR `payload` with `keypair`, returning the
 *  COSE_Sign1 envelope bytes. Mirror of the SDK's `coseSignPayload`. */
export async function signCosePayload(
  payload: Uint8Array,
  keypair: KeypairJson,
): Promise<Uint8Array> {
  if (payload.length === 0) {
    throw new Error("signCosePayload: refusing to sign empty payload");
  }
  const wasm = await loadWasm();
  const cose = wasm.sign_cose_payload(payload, keypair);
  if (cose.length === 0 || cose[0] !== 0x84) {
    throw new Error(
      `signCosePayload: COSE_Sign1 envelope must start with 0x84, got 0x${(
        cose[0] ?? 0
      ).toString(16)}`,
    );
  }
  return cose;
}
