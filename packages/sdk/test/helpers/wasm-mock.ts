// Test-only WASM mock that emulates the relevant exports of `mnemonic-core`
// using `@noble/ed25519` for cryptography. Installed via `__setWasmForTesting`.
//
// Why a mock instead of the real WASM:
//   - The `--target web` build's `init()` calls `fetch(file://...)`, which
//     Node 20 rejects (Bun and Deno accept it, but vitest runs under Node).
//   - Loading the actual WASM in the test process is also slow (~200ms per
//     init). A pure-JS mock keeps tests <1s total.
//
// What we DO NOT mock:
//   - Canonical CBOR encoding (the SDK never builds canonical CBOR — it only
//     wraps server-built bytes). For COSE we return a recognizable envelope
//     starting with 0x84 so client.ts's defense-in-depth check accepts it.
//   - The exact COSE_Sign1 byte layout — tests assert that `coseSignPayload`
//     was *called* with the right inputs, not that the bytes match WASM
//     output (that's covered by Task 4's golden fixture).

import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha2.js";
import bs58 from "./bs58.js";

// Noble v2 requires a sync sha512 for the sync sign() path; provide it.
ed.etc.sha512Sync = (...m: Uint8Array[]) => sha512(ed.etc.concatBytes(...m));

interface KeypairJson {
  secret: number[];
  pubkey_base58: string;
}

function isKeypairJsonShape(v: unknown): v is KeypairJson {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return Array.isArray(o.secret) && typeof o.pubkey_base58 === "string";
}

/** Re-derive the 32-byte Ed25519 seed from a 64-byte secret (Solana layout). */
function seedFromSecret(secret: number[]): Uint8Array {
  // Solana keypair = [seed(32) || pubkey(32)]. The first 32 bytes ARE the
  // Ed25519 private scalar seed. Noble takes the seed directly.
  return new Uint8Array(secret.slice(0, 32));
}

/**
 * Build the mock. Each call returns a fresh module so tests can stage
 * different responses for different fixtures.
 */
export function buildWasmMock(): {
  generate_keypair: () => KeypairJson;
  sign_challenge: (kp: KeypairJson | unknown, bytes: Uint8Array) => Uint8Array;
  sign_cose_payload: (
    payload: Uint8Array,
    kp: KeypairJson | unknown
  ) => Uint8Array;
  import_keypair_json: (s: string) => KeypairJson;
  export_keypair_json: (kp: KeypairJson | unknown) => string;
  // Spies — tests inspect these.
  __calls: {
    sign_challenge: Array<{ keypair: KeypairJson; bytes: Uint8Array }>;
    sign_cose_payload: Array<{ payload: Uint8Array; keypair: KeypairJson }>;
  };
} {
  const __calls = {
    sign_challenge: [] as Array<{ keypair: KeypairJson; bytes: Uint8Array }>,
    sign_cose_payload: [] as Array<{
      payload: Uint8Array;
      keypair: KeypairJson;
    }>,
  };

  function validate(kp: unknown): KeypairJson {
    if (!isKeypairJsonShape(kp)) {
      throw new Error(
        "invalid keypair JSON: not an object with {secret, pubkey_base58}"
      );
    }
    if (kp.secret.length !== 64) {
      throw new Error(
        `invalid keypair: secret must be 64 bytes, got ${kp.secret.length}`
      );
    }
    return kp;
  }

  return {
    generate_keypair(): KeypairJson {
      const seed = ed.utils.randomPrivateKey();
      const pub = ed.getPublicKey(seed);
      const secret = new Uint8Array(64);
      secret.set(seed, 0);
      secret.set(pub, 32);
      return {
        secret: Array.from(secret),
        pubkey_base58: bs58.encode(pub),
      };
    },
    sign_challenge(kp, bytes): Uint8Array {
      const valid = validate(kp);
      __calls.sign_challenge.push({ keypair: valid, bytes });
      const seed = seedFromSecret(valid.secret);
      // Noble v2 sync sign — returns 64 bytes.
      return ed.sign(bytes, seed);
    },
    sign_cose_payload(payload, kp): Uint8Array {
      const valid = validate(kp);
      __calls.sign_cose_payload.push({ payload, keypair: valid });
      // Build a recognizable COSE_Sign1-ish envelope. Real COSE_Sign1 is a
      // tagged CBOR 4-element array; we just need the first byte 0x84 (CBOR
      // array(4)) to satisfy the defense-in-depth check in cose.ts. The body
      // bytes don't matter for SDK unit tests — golden-fixture tests in T4
      // verify byte-equality against the real WASM.
      const seed = seedFromSecret(valid.secret);
      const sig = ed.sign(payload, seed);
      const out = new Uint8Array(payload.length + sig.length + 1);
      out[0] = 0x84;
      out.set(payload, 1);
      out.set(sig, 1 + payload.length);
      return out;
    },
    import_keypair_json(s: string): KeypairJson {
      let parsed: unknown;
      try {
        parsed = JSON.parse(s);
      } catch (e) {
        throw new Error(`malformed keypair JSON: ${(e as Error).message}`);
      }
      return validate(parsed);
    },
    export_keypair_json(kp: KeypairJson | unknown): string {
      return JSON.stringify(validate(kp));
    },
    __calls,
  };
}
