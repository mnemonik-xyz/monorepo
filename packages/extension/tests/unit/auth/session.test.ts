// Session-store tests — exercise the get / set / clear round-trip
// against an in-memory storage shim. Also covers the JWT-expiry helper
// (`jwtExpiresAtMs`) and the auto-expiry semantics of `getSession`.

// Uses vitest imports — works under vitest natively and under `bun test`
// via bun's vitest-compat shim (Decision 11 of the repo CI policy).
import { describe, it, expect, beforeEach } from "vitest";
import {
  getSession,
  setSession,
  clearSession,
  jwtExpiresAtMs,
  type StorageArea,
} from "../../../src/auth/session.js";
import { SESSION_STORAGE_KEY, type Session } from "../../../src/auth/types.js";

// ── In-memory chrome.storage.local shim ─────────────────────────────────────

function makeFakeStorage(): StorageArea & {
  inspect: () => Record<string, unknown>;
} {
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
    async remove(keys) {
      const list = typeof keys === "string" ? [keys] : keys;
      for (const k of list) delete backing[k];
    },
    inspect: () => backing,
  };
}

const VALID_SESSION: Session = {
  jwt: "header.payload.signature",
  googleSub: "google-sub-12345",
  jwtExpiresAt: Date.now() + 60 * 60 * 1000,
  signedInAt: Date.now(),
  profile: {
    sub: "google-sub-12345",
    email: "alice@example.com",
    email_verified: true,
    name: "Alice",
    picture: "https://example.com/p.png",
  },
};

let storage: ReturnType<typeof makeFakeStorage>;

beforeEach(() => {
  storage = makeFakeStorage();
});

describe("session — round-trip", () => {
  it("getSession returns null when nothing has been written", async () => {
    const result = await getSession(storage);
    expect(result).toBeNull();
  });

  it("setSession writes the blob under session.v1, getSession reads it back", async () => {
    await setSession(VALID_SESSION, storage);
    const stored = storage.inspect();
    expect(stored[SESSION_STORAGE_KEY]).toBeDefined();
    const result = await getSession(storage);
    expect(result).not.toBeNull();
    expect(result?.jwt).toBe(VALID_SESSION.jwt);
    expect(result?.googleSub).toBe(VALID_SESSION.googleSub);
    expect(result?.profile.email).toBe("alice@example.com");
  });

  it("clearSession removes the persisted blob", async () => {
    await setSession(VALID_SESSION, storage);
    await clearSession(storage);
    const stored = storage.inspect();
    expect(stored[SESSION_STORAGE_KEY]).toBeUndefined();
    const result = await getSession(storage);
    expect(result).toBeNull();
  });

  it("clearSession is idempotent on an empty store", async () => {
    // Should not throw — chrome.storage.local.remove is itself idempotent.
    await clearSession(storage);
    expect(storage.inspect()).toEqual({});
  });
});

describe("session — auto-expiry on read", () => {
  it("getSession returns null for an expired JWT", async () => {
    const expired: Session = {
      ...VALID_SESSION,
      jwtExpiresAt: Date.now() - 1, // already in the past
    };
    await setSession(expired, storage);
    // Real-clock now() will see jwtExpiresAt < Date.now() → null.
    const result = await getSession(storage);
    expect(result).toBeNull();
  });

  it("getSession respects an injected `now()` source", async () => {
    const future = Date.now() + 60 * 60 * 1000;
    await setSession({ ...VALID_SESSION, jwtExpiresAt: future }, storage);
    // Inject a clock that's already past the expiry.
    const result = await getSession(storage, () => future + 1);
    expect(result).toBeNull();
  });

  // Audit S7 / FW-S-09: a `getSession` call that finds an expired row
  // must REMOVE it from chrome.storage.local before returning null —
  // narrows the window in which a stale JWT can leak via a profile
  // exfiltration.
  it("getSession clears the expired row from storage on read", async () => {
    const expired: Session = {
      ...VALID_SESSION,
      jwtExpiresAt: Date.now() - 1,
    };
    await setSession(expired, storage);
    expect(storage.inspect()[SESSION_STORAGE_KEY]).toBeDefined();

    const result = await getSession(storage);

    expect(result).toBeNull();
    // The expired row MUST be gone after the read.
    expect(storage.inspect()[SESSION_STORAGE_KEY]).toBeUndefined();
  });
});

describe("session — shape validation", () => {
  it("getSession returns null when stored blob has missing fields (forward-compat)", async () => {
    await storage.set({
      [SESSION_STORAGE_KEY]: { jwt: "x", googleSub: "y" }, // missing the rest
    });
    const result = await getSession(storage);
    expect(result).toBeNull();
  });

  it("setSession refuses to persist a malformed shape", async () => {
    let caught: unknown = undefined;
    try {
      // Deliberately drop required fields. Cast through unknown to keep
      // production-code constraints (no `as any`).
      await setSession({ jwt: "x" } as unknown as Session, storage);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(Error);
    // Nothing should have been written.
    expect(storage.inspect()[SESSION_STORAGE_KEY]).toBeUndefined();
  });
});

describe("jwtExpiresAtMs", () => {
  function b64url(obj: unknown): string {
    const json = JSON.stringify(obj);
    return btoa(json)
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
  }

  it("returns ms from the exp claim when present", () => {
    const expSec = Math.floor(Date.now() / 1000) + 3600;
    const jwt = [
      b64url({ alg: "HS256", typ: "JWT" }),
      b64url({ sub: "x", exp: expSec, iat: expSec - 3600 }),
      "sig",
    ].join(".");
    const result = jwtExpiresAtMs(jwt);
    expect(result).toBe(expSec * 1000);
  });

  it("returns null for a malformed JWT", () => {
    expect(jwtExpiresAtMs("not.a.real.token")).toBeNull();
    expect(jwtExpiresAtMs("only-one-segment")).toBeNull();
    expect(jwtExpiresAtMs("")).toBeNull();
  });

  it("returns null when the exp claim is missing or non-numeric", () => {
    const jwt = [
      b64url({ alg: "HS256", typ: "JWT" }),
      b64url({ sub: "x" }), // no exp
      "sig",
    ].join(".");
    expect(jwtExpiresAtMs(jwt)).toBeNull();
  });
});
