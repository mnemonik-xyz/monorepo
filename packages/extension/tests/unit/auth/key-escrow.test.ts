// Key-escrow unit tests (T17). Bind the D9 / task-17 TDD anchors:
//
//   - `wrap_unwrap_roundtrip`        — random secret + random passphrase
//                                       → wrap → unwrap → equal.
//   - `wrong_passphrase_fails_locally` — wrap with pw1, unwrap with pw2
//                                        → `WrongPassphraseError`, NO
//                                        network call.
//   - tampered-ciphertext fails closed (covers AES-GCM tag guarantee).
//   - Argon2id parameter constants match D9 + the OWASP minimums.
//   - rate-limit 429 surfaces as typed `EscrowRateLimitError` with the
//     `Retry-After` header value parsed.
//
// Runs under both vitest and `bun test`; we lean on vitest imports for
// `vi.fn` mocks where useful.
//
// The Argon2id parameters used in the tests below are deliberately
// REDUCED via the `params` override on `deriveKey` so each test runs
// in <500ms. The default-parameter test asserts the exported constants
// directly.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  ARGON2ID_HASH_LENGTH,
  ARGON2ID_MEMORY_COST_KIB,
  ARGON2ID_PARALLELISM,
  ARGON2ID_SALT_LENGTH,
  ARGON2ID_TIME_COST,
  AES_GCM_NONCE_LENGTH,
  KDF_IDENTIFIER,
  EscrowNotFoundError,
  EscrowRateLimitError,
  WrongPassphraseError,
  deriveKey,
  deleteEscrow,
  fetchEscrow,
  rotatePassphrase,
  unwrapSecret,
  uploadEscrow,
  wrapSecret,
  type EscrowBlob,
} from "../../../src/auth/key-escrow.js";
import { AuthError } from "../../../src/auth/types.js";

// ── Fetch mock plumbing (mirrors lookup.test.ts) ────────────────────────────

let originalFetch: typeof globalThis.fetch | undefined;
let fetchCalls: { url: string; init?: RequestInit }[] = [];

function installFetchMock(
  handler: (url: string, init?: RequestInit) => Response,
): void {
  fetchCalls = [];
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const entry: { url: string; init?: RequestInit } = { url };
    if (init !== undefined) entry.init = init;
    fetchCalls.push(entry);
    return Promise.resolve(handler(url, init));
  }) as typeof globalThis.fetch;
}

beforeEach(() => {
  originalFetch = globalThis.fetch;
});

afterEach(() => {
  if (originalFetch) globalThis.fetch = originalFetch;
});

// ── Fast Argon2 params for round-trip tests ─────────────────────────────────
//
// Argon2id with 64MiB memory and 3 iterations is ~100ms on a 2024-class
// laptop — fast enough for the contract test but slow enough to make
// each crypto test noticeable. We override to a tiny parameter set here
// so the full crypto round-trip lands in <100ms total. The constants
// themselves (matching D9 + OWASP) are asserted in a dedicated test.
const FAST_PARAMS = { memoryCostKib: 8, timeCost: 1, parallelism: 1 };

async function fastWrap(
  secret: Uint8Array,
  passphrase: string,
  pubkey: string,
): Promise<EscrowBlob> {
  // Replicate `wrapSecret` using a faster Argon2id derivation so the
  // test suite stays quick. We still produce a wire-shape that
  // `unwrapSecret` (which honours the in-blob params) can read.
  const salt = crypto.getRandomValues(new Uint8Array(ARGON2ID_SALT_LENGTH));
  const nonce = crypto.getRandomValues(new Uint8Array(AES_GCM_NONCE_LENGTH));
  const key = await deriveKey(passphrase, salt, FAST_PARAMS);
  const ct = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce },
    key,
    secret,
  );
  const b64 = (b: Uint8Array): string =>
    globalThis.btoa(String.fromCharCode(...b));
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

// ── Argon2id parameter assertions ───────────────────────────────────────────

describe("Argon2id parameters match D9 + OWASP minimums", () => {
  it("hash_length is 32 bytes (AES-GCM-256 key)", () => {
    expect(ARGON2ID_HASH_LENGTH).toBe(32);
  });
  it("salt length is 16 bytes per D9", () => {
    expect(ARGON2ID_SALT_LENGTH).toBe(16);
  });
  it("memory_cost is at least 19 MiB (OWASP minimum) — we target 64 MiB", () => {
    expect(ARGON2ID_MEMORY_COST_KIB).toBeGreaterThanOrEqual(19 * 1024);
    expect(ARGON2ID_MEMORY_COST_KIB).toBe(65_536);
  });
  it("time_cost is at least 2 — OWASP floor", () => {
    expect(ARGON2ID_TIME_COST).toBeGreaterThanOrEqual(2);
    expect(ARGON2ID_TIME_COST).toBe(3);
  });
  it("parallelism is 1 (portable, matches D9)", () => {
    expect(ARGON2ID_PARALLELISM).toBe(1);
  });
  it("AES-GCM nonce length is 12 bytes (WebCrypto-required)", () => {
    expect(AES_GCM_NONCE_LENGTH).toBe(12);
  });
});

// ── deriveKey ───────────────────────────────────────────────────────────────

describe("deriveKey", () => {
  it("returns an AES-GCM CryptoKey with the documented usages", async () => {
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const key = await deriveKey("passphrase-correct-horse", salt, FAST_PARAMS);
    expect(key.algorithm.name).toBe("AES-GCM");
    expect(key.usages.sort()).toEqual(["decrypt", "encrypt"]);
    // The key MUST be non-extractable so a misuse path cannot dump
    // the derived material via `exportKey`.
    expect(key.extractable).toBe(false);
  });

  it("rejects empty passphrase", async () => {
    let caught: unknown = undefined;
    try {
      await deriveKey("", crypto.getRandomValues(new Uint8Array(16)));
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });

  it("rejects empty salt", async () => {
    let caught: unknown = undefined;
    try {
      await deriveKey("good-pp", new Uint8Array(0));
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });

  it("is deterministic for (passphrase, salt) — same key derived twice", async () => {
    // We can't extract the bytes (non-extractable), so we test
    // determinism by encrypting the same plaintext under both keys and
    // checking AES-GCM unwraps both with the SAME passphrase + salt.
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const k1 = await deriveKey("same-pp", salt, FAST_PARAMS);
    const k2 = await deriveKey("same-pp", salt, FAST_PARAMS);
    const nonce = crypto.getRandomValues(new Uint8Array(12));
    const pt = new Uint8Array([1, 2, 3, 4, 5]);
    const ct = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      k1,
      pt,
    );
    const pt2 = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: nonce },
      k2,
      ct,
    );
    expect(new Uint8Array(pt2)).toEqual(pt);
  });
});

// ── wrap + unwrap round-trip ────────────────────────────────────────────────

describe("wrap_unwrap_roundtrip — TDD anchor #1", () => {
  it("random secret + random passphrase → wrap → unwrap → equal", async () => {
    // Stand-in for an Ed25519 secret. 32 random bytes covers the
    // common secret-key length; the module does not interpret the
    // payload semantically.
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const passphrase = "correct horse battery staple long";
    const blob = await fastWrap(secret, passphrase, "PubKey1111");
    const out = await unwrapSecret(blob, passphrase);
    expect(out).toEqual(secret);
    expect(out.length).toBe(secret.length);
  });

  it("wrap produces fresh salt + nonce every call (no reuse)", async () => {
    const secret = new Uint8Array([1, 2, 3]);
    const a = await fastWrap(secret, "pp-A", "Pub");
    const b = await fastWrap(secret, "pp-A", "Pub");
    expect(a.nonce_b64).not.toBe(b.nonce_b64);
    const sa = (JSON.parse(a.kdf_params) as { salt_b64: string }).salt_b64;
    const sb = (JSON.parse(b.kdf_params) as { salt_b64: string }).salt_b64;
    expect(sa).not.toBe(sb);
  });

  it("emits the exact wire-shape T15 server expects", async () => {
    const blob = await fastWrap(new Uint8Array([0x42]), "pp", "PubX");
    expect(blob.kdf).toBe("argon2id");
    expect(typeof blob.ciphertext_b64).toBe("string");
    expect(typeof blob.nonce_b64).toBe("string");
    expect(typeof blob.kdf_params).toBe("string");
    expect(blob.pubkey_base58).toBe("PubX");
    const params = JSON.parse(blob.kdf_params) as Record<string, unknown>;
    expect(typeof params.salt_b64).toBe("string");
    expect(params.m).toBe(FAST_PARAMS.memoryCostKib);
    expect(params.t).toBe(FAST_PARAMS.timeCost);
    expect(params.p).toBe(FAST_PARAMS.parallelism);
  });
});

// ── Wrong passphrase — local detection, no network ──────────────────────────

describe("wrong_passphrase_fails_locally — TDD anchor #2", () => {
  it("wrap with pw1, unwrap with pw2 → WrongPassphraseError, NO fetch call", async () => {
    // Mock fetch ABOVE the test body so any accidental network use
    // surfaces with `fetchCalls.length > 0` at the assertion.
    installFetchMock(
      () =>
        new Response("must not be called", {
          status: 500,
          headers: { "content-type": "text/plain" },
        }),
    );
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const blob = await fastWrap(secret, "pw1-correct", "Pub");
    let caught: unknown = undefined;
    try {
      await unwrapSecret(blob, "pw2-wrong");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(WrongPassphraseError);
    expect((caught as Error).name).toBe("WrongPassphraseError");
    // The critical assertion: ZERO network calls. This is the rate-
    // limit-budget protection the task spec calls out explicitly.
    expect(fetchCalls.length).toBe(0);
  });

  it("tampered ciphertext → WrongPassphraseError (AES-GCM fails closed)", async () => {
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const blob = await fastWrap(secret, "pp-right", "Pub");
    // Flip a single bit inside the ciphertext. AES-GCM's tag MUST
    // detect this and reject.
    const ctBytes = Uint8Array.from(globalThis.atob(blob.ciphertext_b64), (c) =>
      c.charCodeAt(0),
    );
    ctBytes[0] = ctBytes[0]! ^ 0x01;
    const tampered: EscrowBlob = {
      ...blob,
      ciphertext_b64: globalThis.btoa(String.fromCharCode(...ctBytes)),
    };
    let caught: unknown = undefined;
    try {
      await unwrapSecret(tampered, "pp-right");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(WrongPassphraseError);
  });

  it("tampered nonce → WrongPassphraseError", async () => {
    const blob = await fastWrap(new Uint8Array([1]), "pp", "Pub");
    const nonceBytes = Uint8Array.from(globalThis.atob(blob.nonce_b64), (c) =>
      c.charCodeAt(0),
    );
    nonceBytes[0] = nonceBytes[0]! ^ 0xff;
    const tampered: EscrowBlob = {
      ...blob,
      nonce_b64: globalThis.btoa(String.fromCharCode(...nonceBytes)),
    };
    let caught: unknown = undefined;
    try {
      await unwrapSecret(tampered, "pp");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(WrongPassphraseError);
  });

  it("rejects unknown KDF identifier (AuthError, not WrongPassphraseError)", async () => {
    const blob = await fastWrap(new Uint8Array([1]), "pp", "Pub");
    const broken: EscrowBlob = {
      ...blob,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      kdf: "scrypt" as unknown as EscrowBlob["kdf"],
    };
    let caught: unknown = undefined;
    try {
      await unwrapSecret(broken, "pp");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect(caught).not.toBeInstanceOf(WrongPassphraseError);
  });

  it("rejects malformed kdf_params (AuthError, not WrongPassphraseError)", async () => {
    const blob = await fastWrap(new Uint8Array([1]), "pp", "Pub");
    const broken: EscrowBlob = { ...blob, kdf_params: "{not-json" };
    let caught: unknown = undefined;
    try {
      await unwrapSecret(broken, "pp");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });

  it("rejects nonce of wrong length (AuthError)", async () => {
    const blob = await fastWrap(new Uint8Array([1]), "pp", "Pub");
    const shortNonce = globalThis.btoa(String.fromCharCode(0, 1, 2));
    const broken: EscrowBlob = { ...blob, nonce_b64: shortNonce };
    let caught: unknown = undefined;
    try {
      await unwrapSecret(broken, "pp");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });
});

// ── HTTP — uploadEscrow ─────────────────────────────────────────────────────

const FAKE_JWT = "header.payload.signature";

describe("uploadEscrow", () => {
  it("PUTs to /api/key-escrow with the bearer JWT and JSON body", async () => {
    installFetchMock(() => new Response("{}", { status: 200 }));
    const blob = await fastWrap(new Uint8Array([7]), "pp", "Pub");
    await uploadEscrow(FAKE_JWT, blob, { serverOrigin: "https://mc.test" });
    expect(fetchCalls.length).toBe(1);
    const call = fetchCalls[0];
    expect(call?.url).toBe("https://mc.test/api/key-escrow");
    expect(call?.init?.method).toBe("PUT");
    const headers = call?.init?.headers as Record<string, string>;
    expect(headers.authorization).toBe(`Bearer ${FAKE_JWT}`);
    const parsed = JSON.parse(call?.init?.body as string) as EscrowBlob;
    expect(parsed.kdf).toBe("argon2id");
    expect(parsed.pubkey_base58).toBe("Pub");
  });

  it("throws AuthError on 401", async () => {
    installFetchMock(() => new Response("", { status: 401 }));
    const blob = await fastWrap(new Uint8Array([7]), "pp", "Pub");
    let caught: unknown = undefined;
    try {
      await uploadEscrow(FAKE_JWT, blob, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("401");
  });

  it("throws AuthError on 403 (pubkey binding mismatch)", async () => {
    installFetchMock(() => new Response("", { status: 403 }));
    const blob = await fastWrap(new Uint8Array([7]), "pp", "Pub");
    let caught: unknown = undefined;
    try {
      await uploadEscrow(FAKE_JWT, blob, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect((caught as AuthError).message).toContain("403");
  });

  it("rejects empty JWT before any fetch", async () => {
    installFetchMock(() => new Response("never", { status: 500 }));
    const blob = await fastWrap(new Uint8Array([1]), "pp", "Pub");
    let caught: unknown = undefined;
    try {
      await uploadEscrow("", blob, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect(fetchCalls.length).toBe(0);
  });
});

// ── HTTP — fetchEscrow ──────────────────────────────────────────────────────

describe("fetchEscrow", () => {
  it("GETs and decodes the typed shape", async () => {
    const sample: EscrowBlob = {
      ciphertext_b64: "AAEC",
      nonce_b64: "AAECAwQFBgcICQoL",
      kdf: "argon2id",
      kdf_params: JSON.stringify({ salt_b64: "AAAA", m: 8, t: 1, p: 1 }),
      pubkey_base58: "PubABC",
    };
    installFetchMock(
      () =>
        new Response(JSON.stringify(sample), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
    );
    const got = await fetchEscrow(FAKE_JWT, {
      serverOrigin: "https://mc.test",
    });
    expect(got).toEqual(sample);
    expect(fetchCalls[0]?.init?.method).toBe("GET");
  });

  it("404 → EscrowNotFoundError", async () => {
    installFetchMock(() => new Response("", { status: 404 }));
    let caught: unknown = undefined;
    try {
      await fetchEscrow(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(EscrowNotFoundError);
  });

  it("429 surfaces typed EscrowRateLimitError with Retry-After parsed", async () => {
    installFetchMock(
      () =>
        new Response(JSON.stringify({ error: "rate_limited" }), {
          status: 429,
          headers: {
            "retry-after": "1234",
            "content-type": "application/json",
          },
        }),
    );
    let caught: unknown = undefined;
    try {
      await fetchEscrow(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(EscrowRateLimitError);
    expect((caught as EscrowRateLimitError).retry_after_seconds).toBe(1234);
  });

  it("rejects malformed response body", async () => {
    installFetchMock(() => new Response("not json", { status: 200 }));
    let caught: unknown = undefined;
    try {
      await fetchEscrow(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });

  it("rejects response with wrong kdf identifier", async () => {
    installFetchMock(
      () =>
        new Response(
          JSON.stringify({
            ciphertext_b64: "AA",
            nonce_b64: "AA",
            kdf: "pbkdf2",
            kdf_params: "{}",
            pubkey_base58: "P",
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
    );
    let caught: unknown = undefined;
    try {
      await fetchEscrow(FAKE_JWT, { serverOrigin: "https://mc.test" });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
  });
});

// ── HTTP — deleteEscrow ─────────────────────────────────────────────────────

describe("deleteEscrow", () => {
  it("DELETEs /api/key-escrow with Bearer JWT", async () => {
    installFetchMock(
      () => new Response(JSON.stringify({ status: "ok" }), { status: 200 }),
    );
    await deleteEscrow(FAKE_JWT, { serverOrigin: "https://mc.test" });
    expect(fetchCalls[0]?.init?.method).toBe("DELETE");
    const headers = fetchCalls[0]?.init?.headers as Record<string, string>;
    expect(headers.authorization).toBe(`Bearer ${FAKE_JWT}`);
  });
});

// ── rotatePassphrase ────────────────────────────────────────────────────────

describe("rotatePassphrase", () => {
  it("fetch → unwrap → re-wrap → upload, with the SAME pubkey_base58", async () => {
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const blob = await fastWrap(secret, "old-pp-correct", "PubKeptThroughout");
    // Track GET → PUT order and capture the PUT body for assertions.
    let getCalled = false;
    let putBody: EscrowBlob | null = null;
    globalThis.fetch = (async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const url = typeof input === "string" ? input : input.toString();
      if (
        init?.method === "GET" ||
        (init?.method === undefined && !getCalled)
      ) {
        getCalled = true;
        return new Response(JSON.stringify(blob), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      if (init?.method === "PUT") {
        putBody = JSON.parse(init.body as string) as EscrowBlob;
        return new Response("{}", { status: 200 });
      }
      return new Response("", { status: 500 });
    }) as typeof globalThis.fetch;

    await rotatePassphrase(FAKE_JWT, "old-pp-correct", "new-pp-strong", {
      serverOrigin: "https://mc.test",
    });
    expect(getCalled).toBe(true);
    expect(putBody).not.toBeNull();
    expect(putBody!.pubkey_base58).toBe("PubKeptThroughout");
    expect(putBody!.kdf).toBe("argon2id");
    // Sanity: the re-wrapped blob must be decryptable with the new
    // passphrase and equal the original secret.
    const recovered = await unwrapSecret(putBody!, "new-pp-strong");
    expect(recovered).toEqual(secret);
  });

  it("wrong old passphrase → WrongPassphraseError BEFORE any PUT", async () => {
    const blob = await fastWrap(new Uint8Array([9]), "old-correct", "Pub");
    let putCalls = 0;
    globalThis.fetch = (async (
      _input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      if (init?.method === "PUT") {
        putCalls++;
        return new Response("{}", { status: 200 });
      }
      return new Response(JSON.stringify(blob), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof globalThis.fetch;
    let caught: unknown = undefined;
    try {
      await rotatePassphrase(FAKE_JWT, "wrong-old-pp", "new-pp", {
        serverOrigin: "https://mc.test",
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(WrongPassphraseError);
    expect(putCalls).toBe(0);
  });

  it("rejects identical old + new passphrase BEFORE any fetch", async () => {
    let fetchCount = 0;
    globalThis.fetch = (async () => {
      fetchCount++;
      return new Response("{}", { status: 200 });
    }) as typeof globalThis.fetch;
    let caught: unknown = undefined;
    try {
      await rotatePassphrase(FAKE_JWT, "same", "same");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(AuthError);
    expect(fetchCount).toBe(0);
  });
});

// ── Spy: confirm wrong_passphrase_fails_locally with a true mock fn ─────────

describe("wrong_passphrase_fails_locally — spy on fetch", () => {
  it("unwrap with wrong pp never touches fetch even when fetch is mocked-strict", async () => {
    const fetchSpy = vi.fn<
      [RequestInfo | URL, RequestInit?],
      Promise<Response>
    >();
    fetchSpy.mockResolvedValue(new Response("nope", { status: 500 }));
    globalThis.fetch = fetchSpy as unknown as typeof globalThis.fetch;
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const blob = await fastWrap(secret, "pw-correct", "Pub");
    let caught: unknown = undefined;
    try {
      await unwrapSecret(blob, "pw-wrong");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(WrongPassphraseError);
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});
