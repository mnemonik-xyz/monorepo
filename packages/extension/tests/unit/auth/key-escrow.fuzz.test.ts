// Property-based fuzz tests for `wrapSecret` + `unwrapSecret`. The
// canonical unit test at `tests/unit/auth/key-escrow.test.ts` covers
// deterministic round-trips, error variants, and HTTP shape; this
// file exercises the *round-trip identity* over random secret + random
// passphrase inputs and proves the AES-GCM tag rejects single-byte
// tampering on both the ciphertext and the nonce.
//
// Argon2id at production parameters costs ~100ms per derive. We MUST
// cap the iteration budget so the suite stays under the CI runner's
// soft 60s per-file threshold. With fast Argon2id parameters (m=8 KiB
// / t=1 / p=1) each iteration runs ~10ms and 50 iterations land
// comfortably under 1s on the GitHub Actions ubuntu-latest runner.
//
// We don't depend on `fast-check` — see `tests/unit/messages.fuzz.test.ts`
// for the same rationale; the hand-rolled generator below is the
// minimum surface that the property assertions actually need.

import { describe, it, expect } from "vitest";
import {
  AES_GCM_NONCE_LENGTH,
  ARGON2ID_SALT_LENGTH,
  KDF_IDENTIFIER,
  WrongPassphraseError,
  deriveKey,
  unwrapSecret,
  type EscrowBlob,
} from "../../../src/auth/key-escrow.js";

// ── Iteration / timing budget ────────────────────────────────────────────
//
// 50 iterations of wrap+unwrap with FAST_PARAMS land at ~20ms each on a
// 2024 laptop; we double the floor to 40ms per iteration to give CI
// containers a margin. Total wall-clock budget: 50 * 40 = 2000ms.
const FUZZ_ITERATIONS = 50;

// Reduced Argon2id parameters — keep the fuzz tests fast enough for CI.
// The constants test in `key-escrow.test.ts` binds the *production*
// parameters; here we only need the wrap → unwrap identity to hold for
// arbitrary (secret, passphrase) pairs.
const FAST_PARAMS = { memoryCostKib: 8, timeCost: 1, parallelism: 1 };

// ── Tiny PRNG (mulberry32) ───────────────────────────────────────────────
// Same seedable PRNG as `messages.fuzz.test.ts`. Deterministic seeds let
// a CI failure be reproduced locally with the same input sequence.

function mulberry32(seed: number): () => number {
  let s = seed >>> 0;
  return (): number => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function b64(bytes: Uint8Array): string {
  return globalThis.btoa(String.fromCharCode(...bytes));
}

function unb64(s: string): Uint8Array {
  const bin = globalThis.atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Generate a random secret of length ∈ [1, 256]. */
function randomSecret(r: () => number): Uint8Array {
  const len = 1 + Math.floor(r() * 256);
  const out = new Uint8Array(len);
  for (let i = 0; i < len; i++) out[i] = Math.floor(r() * 256);
  return out;
}

/** Generate a random passphrase of UTF-8 length ∈ [8, 200]. We sample
 *  from a printable-ASCII window so the bytes round-trip through the
 *  WebCrypto `TextEncoder` without surrogate-pair pitfalls. The
 *  hash-wasm Argon2id accepts any UTF-8 string. */
function randomPassphrase(r: () => number): string {
  const len = 8 + Math.floor(r() * 193);
  let s = "";
  for (let i = 0; i < len; i++) {
    s += String.fromCharCode(0x21 + Math.floor(r() * (0x7e - 0x21)));
  }
  return s;
}

/** Replicate `wrapSecret` with reduced Argon2id parameters so the fuzz
 *  budget stays predictable. The wire-shape is identical so
 *  `unwrapSecret` (which honours in-blob params) reads it transparently. */
async function fastWrap(
  secret: Uint8Array,
  passphrase: string,
  pubkey: string,
  fixedSalt?: Uint8Array,
): Promise<EscrowBlob> {
  const salt =
    fixedSalt ?? crypto.getRandomValues(new Uint8Array(ARGON2ID_SALT_LENGTH));
  const nonce = crypto.getRandomValues(new Uint8Array(AES_GCM_NONCE_LENGTH));
  const key = await deriveKey(passphrase, salt, FAST_PARAMS);
  const ct = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce },
    key,
    secret,
  );
  return {
    ciphertext_b64: b64(new Uint8Array(ct)),
    nonce_b64: b64(nonce),
    kdf: KDF_IDENTIFIER,
    kdf_params: JSON.stringify({
      salt_b64: b64(salt),
      m: FAST_PARAMS.memoryCostKib,
      t: FAST_PARAMS.timeCost,
      p: FAST_PARAMS.parallelism,
    }),
    pubkey_base58: pubkey,
  };
}

// ── Property 1: wrap → unwrap is identity ────────────────────────────────

describe("key-escrow fuzz — wrap_unwrap_is_identity (TDD anchor)", () => {
  it(`${String(FUZZ_ITERATIONS)} random (secret, passphrase) pairs round-trip byte-for-byte`, async () => {
    const r = mulberry32(0xdeadbeef);
    for (let i = 0; i < FUZZ_ITERATIONS; i++) {
      const secret = randomSecret(r);
      const passphrase = randomPassphrase(r);
      const blob = await fastWrap(secret, passphrase, "FuzzPub");
      const out = await unwrapSecret(blob, passphrase);
      expect(out.length).toBe(secret.length);
      // Use `Array.from` to compare structurally — `toEqual` on
      // typed arrays is value-equal in vitest but `.from` gives a
      // cleaner diff on the rare failure.
      expect(Array.from(out)).toEqual(Array.from(secret));
    }
  }, /* timeout */ 60_000);
});

// ── Property 2: tampered ciphertext → WrongPassphraseError ───────────────

describe("key-escrow fuzz — ciphertext tampering fails closed", () => {
  it(`${String(FUZZ_ITERATIONS / 2)} random wraps survive a single-byte ciphertext flip`, async () => {
    // Half the iteration budget — each test in this file pays an
    // Argon2id derive, so we trade breadth across two tampering
    // properties.
    const r = mulberry32(0x1ce_b00b);
    const N = Math.floor(FUZZ_ITERATIONS / 2);
    for (let i = 0; i < N; i++) {
      const secret = randomSecret(r);
      const pp = randomPassphrase(r);
      const blob = await fastWrap(secret, pp, "TamperPub");
      const ct = unb64(blob.ciphertext_b64);
      // Flip one bit at a random offset. Even a 1-bit perturbation
      // MUST trip AES-GCM's authentication tag.
      const idx = Math.floor(r() * ct.length);
      ct[idx] = ct[idx]! ^ 0x01;
      const tampered: EscrowBlob = { ...blob, ciphertext_b64: b64(ct) };
      let caught: unknown = undefined;
      try {
        await unwrapSecret(tampered, pp);
      } catch (err) {
        caught = err;
      }
      expect(caught).toBeInstanceOf(WrongPassphraseError);
    }
  }, /* timeout */ 60_000);
});

// ── Property 3: tampered nonce → WrongPassphraseError ────────────────────

describe("key-escrow fuzz — nonce tampering fails closed", () => {
  it(`${String(FUZZ_ITERATIONS / 2)} random wraps survive a single-byte nonce flip`, async () => {
    const r = mulberry32(0x5e57_bee5);
    const N = Math.floor(FUZZ_ITERATIONS / 2);
    for (let i = 0; i < N; i++) {
      const secret = randomSecret(r);
      const pp = randomPassphrase(r);
      const blob = await fastWrap(secret, pp, "NoncePub");
      const nonce = unb64(blob.nonce_b64);
      const idx = Math.floor(r() * nonce.length);
      nonce[idx] = nonce[idx]! ^ 0xff;
      const tampered: EscrowBlob = { ...blob, nonce_b64: b64(nonce) };
      let caught: unknown = undefined;
      try {
        await unwrapSecret(tampered, pp);
      } catch (err) {
        caught = err;
      }
      expect(caught).toBeInstanceOf(WrongPassphraseError);
    }
  }, /* timeout */ 60_000);
});

// ── Property 4: round-trip identity over a fixed salt ──────────────────
//
// Round-1 review (test-reviewer-round1.json finding 3): the canonical
// KDF-determinism unit test lives at `key-escrow.test.ts::deriveKey >
// is deterministic for (passphrase, salt)`. THIS property's value is
// the "rotate" invariant — when the same (passphrase, salt) is reused
// across two wraps, both round-trip to the original secret. AES-GCM tag
// verification implicitly proves the KDF returned the same key both
// times; a non-deterministic KDF would surface here as a
// `WrongPassphraseError` on the second unwrap.

describe("key-escrow fuzz — round-trip over fixed salt (rotate invariant)", () => {
  it("10 random (secret, passphrase) pairs wrap twice under fixed salt → both unwrap byte-for-byte", async () => {
    const r = mulberry32(0xabad_d00d);
    for (let i = 0; i < 10; i++) {
      const secret = randomSecret(r);
      const pp = randomPassphrase(r);
      const fixedSalt = crypto.getRandomValues(
        new Uint8Array(ARGON2ID_SALT_LENGTH),
      );
      const blobA = await fastWrap(secret, pp, "DetPub", fixedSalt);
      const blobB = await fastWrap(secret, pp, "DetPub", fixedSalt);
      // The kdf_params salt_b64 MUST match — fixed salt in, same out.
      // This is the visible-on-the-wire half of the determinism story;
      // the unwrap below is the cryptographic half.
      const sa = (JSON.parse(blobA.kdf_params) as { salt_b64: string })
        .salt_b64;
      const sb = (JSON.parse(blobB.kdf_params) as { salt_b64: string })
        .salt_b64;
      expect(sa).toBe(sb);
      // Both blobs unwrap to the original secret. A non-deterministic
      // KDF would derive a different key on the second wrap and the
      // unwrap below would throw `WrongPassphraseError`.
      const outA = await unwrapSecret(blobA, pp);
      const outB = await unwrapSecret(blobB, pp);
      expect(Array.from(outA)).toEqual(Array.from(secret));
      expect(Array.from(outB)).toEqual(Array.from(secret));
    }
  }, /* timeout */ 60_000);
});
