/**
 * Lazy loader for the wasm-pack `--target web` ES module produced by Task 3.
 *
 * The output lives in `webapp/src/wasm/` (gitignored, regenerated on every
 * `npm run build:wasm`). Vite tree-shakes the import so the WASM bytes are only
 * fetched on routes that actually need them (`/install`, `/sign/*`).
 *
 * The wasm-pack `--target web` shim requires an explicit `init()` call before
 * any export is invoked. We cache the init Promise so concurrent callers share
 * a single fetch.
 */

// The generated module ships its own .d.ts; until the wasm dir is built locally
// we describe just the surface this app uses. After `npm run build:wasm` the
// real types take precedence in IDEs that resolve them.
export interface MnemonicWasm {
  default: (input?: unknown) => Promise<unknown>;
  generate_keypair: () => unknown;
  sign_challenge: (keypair_json: unknown, challenge: Uint8Array) => Uint8Array;
  sign_attestation_bundle: (
    content: string,
    embedding_bytes: Uint8Array,
    content_hash: string,
    owner_pubkey: string,
    keypair_json: unknown
  ) => Uint8Array;
  /**
   * Wrap server-provided canonical-CBOR bytes in a COSE_Sign1 envelope —
   * use this for /api/sign-callback so the signed payload byte-matches
   * what the server stored. (sign_attestation_bundle rebuilds CBOR
   * client-side from a SUBSET of metadata fields, which produces
   * different bytes than the server's original — verify_artifact fails
   * content_integrity in that case.)
   */
  sign_cose_payload: (payload: Uint8Array, keypair_json: unknown) => Uint8Array;
  export_keypair_json: (keypair_json: unknown) => string;
  import_keypair_json: (json_str: string) => unknown;
  /** Cosine similarity between two equal-length f32 vectors (0.0 on mismatch). */
  cosine_similarity: (a: Float32Array, b: Float32Array) => number;
  /**
   * Verify a chunk's COSE_Sign1 bytes (fetched from Arweave) and return its
   * verified content + decompressed embedding. Trustless: the signature is
   * checked inside WASM, so the gateway is not trusted.
   */
  verify_and_extract_memory: (cose_bytes: Uint8Array) => {
    valid: boolean;
    signer: string;
    content: string;
    embedding: number[];
  };
}

let cached: Promise<MnemonicWasm> | null = null;

export function loadWasm(): Promise<MnemonicWasm> {
  if (cached) return cached;
  cached = (async () => {
    // The path is relative to this file at build time; Vite will resolve it.
    // @ts-expect-error — the wasm/ directory is generated; types resolve post-build.
    const mod: MnemonicWasm = await import("../wasm/mnemonic_core.js");
    await mod.default();
    return mod;
  })();
  return cached;
}

/**
 * Reset the cached init promise — used by tests that mock the module.
 * Production code should never call this.
 */
export function __resetWasmForTests(): void {
  cached = null;
}
