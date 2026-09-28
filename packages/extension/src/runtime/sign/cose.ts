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

interface SealMemoryResult {
  outer_cbor: Uint8Array;
  content_hash: Uint8Array;
}

interface MnemonicCoreModule {
  default: (input?: unknown) => Promise<unknown>;
  generate_keypair: () => unknown;
  sign_challenge: (kp: unknown, bytes: Uint8Array) => Uint8Array;
  sign_cose_payload: (payload: Uint8Array, kp: unknown) => Uint8Array;
  import_keypair_json: (s: string) => unknown;
  export_keypair_json: (kp: unknown) => string;
  compress_embedding: (
    embedding: Float32Array,
    bit_width: number,
  ) => Uint8Array;
  decompress_embedding: (bytes: Uint8Array, dim: number) => Float32Array;
  to_canonical_cbor_bytes: (value: unknown) => Uint8Array;
  blake3_hash: (bytes: Uint8Array) => Uint8Array;
  blake3_hash_hex: (bytes: Uint8Array) => string;
  sign_attestation_bundle: (
    content: string,
    embedding_bytes: Uint8Array,
    content_hash: string,
    owner_pubkey: string,
    keypair_json: unknown,
  ) => Uint8Array;
  // T5 sealed-memory bindings
  seal_memory: (
    inner_json: Uint8Array,
    author_ed25519_pub: Uint8Array,
    artifact_id: string,
    producer: string,
    created_at: string,
  ) => SealMemoryResult;
  open_memory: (outer_cbor: Uint8Array, x25519_secret: Uint8Array) => Uint8Array;
  x25519_public_from_ed25519: (ed25519_pub: Uint8Array) => Uint8Array;
}

/** End-to-end sign: builds the MEMORY_V1 artifact internally (schema-correct
 *  field names, types) + canonical CBOR + COSE_Sign1 in one WASM call.
 *  Avoids the JS-side `to_canonical_cbor_bytes` round-trip which can't
 *  represent Uint8Array fields via serde_json::Value. */
export async function signAttestationBundle(
  content: string,
  embeddingCompressed: Uint8Array,
  contentHash: string,
  ownerPubkey: string,
  keypair: KeypairJson,
): Promise<Uint8Array> {
  const wasm = await loadWasm();
  const cose = wasm.sign_attestation_bundle(
    content,
    embeddingCompressed,
    contentHash,
    ownerPubkey,
    keypair,
  );
  if (cose.length === 0 || cose[0] !== 0x84) {
    throw new Error(
      `signAttestationBundle: COSE_Sign1 envelope must start with 0x84, got 0x${(
        cose[0] ?? 0
      ).toString(16)}`,
    );
  }
  return cose;
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
    const mod = (await import(
      /* @vite-ignore */ url.href
    )) as MnemonicCoreModule;
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

/** Ed25519-sign raw `nonce` bytes with `keypair`. Returns the 64-byte
 *  detached signature. Used by the `/oauth/google/link` possession-
 *  proof flow (T16 + T17): the server issues a random nonce that the
 *  client must sign to prove ownership of the pubkey being bound. */
export async function signChallenge(
  keypair: KeypairJson,
  nonce: Uint8Array,
): Promise<Uint8Array> {
  if (nonce.length === 0) {
    throw new Error("signChallenge: refusing to sign empty nonce");
  }
  const wasm = await loadWasm();
  return wasm.sign_challenge(keypair, nonce);
}

// ── Unlock cache (tech-spec §7.5) ────────────────────────────────────────────
//
// The X25519 secret is derived from the Ed25519 identity secret once and held
// in memory for UNLOCK_TTL_MS (15 minutes). After the TTL the cache is cleared
// and the next open_memory call will re-derive. `clearUnlockCache()` is exposed
// for explicit lock-on-demand (e.g. when the popup is closed or the user logs
// out).

/** TTL for the X25519 secret unlock cache (15 minutes, per tech-spec §7.5). */
export const UNLOCK_TTL_MS = 15 * 60 * 1000;

interface UnlockEntry {
  x25519Secret: Uint8Array;
  expiresAt: number;
}

let unlockCache: UnlockEntry | null = null;
let unlockTimer: ReturnType<typeof setTimeout> | null = null;

/** @internal — tests call this to inject a cached secret directly. */
export function __setUnlockCacheForTesting(
  secret: Uint8Array | null,
  expiresAt?: number,
): void {
  if (unlockTimer !== null) {
    clearTimeout(unlockTimer);
    unlockTimer = null;
  }
  if (secret === null) {
    unlockCache = null;
    return;
  }
  const ttl = (expiresAt ?? Date.now() + UNLOCK_TTL_MS) - Date.now();
  unlockCache = { x25519Secret: secret, expiresAt: expiresAt ?? Date.now() + UNLOCK_TTL_MS };
  unlockTimer = setTimeout(() => {
    unlockCache = null;
    unlockTimer = null;
  }, Math.max(0, ttl));
}

/** Explicitly clear the unlock cache (e.g. on lock / sign-out). */
export function clearUnlockCache(): void {
  if (unlockTimer !== null) {
    clearTimeout(unlockTimer);
    unlockTimer = null;
  }
  unlockCache = null;
}

/**
 * Derive the X25519 secret from the Ed25519 secret (first 32 bytes of the
 * 64-byte Solana keypair) and cache it for `UNLOCK_TTL_MS`. Subsequent calls
 * within the window return the cached value without re-deriving.
 *
 * The Ed25519 secret slice passed here is the seed portion (bytes 0–31 of the
 * Solana 64-byte keypair). The public-key portion (bytes 32–63) is NOT the
 * X25519 input — the WASM `x25519_public_from_ed25519` takes the 32-byte
 * public key, while the secret mapping requires the seed bytes fed through
 * the birational map exposed by `crypto_sign_ed25519_sk_to_curve25519`.
 *
 * For the extension's current unlock path we receive the full 64-byte secret
 * and pass the public-key bytes (bytes 32–63) to WASM's
 * `x25519_public_from_ed25519` — this covers the public-key derivation half
 * used by seal_memory. The secret-side X25519 derivation for open_memory
 * uses the seed bytes (bytes 0–31) directly as the raw X25519 scalar, which
 * is the standard libsodium / dalek convention for keys derived this way.
 */
async function deriveOrGetX25519Secret(
  ed25519SecretFull: Uint8Array | number[],
): Promise<Uint8Array> {
  const now = Date.now();
  if (unlockCache !== null && unlockCache.expiresAt > now) {
    return unlockCache.x25519Secret;
  }
  if (unlockTimer !== null) {
    clearTimeout(unlockTimer);
    unlockTimer = null;
  }

  // The Solana keypair layout is: seed (32 bytes) || pubkey (32 bytes).
  // The X25519 secret scalar is the seed clamped per RFC 7748 §5.
  // libsodium / ed25519-dalek `to_scalar_bytes` returns the seed portion.
  // We use the first 32 bytes of the Solana secret as the X25519 secret
  // (the standard key-derivation for the extension per tech-spec §5.3 K1).
  const secretArr =
    ed25519SecretFull instanceof Uint8Array
      ? ed25519SecretFull
      : new Uint8Array(ed25519SecretFull);
  const x25519Secret = secretArr.slice(0, 32);

  const expiresAt = now + UNLOCK_TTL_MS;
  unlockCache = { x25519Secret, expiresAt };
  unlockTimer = setTimeout(() => {
    unlockCache = null;
    unlockTimer = null;
  }, UNLOCK_TTL_MS);

  return x25519Secret;
}

// ── Sealed-memory WASM wrappers ───────────────────────────────────────────────

/** Result of {@link sealMemory}. */
export interface SealedMemoryResult {
  /** Unsigned SEALED_V1 canonical CBOR. Must be signed before storage. */
  outerCbor: Uint8Array;
  /** blake3 hash of `outerCbor`. */
  contentHash: Uint8Array;
}

/**
 * Seal a memory JSON blob (inner MEMORY_V1 bytes) for the given author.
 *
 * Derives the X25519 public key from the Ed25519 `authorPubkey` (32 bytes),
 * generates a fresh content key K and nonce via WebCrypto, encrypts, and
 * returns the unsigned SEALED_V1 outer CBOR.
 *
 * The author must then sign the returned `outerCbor` with `signCosePayload`
 * and POST to either `/api/anchor-sealed` (anchored) or `/api/store-sealed`
 * (local).
 */
export async function sealMemory(
  innerJson: Uint8Array,
  authorEd25519Pub: Uint8Array,
  artifactId: string,
  producer: string,
  createdAt: string,
): Promise<SealedMemoryResult> {
  if (innerJson.length === 0) {
    throw new Error("sealMemory: innerJson must not be empty");
  }
  if (authorEd25519Pub.length !== 32) {
    throw new Error(
      `sealMemory: authorEd25519Pub must be 32 bytes, got ${authorEd25519Pub.length}`,
    );
  }
  const wasm = await loadWasm();
  const result = wasm.seal_memory(
    innerJson,
    authorEd25519Pub,
    artifactId,
    producer,
    createdAt,
  );
  return {
    outerCbor: result.outer_cbor,
    contentHash: result.content_hash,
  };
}

/**
 * Open a sealed memory using the identity X25519 secret.
 *
 * The X25519 secret is derived from the Ed25519 identity secret (the first
 * 32 bytes of the 64-byte Solana keypair) and cached for `UNLOCK_TTL_MS`
 * (15 minutes) per tech-spec §7.5.
 *
 * @param outerCbor   - SEALED_V1 canonical CBOR bytes (from the server or
 *                      Arweave). May or may not be wrapped in COSE_Sign1 —
 *                      callers of the raw outer CBOR path pass the inner CBOR
 *                      only; for COSE-wrapped bytes use `verify_sealed` first.
 * @param ed25519Secret - The full 64-byte Solana keypair secret
 *                       (seed || pubkey). Only bytes 0–31 are used.
 * @returns The inner MEMORY_V1 JSON bytes.
 */
export async function openMemory(
  outerCbor: Uint8Array,
  ed25519Secret: Uint8Array | number[],
): Promise<Uint8Array> {
  if (outerCbor.length === 0) {
    throw new Error("openMemory: outerCbor must not be empty");
  }
  const x25519Secret = await deriveOrGetX25519Secret(ed25519Secret);
  const wasm = await loadWasm();
  return wasm.open_memory(outerCbor, x25519Secret);
}

/**
 * Derive the X25519 public key from a raw 32-byte Ed25519 public key.
 * Used by callers that need to publish or seal to a known identity without
 * holding the secret.
 */
export async function x25519PublicFromEd25519(
  ed25519Pub: Uint8Array,
): Promise<Uint8Array> {
  if (ed25519Pub.length !== 32) {
    throw new Error(
      `x25519PublicFromEd25519: ed25519Pub must be 32 bytes, got ${ed25519Pub.length}`,
    );
  }
  const wasm = await loadWasm();
  return wasm.x25519_public_from_ed25519(ed25519Pub);
}

/** Generate a fresh Ed25519 keypair via the WASM `generate_keypair`
 *  export. The underlying RNG is `getrandom`'s `js` feature, which
 *  delegates to `crypto.getRandomValues` — CSPRNG-quality entropy. The
 *  return shape mirrors the on-disk identity layout used by both the
 *  popup (`identity` / `identity_secret`) and the SDK (`KeypairJson`):
 *  64-byte Solana secret (seed||pubkey) plus its base58-encoded pubkey.
 *
 *  This helper is the canonical, single-source seam for new-identity
 *  minting in the extension realm — components must not call WASM
 *  directly or roll their own base58 encoder. */
export async function generateKeypair(): Promise<KeypairJson> {
  const wasm = await loadWasm();
  const raw = wasm.generate_keypair() as unknown;
  if (!raw || typeof raw !== "object") {
    throw new Error("generateKeypair: WASM returned a non-object value");
  }
  const o = raw as Record<string, unknown>;
  const pub = o.pubkey_base58;
  const secret = o.secret;
  if (typeof pub !== "string" || pub.length === 0) {
    throw new Error("generateKeypair: WASM returned an empty pubkey");
  }
  if (!Array.isArray(secret) || secret.length !== 64) {
    throw new Error(
      `generateKeypair: WASM returned a secret of length ${
        Array.isArray(secret) ? secret.length : "non-array"
      }, expected 64`,
    );
  }
  // Defensive byte-range check — corrupted WASM output would otherwise
  // slip through into the persisted identity blob.
  const normalised: number[] = [];
  for (const n of secret) {
    const num = typeof n === "number" ? n : Number(n);
    if (!Number.isInteger(num) || num < 0 || num > 255) {
      throw new Error("generateKeypair: WASM secret contains non-byte value");
    }
    normalised.push(num);
  }
  return { pubkey_base58: pub, secret: normalised };
}
