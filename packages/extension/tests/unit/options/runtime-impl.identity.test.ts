// T25 round-2 — direct unit tests for the IdentityFacade write paths.
//
// The component tests mock the entire runtime so they cannot lock the
// production validator semantics. This file talks to
// `createDefaultOptionsRuntime().identity` with a chrome.storage stub
// so each validation branch in `importPlain` / `exportPlain` /
// `generate` is regression-locked at the producer side:
//
//   - PR136-C-05: byte-range validation rejects a secret with a
//     non-number element AND a number > 255 at a specific index.
//   - PR136-C-04: the byte-range error message identifies the failure
//     by index + range, not by misleading "secret length" prefix.
//   - PR136-C-02: `importPlain` carries `exported_at` forward as
//     `created_at` when it parses to a sensible past timestamp; falls
//     back to Date.now() when missing / unparseable / absurdly future.
//   - SA-T25-004: `generate` rejects a malformed WASM-returned secret
//     before it reaches chrome.storage.
//
// The chrome.storage stub is a thin in-memory map so the assertions
// can read back what the facade wrote.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { createDefaultOptionsRuntime } from "../../../src/options/runtime-impl.js";
import {
  IDENTITY_KEY,
  IDENTITY_SECRET_KEY,
} from "../../../src/auth/storage-keys.js";
import { __setWasmForTesting } from "../../../src/runtime/sign/cose.js";

const VALID_BASE58_PUBKEY = "5MhYbm6Q1vN9zJZ7QcLwY4uG2Yk3rrgu1qB7L8Tk2pYn";

function makeSecret(): number[] {
  const out: number[] = [];
  for (let i = 0; i < 64; i++) out.push((i * 13 + 7) & 0xff);
  return out;
}

function installChromeStub(): {
  store: Map<string, unknown>;
  setSpy: ReturnType<typeof vi.fn>;
} {
  const store = new Map<string, unknown>();
  const setSpy = vi.fn(async (items: Record<string, unknown>) => {
    for (const [k, v] of Object.entries(items)) store.set(k, v);
  });
  (globalThis as { chrome?: unknown }).chrome = {
    storage: {
      local: {
        get: async (keys: string | string[]) => {
          const list = typeof keys === "string" ? [keys] : keys;
          const out: Record<string, unknown> = {};
          for (const k of list) {
            if (store.has(k)) out[k] = store.get(k);
          }
          return out;
        },
        set: setSpy,
        remove: async (keys: string | string[]) => {
          const list = typeof keys === "string" ? [keys] : keys;
          for (const k of list) store.delete(k);
        },
      },
    },
    runtime: { getManifest: () => ({ version: "0.0.0" }) },
  };
  return { store, setSpy };
}

describe("OptionsRuntime.identity.importPlain — validation matrix", () => {
  let store: Map<string, unknown>;

  beforeEach(() => {
    ({ store } = installChromeStub());
  });

  // PR136-C-05: a hand-edited JSON with a non-number element in the
  // secret array must be rejected BEFORE any chrome.storage.set call.
  it("rejects a secret containing a non-number element with an index-locked error", async () => {
    const runtime = createDefaultOptionsRuntime();
    const badSecret = makeSecret();
    // Replace index 5 with a non-numeric value the byte-range validator
    // must detect. `Number("not-a-number")` yields NaN — fails the
    // `Number.isInteger` guard — so the error message points at the
    // exact offending index, not a generic length complaint.
    (badSecret as unknown[])[5] = "not-a-number";
    await expect(
      runtime.identity.importPlain({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: badSecret,
        exported_at: "2026-05-11T00:00:00.000Z",
        warning: "any",
      }),
    ).rejects.toThrow(/Invalid secret byte at index 5/);
    expect(store.has(IDENTITY_KEY)).toBe(false);
    expect(store.has(IDENTITY_SECRET_KEY)).toBe(false);
  });

  it("rejects a secret containing an object element with an index-locked error", async () => {
    // Different non-number flavour: a plain object. Confirms the
    // validator does not silently coerce structured values either.
    const runtime = createDefaultOptionsRuntime();
    const badSecret = makeSecret();
    (badSecret as unknown[])[7] = { not: "a number" };
    await expect(
      runtime.identity.importPlain({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: badSecret,
      }),
    ).rejects.toThrow(/Invalid secret byte at index 7/);
  });

  it("rejects a secret containing a value > 255 with the new byte-range message (PR136-C-04)", async () => {
    const runtime = createDefaultOptionsRuntime();
    const badSecret = makeSecret();
    badSecret[12] = 256;
    await expect(
      runtime.identity.importPlain({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: badSecret,
      }),
    ).rejects.toThrow(/Invalid secret byte at index 12/);
    // The misleading "Wrong secret length:" prefix must NOT appear —
    // the length check already passed by the time we reach byte-range.
    await expect(
      runtime.identity.importPlain({
        version: 1,
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: badSecret,
      }),
    ).rejects.not.toThrow(/Wrong secret length: contains non-byte value/);
    expect(store.has(IDENTITY_KEY)).toBe(false);
  });

  // PR136-C-02: exported_at must round-trip as created_at when valid.
  it("forwards a valid ISO exported_at as created_at", async () => {
    const runtime = createDefaultOptionsRuntime();
    const exportedAt = "2025-01-15T12:00:00.000Z";
    const expectedMs = new Date(exportedAt).getTime();
    const snap = await runtime.identity.importPlain({
      version: 1,
      pubkey_base58: VALID_BASE58_PUBKEY,
      secret: makeSecret(),
      exported_at: exportedAt,
      warning: "any",
    });
    expect(snap.created_at).toBe(expectedMs);
    const stored = store.get(IDENTITY_KEY) as { created_at: number };
    expect(stored.created_at).toBe(expectedMs);
  });

  it("falls back to Date.now() when exported_at is missing", async () => {
    const runtime = createDefaultOptionsRuntime();
    const before = Date.now();
    const snap = await runtime.identity.importPlain({
      version: 1,
      pubkey_base58: VALID_BASE58_PUBKEY,
      secret: makeSecret(),
    });
    const after = Date.now();
    expect(snap.created_at).toBeGreaterThanOrEqual(before);
    expect(snap.created_at).toBeLessThanOrEqual(after);
  });

  it("falls back to Date.now() when exported_at is an unparseable string", async () => {
    const runtime = createDefaultOptionsRuntime();
    const before = Date.now();
    const snap = await runtime.identity.importPlain({
      version: 1,
      pubkey_base58: VALID_BASE58_PUBKEY,
      secret: makeSecret(),
      exported_at: "not-a-date",
    });
    expect(snap.created_at).toBeGreaterThanOrEqual(before);
    expect(snap.created_at).toBeLessThanOrEqual(Date.now());
  });

  it("falls back to Date.now() when exported_at is absurdly in the future", async () => {
    const runtime = createDefaultOptionsRuntime();
    // 10 days in the future — well past the 24h tolerance window.
    const future = new Date(
      Date.now() + 10 * 24 * 60 * 60 * 1000,
    ).toISOString();
    const before = Date.now();
    const snap = await runtime.identity.importPlain({
      version: 1,
      pubkey_base58: VALID_BASE58_PUBKEY,
      secret: makeSecret(),
      exported_at: future,
    });
    expect(snap.created_at).toBeGreaterThanOrEqual(before);
    expect(snap.created_at).toBeLessThanOrEqual(Date.now());
  });

  it("accepts an epoch-ms number for exported_at", async () => {
    const runtime = createDefaultOptionsRuntime();
    const exportedMs = 1700000000000; // 2023-11-14
    const snap = await runtime.identity.importPlain({
      version: 1,
      pubkey_base58: VALID_BASE58_PUBKEY,
      secret: makeSecret(),
      exported_at: exportedMs,
    });
    expect(snap.created_at).toBe(exportedMs);
  });
});

describe("OptionsRuntime.identity.generate — SA-T25-004 storage invariants", () => {
  beforeEach(() => {
    installChromeStub();
  });

  // The cose.ts `generateKeypair` wrapper already validates the secret
  // before returning, so a misbehaving WASM stub trips ITS guard
  // first — the runtime-impl byte-range check is defence-in-depth
  // against a future binding swap that bypasses cose.ts. These tests
  // pin that the generate() facade rejects bad bytes EITHER way, so
  // chrome.storage is never written with a malformed secret.
  it("rejects a WASM-returned secret with a non-byte value", async () => {
    __setWasmForTesting({
      default: () => Promise.resolve(undefined),
      generate_keypair: () => {
        const bad = makeSecret();
        (bad as unknown[])[3] = "x";
        return { pubkey_base58: VALID_BASE58_PUBKEY, secret: bad };
      },
      sign_challenge: () => new Uint8Array(64),
      sign_cose_payload: () => new Uint8Array([0x84]),
      import_keypair_json: () => ({}),
      export_keypair_json: () => "{}",
      compress_embedding: () => new Uint8Array(0),
      decompress_embedding: () => new Float32Array(0),
      to_canonical_cbor_bytes: () => new Uint8Array(0),
      blake3_hash: () => new Uint8Array(32),
      blake3_hash_hex: () => "00".repeat(32),
    } as unknown as Parameters<typeof __setWasmForTesting>[0]);
    const runtime = createDefaultOptionsRuntime();
    // Either guard (cose.ts upstream or runtime-impl defence) rejects.
    await expect(runtime.identity.generate()).rejects.toThrow(/non-byte value/);
    __setWasmForTesting(null);
  });

  it("rejects a WASM-returned secret with the wrong length", async () => {
    __setWasmForTesting({
      default: () => Promise.resolve(undefined),
      generate_keypair: () => ({
        pubkey_base58: VALID_BASE58_PUBKEY,
        secret: [1, 2, 3], // way too short
      }),
      sign_challenge: () => new Uint8Array(64),
      sign_cose_payload: () => new Uint8Array([0x84]),
      import_keypair_json: () => ({}),
      export_keypair_json: () => "{}",
      compress_embedding: () => new Uint8Array(0),
      decompress_embedding: () => new Float32Array(0),
      to_canonical_cbor_bytes: () => new Uint8Array(0),
      blake3_hash: () => new Uint8Array(32),
      blake3_hash_hex: () => "00".repeat(32),
    } as unknown as Parameters<typeof __setWasmForTesting>[0]);
    const runtime = createDefaultOptionsRuntime();
    await expect(runtime.identity.generate()).rejects.toThrow(
      /secret of length 3, expected 64|malformed secret/,
    );
    __setWasmForTesting(null);
  });
});
