// T25b unit tests for `ensureLocalIdentity`. Two anchored behaviours:
//
//   1. Null storage → generates a fresh Ed25519 keypair, persists under
//      the canonical `identity` + `identity_secret` keys, returns the
//      same shape `runtime-impl.ts::loadIdentity` consumes.
//
//   2. Existing valid identity → returns the stored value unchanged
//      (idempotent — no re-mint on subsequent popup mounts).
//
// Uses an in-memory chrome.storage.local shim — identical pattern to
// `tests/unit/auth/session.test.ts` so the test surface is consistent
// across the auth helpers.

import { describe, it, expect, beforeEach } from "vitest";
import {
  base58Encode,
  ensureLocalIdentity,
  __resetEnsureLocalIdentityCache,
} from "../../../src/auth/local-identity.js";

// ── Fake chrome.storage.local shim ──────────────────────────────────────────

interface FakeStorage {
  get: (
    keys: string | string[] | Record<string, unknown> | null,
  ) => Promise<Record<string, unknown>>;
  set: (items: Record<string, unknown>) => Promise<void>;
  inspect: () => Record<string, unknown>;
  clear: () => Promise<void>;
}

function makeFakeStorage(): FakeStorage {
  let backing: Record<string, unknown> = {};
  return {
    async get(keys) {
      const out: Record<string, unknown> = {};
      const list =
        typeof keys === "string"
          ? [keys]
          : Array.isArray(keys)
            ? keys
            : keys
              ? Object.keys(keys)
              : Object.keys(backing);
      for (const k of list) {
        if (k in backing) out[k] = backing[k];
      }
      return out;
    },
    async set(items) {
      backing = { ...backing, ...items };
    },
    inspect: () => ({ ...backing }),
    async clear() {
      backing = {};
    },
  };
}

function installChromeStub(storage: FakeStorage): void {
  (globalThis as { chrome?: unknown }).chrome = {
    storage: {
      local: storage,
    },
  };
}

let storage: FakeStorage;

beforeEach(() => {
  storage = makeFakeStorage();
  installChromeStub(storage);
  // Clear the module-level in-flight cache so each test gets a fresh
  // mint round (otherwise the singleton would leak between tests).
  __resetEnsureLocalIdentityCache();
});

// ── base58Encode sanity ─────────────────────────────────────────────────────

describe("base58Encode", () => {
  it("handles empty input", () => {
    expect(base58Encode(new Uint8Array(0))).toBe("");
  });

  it("encodes leading zero bytes as 1s", () => {
    // Two leading zero bytes → "11", then 0x01 = 1 = '2' in the
    // Bitcoin alphabet.
    expect(base58Encode(new Uint8Array([0, 0, 1]))).toBe("112");
  });

  it("encodes known fixture (Bitcoin alphabet)", () => {
    // 0x00010203 → '1Ldp' — one leading zero byte encodes as "1",
    // then 0x010203 = 66051 decodes to base58 digits 19,11,15 i.e.
    // 'L','d','p' in the alphabet (reversed for output).
    expect(base58Encode(new Uint8Array([0x00, 0x01, 0x02, 0x03]))).toBe("1Ldp");
  });
});

// ── ensureLocalIdentity ─────────────────────────────────────────────────────

describe("ensureLocalIdentity — null storage", () => {
  it("generates a fresh keypair, persists under canonical keys, returns matching shape", async () => {
    const id = await ensureLocalIdentity();
    expect(typeof id.pubkey_base58).toBe("string");
    expect(id.pubkey_base58.length).toBeGreaterThan(0);
    expect(Array.isArray(id.secret)).toBe(true);
    expect(id.secret).toHaveLength(64);
    // All bytes are integers in 0..255.
    for (const b of id.secret) {
      expect(Number.isInteger(b)).toBe(true);
      expect(b).toBeGreaterThanOrEqual(0);
      expect(b).toBeLessThanOrEqual(255);
    }
    // Persisted under canonical keys.
    const stored = storage.inspect();
    expect(stored.identity).toEqual({ pubkey_base58: id.pubkey_base58 });
    expect(stored.identity_secret).toEqual(id.secret);
  });

  it("subsequent calls return the same persisted identity (idempotent)", async () => {
    const first = await ensureLocalIdentity();
    const second = await ensureLocalIdentity();
    expect(second.pubkey_base58).toBe(first.pubkey_base58);
    expect(second.secret).toEqual(first.secret);
  });
});

describe("ensureLocalIdentity — existing valid identity", () => {
  it("returns the stored value verbatim; does not re-mint", async () => {
    // Seed canonical keys with a known-good 64-byte secret + matching
    // pubkey string. We don't care that the pubkey actually derives
    // from this secret for the test — `ensureLocalIdentity` only
    // validates shape, not derivation. Mirrors the contract the
    // runtime-impl loader applies before signing.
    const secret = Array.from({ length: 64 }, (_, i) => (i + 1) % 256);
    await storage.set({
      identity: { pubkey_base58: "FakePubkey111111111111" },
      identity_secret: secret,
    });

    const id = await ensureLocalIdentity();
    expect(id.pubkey_base58).toBe("FakePubkey111111111111");
    expect(id.secret).toEqual(secret);

    // No re-write — storage retained the seeded values.
    const stored = storage.inspect();
    expect(stored.identity).toEqual({
      pubkey_base58: "FakePubkey111111111111",
    });
    expect(stored.identity_secret).toEqual(secret);
  });

  it("regenerates when stored secret has wrong length", async () => {
    await storage.set({
      identity: { pubkey_base58: "FakePubkey" },
      identity_secret: Array.from({ length: 32 }, () => 0),
    });
    const id = await ensureLocalIdentity();
    expect(id.secret).toHaveLength(64);
    // pubkey was overwritten with a fresh one — the bogus storage was
    // not trustworthy.
    expect(id.pubkey_base58).not.toBe("FakePubkey");
  });

  it("regenerates when identity is missing but identity_secret present", async () => {
    await storage.set({
      identity_secret: Array.from({ length: 64 }, () => 5),
    });
    const id = await ensureLocalIdentity();
    expect(id.pubkey_base58.length).toBeGreaterThan(0);
    expect(id.secret).toHaveLength(64);
    // Was overwritten with a fresh generated keypair.
    const stored = storage.inspect();
    expect((stored.identity as { pubkey_base58: string }).pubkey_base58).toBe(
      id.pubkey_base58,
    );
    expect(stored.identity_secret).toEqual(id.secret);
  });
});

// ── concurrent-call dedupe (PR135-C-01) ─────────────────────────────────────

describe("ensureLocalIdentity — concurrent callers", () => {
  it("mints exactly one keypair when called twice in parallel from a null storage", async () => {
    // StrictMode double-render + parallel bootstrap effects would
    // call this concurrently. The in-flight cache must coalesce both
    // into one mint round so the second `set` doesn't overwrite the
    // first with a different keypair.
    const [a, b] = await Promise.all([
      ensureLocalIdentity(),
      ensureLocalIdentity(),
    ]);
    expect(a.pubkey_base58).toBe(b.pubkey_base58);
    expect(a.secret).toEqual(b.secret);
    // Storage holds exactly the resolved identity, not a clobbered
    // second-mint value.
    const stored = storage.inspect();
    expect(stored.identity).toEqual({ pubkey_base58: a.pubkey_base58 });
    expect(stored.identity_secret).toEqual(a.secret);
  });
});
